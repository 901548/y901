//! D3D11 后端：`y901_ui::RenderCtx` + `TextMetrics` 的唯一实现。
//!
//! 这一层是框架与 GPU 之间唯一的桥：
//! - `fill_rect` / `draw_text` → 追加到实例数组（同一帧只有一次 DrawInstanced）
//! - DirectWrite 字形 → 图集纹理，增量上传
//!
//! 框架和组件都看不到 D3D11 类型，只看到 `RenderCtx`。

// 诊断 API：`pub mod d3d11`：内部模块对外可见仅为测试访问 `Gfx` 诊断接口
// 归类：内部模块 —— d3d11 仅供 backend 内部使用，不对外导出
#[allow(dead_code)] // Atlas/Placement 是给多页图集预留的 API，暂由 SkylinePacker 先行使用
pub mod d3d11;
pub mod font;

use windows_sys::Win32::Foundation::HWND;

use y901_runtime::render_stats::{BackendKind, RenderStats, StageId, StageTimes};
use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle};
// 字形顶部公式只有一份实现（软/硬后端共用，防止两边漂移）
use y901_render::font::glyph_top;

use d3d11::{Gfx, Instance};
use font::TextEngine;

/// 单元格尺寸不再有常量：它由字号推出（`cell_metrics_for`），
/// 存在 `D3dBackend::cell` 里，随 `set_font` 更新。
/// 16px 时是 8×16（与历史行为一致，由 `cell_metrics_for` 的测试固化）。
/// 默认字体尺寸（逻辑像素）。真实值来自 `Options.font`，这里只作为文档与默认值来源。
///
/// **字号是全局的**：文本引擎只用一个字号，不支持同一文档内不同字号（富文本）。
/// 若将来需要富文本，再给 `TextStyle` 加 `font_size` 字段并让 `measure_text` 读它。
const FONT_SIZE: f32 = 16.0;
/// 图集未同步的警告只打一次（避免每帧刷屏，同时 release 也有提示）。
static ATLAS_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 默认字体族（与 `FontConfig::default()` 保持一致）。
const FONT_FAMILY: &str = "NSimSun";

/// 默认字体配置的唯一来源：常量与 `FontConfig::default()` 必须一致。
///
/// 用 `debug_assert` 固化，避免两处各改一半。
fn default_font_config() -> y901_runtime::FontConfig {
    let cfg = y901_runtime::FontConfig::default();
    debug_assert_eq!(cfg.family, FONT_FAMILY, "默认族名不一致");
    debug_assert_eq!(cfg.size, FONT_SIZE, "默认字号不一致");
    cfg
}

// ---------------------------------------------------------------------------
// 分阶段统计的累计器（后端私有）
// ---------------------------------------------------------------------------
//
// 为什么需要「累计器」而不是直接 `StageTimes::set`：设备丢失时 `present` 会
// 重建设备并**再跑一次** `render_batched`（`present` 里那两处调用），于是本帧的
// submit / present 各有两个**不连续**区间。`StageTimes::set` 是覆盖写，
// 第二次会吃掉第一次 —— 必须先在本地累加，帧末一次写入。
//
// **`Option` 的三态语义（不得用 `> ZERO` 推断）**：
// - `None`          —— 本帧**从未执行**该阶段（保持未测量，`stage()` 给 Unavailable）；
// - `Some(ZERO)`    —— 执行了，但计时粒度内为 0（**合法测量值**）；
// - `Some(d)`, d>0  —— 执行了，耗时 d。

/// 单阶段累计器。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Acc(Option<std::time::Duration>);

impl Acc {
    /// 累加一段区间。`d` 允许为 0（时钟粒度），也允许被多次累加（重试路径）。
    #[inline]
    pub(crate) fn add(&mut self, d: std::time::Duration) {
        self.0 = Some(self.0.map_or(d, |s| s + d));
    }

    /// 是否执行过（用于断言/统计，不用于决定"要不要写 0"）。
    #[inline]
    pub(crate) fn seen(self) -> bool {
        self.0.is_some()
    }

    /// 帧末写入：**执行过就写（即使为 0）**；没执行过保持 `None`。
    #[inline]
    pub(crate) fn flush(self, times: &mut StageTimes, id: StageId) {
        if let Some(sum) = self.0 {
            times.set(id, sum);
        }
    }
}

/// 一帧的后端累计器（`BackendAcc::default()` 即"本帧什么都没执行"）。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BackendAcc {
    /// 图集上传（`sync_atlas` + 设备重建后的全量重传，可能 2 段）。
    pub(crate) upload: Acc,
    /// CPU 提交（`build_batches` + `render_batched` 到 present 之前；重试时 2 段）。
    pub(crate) submit: Acc,
    /// `Present` 的 CPU 同步阻塞（重试时 2 段）。
    pub(crate) present: Acc,
    /// 本帧实际提交的实例数（受 `INSTANCE_CAPACITY` 截断）。
    pub(crate) drawn: Option<u32>,
    /// 本帧批次数。
    pub(crate) batch_count: Option<u32>,
    /// 本帧上传的图集字节数。
    pub(crate) atlas_bytes: u64,
}

impl BackendAcc {
    /// 把累计器写进 `RenderStats`（帧末、`take_render_stats` 之前调用一次）。
    pub(crate) fn flush_into(&self, stats: &mut RenderStats) {
        self.upload.flush(&mut stats.stages, StageId::Upload);
        self.submit.flush(&mut stats.stages, StageId::BackendSubmit);
        self.present
            .flush(&mut stats.stages, StageId::PresentBlocking);
        if let Some(n) = self.drawn {
            stats.counters.instance_drawn = n;
        }
        if let Some(n) = self.batch_count {
            stats.counters.batch_count = n;
        }
        stats.counters.atlas_upload_bytes = self.atlas_bytes;
    }
}

/// D3D11 渲染后端：持有设备、交换链与实例缓冲；它不认识 UI 树，只认实例。
pub struct D3dBackend {
    /// D3D11 设备与交换链。设备丢失后由 `Gfx::rebuild` 重建，因此这里是可重建的唯一入口。
    // **收紧为 crate 内可见**：`Gfx` 是设备/交换链的实现细节，框架使用者不应直接持有它
    // （全仓唯一访问者都在本 crate 内，含下面的单元测试）。
    pub(crate) gfx: Gfx,
    text: Box<dyn crate::backend::font::FontEngine>,
    instances: Vec<Instance>,
    viewport: Size,
    /// 剪辀栈 / 变换栈 / 悬停 / 主题 / 动画：与 SoftBackend 共用。
    state: y901_render::render_state::RenderState,
    /// 窗口句柄：`RenderBackend::present` 无参，必须记住它。
    hwnd: HWND,
    /// 当前尺寸：trait 的 `present` 无参，必须记住它。
    size: (u32, u32),
    /// 上一次 `present` 是否被遮挡（D3D11 只能从 `Present` 返回值得知）。
    last_occluded: bool,
    /// **窗口级 DPI 缩放**（96 = 100%），由 `window.rs` 在创建后注入。
    ///
    /// 为何不在此调 `GetDpiForSystem`：实测本机系统 DPI=96 而窗口 DPI=120，
    /// PMv2 进程下两者**可以不同**，用系统 DPI 会静默错位。
    dpi_scale: f32,
    /// 渲染主题（启动时由 Options.theme 注入，None 时用 DARK）。
    /// 动画快照（每帧由 `UiTree::render` 通过 `set_animations` 推入）。
    ///
    /// 缺它的话 `RenderCtx::animation` 只能恒返回 1.0，
    /// 于是 Popup 打开动画在 D3D 上“直接到位”而不渐变。
    /// 图片实例的位置记录：`(start, count, texture_id)`。
    ///
    /// 为什么用"记录区间"而不是逐实例标记：图集实例的顺序就是 Z 序，
    /// 只需在帧末把这些区间与中间的图集段交替排开即可，实例数组本身不动。
    image_marks: Vec<(usize, usize, u64)>,
    /// 当前单元格尺寸（由字号推出；set_font 会更新它）。
    ///
    /// **必须与 TextEngine 使用的 cell 尺寸一致**：measure_text/line_height/glyph_offset/draw_text 都读它。
    cell: Size,
    /// 变换栈。第 1 步只建空栈：所有出图路径都过 `map_pos`/`map_size`，
    /// 但**栈为空时纯透传**——先证明恒等（像素哈希不变），再接真变换。
    /// **分阶段统计**（`Y901_RENDER_STATS` 开启且本帧有渲染时才有值）。
    ///
    /// `None` = 统计关闭 / 本帧未产生统计。每帧由 `stats_begin_frame` 重置，
    /// 由 `take_render_stats` 取走（取走语义 ⇒ 每帧最多消费一次）。
    stats: Option<RenderStats>,
    /// 本帧的后端私有累计器（设备丢失重试时可能有多段）。
    acc: BackendAcc,
    /// 后端统计的帧号（仅用于 JSON 可读性，Runtime 侧另有自己的帧号）。
    stats_frame: u64,
    /// 仅测试：每次 `present` 前记下的 `sync_interval`。
    #[cfg(test)]
    pub present_sync_log: std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
}

impl D3dBackend {
    /// 初始化 D3D11 设备、交换链与 DirectWrite 文本引擎。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `hwnd` 是有效且未销毁的窗口句柄，且调用期间不会被其它线程销毁；
    /// - 调用发生在拥有该窗口的线程上；
    /// - `width`、`height` 均大于 0。
    ///
    /// 违反会导致：交换链绑定到悬垂 HWND，`Present` 时访问已释放的窗口内存
    /// （未定义行为）；DirectWrite 一侧失败则返回 `Err`，可恢复。
    pub unsafe fn new(
        hwnd: HWND,
        width: u32,
        height: u32,
        theme: y901_ui::theme::Theme,
        font: &y901_runtime::FontConfig,
    ) -> Result<Self, String> {
        // 逐字节核对 CPU 实例布局与 GPU InputLayout：错一个字节整屏都是乱码。
        d3d11::verify_instance_layout()?;

        let gfx = Gfx::new(hwnd, width, height)?;
        let image_marks: Vec<(usize, usize, u64)> = Vec::new();
        // 字号是全局的（不支持富文本）：cell 尺寸由字号推出，见 cell_metrics_for。
        let (cell_w, cell_h) = crate::backend::font::cell_metrics_for(font.size);
        // 字形引擎选择：`Y901_FONT=self` 用自研，否则 DirectWrite（默认）。
        // 失败就**自动回退**到 DirectWrite，绝不让应用起不来。
        let want_self = std::env::var("Y901_FONT")
            .map(|v| v == "self")
            .unwrap_or(false);
        let (text, engine_name): (Box<dyn crate::backend::font::FontEngine>, &str) = if want_self {
            match crate::font_engine::MyFontEngine::new(
                crate::font_engine::load_font(&crate::font_engine::resolve_font_path(&font.family))
                    .unwrap_or_default(),
                font.size,
                Size::new(cell_w, cell_h),
            ) {
                Ok(e) => {
                    eprintln!("[y901] 字形引擎：自研 MyFontEngine（ttf-parser + CPU 光栅化）");
                    (Box::new(e), "self")
                }
                Err(err) => {
                    eprintln!("[y901] 自研引擎初始化失败（{err}），回退 DirectWrite");
                    (
                        Box::new(TextEngine::new(&font.family, font.size, cell_w, cell_h)?),
                        "legacy",
                    )
                }
            }
        } else {
            (
                Box::new(TextEngine::new(&font.family, font.size, cell_w, cell_h)?),
                "legacy",
            )
        };
        eprintln!(
            "[y901] backend ready: engine={engine_name} ascent={:.2} atlas={}",
            text.ascent(),
            text.atlas_size()
        );

        let mut backend = Self {
            #[cfg(test)]
            present_sync_log: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            gfx,
            image_marks,
            text,
            instances: Vec::with_capacity(4096),
            viewport: Size::new(width as f32, height as f32),
            hwnd,
            size: (width, height),
            last_occluded: false,
            state: y901_render::render_state::RenderState::new(theme),
            cell: Size::new(cell_w, cell_h),
            dpi_scale: 1.0,
            stats: None,
            acc: BackendAcc::default(),
            stats_frame: 0,
        };
        let _ = backend.sync_atlas()?;
        Ok(backend)
    }

    /// 入队一个实例，并在这里统一写入**当前不透明度**到 `params.w`。
    ///
    /// 为何在入队处统一处理：六类构造点各自写一遍容易漏（已有
    /// `stroke_round_rect` 漏 `map_rect` 的教训）；同时避开了 gradient 的
    /// `color_end` 问题——不提前乘 alpha，而在 shader 的**最终输出**上乘一次。
    fn push_instance(&mut self, mut inst: Instance) {
        inst.params[3] = self.state.current_opacity();
        self.instances.push(inst);
    }

    /// 运行时切换主题：只改字段，**不碰图集**。
    ///
    /// 输入：新主题（`Theme` 是 `Copy`，开销可忽略）。
    /// 返回：`Ok(())`；当前实现不会失败（保留 `Result` 是为了将来"主题影响图集"时
    /// 不改签名）。
    /// 副作用：`theme()` 立即返回新值；**需要调用方触发全量重绘**（颜色变了）——
    /// 后端不认识 `UiTree`，由 runtime 调 `tree.invalidate_all()`。
    ///
    /// 说明：主题只含颜色，不含字形信息，因此**不需要重建图集**。
    /// 设置 **窗口级 DPI 缩放比**（物理 / 逻辑），由 `window.rs` 在窗口创建后调用。
    pub fn set_dpi_scale(&mut self, scale: f32) {
        self.dpi_scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
    }

    /// 切换主题（重建后需重传字形图集与清屏色）。
    pub fn set_theme(&mut self, theme: y901_ui::theme::Theme) -> Result<(), String> {
        self.state.set_theme(theme);
        Ok(())
    }

    /// 运行时切换字体：重建文本引擎，换字体族与字号。
    ///
    /// 输入：新字体配置（`family` + `size`）。
    /// 返回：`Ok(())` = 已切换；`Err(msg)` = 新引擎创建失败。
    /// 副作用（成功时）：`self.text` 换成新引擎（其图集为空、`reset_pending = true`，
    /// 下一帧全量重传）、`self.cell` 更新为新字号推出的尺寸。
    /// **需要调用方**：`tree.invalidate_all()` + `mark_layout_dirty` + 重排
    /// （cell 尺寸变了，所有排版都得重算）。
    ///
    /// **失败时保持旧引擎，不破坏当前渲染状态**——顺序是"先建新的，成功后才换"，
    /// 绝不出现"新引擎建失败但图集已清空"的中间态（那会整屏缺字）。
    pub fn set_font(&mut self, font: &y901_runtime::FontConfig) -> Result<(), String> {
        let (cell_w, cell_h) = crate::backend::font::cell_metrics_for(font.size);
        // 1) 先创建新引擎：这一步失败就直接返回，旧引擎与旧 cell 尺寸原封不动
        let text = TextEngine::new(&font.family, font.size, cell_w, cell_h)?;
        // 2) 成功后才替换（新引擎的 reset_pending = true → 图集整张重传）
        self.text = Box::new(text);
        self.cell = Size::new(cell_w, cell_h);
        Ok(())
    }

    /// 每帧开始：清空实例数组，并推进图集帧号（LRU 用）。
    pub fn begin(&mut self) {
        self.instances.clear();
        self.image_marks.clear();
        // **基座裁剪 = 客户区**：之后所有 fill_rect / 字形都必须落在窗口内。
        // 少了这一层，组件只要把内容画到负坐标（例如终端多算了一行），
        // 就会画到窗口标题栏缝隙里。
        self.state.clear_clips();
        self.state
            .push_clip(Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h));
        self.text.begin_frame();
    }

    // ---- 分阶段统计（A 阶段；`pub(crate)`，不是公开面）----

    /// 每帧开始：清空累计器；统计开启时复用/新建统计对象，关闭时**清空且不分配**。
    ///
    /// 关闭时只做一次 `Option` 赋值（不读时钟、不分配）；`enabled()` 内部是
    /// `OnceLock`，所以这里每帧的代价是一次原子读 + 一次分支。
    pub(crate) fn stats_begin_frame(&mut self) {
        self.acc = BackendAcc::default();
        if y901_runtime::render_stats::enabled() {
            self.stats_frame = self.stats_frame.wrapping_add(1);
            match self.stats.as_mut() {
                // 复用上一帧的对象（零分配），并清掉上一帧的阶段/计数器。
                Some(s) => s.reset_for_next_frame(self.stats_frame),
                None => self.stats = Some(RenderStats::new(self.stats_frame, BackendKind::D3d11)),
            }
        } else {
            // 关闭统计：连对象都不留（`take_render_stats` 因此返回 `None`）。
            self.stats = None;
        }
    }

    /// 打点起点；**统计关闭时返回 `None` 且不读时钟**。
    #[inline]
    pub(crate) fn stats_mark(&self) -> Option<std::time::Instant> {
        if self.stats.is_some() {
            Some(std::time::Instant::now())
        } else {
            None
        }
    }

    /// 记录**单区间**阶段（`render_prepare` / `command_gen`）。
    ///
    /// 只有这两个阶段走单区间；`upload` / `backend_submit` / `present_blocking`
    /// 可能有多段（设备丢失重试），走 [`BackendAcc`] 累计。
    #[inline]
    pub(crate) fn stats_record(&mut self, id: StageId, start: Option<std::time::Instant>) {
        if let (Some(s), Some(t)) = (self.stats.as_mut(), start) {
            s.stages.set(id, t.elapsed());
        }
    }

    /// 记录脏区计数（块数 + 覆盖像素数）。
    #[inline]
    pub(crate) fn stats_dirty(&mut self, rects: u32, pixels: u64) {
        if let Some(s) = self.stats.as_mut() {
            s.counters.dirty_rect_count = rects;
            s.counters.damaged_pixels = pixels;
        }
    }

    /// 跳过帧（`dirty` 为空）：后端**没有渲染**，但统计对象照常交付。
    ///
    /// `frame_skipped=true` 只表示"后端跳过渲染"；Runtime 侧已测量的阶段
    /// （事件/layout/after_frame/after_present/total_frame）不受影响。
    pub(crate) fn stats_mark_skipped(&mut self) {
        if let Some(s) = self.stats.as_mut() {
            s.counters.frame_skipped = true;
        }
        self.acc = BackendAcc::default();
    }

    /// 帧末：把累计器与计数器写进统计对象，然后**取走**（`take` 语义）。
    ///
    /// 调用点：`D3dBackend::present` 的成功出口。错误出口**不调用**它，
    /// 而是直接丢弃 `stats`（失败帧不交付统计，见 `present` 的注释）。
    fn stats_finish(&mut self, occluded: bool) {
        let Some(mut s) = self.stats.take() else {
            self.acc = BackendAcc::default();
            return;
        };
        self.acc.flush_into(&mut s);
        s.counters.instance_pushed = self.instances.len() as u32;
        if !self.acc.submit.seen() {
            // 本帧没执行过提交（例如整帧被跳过）→ 明确写 0，而不是沿用上一帧的值。
            s.counters.instance_drawn = 0;
        }
        s.counters.occluded = occluded;
        s.counters.frame_skipped = false;
        self.stats = Some(s);
        self.acc = BackendAcc::default();
    }

    /// 取走本帧统计（**取走语义**：第二次调用必为 `None`）。
    pub(crate) fn take_render_stats(&mut self) -> Option<RenderStats> {
        self.stats.take()
    }

    /// 把 CPU 图集的脏区同步到 GPU。首次调用会创建纹理。
    /// # Safety
    ///
    /// `self` 必须是**本线程且已初始化**的后端；图集、字形引擎与 D3D 设备已就绪。
    /// 违反后果：设备未就绪时会向空指针上传像素（崩溃点在驱动层）；
    /// 跨线程调用会让图集与上传命令交错，字形出现随机块状污点。
    /// 返回本帧**实际上传的字节数**（`None` = 没有需要上传的内容）。
    ///
    /// 为什么要返回字节数而不是 `()`：统计里 `atlas_upload_bytes` 是计数器，
    /// 而"是否执行过 upload 阶段"也由它判定 —— `None` 表示阶段未执行，
    /// 不能靠"耗时 > 0"去猜（那会把极快上传误判成没执行）。
    unsafe fn sync_atlas(&mut self) -> Result<Option<u64>, String> {
        let (reset, dirty) = self.text.take_dirty();
        if !reset && dirty.is_empty() {
            return Ok(None);
        }
        let size = self.text.atlas_size() as u64;
        let bytes: u64 = if reset {
            // 整张重传：`size × size` 像素 × 4 字节
            size * size * 4
        } else {
            dirty
                .iter()
                .map(|d| d.width as u64 * d.height as u64 * 4)
                .sum()
        };
        let rects: Vec<(u32, u32, u32, u32)> = dirty
            .iter()
            .map(|d| (d.x, d.y, d.width, d.height))
            .collect();
        self.gfx
            .update_atlas(self.text.atlas(), self.text.atlas_size(), reset, &rects)
            .map(|()| Some(bytes))
    }

    /// 回读后备缓冲（BGRA）：供快照 / 像素数学验证使用。
    ///
    /// # Safety
    ///
    /// 必须在拥有窗口的线程上调用，且已完成至少一次 present。
    pub unsafe fn read_back_pixels(&self) -> Option<(Vec<u8>, u32, u32)> {
        self.gfx.read_back_pixels()
    }

    /// 请求在下一帧 present 之前快照（回读像素）。
    pub fn request_snapshot(&mut self) {
        self.gfx.request_snapshot();
    }

    /// 取走上次快照。
    pub fn take_snapshot(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        self.gfx.take_snapshot()
    }

    /// 设置本帧脏区（`None` = 整窗）。由 `Window::render` 在 `tree.render` 之前算好。
    ///
    /// 输入：`rect = (left, top, right, bottom)` 像素坐标；`None` 表示整窗重绘。
    /// 副作用：下一帧只清/画这块区域（scissor）。脏区为空时调用方应传 `None`。
    pub fn set_dirty_bounds(&mut self, rects: Option<Vec<(i32, i32, i32, i32)>>) {
        self.gfx.set_dirty_bounds(rects);
    }

    /// 提交一帧。返回是否需要继续重绘。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `hwnd` 仍是创建本后端时使用的窗口，且尚未销毁；
    /// - `width`、`height` 是**当前**客户区尺寸（不一致会走重建路径）；
    /// - 同线程调用。
    ///
    /// 违反会导致：重建路径用悬垂 HWND 创建交换链（未定义行为），
    /// 或尺寸反复不一致造成资源重建风暴。
    ///
    /// 返回：
    /// - `Ok(true)`  —— 成功上屏，**未被遮挡**；
    /// - `Ok(false)` —— 成功上屏，**被遮挡**（`DXGI_STATUS_OCCLUDED`）；
    /// - `Err(e)`    —— **上屏失败**（首次建设备失败 / 设备丢失后重建失败 / `Present` 返回错误）。
    ///
    /// 旧签名返 `bool`，**失败与"被遮挡"同为 `false`** —— 上层无法区分「屏幕没变」与「渲染已坏」。
    pub unsafe fn present(
        &mut self,
        hwnd: HWND,
        width: u32,
        height: u32,
        sync_interval: u32,
    ) -> Result<bool, String> {
        // **测试开关**：强制 present 失败，用于验证 `render_inner` 的 `present?` 传播路径
        // （设备丢失 / 交换链失效的真实场景无法在测试中构造）。
        //
        // 位置：**在任何 unsafe 操作之前** —— 这样开关生效时
        // 不会触碰任何未定义行为前提（本函数是 `unsafe fn`，但开关本身不需要 unsafe）。
        // 与 `Y901_SIMULATE_RENDER_FAIL` 同构，`OnceLock` 缓存（零热路径成本）。
        static SIMULATE_PRESENT_FAIL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *SIMULATE_PRESENT_FAIL
            .get_or_init(|| std::env::var_os("Y901_SIMULATE_PRESENT_FAIL").is_some())
        {
            // 失败帧不交付统计：**丢弃残留**，避免下一次 `take_render_stats`
            // 读到"有 Submit 但没有 Present/上屏状态"的半帧数据。
            self.stats = None;
            return Err("模拟 present 失败（Y901_SIMULATE_PRESENT_FAIL）".to_string());
        }

        if self.gfx.is_empty() {
            if let Err(err) = self.gfx.rebuild(hwnd, width, height) {
                // **首次建设备失败就是真失败**：没有后备路径，上层必须知道。
                self.stats = None;
                return Err(format!("初始化 D3D11 设备失败：{err}"));
            }
        }

        // ---- ① 图集上传（`Some(bytes)` 才表示本阶段真的执行了）----
        let t_upload = self.stats_mark();
        match self.sync_atlas() {
            Ok(Some(bytes)) => {
                if let Some(t) = t_upload {
                    self.acc.upload.add(t.elapsed());
                }
                self.acc.atlas_bytes += bytes;
            }
            // `None` = 本帧没有需要上传的内容 → `upload` 阶段保持未测量（不写 0）。
            Ok(None) => {}
            Err(err) => eprintln!("[y901] atlas sync failed: {err}"),
        }

        // sync_interval = 0：不阻塞在 vsync 上。
        // 我们是按需重绘（invalidate 驱动），不需要垂直同步；
        // 用 1 时窗口被遮挡会让 Present 永久阻塞，把消息循环卡死。
        // 帧末把"图片区间"与"图集区间"交替排开。
        // 没有图片时 `batches` 为空 → `render_batched` 退化为单次 draw，与改动前一致。
        // ---- ② 批次规划（CPU；并入 `backend_submit`）----
        let t_build = self.stats_mark();
        let batches = self.build_batches();
        if let Some(t) = t_build {
            self.acc.submit.add(t.elapsed());
        }
        self.acc.batch_count = Some(batches.len() as u32);
        #[cfg(test)]
        self.present_sync_log.lock().unwrap().push(sync_interval);
        // `render_batched` 内部自己记 submit 段（到 present 之前）与 present 段，
        // 并回填 `drawn`（受 `INSTANCE_CAPACITY` 截断后的实际实例数）。
        // 借用技巧：`acc` 只借 `self.acc`、`render_batched` 只借 `self.gfx` 与
        // `self.instances` —— 三个**不同字段**，因此可以同时存在。
        let acc = if self.stats.is_some() {
            Some(&mut self.acc)
        } else {
            None
        };
        let mut hr = self
            .gfx
            .render_batched_timed(&self.instances, &batches, sync_interval, acc);

        if Gfx::device_lost(hr) {
            eprintln!("[y901] device lost: 0x{:08X}, rebuilding", hr as u32);
            // 设备重建计入 `backend_submit`（它在两次提交之间，属提交侧准备）。
            let t_rebuild = self.stats_mark();
            if let Err(err) = self.gfx.rebuild(hwnd, width, height) {
                // 设备丢失 + 重建失败 = 这个窗口已无法渲染：**丢弃残留统计**再上报。
                self.stats = None;
                return Err(format!("设备丢失后重建失败：{err}"));
            }
            if let Some(t) = t_rebuild {
                self.acc.submit.add(t.elapsed());
            }
            // 设备重建后纹理是空的：全量重传（**计入 upload 的第二段**）
            let t_reupload = self.stats_mark();
            match self
                .gfx
                .update_atlas(self.text.atlas(), self.text.atlas_size(), true, &[])
            {
                Ok(()) => {
                    if let Some(t) = t_reupload {
                        self.acc.upload.add(t.elapsed());
                    }
                    let size = self.text.atlas_size() as u64;
                    self.acc.atlas_bytes += size * size * 4;
                }
                Err(err) => eprintln!("[y901] atlas re-upload failed: {err}"),
            }
            let acc = if self.stats.is_some() {
                Some(&mut self.acc)
            } else {
                None
            };
            hr = self
                .gfx
                .render_batched_timed(&self.instances, &batches, sync_interval, acc);
        }

        if hr < 0 {
            // 设备丢失已在上面尝试重建；能到这里说明重建后仍失败。
            self.stats = None;
            return Err(format!("Present 失败：0x{:08X}", hr as u32));
        }
        // `Ok(visible)`：`true` = 已上屏且未被遮挡；`false` = 已上屏但被遮挡（DXGI_STATUS_OCCLUDED）。
        let visible = !Gfx::is_occluded(hr);
        // ---- ③ 帧末收口（累计器 → RenderStats → take）----
        self.stats_finish(!visible);
        Ok(visible)
    }

    /// 返回至今为止记录的 `sync_interval` 序列（仅测试构建）。
    #[cfg(test)]
    pub fn take_present_sync_log(&self) -> Vec<u32> {
        self.present_sync_log.lock().unwrap().clone()
    }

    /// 记录新的客户区尺寸并尝试同步交换链。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `width`、`height` 大于 0（0 尺寸会让 `ResizeBuffers` 失败）；
    /// - 同线程调用，且此刻没有外部代码持有本后端的 back buffer 引用。
    ///
    /// 违反会导致：`ResizeBuffers` 返回 `DXGI_ERROR_INVALID_CALL`；
    /// 本方法只打印错误并保留旧尺寸，不会 panic。
    /// 调整交换链与自持 RT 尺寸。
    ///
    /// 返回：`true` = 自持 RT 被重建（**内容丢失，调用方必须整窗重绘**）。
    /// # Safety
    ///
    /// 只能在**拥有该窗口的 GUI 线程**调用；`width`/`height` 必须 > 0 且不超过设备最大纹理尺寸。
    /// 违反后果：跨线程重建交换链会拿到属于另一线程的 HWND，`Present` 持续失败（黑屏）；
    /// 零尺寸会让后备缓冲创建失败，而本函数只返回 `false`，调用方很容易忽略。
    pub unsafe fn resize(&mut self, width: u32, height: u32) -> bool {
        self.viewport = Size::new(width as f32, height as f32);
        // 交换链尺寸更新交给 present 前的 rebuild 路径；这里只记尺寸。
        match self.gfx.resize(width, height) {
            Ok(rebuilt) => rebuilt,
            Err(err) => {
                eprintln!("[y901] swapchain resize failed: {err}");
                false
            }
        }
    }

    /// 当前帧的实例数（诊断用）。
    // 诊断 API：`instance_count()`：诊断，测试断言提交实例数
    // 归类：诊断 API —— `instance_count` 供 Y901_PROFILE 输出单帧实例数
    #[allow(dead_code)] // 诊断接口，主循环不调用
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    // 诊断 API：`live_objects()`：COM 泄漏诊断，测试断言"用完全部释放"
    // 归类：诊断 API —— 与 d3d11::live_objects 同义，供外部泄漏检查
    #[allow(dead_code)] // 诊断接口，主循环不调用
    /// 当前存活的 COM 对象计数（只增不减说明有泄漏）。测试用它断言"用完全部释放"，比肉眼看画面可靠。
    pub fn live_objects() -> isize {
        Gfx::live_objects()
    }

    fn clip_of(&self, rect: Rect) -> Option<Rect> {
        match self.state.current_clip() {
            Some(clip) => {
                let r = rect.intersect(clip);
                if r.is_empty() {
                    None
                } else {
                    Some(r)
                }
            }
            None => Some(rect),
        }
    }

    /// 把字形推进实例列表。
    ///
    /// `span` 来自格契约（`y901_core::width_of_char`）：ASCII=1，CJK=2。
    /// **不是**字形 advance，而是"这个字形应占几格"。
    /// 居中补偿 `offset_x = (span*CELL_W - advance)/2`：字体 advance 与格宽
    /// 不精确相等时（跨平台常见），字形仍落在格子中央而不是贴左。
    /// 把屏幕坐标过一遍变换栈。
    ///
    /// **栈为空时直接返回原值**，不做任何浮点运算——
    /// `* 1.0` 或 `+ 0.0` 足以让像素哈希漂移，而哈希门禁会把它判成渲染回归。
    fn map_pos(&self, p: y901_ui::Pos) -> y901_ui::Pos {
        if self.state.transforms_empty() {
            return p;
        }
        let mut x = p.x;
        let mut y = p.y;
        for (o, s) in self.state.transforms().iter().rev() {
            x = o.x + (x - o.x) * s;
            y = o.y + (y - o.y) * s;
        }
        y901_ui::Pos::new(x, y)
    }

    /// 把尺寸过一遍变换栈（只乘缩放，不加平移）。栈为空时直接返回。
    fn map_size(&self, sz: y901_ui::Size) -> y901_ui::Size {
        if self.state.transforms_empty() {
            return sz;
        }
        let k = self.current_scale();
        y901_ui::Size::new(sz.w * k, sz.h * k)
    }

    /// 把一个矩形过一遍变换栈。栈为空时**逐位原样返回**。
    fn map_rect(&self, r: Rect) -> Rect {
        if self.state.transforms_empty() {
            return r;
        }
        let o = self.map_pos(y901_ui::Pos::new(r.x, r.y));
        let sz = self.map_size(y901_ui::Size::new(r.w, r.h));
        Rect::new(o.x, o.y, sz.w, sz.h)
    }

    /// 当前缩放乘积（栈为空时恒为 1.0）。
    fn current_scale(&self) -> f32 {
        self.state.current_scale()
    }

    /// **按字号绘制文本**（per-text 字号，与 soft 的 `draw_text_sized` 同构）。
    ///
    /// `px_final = px * current_scale()`；cell 用 `cell_metrics_for(px_final)`（与度量同源）；
    /// 字形用 `glyph_at_size(ch, px_final)` 并 **1:1 绘制**（不走 mip 选级，避免双重缩放）。
    fn draw_text_sized(&mut self, origin: Pos, text: &str, style: &TextStyle, px: f32) -> f32 {
        // **DPI × 变换缩放**（两个不同来源，不重复）。
        let px_final = (px * self.dpi_scale * self.current_scale()).max(1.0);
        let (cell_w, cell_h) = y901_render::cell_metrics_for(px_final);
        // 基线比例 = ascent / cell_h（字体属性，不随字号变），与 soft 同式
        let base_ascent = self.text.ascent();
        let base_cell_h = self.cell.h;
        let ascent_ratio = if base_cell_h > 0.0 {
            base_ascent / base_cell_h
        } else {
            0.8
        };
        let ascent = cell_h * ascent_ratio;

        let mut pen_x = origin.x;
        let start = pen_x;
        for ch in text.chars() {
            // 格数的**唯一来源**是 `y901_core::Grid::width_of_char`（经 `cell_span`），
            // **不做二次加工**；`span = 0` 时仍绘制字形、推进 0。
            let span = self.text.cell_span(ch) as f32;
            if style.bg.a > 0.0 {
                self.fill_rect(Rect::new(pen_x, origin.y, span * cell_w, cell_h), style.bg);
            }
            if ch != ' ' {
                self.push_glyph_sized(
                    pen_x, origin.y, span, ch, style.fg, style.bold, px_final, ascent, cell_w,
                );
            }
            if style.underline {
                self.fill_rect(
                    Rect::new(pen_x, origin.y + cell_h - 2.0, span * cell_w, 1.0),
                    style.fg,
                );
            }
            pen_x += span * cell_w;
        }
        pen_x - start
    }

    /// 按字号推入一个字形四边形（1:1，不做 mip 缩放）。
    ///
    /// 为何不改 `push_glyph` 加参数：那会让旧路径的每个调用点都要传
    /// `None + ascent + cell_w`，**旧路径变得难以对比 diff**。新增方法让旧路径**一字未动**。
    #[allow(clippy::too_many_arguments)]
    fn push_glyph_sized(
        &mut self,
        x: f32,
        y: f32,
        span: f32,
        ch: char,
        color: Color,
        bold: bool,
        px: f32,
        ascent: f32,
        cell_w: f32,
    ) {
        let atlas_size = self.text.atlas_size() as f32;
        // 先取字形属性（避开 `&mut self.text` 与 `self.map_pos` 的借用冲突）
        let picked = match self.text.glyph_at_size(ch, px) {
            Some(g) if g.width > 0 && g.height > 0 => Some((
                g.width,
                g.height,
                g.bearing_x,
                g.bearing_y,
                g.atlas_x,
                g.atlas_y,
                g.advance,
            )),
            _ => None,
        };
        let Some((gw, gh, gbx, gby, gax, gay, gadv)) = picked else {
            return;
        };

        let offset_x = glyph_offset(span, cell_w, gadv);
        let top = glyph_top(y, ascent, gby as f32, gh as f32);
        // **整像素吸附**：见 `glyph_top` / `snap_glyph_pos` 的说明。
        let pos = snap_glyph_pos(self.map_pos(Pos::new(x + offset_x + gbx as f32, top)));
        // **不调 `map_size`**：字号已按 `px * scale` 光栅化，再乘一次就是双重缩放。
        let size = [gw as f32, gh as f32];
        let uv = [
            gax as f32 / atlas_size,
            gay as f32 / atlas_size,
            (gax + gw) as f32 / atlas_size,
            (gay + gh) as f32 / atlas_size,
        ];
        let rgba = [color.r, color.g, color.b, color.a];

        let Some(clip) = self.state.current_clip() else {
            return;
        };
        let Some(quad) = clip_quad(pos, size, uv, clip) else {
            return;
        };
        self.push_instance(Instance {
            pos: [quad.pos.x, quad.pos.y],
            size: quad.size,
            uv_rect: quad.uv,
            color: rgba,
            ..Default::default()
        });
        if bold {
            self.push_instance(Instance {
                pos: [quad.pos.x + 1.0, quad.pos.y],
                size: quad.size,
                uv_rect: quad.uv,
                color: rgba,
                ..Default::default()
            });
        }
    }

    fn push_glyph(&mut self, x: f32, y: f32, span: f32, ch: char, color: Color, bold: bool) {
        let atlas_size = self.text.atlas_size() as f32;
        let ascent = self.text.ascent();

        // 先按**当前缩放**选一级字形；`scale >= 1` 时这个调用就是 L0（不生成 mip）。
        // 用 `ScaledGlyph` 而不是 `GlyphInfo`：它带着"实际用的级 + 该级的图集矩形 + 该级自己的 bearing"。
        let scale = self.current_scale();
        let Some(sel) = self.text.glyph_at_scale(ch, scale) else {
            return;
        };
        if sel.w == 0 || sel.h == 0 {
            return;
        }

        // **在"映射后的屏幕空间"里算出矩形**（与 `soft.rs` 同构）。
        //
        // 为什么不能像旧版那样"未缩放空间算完再整体映射一次"：
        // 基线里既有 `ascent`（要 ×k）又有**字形高度**（也要 ×k），
        // 两者在未缩放空间相加后再乘 k，与"先各自乘 k 再相加"差一个
        // `(k-1) * (ascent - height)` —— 缩小时这会让基线整体偏上/偏下，
        // 与 CPU 后端对不上（`backend_parity` 在 0.3 缩放下抓到的正是这个）。
        //
        // **mip 级的 bearing 是该级自己的像素量纲**（每级独立重新光栅化），
        // 先乘 `h / slot.height`（≈ 2^level）换回 L0，再乘 k 到屏幕。
        let k = scale.max(1e-6);
        let offset_x = glyph_offset(span, self.cell.w, sel.advance);
        let slot_h = sel.slot.height.max(1) as f32;
        let level_k = sel.h as f32 / slot_h;
        let bear_x_l0 = sel.slot.bearing_x * level_k;
        let bear_y_l0 = sel.slot.bearing_y * level_k;

        let origin = self.map_pos(Pos::new(x, y));
        // 目标尺寸取整；**若该级位图本身就等于目标尺寸，就按 1:1 贴**（避免多余的半像素重采样）。
        let target = [
            (sel.w as f32 * k).round().max(1.0),
            (sel.h as f32 * k).round().max(1.0),
        ];
        let slot_size = [sel.slot.width as f32, sel.slot.height as f32];
        let one_to_one = sel.level > 0
            && (slot_size[0] - target[0]).abs() <= 1.0
            && (slot_size[1] - target[1]).abs() <= 1.0;
        let (size, bear_x_final, bear_y_final) = if one_to_one {
            // 1:1：位置用该级自己的 bearing（级内像素），尺寸用位图尺寸
            (slot_size, sel.slot.bearing_x, sel.slot.bearing_y)
        } else {
            (target, bear_x_l0 * k, bear_y_l0 * k)
        };
        // 四边形尺寸取整后，**顶部也要用同一个取整高度**：
        // `glyph_top` 里是 `基线 − bearing − 高度`，若高度用未取整的 3.6
        // 而四边形按 4 行画，就会整体下移半像素，与 CPU 后端差一行。
        let top = glyph_top(origin.y, ascent * k, bear_y_final, size[1]);
        // **整像素吸附**：位置与尺寸都吸到整数，避免线性采样在半像素上重采样。
        let pos = snap_glyph_pos(Pos::new(origin.x + (offset_x + bear_x_final) * k, top));
        // UV 指向**所选级**在图集里的矩形
        let uv = [
            sel.slot.atlas_x as f32 / atlas_size,
            sel.slot.atlas_y as f32 / atlas_size,
            (sel.slot.atlas_x + sel.slot.width) as f32 / atlas_size,
            (sel.slot.atlas_y + sel.slot.height) as f32 / atlas_size,
        ];
        let rgba = [color.r, color.g, color.b, color.a];

        // 字形也要裁剪：`clip_of` 只作用于 fill_rect，字形是带 UV 的矩形，
        // 必须按裁剪框缩放并重映射 UV（否则会画到客户区外，见 §34）。
        let Some(clip) = self.state.current_clip() else {
            return;
        };
        let Some(quad) = clip_quad(pos, size, uv, clip) else {
            return;
        };

        self.push_instance(Instance {
            pos: [quad.pos.x, quad.pos.y],
            size: quad.size,
            uv_rect: quad.uv,
            color: rgba,
            ..Default::default()
        });

        // 假粗体：右移 1px 再画一遍（比加载粗体字形便宜，且等宽不变）。
        //
        // **必须用裁剪后的 `quad`**：第一条实例已经被 clip 收窄，第二条若还用
        // 原始的 `pos/size/uv`，粗体字就会画到 clip 之外（与 `push_glyph_sized` 不一致）。
        if bold {
            self.push_instance(Instance {
                pos: [quad.pos.x + 1.0, quad.pos.y],
                size: quad.size,
                uv_rect: quad.uv,
                color: rgba,
                ..Default::default()
            });
        }
    }
}

// ---------------------------------------------------------------------------
// RenderBackend：平台无关的渲染契约实现（跨平台 v3 阶段 1 第 2 步）
// ---------------------------------------------------------------------------

/// `GfxInstance` → GPU 顶点结构。
///
/// 两者字段顺序与类型完全一致（48 字节）；`debug_assert` 在开发期兜底，
/// 发布期由 `gfx_instance_layout_matches_d3d_instance` 测试保证。
fn to_gpu_instance(i: &y901_runtime::gfx::GfxInstance) -> Instance {
    debug_assert_eq!(
        std::mem::size_of::<y901_runtime::gfx::GfxInstance>(),
        std::mem::size_of::<Instance>(),
        "GfxInstance 与 GPU Instance 布局不一致"
    );
    Instance {
        pos: i.pos,
        size: i.size,
        uv_rect: i.uv_rect,
        color: i.color,
        color_end: i.color_end,
        gradient: i.gradient,
        params: i.params,
    }
}

impl y901_runtime::gfx::RenderBackend for D3dBackend {
    /// 只有 Win32 句柄能交给 D3D11；其它平台直接拒绝。
    fn create(
        handle: y901_runtime::gfx::PlatformWindowHandle,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        use y901_runtime::gfx::PlatformWindowHandle;
        let hwnd = match handle {
            PlatformWindowHandle::Win32(ptr) => ptr as HWND,
            PlatformWindowHandle::X11(_)
            | PlatformWindowHandle::Wayland(_)
            | PlatformWindowHandle::Other(_) => {
                return Err("D3dBackend 只支持 Win32 窗口句柄".to_string())
            }
        };
        // SAFETY: 调用方通过 `PlatformWindowHandle::Win32` 承诺句柄有效且同线程。
        // 这里只能用默认主题/字体：`RenderBackend::create` 不接收 `Options`，
        // 需要自定义时走 `D3dBackend::new`（`Window::create` 就是这么做的）。
        unsafe {
            Self::new(
                hwnd,
                width,
                height,
                y901_ui::theme::Theme::DARK,
                &default_font_config(),
            )
        }
    }

    fn rebuild(&mut self, width: u32, height: u32) -> Result<(), String> {
        // SAFETY: hwnd 由 create 时保存，仍然有效（调用方负责窗口生命周期）。
        unsafe { self.gfx.rebuild(self.hwnd, width, height) }?;
        self.size = (width, height);
        self.viewport = Size::new(width as f32, height as f32);
        // 图集纹理随设备一起丢失：必须全量重传，否则后续 present 采样空纹理 → 整屏缺字。
        // SAFETY: 同上；reset=true 表示整张重传。
        unsafe {
            self.gfx
                .update_atlas(self.text.atlas(), self.text.atlas_size(), true, &[])
        }
        .map_err(|err| format!("rebuild 后图集重传失败: {err}"))?;
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        // SAFETY: 同 rebuild；尺寸来自当前客户区。
        let rebuilt = unsafe { D3dBackend::resize(self, width, height) };
        if !rebuilt && self.size != (width, height) {
            // **区分 false 的两种含义**：
            // `D3dBackend::resize` 返回 `false` 时，**尺寸没变**与**重建失败**都是 false ——
            // 它的签名无法区分这两种情况。
            // 这里用"尺寸是否真的变了"做一次**外部判断**：
            // 尺寸没变 → false 是正常的"无需重建"；
            // 尺寸变了却仍 false → 重建失败，必须可见。
            eprintln!("[y901] D3dBackend::resize 失败：尺寸已变为 {width}x{height} 但交换链未重建");
        }
        self.size = (width, height);
        Ok(())
    }

    fn present(&mut self, frame: &y901_runtime::gfx::Frame) -> Result<(), String> {
        // 1) 用本帧实例替换内部缓冲（D3D11 的 present 从 self.instances 取数据）
        self.instances.clear();
        self.image_marks.clear();
        self.instances
            .extend(frame.instances.iter().map(to_gpu_instance));

        // 2) 图集：D3dBackend 的图集由 TextEngine 自己管理（sync_atlas 内部取脏区），
        //    因此 `frame.atlas_updates` 在这里不被使用。这是当前实现的已知偏差：
        //    平台无关后端（wgpu 等）会用它们；D3D11 后端不需要。
        //    TODO(§18)：抽 GlyphProvider 时把图集管理统一到 trait。
        //
        // 图集未同步 = 采样空纹理 = 整屏缺字。debug 断言 + release 只警告一次。
        if !self.gfx.has_atlas() && !self.text.atlas().is_empty() {
            debug_assert!(false, "present 时图集未同步（rebuild 后未重传图集）");
            if !ATLAS_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!(
                    "[y901] 严重：present 时图集未同步（rebuild 后未重传？），画面将缺字。\
                     这条日志每个进程只打一次；看到它就说明 rebuild 逻辑有 bug。"
                );
            }
        }
        let (w, h) = self.size;
        // SAFETY: hwnd 与尺寸均由本结构保存，调用方保证窗口存活。
        // 新签名：`Ok(visible)` = 上屏成功（`false` 仅表示被遮挡）；`Err` = 上屏失败。
        let visible = unsafe { D3dBackend::present(self, self.hwnd, w, h, 1) }?;
        // `last_occluded` 语义：**被遮挡**。`visible == false` 即被遮挡。
        self.last_occluded = !visible;
        Ok(())
    }

    fn is_occluded(&self) -> bool {
        self.last_occluded
    }

    fn instance_count(&self) -> usize {
        self.instances.len()
    }
}

impl TextMetrics for D3dBackend {
    fn measure_text(&self, text: &str, style: &TextStyle) -> Size {
        // **per-text 字号**：与 `SoftBackend` **同源**（都调 `cell_metrics_for`），
        // `None` 时用引擎默认 cell —— 两后端对同一 `TextStyle` 必须给出同样的度量，
        // 否则 D3D/软渲染两条路径的布局会分叉。
        let cell = match style.size {
            Some(px) if px > 0.0 => {
                // **per-text 字号乘 DPI**（与 soft 同源）
                let (w, h) = y901_render::cell_metrics_for(px * self.dpi_scale);
                y901_ui::Size::new(w, h)
            }
            _ => self.cell,
        };
        let cols: u32 = text.chars().map(|c| self.text.cell_span(c)).sum();
        Size::new(cols as f32 * cell.w, cell.h)
    }

    fn line_height(&self, style: &TextStyle) -> f32 {
        match style.size {
            Some(px) if px > 0.0 => y901_render::cell_metrics_for(px * self.dpi_scale).1,
            _ => self.cell.h,
        }
    }

    /// 墨迹深度 = `(ascent + descent) × 字号比`（见 `TextMetrics::ink_bounds`）。
    fn ink_bounds(&self, style: &TextStyle) -> (f32, f32) {
        let base_px = self.cell.h.max(1.0);
        let px = match style.size {
            Some(px) if px.is_finite() && px > 0.0 => px * self.dpi_scale,
            _ => base_px,
        };
        let k = px / base_px;
        (0.0, (self.text.ascent() + self.text.descent()) * k)
    }

    fn scale(&self) -> f32 {
        // **窗口级 DPI**（96 = 100%），字段由 `window.rs` 注入。
        //
        // 何以不再用 `GetDpiForSystem`：**实测本机系统 DPI=96 而窗口 DPI=120** ——
        // PMv2 进程下两者可以不同，用系统 DPI 会在这类机器上静默错位。
        self.dpi_scale
    }
}

impl D3dBackend {
    /// 把 `image_marks` 转成按顺序的批次列表。
    ///
    /// 图集段与图片段交替排开，**不重排**实例：重排会改变重叠元素的层次。
    fn build_batches(&self) -> Vec<crate::backend::d3d11::Batch> {
        use crate::backend::d3d11::Batch;
        // 诊断开关（复现 Grid+Image 空白时用）：强制退化回"单次 draw"。
        if std::env::var_os("Y901_NO_IMAGE_BATCH").is_some() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut cursor = 0usize;
        for &(start, count, id) in &self.image_marks {
            if start > cursor {
                out.push(Batch {
                    start: cursor as u32,
                    count: (start - cursor) as u32,
                    image: None,
                });
            }
            if count > 0 {
                out.push(Batch {
                    start: start as u32,
                    count: count as u32,
                    image: Some(id),
                });
            }
            cursor = (start + count).max(cursor);
        }
        if cursor < self.instances.len() {
            out.push(Batch {
                start: cursor as u32,
                count: (self.instances.len() - cursor) as u32,
                image: None,
            });
        }
        out
    }
}

impl RenderCtx for D3dBackend {
    fn upload_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> y901_ui::image::TextureId {
        // Safety: 与其余显示路径同一线程、且 Gfx 已初始化（未初始化时返回 0）。
        let id = unsafe { self.gfx.upload_image(width, height, rgba) };
        y901_ui::image::TextureId(id)
    }

    fn draw_image(
        &mut self,
        rect: Rect,
        texture: y901_ui::image::TextureId,
        style: y901_ui::image::ImageStyle,
    ) {
        if !texture.is_valid() {
            return;
        }
        let Some((_, tw, th)) = self.gfx.image(texture.0) else {
            return;
        };
        let rect = self.map_rect(rect);
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        // 适配模式复用纯函数：后端不重推 fit 数学，避免两个后端行为不一致。
        let (dst, src) = style.layout(r, tw, th);
        if dst.w <= 0.0 || dst.h <= 0.0 || src.w <= 0.0 || src.h <= 0.0 {
            return;
        }
        let (fw, fh) = (tw.max(1) as f32, th.max(1) as f32);
        let uv = [
            src.x / fw,
            src.y / fh,
            (src.x + src.w) / fw,
            (src.y + src.h) / fh,
        ];
        let tint = style.tint.unwrap_or(Color::WHITE);
        let start = self.instances.len();
        self.push_instance(Instance::solid(
            [dst.x, dst.y],
            [dst.w, dst.h],
            uv,
            [tint.r, tint.g, tint.b, tint.a],
        ));
        self.image_marks.push((start, 1, texture.0));
    }

    fn retain_texture(&mut self, id: y901_ui::image::TextureId) {
        if id.is_valid() {
            self.gfx.retain_image(id.0);
        }
    }

    fn release_texture(&mut self, id: y901_ui::image::TextureId) {
        if id.is_valid() {
            self.gfx.release_image(id.0);
        }
    }

    fn is_hovered(&self, id: &str) -> bool {
        self.state.is_hovered(id)
    }

    fn set_hovered(&mut self, id: Option<&str>) {
        self.state.set_hovered(id);
    }

    fn size(&self) -> Size {
        self.viewport
    }

    /// 主题来自 `Options.theme`（`Window::create` 注入），默认 DARK。
    fn theme(&self) -> &y901_ui::theme::Theme {
        self.state.theme()
    }

    /// 与 `SoftBackend` 同源：面板背景色（Tabs 过渡遮罩等处使用）。
    ///
    /// 旧版缺失时调用方会回退到黑色，于是遮罩在 D3D 上变成一块黑。
    fn background_color(&self) -> Color {
        self.state
            .theme()
            .style(y901_ui::Role::PANEL, y901_ui::theme::State::Normal)
            .bg
    }

    /// 推入本帧动画快照（与 `SoftBackend` 行为一致）。
    fn set_animations(&mut self, snapshot: &y901_ui::animation::AnimSnapshot) {
        self.state.set_animations(snapshot);
    }

    /// 查询动画进度；键不存在时回退到 `1.0`（即“已完成”）。
    fn animation(&self, key: u64) -> f32 {
        self.state.animation(key)
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        // 先过变换栈（空栈时是纯透传），再裁剪到当前视口
        let rect = self.map_rect(rect);
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        self.push_instance(Instance::solid(
            [r.x, r.y],
            [r.w, r.h],
            // 纯色哨兵：uv 两端相等，PS 直接输出 color
            [0.0, 0.0, 0.0, 0.0],
            [color.r, color.g, color.b, color.a],
        ));
    }

    /// GPU 渐变：只提交 **一个** 实例，由像素着色器逐像素插值。
    ///
    /// 为什么不用默认实现（CPU 切条带）：条带在斜向渐变下无法表达
    /// "同一行内颜色也随 x 变化"，只能逐行变色；GPU 实例能做到逐像素精确。
    fn fill_gradient_rect_dir(&mut self, rect: Rect, from: Color, to: Color, dir: (f32, f32)) {
        let rect = self.map_rect(rect);

        let Some(r) = self.clip_of(rect) else {
            return;
        };
        self.push_instance(Instance::gradient_rect(
            [r.x, r.y],
            [r.w, r.h],
            [from.r, from.g, from.b, from.a],
            [to.r, to.g, to.b, to.a],
            [dir.0, dir.1],
        ));
    }

    /// 圆角矩形：GPU SDF（提交 1 个实例，像素着色器算覆盖率）。
    ///
    /// 与 CPU 默认实现的区别：
    /// - CPU 版逐扫描线拼矩形，每行 3 次 draw，且覆盖率是“整行平均”的近似；
    /// - GPU 版逐像素精确 SDF，1 次 draw，且四角与直边连接处天然连续。
    fn fill_round_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        let rect = self.map_rect(rect);
        // 空栈时 `current_scale()` 恒为 1.0，不会引入浮点往返
        let radius = if self.state.transforms_empty() {
            radius
        } else {
            radius * self.current_scale()
        };
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        if radius <= 0.5 {
            self.fill_rect(r, color);
            return;
        }
        self.push_instance(Instance::round_rect(
            [r.x, r.y],
            [r.w, r.h],
            [color.r, color.g, color.b, color.a],
            radius,
        ));
    }

    /// 圆角 + 渐变：GPU 单实例（SDF 裁切 × 渐变插值）。
    fn fill_gradient_round_rect(
        &mut self,
        rect: Rect,
        from: Color,
        to: Color,
        dir: (f32, f32),
        radius: f32,
    ) {
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        if radius <= 0.5 {
            self.fill_gradient_rect_dir(r, from, to, dir);
            return;
        }
        self.push_instance(Instance::gradient_round_rect(
            [r.x, r.y],
            [r.w, r.h],
            [from.r, from.g, from.b, from.a],
            [to.r, to.g, to.b, to.a],
            [dir.0, dir.1],
            radius,
        ));
    }

    /// 圆角描边：GPU SDF 单实例（`|d| <= width/2` 的轮廓带）。
    ///
    /// 与 CPU 兜底的差别：CPU 版用"直边四条 + 四个圆角逐行弦长" 拼接，
    /// 接缝处有瞬断；GPU 版是同一个 `d`，天然连续且 1 次 draw。
    /// 描边：GPU SDF 一次 draw（`|d| <= width/2` 的环）。
    ///
    /// **必须与 `fill_round_rect` 同步处理 transform**：
    /// - `map_rect`：位置与尺寸随变换矩阵走
    /// - `radius * scale`：否则放大后圆角仍是原来那么小
    /// - `width * scale`：否则线宽不跟随缩放（细到不可见）
    ///
    /// 旧版只做了 `clip_of`，上面三件事都没做，于是**有 transform 时描边直接错位**。
    fn stroke_round_rect(&mut self, rect: Rect, color: Color, width: f32, radius: f32) {
        if width <= 0.0 {
            return;
        }
        let rect = self.map_rect(rect);
        let (radius, width) = if self.state.transforms_empty() {
            (radius, width)
        } else {
            let s = self.state.current_scale();
            (radius * s, width * s)
        };
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        self.push_instance(Instance::round_stroke(
            [r.x, r.y],
            [r.w, r.h],
            [color.r, color.g, color.b, color.a],
            width,
            radius,
        ));
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.fill_rect(Rect::new(rect.x, rect.y, rect.w, width), color);
        self.fill_rect(
            Rect::new(rect.x, rect.bottom() - width, rect.w, width),
            color,
        );
        self.fill_rect(Rect::new(rect.x, rect.y, width, rect.h), color);
        self.fill_rect(
            Rect::new(rect.right() - width, rect.y, width, rect.h),
            color,
        );
    }

    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
        // **per-text 字号路径**：与 soft 同构。`size: None` 走下面的**原路径**（逐字未动）。
        if let Some(px) = style.size {
            if px.is_finite() && px > 0.0 {
                return self.draw_text_sized(origin, text, style, px);
            }
        }
        let mut pen_x = origin.x;
        for ch in text.chars() {
            // 格数只有一个来源（`y901_core::Grid::width_of_char` 经 `cell_span`），不做二次加工；
            // `span = 0` 时仍绘制字形、推进 0（组合字符需叠在前字上）。
            let span = self.text.cell_span(ch) as f32;
            // 背景（如果有）
            if style.bg.a > 0.0 {
                self.fill_rect(
                    Rect::new(pen_x, origin.y, span * self.cell.w, self.cell.h),
                    style.bg,
                );
            }
            if ch != ' ' {
                self.push_glyph(pen_x, origin.y, span, ch, style.fg, style.bold);
            }
            if style.underline {
                self.fill_rect(
                    Rect::new(pen_x, origin.y + self.cell.h - 2.0, span * self.cell.w, 1.0),
                    style.fg,
                );
            }
            pen_x += span * self.cell.w;
        }
        pen_x - origin.x
    }

    fn push_transform(&mut self, origin: Pos, scale: f32) {
        // 与 SoftBackend 一致：非法 scale（NaN / ≤ 0）**不压栈**，
        // 否则会把整个子树的坐标变成 NaN 或反向，而那是不可视且难排查的。
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }
        self.state.push_transform(origin, scale);
    }

    fn pop_transform(&mut self) {
        self.state.pop_transform();
    }

    /// 压入一层不透明度（委托给 RenderState，嵌套乘积）。
    fn push_opacity(&mut self, opacity: f32) {
        self.state.push_opacity(opacity);
    }

    /// 弹出栈顶不透明度。
    fn pop_opacity(&mut self) {
        self.state.pop_opacity();
    }

    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.state.current_clip() {
            Some(outer) => rect.intersect(outer),
            None => rect,
        };
        self.state.push_clip(clipped);
    }

    fn pop_clip(&mut self) {
        self.state.pop_clip();
    }
}

/// 一个带纹理坐标的矩形（供裁剪使用）。
struct ClippedQuad {
    pos: Pos,
    size: [f32; 2],
    uv: [f32; 4],
}

/// 把带 UV 的矩形裁到 `clip` 内，并按比例重映射 UV。
///
/// 输入：`pos`/`size`（屏幕矩形）、`uv = [u0, v0, u1, v1]`、`clip`（裁剪框）。
/// 返回：完全在框外时 `None`；否则返回裁剪后的矩形与重映射的 UV。
/// 失败：无（纯几何运算，不 panic）。
///
/// 为什么需要：字形以实例矩形绘制，`clip_of` 只服务于 `fill_rect`。
/// 不做这一步，越界字形会画到客户区外（终端多算一行时表现为
/// "文字画到标题栏缝隙里"）。
fn clip_quad(pos: Pos, size: [f32; 2], uv: [f32; 4], clip: Rect) -> Option<ClippedQuad> {
    if size[0] <= 0.0 || size[1] <= 0.0 {
        return None;
    }
    let quad = Rect::new(pos.x, pos.y, size[0], size[1]);
    let inter = quad.intersect(clip);
    if inter.is_empty() {
        return None;
    }
    // 比例：裁剪后占原来的多少
    let sx = inter.w / quad.w;
    let sy = inter.h / quad.h;
    let du = uv[2] - uv[0];
    let dv = uv[3] - uv[1];
    // 相对于原矩形左上角的偏移比例
    let ox = (inter.x - quad.x) / quad.w;
    let oy = (inter.y - quad.y) / quad.h;
    Some(ClippedQuad {
        pos: Pos::new(inter.x, inter.y),
        size: [inter.w, inter.h],
        uv: [
            uv[0] + du * ox,
            uv[1] + dv * oy,
            uv[0] + du * (ox + sx),
            uv[1] + dv * (oy + sy),
        ],
    })
}

/// 把已映射到屏幕坐标的字形四边形左上角**吸附到整像素**。
///
/// 与 [`glyph_top`] 同一个理由：位图 1:1、采样器线性，分数位置即糊。
/// 只吸附字形（带 UV 采样），纯色矩形与 SDF 圆角**不吸附** ——
/// 它们靠覆盖率公式做抗锯齿，保留小数位置才有平滑过渡。
#[inline]
fn snap_glyph_pos(p: Pos) -> Pos {
    Pos::new(p.x.round(), p.y.round())
}

/// 字形在格子里的水平居中偏移。
///
/// `span * cell_w` 是"该字形应占的宽度"（格契约），`advance` 是字体实际推进宽度。
/// 二者不精确相等时（跨平台常见：后备字体、合成字形），把差值的一半作为左偏移，
/// 让字形居中而不是贴左。
///
/// 精确 2:1 的字体（如 NSimSun 16px）会得到 0.0 —— Windows 外观不变。
pub(crate) fn glyph_offset(span: f32, cell_w: f32, advance: f32) -> f32 {
    (span * cell_w - advance) * 0.5
}

#[cfg(test)]
mod glyph_geometry_tests {
    use super::{glyph_offset, glyph_top, snap_glyph_pos};
    use y901_ui::Pos;

    /// D3D quad top must follow the shared bottom-bearing convention, including descenders.
    #[test]
    fn glyph_top_uses_y_up_bottom_bearing() {
        // origin_y + ascent 就是基线；这里 ascent=0，基线=50
        assert_eq!(glyph_top(50.0, 0.0, 3.0, 12.0), 35.0);
        assert_eq!(glyph_top(50.0, 0.0, -2.0, 15.0), 37.0);
    }

    /// **清晰度回归守卫**：分数基线必须被吸附成整数顶部。
    ///
    /// 背景（实测）：125% 缩放 + 字号 16 → 物理 20px 时 `ascent = 13.75`，
    /// 旧实现让四边形顶部落在 `x.75`，线性采样把竖笔摊成 3 个像素、
    /// 峰值亮度只有正文色的 57%（"字体发糊"）。
    #[test]
    fn fractional_baseline_snaps_to_integer_top() {
        // 基线 = 50 + 13.75 = 63.75 → round = 64
        let top = glyph_top(50.0, 13.75, 0.0, 13.0);
        assert_eq!(top, 51.0);
        assert_eq!(top.fract(), 0.0, "字形顶部必须是整数，实际 {top}");
    }

    /// 与 `y901_render::soft` 的 `glyph_top` **同一条整数基线**：
    /// 两个后端对同一棵树必须给出相同的顶部，否则软/硬渲染会错行。
    #[test]
    fn matches_soft_backend_baseline() {
        for (origin_y, ascent, bearing_y, height) in [
            (50.0_f32, 13.75_f32, 0.0_f32, 13.0_f32),
            (0.0, 17.19, -2.0, 20.0),
            (24.5, 10.4, 1.0, 15.0),
            (7.25, 20.0, -4.0, 24.0),
        ] {
            let d3d = glyph_top(origin_y, ascent, bearing_y, height);
            // soft 的公式：round(基线) − bearing − height
            let soft = (origin_y + ascent).round() - bearing_y - height;
            assert_eq!(d3d, soft, "软/硬基线不一致：{origin_y} + {ascent}");
            assert_eq!(d3d.fract(), 0.0, "顶部必须为整数");
        }
    }

    /// 字形四边形左上角吸附到最近的整像素（不吸附尺寸，尺寸由字号决定）。
    #[test]
    fn glyph_pos_snaps_to_pixel_grid() {
        let snapped = snap_glyph_pos(Pos::new(66.25, 63.75));
        assert_eq!((snapped.x, snapped.y), (66.0, 64.0));
        // 已经是整数时必须原样返回（避免多余浮点往返让哈希漂移）
        let exact = snap_glyph_pos(Pos::new(53.0, 144.0));
        assert_eq!((exact.x, exact.y), (53.0, 144.0));
    }

    /// 精确 2:1：offset 必须为 0（Windows 现有外观的回归保证）。
    #[test]
    fn exact_2to1_has_zero_offset() {
        assert_eq!(glyph_offset(2.0, 8.0, 16.0), 0.0);
        assert_eq!(glyph_offset(1.0, 8.0, 8.0), 0.0);
    }

    /// advance 偏小 10%（14.4 vs 16）：offset 为正，且字形居中在 2 格内。
    #[test]
    fn cjk_offset_centers_glyph_in_two_cells() {
        let cell_w = 8.0;
        let advance = 1.8 * cell_w; // 14.4
        let off = glyph_offset(2.0, cell_w, advance);
        assert!(off > 0.0, "advance 偏小时 offset 应为正，实际 {off}");
        assert!(
            (off - (2.0 * cell_w - advance) / 2.0).abs() < 1e-5,
            "offset 必须是差值的一半"
        );
        // 字形左边界 = 0 + off，墨迹宽 = advance，中心应落在 2 格中点 (8.0)
        let center = off + advance / 2.0;
        assert!(
            (center - cell_w).abs() < 1e-4,
            "字形中心应落在 2 格中点 {cell_w}，实际 {center}"
        );
    }

    /// advance 偏大（17 vs 16）：offset 为负，同样居中。
    #[test]
    fn oversized_advance_gets_negative_offset() {
        let off = glyph_offset(2.0, 8.0, 17.0);
        assert!(off < 0.0);
        let center = off + 17.0 / 2.0;
        assert!((center - 8.0).abs() < 1e-4, "仍应居中，实际 {center}");
    }
}

#[cfg(test)]
mod gfx_layout_tests {
    /// `GfxInstance` 与 GPU 顶点结构必须逐字节一致。
    ///
    /// 布局错位的后果不是"画面难看"，而是**整屏乱码**——
    /// 这是本仓库里少数必须用测试钉死的不变量。
    #[test]
    fn gfx_instance_layout_matches_d3d_instance() {
        use y901_runtime::gfx::GfxInstance;
        assert_eq!(
            std::mem::size_of::<GfxInstance>(),
            std::mem::size_of::<super::Instance>(),
            "GfxInstance 与 d3d11::Instance 大小不一致"
        );
        // 96 = pos(8) + size(8) + uv(16) + color(16) + color_end(16) + gradient(16) + params(16)
        assert_eq!(std::mem::size_of::<GfxInstance>(), 96);
        // 大小相同但字段错位同样会乱码：逐个字段喂不同值再比对
        let a = GfxInstance {
            pos: [1.0, 2.0],
            size: [3.0, 4.0],
            uv_rect: [5.0, 6.0, 7.0, 8.0],
            color: [9.0, 10.0, 11.0, 12.0],
            color_end: [13.0, 14.0, 15.0, 16.0],
            gradient: [17.0, 18.0, 1.0, 19.0],
            params: [20.0, 21.0, 22.0, 23.0],
        };
        let b = super::to_gpu_instance(&a);
        assert_eq!(b.pos, a.pos);
        assert_eq!(b.size, a.size);
        assert_eq!(b.uv_rect, a.uv_rect);
        assert_eq!(b.color, a.color);
        assert_eq!(b.color_end, a.color_end, "渐变终点色必须逐字节传递");
        assert_eq!(b.gradient, a.gradient, "渐变参数必须逐字节传递");
        assert_eq!(b.params, a.params, "圆角参数必须逐字节传递");
    }
}

/// 真机测试：设备重建后图集必须被重传。
///
/// 需要真实窗口 + D3D11 设备，CI 跑不了，所以 `#[ignore]`。
/// 手动运行：`cargo test -p y901-backend-win32 -- --ignored rebuild_reuploads_atlas`
#[cfg(test)]
mod rebuild_atlas_tests {
    use y901_runtime::gfx::{PlatformWindowHandle, RenderBackend};

    #[test]
    #[ignore = "需要真实窗口与 D3D11 设备"]
    fn rebuild_reuploads_atlas() {
        let window = match crate::Window::create(&y901_runtime::Options::default()) {
            Ok(w) => w,
            Err(err) => {
                eprintln!("[skip] 无法创建窗口（需要交互式桌面）：{err}");
                return;
            }
        };
        let hwnd = window.hwnd();
        let handle = PlatformWindowHandle::Win32(hwnd);
        let mut backend = match <super::D3dBackend as RenderBackend>::create(handle, 800, 600) {
            Ok(b) => b,
            Err(err) => {
                // 无 GPU 访问（远程会话 / 权限受限）时跳过：这不是产品缺陷
                eprintln!("[skip] 无法创建 D3D11 设备：{err}");
                return;
            }
        };
        assert!(backend.gfx.has_atlas(), "初次创建后应已有图集");

        backend.rebuild(800, 600).expect("rebuild");
        assert!(
            backend.gfx.has_atlas(),
            "rebuild 后必须重新上传图集，否则采样空纹理"
        );
    }
}

#[cfg(test)]
mod rebuild_source_tests {
    /// 结构测试：`rebuild` 方法体里必须调用 `update_atlas`。
    ///
    /// 为什么不是行为测试：真实 rebuild 需要 GPU 设备，CI/无显卡环境跑不了。
    /// 这条测试是"误删防线"：有人删掉重传逻辑时它会红。
    /// 已知弱点：改注释也可能骗过它（源码级匹配的通病）。
    #[test]
    fn rebuild_calls_update_atlas() {
        let src = include_str!("mod.rs");
        let start = src.find("fn rebuild(&mut self").expect("rebuild 方法存在");
        // 方法体到下一个同级方法（4 空格缩进的 `fn `）为止
        let rest = &src[start..];
        let end = rest[1..]
            .find("\n    fn ")
            .map(|i| i + 1)
            .unwrap_or(rest.len());
        let body = &rest[..end];
        assert!(
            body.contains("update_atlas"),
            "rebuild 必须重传图集，否则设备重建后整屏缺字：\n{body}"
        );
        assert!(
            body.contains("true"),
            "重传时必须 reset=true（全量重传），body:\n{body}"
        );
    }
}

#[cfg(test)]
mod clip_quad_tests {
    use super::clip_quad;
    use y901_ui::{Pos, Rect};

    /// 完全在裁剪框内：原样返回（UV 不变）。
    #[test]
    fn inside_is_unchanged() {
        let q = clip_quad(
            Pos::new(10.0, 10.0),
            [8.0, 16.0],
            [0.0, 0.0, 0.1, 0.2],
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )
        .expect("应保留");
        assert_eq!(q.pos, Pos::new(10.0, 10.0));
        assert_eq!(q.size, [8.0, 16.0]);
        assert_eq!(q.uv, [0.0, 0.0, 0.1, 0.2]);
    }

    /// 完全在框外：丢弃（这就是"文字画到标题栏外"的修复点）。
    #[test]
    fn outside_is_dropped() {
        let q = clip_quad(
            Pos::new(-50.0, -50.0),
            [8.0, 16.0],
            [0.0, 0.0, 0.1, 0.2],
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        assert!(q.is_none(), "框外字形必须丢弃");
    }

    /// 顶部越界一半：高度减半，V 坐标同步减半（UV 重映射正确）。
    #[test]
    fn partial_top_overflow_shrinks_and_remaps_uv() {
        // 字形在 y=-8，高 16 → 上半 8px 在框外
        let q = clip_quad(
            Pos::new(10.0, -8.0),
            [8.0, 16.0],
            [0.0, 0.0, 0.1, 1.0],
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )
        .expect("部分可见应保留");
        assert_eq!(q.pos.y, 0.0, "裁剪后应从 y=0 开始");
        assert!(
            (q.size[1] - 8.0).abs() < 1e-4,
            "高度应减半，实际 {}",
            q.size[1]
        );
        // 原来 v 从 0..1，被切掉上半 → 剩下的 v 是 0.5..1.0
        assert!(
            (q.uv[1] - 0.5).abs() < 1e-4,
            "v0 应为 0.5，实际 {}",
            q.uv[1]
        );
        assert!(
            (q.uv[3] - 1.0).abs() < 1e-4,
            "v1 应为 1.0，实际 {}",
            q.uv[3]
        );
    }

    /// 零尺寸不产生实例（避免除零 / 退化四边形）。
    #[test]
    fn zero_size_is_dropped() {
        assert!(clip_quad(
            Pos::new(0.0, 0.0),
            [0.0, 10.0],
            [0.0, 0.0, 1.0, 1.0],
            Rect::new(0.0, 0.0, 100.0, 100.0)
        )
        .is_none());
    }
}

#[cfg(test)]
mod render_stats_tests {
    use super::{Acc, BackendAcc};
    use std::time::Duration;
    use y901_runtime::render_stats::{BackendKind, RenderStats, StageId, StageTimes};

    // ---- 累计器的三态语义（本轮核心：不得用 `> ZERO` 推断执行）----

    #[test]
    fn acc_not_seen_stays_unmeasured() {
        let acc = Acc::default();
        assert!(!acc.seen());
        let mut times = StageTimes::default();
        acc.flush(&mut times, StageId::Upload);
        assert!(
            times.get(StageId::Upload).is_none(),
            "从未执行过的阶段必须保持 None（不能写 0）"
        );
    }

    #[test]
    fn acc_zero_duration_is_written_as_zero() {
        let mut acc = Acc::default();
        acc.add(Duration::ZERO);
        assert!(acc.seen(), "执行过（哪怕耗时为 0）必须被标记为 seen");
        let mut times = StageTimes::default();
        acc.flush(&mut times, StageId::Upload);
        assert_eq!(
            times.get(StageId::Upload),
            Some(Duration::ZERO),
            "执行了但耗时为零 → 写成 Some(ZERO)，与 None 严格区分"
        );
    }

    #[test]
    fn acc_accumulates_disjoint_intervals() {
        // 模拟设备丢失重试：两段不连续区间必须**相加**，不是取最后一段。
        let mut acc = Acc::default();
        acc.add(Duration::from_micros(7));
        acc.add(Duration::from_micros(11));
        let mut times = StageTimes::default();
        acc.flush(&mut times, StageId::BackendSubmit);
        assert_eq!(
            times.get(StageId::BackendSubmit),
            Some(Duration::from_micros(18))
        );
    }

    #[test]
    fn acc_zero_segment_inside_a_retry_does_not_lose_the_other_segment() {
        // 首次 Present 在设备丢失时立即失败（≈0），第二段才是真实阻塞。
        let mut acc = Acc::default();
        acc.add(Duration::ZERO);
        acc.add(Duration::from_micros(3));
        let mut times = StageTimes::default();
        acc.flush(&mut times, StageId::PresentBlocking);
        assert_eq!(
            times.get(StageId::PresentBlocking),
            Some(Duration::from_micros(3)),
            "0 段不得抹掉另一段"
        );
    }

    // ---- 累计器 → RenderStats 的收口 ----

    #[test]
    fn flush_into_writes_only_executed_stages_and_all_counters() {
        let mut acc = BackendAcc::default();
        acc.submit.add(Duration::from_micros(5));
        acc.atlas_bytes = 4096;
        acc.drawn = Some(12);
        acc.batch_count = Some(3);
        let mut s = RenderStats::new(1, BackendKind::D3d11);
        acc.flush_into(&mut s);

        assert_eq!(
            s.stages.get(StageId::BackendSubmit),
            Some(Duration::from_micros(5))
        );
        // 没执行的两个阶段必须保持 None。
        assert!(s.stages.get(StageId::Upload).is_none());
        assert!(s.stages.get(StageId::PresentBlocking).is_none());
        assert_eq!(s.counters.atlas_upload_bytes, 4096);
        assert_eq!(s.counters.instance_drawn, 12);
        assert_eq!(s.counters.batch_count, 3);
    }

    #[test]
    fn flush_into_never_writes_an_unmeasurable_stage() {
        // 直接证明：累计器不可能把 gpu_execute / compositor 变成 Measured
        //（这两个 StageId 在 flush_into 里根本没出现）。
        let mut acc = BackendAcc::default();
        acc.submit.add(Duration::from_micros(1));
        acc.present.add(Duration::from_micros(2));
        acc.upload.add(Duration::from_micros(3));
        let mut s = RenderStats::new(1, BackendKind::D3d11);
        acc.flush_into(&mut s);
        assert!(s.stages.get(StageId::GpuExecute).is_none());
        assert!(s.stages.get(StageId::Compositor).is_none());
        // 且整份数据能通过自检（说明没有自相矛盾）。
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn acc_is_copy_and_default_is_empty() {
        // 累计器必须能零成本重置（`BackendAcc::default()` 不分配、不读时钟）。
        let acc = BackendAcc {
            submit: {
                let mut a = Acc::default();
                a.add(Duration::from_micros(1));
                a
            },
            ..BackendAcc::default()
        };
        let reset = BackendAcc::default();
        assert!(acc.submit.seen());
        assert!(!reset.submit.seen());
        assert!(!reset.upload.seen());
        assert!(!reset.present.seen());
        assert!(reset.drawn.is_none());
        assert_eq!(reset.atlas_bytes, 0);
    }
}
