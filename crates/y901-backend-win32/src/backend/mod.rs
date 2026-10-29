//! D3D11 后端：`y901_ui::RenderCtx` + `TextMetrics` 的唯一实现。
//!
//! 这一层是框架与 GPU 之间唯一的桥：
//! - `fill_rect` / `draw_text` → 追加到实例数组（同一帧只有一次 DrawInstanced）
//! - DirectWrite 字形 → 图集纹理，增量上传
//!
//! 框架和组件都看不到 D3D11 类型，只看到 `RenderCtx`。

#[allow(dead_code)] // Atlas/Placement 是给多页图集预留的 API，暂由 SkylinePacker 先行使用
pub mod atlas;
pub mod d3d11;
pub mod font;


use windows_sys::Win32::Foundation::HWND;

use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextStyle, TextMetrics};

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

pub struct D3dBackend {
    pub gfx: Gfx,
    text: TextEngine,
    instances: Vec<Instance>,
    viewport: Size,
    clips: Vec<Rect>,
    /// 窗口句柄：`RenderBackend::present` 无参，必须记住它。
    hwnd: HWND,
    /// 当前尺寸：trait 的 `present` 无参，必须记住它。
    size: (u32, u32),
    /// 上一次 `present` 是否被遮挡（D3D11 只能从 `Present` 返回值得知）。
    last_occluded: bool,
    /// 渲染主题（启动时由 Options.theme 注入，None 时用 DARK）。
    theme: y901_ui::theme::Theme,
    /// 当前单元格尺寸（由字号推出；set_font 会更新它）。
    ///
    /// **必须与 TextEngine 使用的 cell 尺寸一致**：measure_text/line_height/glyph_offset/draw_text 都读它。
    cell: Size,
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
        // 字号是全局的（不支持富文本）：cell 尺寸由字号推出，见 cell_metrics_for。
        let (cell_w, cell_h) = crate::backend::font::cell_metrics_for(font.size);
        let text = TextEngine::new(&font.family, font.size, cell_w, cell_h)?;
        eprintln!(
            "[y901] backend ready: ascent={:.2} descent={:.2} atlas={}",
            text.ascent, text.descent, text.atlas_size
        );

        let mut backend = Self {
            gfx,
            text,
            instances: Vec::with_capacity(4096),
            viewport: Size::new(width as f32, height as f32),
            clips: Vec::new(),
            hwnd,
            size: (width, height),
            last_occluded: false,
            theme,
            cell: Size::new(cell_w, cell_h),
        };
        backend.sync_atlas()?;
        Ok(backend)
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
    pub fn set_theme(&mut self, theme: y901_ui::theme::Theme) -> Result<(), String> {
        self.theme = theme;
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
        self.text = text;
        self.cell = Size::new(cell_w, cell_h);
        Ok(())
    }

    /// 每帧开始：清空实例数组，并推进图集帧号（LRU 用）。
    pub fn begin(&mut self) {
        self.instances.clear();
        // **基座裁剪 = 客户区**：之后所有 fill_rect / 字形都必须落在窗口内。
        // 少了这一层，组件只要把内容画到负坐标（例如终端多算了一行），
        // 就会画到窗口标题栏缝隙里。
        self.clips.clear();
        self.clips.push(Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h));
        self.text.begin_frame();
    }

    /// 把 CPU 图集的脏区同步到 GPU。首次调用会创建纹理。
    unsafe fn sync_atlas(&mut self) -> Result<(), String> {
        let (reset, dirty) = self.text.take_dirty();
        if !reset && dirty.is_empty() {
            return Ok(());
        }
        let rects: Vec<(u32, u32, u32, u32)> = dirty
            .iter()
            .map(|d| (d.x, d.y, d.width, d.height))
            .collect();
        self.gfx
            .update_atlas(&self.text.atlas, self.text.atlas_size, reset, &rects)
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
    pub fn set_dirty_bounds(&mut self, rect: Option<(i32, i32, i32, i32)>) {
        self.gfx.set_dirty_bounds(rect);
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
    /// 或尺寸反复不一致造成资源重建风暴。设备丢失属于预期，返回 `false` 而非崩溃。
    pub unsafe fn present(&mut self, hwnd: HWND, width: u32, height: u32) -> bool {
        if self.gfx.is_empty() {
            if let Err(err) = self.gfx.rebuild(hwnd, width, height) {
                eprintln!("[y901] rebuild failed: {err}");
                return false;
            }
        }

        if let Err(err) = self.sync_atlas() {
            eprintln!("[y901] atlas sync failed: {err}");
        }

        // sync_interval = 0：不阻塞在 vsync 上。
        // 我们是按需重绘（invalidate 驱动），不需要垂直同步；
        // 用 1 时窗口被遮挡会让 Present 永久阻塞，把消息循环卡死。
        let mut hr = self.gfx.render(&self.instances, 0);

        if Gfx::device_lost(hr) {
            eprintln!("[y901] device lost: 0x{:08X}, rebuilding", hr as u32);
            if let Err(err) = self.gfx.rebuild(hwnd, width, height) {
                eprintln!("[y901] rebuild failed: {err}");
                return false;
            }
            // 设备重建后纹理是空的：全量重传
            if let Err(err) = self
                .gfx
                .update_atlas(&self.text.atlas, self.text.atlas_size, true, &[])
            {
                eprintln!("[y901] atlas re-upload failed: {err}");
            }
            hr = self.gfx.render(&self.instances, 0);
        }

        if hr < 0 {
            eprintln!("[y901] Present failed: 0x{:08X}", hr as u32);
            return false;
        }
        !Gfx::is_occluded(hr)
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
    pub unsafe fn resize(&mut self, width: u32, height: u32) {
        self.viewport = Size::new(width as f32, height as f32);
        // 交换链尺寸更新交给 present 前的 rebuild 路径；这里只记尺寸。
        if let Err(err) = self.gfx.resize(width, height) {
            eprintln!("[y901] swapchain resize failed: {err}");
        }
    }

    /// 当前帧的实例数（诊断用）。
    #[allow(dead_code)] // 诊断接口，主循环不调用
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    #[allow(dead_code)] // 诊断接口，主循环不调用
    pub fn live_objects() -> isize {
        Gfx::live_objects()
    }

    fn clip_of(&self, rect: Rect) -> Option<Rect> {
        match self.clips.last() {
            Some(clip) => {
                let r = rect.intersect(*clip);
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
    fn push_glyph(&mut self, x: f32, y: f32, span: f32, ch: char, color: Color, bold: bool) {
        let atlas_size = self.text.atlas_size as f32;
        let ascent = self.text.ascent;

        let Some(glyph) = self.text.glyph(ch).cloned() else {
            return;
        };
        if glyph.is_empty() {
            return;
        }

        let offset_x = glyph_offset(span, self.cell.w, glyph.advance);
        let pos = Pos::new(
            x + offset_x + glyph.bearing_x as f32,
            y + ascent + glyph.bearing_y as f32,
        );
        let size = [glyph.width as f32, glyph.height as f32];
        let uv = [
            glyph.atlas_x as f32 / atlas_size,
            glyph.atlas_y as f32 / atlas_size,
            (glyph.atlas_x + glyph.width) as f32 / atlas_size,
            (glyph.atlas_y + glyph.height) as f32 / atlas_size,
        ];
        let rgba = [color.r, color.g, color.b, color.a];

        // 字形也要裁剪：`clip_of` 只作用于 fill_rect，字形是带 UV 的矩形，
        // 必须按裁剪框缩放并重映射 UV（否则会画到客户区外，见 §34）。
        let Some(clip) = self.clips.last().copied() else {
            return;
        };
        let Some(quad) = clip_quad(pos, size, uv, clip) else {
            return;
        };

        self.instances.push(Instance {
            pos: [quad.pos.x, quad.pos.y],
            size: quad.size,
            uv_rect: quad.uv,
            color: rgba,
                ..Default::default()
            });

        // 假粗体：右移 1px 再画一遍（比加载粗体字形便宜，且等宽不变）
        if bold {
            self.instances.push(Instance {
                pos: [pos.x + 1.0, pos.y],
                size,
                uv_rect: uv,
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
        unsafe { self.gfx.update_atlas(&self.text.atlas, self.text.atlas_size, true, &[]) }
            .map_err(|err| format!("rebuild 后图集重传失败: {err}"))?;
        Ok(())
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        // SAFETY: 同 rebuild；尺寸来自当前客户区。
        unsafe { D3dBackend::resize(self, width, height) };
        self.size = (width, height);
        Ok(())
    }

    fn present(&mut self, frame: &y901_runtime::gfx::Frame) -> Result<(), String> {
        // 1) 用本帧实例替换内部缓冲（D3D11 的 present 从 self.instances 取数据）
        self.instances.clear();
        self.instances
            .extend(frame.instances.iter().map(to_gpu_instance));

        // 2) 图集：D3dBackend 的图集由 TextEngine 自己管理（sync_atlas 内部取脏区），
        //    因此 `frame.atlas_updates` 在这里不被使用。这是当前实现的已知偏差：
        //    平台无关后端（wgpu 等）会用它们；D3D11 后端不需要。
        //    TODO(§18)：抽 GlyphProvider 时把图集管理统一到 trait。
        //
        // 图集未同步 = 采样空纹理 = 整屏缺字。debug 断言 + release 只警告一次。
        if !self.gfx.has_atlas() && !self.text.atlas.is_empty() {
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
        let ok = unsafe { D3dBackend::present(self, self.hwnd, w, h) };
        self.last_occluded = !ok;
        if ok {
            Ok(())
        } else {
            // false 可能表示"被遮挡"（非错误）或"设备丢失且重建失败"。
            // 调用方用 `is_occluded()` 区分。
            Err("present 未完成（遮挡或设备丢失，见 is_occluded）".to_string())
        }
    }

    fn is_occluded(&self) -> bool {
        self.last_occluded
    }

    fn instance_count(&self) -> usize {
        self.instances.len()
    }
}

impl TextMetrics for D3dBackend {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        let cols: u32 = text.chars().map(|c| self.text.cell_span(c)).sum();
        Size::new(cols as f32 * self.cell.w, self.cell.h)
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        self.cell.h
    }

    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for D3dBackend {
    fn size(&self) -> Size {
        self.viewport
    }

    /// 主题来自 `Options.theme`（`Window::create` 注入），默认 DARK。
    fn theme(&self) -> &y901_ui::theme::Theme {
        &self.theme
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        self.instances.push(Instance::solid(
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
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        self.instances.push(Instance::gradient_rect(
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
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        if radius <= 0.5 {
            self.fill_rect(r, color);
            return;
        }
        self.instances.push(Instance::round_rect(
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
        self.instances.push(Instance::gradient_round_rect(
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
    fn stroke_round_rect(&mut self, rect: Rect, color: Color, width: f32, radius: f32) {
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        if width <= 0.0 {
            return;
        }
        self.instances.push(Instance::round_stroke(
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
        let mut pen_x = origin.x;
        for ch in text.chars() {
            let span = self.text.cell_span(ch).max(1) as f32;
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

    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.clips.last() {
            Some(outer) => rect.intersect(*outer),
            None => rect,
        };
        self.clips.push(clipped);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
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
mod glyph_offset_tests {
    use super::glyph_offset;

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
        assert!((q.size[1] - 8.0).abs() < 1e-4, "高度应减半，实际 {}", q.size[1]);
        // 原来 v 从 0..1，被切掉上半 → 剩下的 v 是 0.5..1.0
        assert!((q.uv[1] - 0.5).abs() < 1e-4, "v0 应为 0.5，实际 {}", q.uv[1]);
        assert!((q.uv[3] - 1.0).abs() < 1e-4, "v1 应为 1.0，实际 {}", q.uv[3]);
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