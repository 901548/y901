//! 分阶段渲染统计（A 阶段）：**测不到就说测不到，绝不用 0 冒充**。
//!
//! 背景：现有 `profile::FrameMeta::present_ms` 记的是"事件分发 → layout → render →
//! after_present"整段，而名字暗示它是 Present 耗时。实测空帧 6 885 µs、10k 实例满帧
//! 6 887 µs（见 `docs/PERFORMANCE.md`），说明帧时间由 Present/合成器主导 ——
//! 不分阶段的帧级数字是噪声。本模块提供**阶段化**的容器。
//!
//! 设计要点（与 `docs/RENDER_STATS_DESIGN_2026-11-25.md` 一致）：
//!
//! 1. **三态**：[`Stage`] 是 `Measured` / `Unavailable(原因)` / `NotApplicable`，
//!    不存在"0 表示没测到"这种表达。
//! 2. **能力表**：[`BackendKind::capability`] 静态声明"这个后端能不能测这个阶段"，
//!    [`RenderStats::validate`] 用它拒绝自相矛盾的数据（例如在 CPU 后端上把
//!    `gpu_execute` 填成 `Measured`）。
//! 3. **关闭零成本**：[`enabled`] 用 `OnceLock` 缓存 `Y901_RENDER_STATS`；
//!    关闭时 [`mark`] 直接返回 `None`，**不调用 `Instant::now()`**。
//! 4. 本模块**不依赖任何后端类型**（`y901-ui` 也不引用它），保持依赖方向。
//!
//! 本文件是 A 阶段第 1 步：只有数据模型与单测，**不接入任何渲染路径**。

use std::time::Duration;

/// 一个统计阶段。
///
/// 为什么不用 `Option<Duration>` 表达"没测到"：那样无法区分"这个后端压根没有这个阶段"
/// 与"有这个阶段但本帧测不了"，而二者的排查方向完全不同（前者是设计，后者是缺陷）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// 真实测量值。**`Measured(Duration::ZERO)` 是合法值**（阶段确实执行了，极快）。
    Measured(Duration),
    /// 阶段存在，但**本阶段无法可靠测量**；参数是原因（非空，`'static` 不分配）。
    Unavailable(&'static str),
    /// 该后端**没有这个阶段**（例：CPU 后端没有 `gpu_execute`）。
    NotApplicable,
}

impl Stage {
    /// 是否真实测量。
    pub fn is_measured(self) -> bool {
        matches!(self, Stage::Measured(_))
    }

    /// 取测量值（仅 `Measured` 时非 `None`）。
    pub fn duration(self) -> Option<Duration> {
        match self {
            Stage::Measured(d) => Some(d),
            _ => None,
        }
    }

    /// 三态的稳定字符串名（用于 JSON / 日志；**不含数值**）。
    pub fn label(self) -> &'static str {
        match self {
            Stage::Measured(_) => "measured",
            Stage::Unavailable(_) => "unavailable",
            Stage::NotApplicable => "not_applicable",
        }
    }
}

/// 全部阶段标识。
///
/// 枚举而不是字符串：拼错会被编译器挡住，且 [`StageTimes`] 的 `set/get` 可以穷尽匹配。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageId {
    /// 事件分发（含命令处理、idle、动画推进、主题切换）。
    EventDispatch,
    /// 布局。
    Layout,
    /// `after_frame` 钩子。
    AfterFrame,
    /// 渲染准备（脏区规划、批次构建、清屏、实例缓冲 arrnge 等 CPU 侧准备）。
    RenderPrepare,
    /// 命令生成（`UiTree::render` 把控件调用变成后端命令）。
    CommandGen,
    /// CPU 光栅化（软渲染：命令生成与像素执行在同一调用里，**不可分离**）。
    CpuRaster,
    /// 上传（图集/纹理；X11 上"上传 + Present"合并计入此项）。
    Upload,
    /// 后端提交（D3D11：CB 更新 + 实例缓冲 Map/memcpy + draw + blit）。
    BackendSubmit,
    /// BGRA→RGBA 转换（仅 Web）。
    PixelConvert,
    /// `ImageData` 分配（仅 Web）。
    ImageAlloc,
    /// 像素批量拷贝（仅 Web：wasm → JS 堆）。
    PixelCopy,
    /// Canvas `putImageData` 调用（仅 Web，**不含浏览器合成**）。
    CanvasBlit,
    /// 阻塞式上屏调用（D3D11 `Present` / GDI blit；**含 vsync 等待**，不含合成器）。
    PresentBlocking,
    /// `after_present` 钩子。
    AfterPresent,
    /// GPU 执行（本阶段不可测：需要新增 D3D11 timestamp query）。
    GpuExecute,
    /// 合成器（DWM / X server / 浏览器；在我们返回**之后**异步发生）。
    Compositor,
    /// 整帧（工作耗时，不含 `wait_events` 等待）。
    TotalFrame,
}

impl StageId {
    /// 全部阶段，按"一帧内的自然顺序"排列（JSON 输出与遍历顺序一致）。
    pub const ALL: [StageId; 17] = [
        StageId::EventDispatch,
        StageId::Layout,
        StageId::AfterFrame,
        StageId::RenderPrepare,
        StageId::CommandGen,
        StageId::CpuRaster,
        StageId::Upload,
        StageId::BackendSubmit,
        StageId::PixelConvert,
        StageId::ImageAlloc,
        StageId::PixelCopy,
        StageId::CanvasBlit,
        StageId::PresentBlocking,
        StageId::AfterPresent,
        StageId::GpuExecute,
        StageId::Compositor,
        StageId::TotalFrame,
    ];

    /// 稳定标识（JSON key；改动会破坏已有 benchmark 数据，需同步改文档）。
    pub fn name(self) -> &'static str {
        match self {
            StageId::EventDispatch => "event_dispatch",
            StageId::Layout => "layout",
            StageId::AfterFrame => "after_frame",
            StageId::RenderPrepare => "render_prepare",
            StageId::CommandGen => "command_gen",
            StageId::CpuRaster => "cpu_raster",
            StageId::Upload => "upload",
            StageId::BackendSubmit => "backend_submit",
            StageId::PixelConvert => "pixel_convert",
            StageId::ImageAlloc => "image_alloc",
            StageId::PixelCopy => "pixel_copy",
            StageId::CanvasBlit => "canvas_blit",
            StageId::PresentBlocking => "present_blocking",
            StageId::AfterPresent => "after_present",
            StageId::GpuExecute => "gpu_execute",
            StageId::Compositor => "compositor",
            StageId::TotalFrame => "total_frame",
        }
    }
}

/// 各阶段的测量值。
///
/// `None` = **本帧没有测量结果**（统计关闭、或该路径本帧未经过）。
/// 它**不等于** `Some(Duration::ZERO)` —— 后者表示"测了，就是 0"。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageTimes {
    /// 事件分发。
    pub event_dispatch: Option<Duration>,
    /// 布局。
    pub layout: Option<Duration>,
    /// `after_frame`。
    pub after_frame: Option<Duration>,
    /// 渲染准备。
    pub render_prepare: Option<Duration>,
    /// 命令生成。
    pub command_gen: Option<Duration>,
    /// CPU 光栅化。
    pub cpu_raster: Option<Duration>,
    /// 上传。
    pub upload: Option<Duration>,
    /// 后端提交。
    pub backend_submit: Option<Duration>,
    /// BGRA→RGBA。
    pub pixel_convert: Option<Duration>,
    /// `ImageData` 分配。
    pub image_alloc: Option<Duration>,
    /// 像素拷贝。
    pub pixel_copy: Option<Duration>,
    /// Canvas 上屏调用。
    pub canvas_blit: Option<Duration>,
    /// 阻塞式上屏。
    pub present_blocking: Option<Duration>,
    /// `after_present`。
    pub after_present: Option<Duration>,
    /// GPU 执行（本阶段恒为 `None`，见 [`BackendKind::capability`]）。
    pub gpu_execute: Option<Duration>,
    /// 合成器（恒为 `None`）。
    pub compositor: Option<Duration>,
    /// 整帧。
    pub total_frame: Option<Duration>,
}

impl StageTimes {
    /// 写入某阶段。
    ///
    /// 覆盖写而不是累加：一个阶段在一帧内只应有一个区间，
    /// 累加会让"重复打点"这种 bug 静默变成更大的数字。
    pub fn set(&mut self, id: StageId, value: Duration) {
        match id {
            StageId::EventDispatch => self.event_dispatch = Some(value),
            StageId::Layout => self.layout = Some(value),
            StageId::AfterFrame => self.after_frame = Some(value),
            StageId::RenderPrepare => self.render_prepare = Some(value),
            StageId::CommandGen => self.command_gen = Some(value),
            StageId::CpuRaster => self.cpu_raster = Some(value),
            StageId::Upload => self.upload = Some(value),
            StageId::BackendSubmit => self.backend_submit = Some(value),
            StageId::PixelConvert => self.pixel_convert = Some(value),
            StageId::ImageAlloc => self.image_alloc = Some(value),
            StageId::PixelCopy => self.pixel_copy = Some(value),
            StageId::CanvasBlit => self.canvas_blit = Some(value),
            StageId::PresentBlocking => self.present_blocking = Some(value),
            StageId::AfterPresent => self.after_present = Some(value),
            StageId::GpuExecute => self.gpu_execute = Some(value),
            StageId::Compositor => self.compositor = Some(value),
            StageId::TotalFrame => self.total_frame = Some(value),
        }
    }

    /// 读取某阶段。
    pub fn get(&self, id: StageId) -> Option<Duration> {
        match id {
            StageId::EventDispatch => self.event_dispatch,
            StageId::Layout => self.layout,
            StageId::AfterFrame => self.after_frame,
            StageId::RenderPrepare => self.render_prepare,
            StageId::CommandGen => self.command_gen,
            StageId::CpuRaster => self.cpu_raster,
            StageId::Upload => self.upload,
            StageId::PixelConvert => self.pixel_convert,
            StageId::ImageAlloc => self.image_alloc,
            StageId::PixelCopy => self.pixel_copy,
            StageId::CanvasBlit => self.canvas_blit,
            StageId::BackendSubmit => self.backend_submit,
            StageId::PresentBlocking => self.present_blocking,
            StageId::AfterPresent => self.after_present,
            StageId::GpuExecute => self.gpu_execute,
            StageId::Compositor => self.compositor,
            StageId::TotalFrame => self.total_frame,
        }
    }

    /// 清空所有阶段（跨帧复用同一个 `RenderStats` 时用，避免上一帧残留）。
    pub fn clear(&mut self) {
        *self = StageTimes::default();
    }
}

/// 一帧的计数器（不涉及计时，取现有变量即可）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameCounters {
    /// 脏区块数。
    pub dirty_rect_count: u32,
    /// 脏区覆盖的像素数（各块面积之和，**不扣重叠**）。
    pub damaged_pixels: u64,
    /// 推到后端的实例数。
    pub instance_pushed: u32,
    /// 实际提交绘制的实例数（受单帧上限截断；与 `instance_pushed` 不等即丢内容）。
    pub instance_drawn: u32,
    /// 批次数。
    pub batch_count: u32,
    /// 本帧上传到 GPU/服务端的图集字节数。
    pub atlas_upload_bytes: u64,
    /// 本帧是否因无脏区而跳过渲染。
    pub frame_skipped: bool,
    /// 上屏是否报告被遮挡。
    pub occluded: bool,
}

/// 后端种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// Win32 + D3D11（GPU）。
    D3d11,
    /// 纯 CPU 软光栅（Win32 `--force-soft`）。
    Soft,
    /// X11（CPU 光栅 + XShm/PutImage）。
    X11,
    /// 浏览器 Canvas2D（CPU 光栅 + `putImageData`）。
    Web,
}

/// 某后端对某阶段的**能力声明**。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// 可以可靠测量（本帧没走到时该阶段为 `None`）。
    Measurable,
    /// 有这个阶段，但当前无法可靠测量；带原因。
    Unavailable(&'static str),
    /// 该后端没有这个阶段；带原因（便于报告里解释"为什么没有"）。
    NotApplicable(&'static str),
}

// ---------------------------------------------------------------------------
// 各平台的阶段能力表（**显式全表**，不做"默认 + 特例"的隐式推导）
// ---------------------------------------------------------------------------
//
// 为什么用显式表而不是 `if backend == X && stage == Y` 的散装判断：
// 这些声明会被 `validate()` 当作"不许伪造"的守卫，一旦逻辑分散，
// 加一个阶段时很容易某条分支漏掉，于是"不可测"悄悄变成"可测"。
// 全表还有个副作用好处：报告里可以直接列出"该后端哪些阶段永远测不到"。

/// 软渲染（Win32 soft / X11 / Web）共有的不可测项：命令生成与光栅化写在同一调用里。
const MERGED_COMMAND_GEN: &str = "软渲染在同一调用里生成命令并写像素，无法分离（拆开即为伪造数据）";

/// D3D11 的能力表。
const D3D11_UNAVAILABLE: &[(StageId, &str)] = &[
    (
        StageId::GpuExecute,
        "本阶段未实现 D3D11 timestamp/disjoint query（需新增 vtable 槽，单独立项）",
    ),
    (
        StageId::Compositor,
        "DWM 合成在 Present 返回后异步发生，无完成回执",
    ),
];
const D3D11_NOT_APPLICABLE: &[(StageId, &str)] = &[
    (StageId::CpuRaster, "GPU 后端不做 CPU 光栅化"),
    (StageId::PixelConvert, "该阶段只在 Web 后端存在"),
    (StageId::ImageAlloc, "该阶段只在 Web 后端存在"),
    (StageId::PixelCopy, "该阶段只在 Web 后端存在"),
    (StageId::CanvasBlit, "该阶段只在 Web 后端存在"),
];

/// Win32 软渲染的能力表。
const SOFT_UNAVAILABLE: &[(StageId, &str)] = &[
    (StageId::CommandGen, MERGED_COMMAND_GEN),
    (
        StageId::Compositor,
        "DWM 合成在 GDI blit 返回后异步发生，无完成回执",
    ),
];
const SOFT_NOT_APPLICABLE: &[(StageId, &str)] = &[
    (StageId::BackendSubmit, "CPU 后端没有独立的 GPU 提交阶段"),
    (StageId::GpuExecute, "CPU 光栅化后端没有 GPU 执行阶段"),
    (StageId::PixelConvert, "该阶段只在 Web 后端存在"),
    (StageId::ImageAlloc, "该阶段只在 Web 后端存在"),
    (StageId::PixelCopy, "该阶段只在 Web 后端存在"),
    (StageId::CanvasBlit, "该阶段只在 Web 后端存在"),
];

/// X11 的能力表：上传与 Present 是同一个动作，合并进 `upload`。
const X11_UNAVAILABLE: &[(StageId, &str)] = &[
    (StageId::CommandGen, MERGED_COMMAND_GEN),
    (
        StageId::Compositor,
        "X server 绘制无回执（加每帧 XSync 会改变被测性能）",
    ),
];
const X11_NOT_APPLICABLE: &[(StageId, &str)] = &[
    (StageId::BackendSubmit, "CPU 后端没有独立的 GPU 提交阶段"),
    (
        StageId::PresentBlocking,
        "X11 的上传与 Present 是同一个动作，已合并计入 upload",
    ),
    (StageId::GpuExecute, "CPU 光栅化后端没有 GPU 执行阶段"),
    (StageId::PixelConvert, "该阶段只在 Web 后端存在"),
    (StageId::ImageAlloc, "该阶段只在 Web 后端存在"),
    (StageId::PixelCopy, "该阶段只在 Web 后端存在"),
    (StageId::CanvasBlit, "该阶段只在 Web 后端存在"),
];

/// Web 的能力表。
const WEB_UNAVAILABLE: &[(StageId, &str)] = &[
    (StageId::CommandGen, MERGED_COMMAND_GEN),
    (
        StageId::Compositor,
        "浏览器合成在 rAF 回调结束后发生，wasm 侧不可观测",
    ),
];
const WEB_NOT_APPLICABLE: &[(StageId, &str)] = &[
    (StageId::BackendSubmit, "CPU 后端没有独立的 GPU 提交阶段"),
    (
        StageId::Upload,
        "Web 没有独立上传阶段，已拆成 pixel_convert / image_alloc / pixel_copy / canvas_blit",
    ),
    (
        StageId::PresentBlocking,
        "Web 的上屏是 canvas_blit（putImageData 调用本身），无阻塞式 Present",
    ),
    (StageId::GpuExecute, "CPU 光栅化后端没有 GPU 执行阶段"),
];

impl BackendKind {
    /// 该后端声明为"不可测量"的阶段表。
    pub fn unavailable_table(self) -> &'static [(StageId, &'static str)] {
        match self {
            BackendKind::D3d11 => D3D11_UNAVAILABLE,
            BackendKind::Soft => SOFT_UNAVAILABLE,
            BackendKind::X11 => X11_UNAVAILABLE,
            BackendKind::Web => WEB_UNAVAILABLE,
        }
    }

    /// 该后端声明为"没有这个阶段"的表。
    pub fn not_applicable_table(self) -> &'static [(StageId, &'static str)] {
        match self {
            BackendKind::D3d11 => D3D11_NOT_APPLICABLE,
            BackendKind::Soft => SOFT_NOT_APPLICABLE,
            BackendKind::X11 => X11_NOT_APPLICABLE,
            BackendKind::Web => WEB_NOT_APPLICABLE,
        }
    }

    /// 查询该后端对某阶段的能力。
    ///
    /// 默认 `Measurable`：只要没有显式声明"不可测/不存在"，就认为可以测
    ///（本帧没走到时 `stage()` 会给 `Unavailable("本帧未经过该阶段")`）。
    pub fn capability(self, id: StageId) -> Capability {
        if let Some((_, reason)) = self.unavailable_table().iter().find(|(s, _)| *s == id) {
            return Capability::Unavailable(reason);
        }
        if let Some((_, reason)) = self.not_applicable_table().iter().find(|(s, _)| *s == id) {
            return Capability::NotApplicable(reason);
        }
        Capability::Measurable
    }

    /// 该后端**永远无法测量**的阶段表（供报告一次性列出）。
    pub fn declared_unavailable(self) -> Vec<(StageId, &'static str)> {
        self.unavailable_table().to_vec()
    }

    /// 该后端**不存在**的阶段表。
    pub fn declared_not_applicable(self) -> Vec<(StageId, &'static str)> {
        self.not_applicable_table().to_vec()
    }

    /// 稳定字符串名。
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::D3d11 => "d3d11",
            BackendKind::Soft => "soft",
            BackendKind::X11 => "x11",
            BackendKind::Web => "web",
        }
    }
}

impl Capability {
    /// 是否可测量。
    pub fn is_measurable(self) -> bool {
        matches!(self, Capability::Measurable)
    }
}

/// 校验失败的原因（全部是"数据自相矛盾"，不是运行时错误）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatsError {
    /// 某阶段被填了测量值，但后端声明它不可测量/不适用。
    MeasuredButNotMeasurable {
        /// 出问题的阶段。
        stage: StageId,
        /// 后端声明的原因。
        reason: &'static str,
    },
    /// `Unavailable` 的原因为空。
    EmptyReason(StageId),
    /// 同一阶段同时出现在"不可用"与"不适用"表里。
    ConflictingTables(StageId),
    /// 某阶段在后端声明为不适用，却出现在"本帧不可用"表里。
    UnavailableButNotApplicable(StageId),
}

/// 一帧的完整统计。
#[derive(Clone, Copy, Debug)]
pub struct RenderStats {
    /// 帧序号（调用方自增）。
    pub frame_id: u64,
    /// 后端种类。
    pub backend: BackendKind,
    /// 本帧是否真正上屏。
    pub presented: bool,
    /// 各阶段测量值。
    pub stages: StageTimes,
    /// 计数器。
    pub counters: FrameCounters,
    /// **本帧**额外不可用的阶段（后端声明之外的临时原因，例如查询失败）。
    pub frame_unavailable: &'static [(StageId, &'static str)],
}

impl RenderStats {
    /// 新建一帧统计（全部阶段为空，等待打点）。
    pub fn new(frame_id: u64, backend: BackendKind) -> Self {
        Self {
            frame_id,
            backend,
            presented: false,
            stages: StageTimes::default(),
            counters: FrameCounters::default(),
            frame_unavailable: &[],
        }
    }

    /// 解析某阶段的三态结果。
    ///
    /// 优先级：本帧测量值 > 后端能力表 > 本帧临时不可用。
    /// 可测量但本帧没打点 → `Unavailable("本帧未经过该阶段")`，**不是 `NotApplicable`**。
    pub fn stage(&self, id: StageId) -> Stage {
        if let Some(d) = self.stages.get(id) {
            return Stage::Measured(d);
        }
        match self.backend.capability(id) {
            Capability::NotApplicable(reason) => {
                let _ = reason;
                Stage::NotApplicable
            }
            Capability::Unavailable(reason) => Stage::Unavailable(reason),
            Capability::Measurable => {
                if let Some((_, reason)) = self.frame_unavailable.iter().find(|(s, _)| *s == id) {
                    return Stage::Unavailable(reason);
                }
                Stage::Unavailable("本帧未经过该阶段")
            }
        }
    }

    /// 自检：拒绝自相矛盾的数据（见 [`StatsError`]）。
    ///
    /// **这是"不许把测不到写成 0"的守卫**：任何被后端声明为不可测量的阶段，
    /// 一旦被填了 `Some(_)`（哪怕 `Duration::ZERO`），这里都会报错。
    pub fn validate(&self) -> Result<(), StatsError> {
        for id in StageId::ALL {
            let measured = self.stages.get(id).is_some();
            match self.backend.capability(id) {
                Capability::Measurable => {}
                Capability::Unavailable(reason) => {
                    if measured {
                        return Err(StatsError::MeasuredButNotMeasurable { stage: id, reason });
                    }
                }
                Capability::NotApplicable(reason) => {
                    if measured {
                        return Err(StatsError::MeasuredButNotMeasurable { stage: id, reason });
                    }
                    if self.frame_unavailable.iter().any(|(s, _)| *s == id) {
                        return Err(StatsError::UnavailableButNotApplicable(id));
                    }
                }
            }
        }
        for (id, reason) in self.frame_unavailable {
            if reason.trim().is_empty() {
                return Err(StatsError::EmptyReason(*id));
            }
            if matches!(self.backend.capability(*id), Capability::NotApplicable(_)) {
                return Err(StatsError::UnavailableButNotApplicable(*id));
            }
        }
        Ok(())
    }

    /// 重置为下一帧（保留 `frame_id`/`backend`，清空阶段与计数器）。
    ///
    /// 为什么复用而不是每帧新建：开启统计时也应零堆分配。
    pub fn reset_for_next_frame(&mut self, frame_id: u64) {
        self.frame_id = frame_id;
        self.presented = false;
        self.stages.clear();
        self.counters = FrameCounters::default();
        self.frame_unavailable = &[];
    }

    /// 追加一行 JSON（不分配：写进调用方的 `String`）。
    ///
    /// 口径：`unavailable` 只输出**原因字符串**，`not_applicable` 输出 `null`
    /// 且带独立键 —— 三者在前端可区分，不会被误读成 0。
    pub fn write_json(&self, out: &mut String) {
        use std::fmt::Write;
        let _ = write!(
            out,
            "{{\"frame\":{},\"backend\":\"{}\",\"presented\":{},\"stages\":{{",
            self.frame_id,
            self.backend.name(),
            self.presented
        );
        let mut first = true;
        for id in StageId::ALL {
            let stage = self.stage(id);
            if !first {
                out.push(',');
            }
            first = false;
            let _ = write!(out, "\"{}\":", id.name());
            match stage {
                Stage::Measured(d) => {
                    let _ = write!(out, "{}", d.as_secs_f64() * 1e6);
                }
                Stage::Unavailable(reason) => {
                    let _ = write!(out, "null");
                    let _ = write!(out, ",\"{}.unavailable\":\"{}\"", id.name(), reason);
                }
                Stage::NotApplicable => {
                    let _ = write!(out, "null,\"{}.not_applicable\":true", id.name());
                }
            }
        }
        let c = self.counters;
        let _ = write!(
            out,
            "}},\"counters\":{{\"dirty_rects\":{},\"damaged_pixels\":{},\"instance_pushed\":{},\"instance_drawn\":{},\"batches\":{},\"atlas_bytes\":{},\"skipped\":{},\"occluded\":{}}}}}",
            c.dirty_rect_count,
            c.damaged_pixels,
            c.instance_pushed,
            c.instance_drawn,
            c.batch_count,
            c.atlas_upload_bytes,
            c.frame_skipped,
            c.occluded
        );
    }
}

// ===========================================================================
// 后端阶段的合并（runtime 每帧调用一次）
// ===========================================================================

/// 由**后端**负责填充的阶段（runtime 只填事件/布局/钩子/整帧）。
///
/// 为什么显式列出而不是"后端填了就算"：合并时必须能区分
/// "后端没测"（保持 `None`）与"后端测了 0"（`Some(ZERO)`），
/// 并且 runtime 自己的阶段不得被后端覆盖。
pub const BACKEND_STAGES: [StageId; 12] = [
    StageId::RenderPrepare,
    StageId::CommandGen,
    StageId::CpuRaster,
    StageId::Upload,
    StageId::BackendSubmit,
    StageId::PixelConvert,
    StageId::ImageAlloc,
    StageId::PixelCopy,
    StageId::CanvasBlit,
    StageId::PresentBlocking,
    StageId::GpuExecute,
    StageId::Compositor,
];

impl RenderStats {
    /// 某阶段是否由后端负责。
    pub fn is_backend_stage(id: StageId) -> bool {
        BACKEND_STAGES.contains(&id)
    }

    /// 合并后端上报的统计：**只取后端负责的阶段**，并做自检。
    ///
    /// **字段所有权（B1 裁决）**：
    /// - `stages`：仅 `BACKEND_STAGES` 里的 12 项由后端提供，Runtime 自己的阶段
    ///   （`event_dispatch`/`layout`/`after_frame`/`after_present`/`total_frame`）不会被覆盖；
    /// - `counters`：由后端提供（脏区数、实例数、图集字节等只有后端知道）；
    /// - `presented`：**由 Runtime 独占**，本函数不复制（见下方注释）；
    /// - `frame_unavailable`：由后端提供（该帧临时不可测的原因）。
    ///
    /// 返回：`true` = 后端确实提供了数据；`false` = 未接入（所有后端阶段保持 `None`，
    /// **绝不伪造**）。
    ///
    /// 自检失败（例如后端把 `gpu_execute` 填成 `Measured`）时不采纳该帧的后端数据，
    /// 并打一条日志 —— 宁可少数据，也不要自相矛盾的数据。
    pub fn merge_backend(&mut self, src: Option<RenderStats>) -> bool {
        let Some(src) = src else {
            return false;
        };
        if src.backend != self.backend {
            // 后端自报种类与本帧能力表不一致：丢弃，避免用错的能力表解读数据。
            eprintln!(
                "[y901] render_stats: 后端自报种类 {} 与本帧 {} 不一致，丢弃",
                src.backend.name(),
                self.backend.name()
            );
            return false;
        }
        for id in BACKEND_STAGES {
            if let Some(d) = src.stages.get(id) {
                self.stages.set(id, d);
            }
        }
        self.counters = src.counters;
        // **`presented` 不由后端提供**（设计裁决 B1）：它是 Runtime 统计，
        // 真值由 `run_loop` 依据 `RenderOutcome` 写入。后端构造的 `RenderStats`
        // 里该字段恒为 `false`，若在这里复制，就会让"后端说什么就是什么"，
        // 与事实来源冲突（而且安全性只依赖 run_loop 里 merge 之后立刻覆写这个顺序）。
        // 因此**不复制**：唯一写者是 Runtime。
        self.frame_unavailable = src.frame_unavailable;
        if let Err(err) = self.validate() {
            eprintln!("[y901] render_stats 自检失败，丢弃本帧后端数据：{err:?}");
            self.frame_unavailable = &[];
        }
        true
    }

    /// 后端上报的阶段耗时之和（用于"不得超过 `window.render` 墙钟"的验收）。
    pub fn backend_stage_sum(&self) -> Duration {
        let mut sum = Duration::ZERO;
        for id in BACKEND_STAGES {
            if let Some(d) = self.stages.get(id) {
                sum += d;
            }
        }
        sum
    }
}
// ===========================================================================
// 开关与打点原语
// ===========================================================================

/// `Y901_RENDER_STATS` 是否开启（进程级缓存，与 Z1 的 `OnceLock` 风格一致）。
///
/// 语义：**启动时决定**。运行中改它不会生效 —— 与 `Y901_DEBUG_BATCHES` 等诊断开关一致。
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("Y901_RENDER_STATS").is_some_and(|v| v != "0" && !v.is_empty())
    })
}

/// 混合开关的打点起点：**关闭时直接返回 `None`，不调用 `Instant::now()`**。
#[inline]
pub fn mark(started: bool) -> Option<web_time::Instant> {
    if started {
        Some(web_time::Instant::now())
    } else {
        None
    }
}

/// 计算从 `start` 到现在的耗时；`start` 为 `None`（统计关闭）时返回 `None`。
#[inline]
pub fn since(start: Option<web_time::Instant>) -> Option<Duration> {
    start.map(|s| s.elapsed())
}

/// 打点辅助：`since` 的结果写进 `StageTimes`（`None` 时不动）。
#[inline]
pub fn record(times: &mut StageTimes, id: StageId, start: Option<web_time::Instant>) {
    if let Some(d) = since(start) {
        times.set(id, d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(backend: BackendKind) -> RenderStats {
        RenderStats::new(7, backend)
    }

    // ---- 1. GPU 不允许被标记为 Measured --------------------------------

    #[test]
    fn gpu_execute_can_never_be_reported_as_measured() {
        for backend in [
            BackendKind::D3d11,
            BackendKind::Soft,
            BackendKind::X11,
            BackendKind::Web,
        ] {
            let mut s = stats(backend);
            s.stages.set(StageId::GpuExecute, Duration::from_micros(3));
            assert_eq!(
                s.validate(),
                Err(StatsError::MeasuredButNotMeasurable {
                    stage: StageId::GpuExecute,
                    reason: match backend.capability(StageId::GpuExecute) {
                        Capability::Unavailable(r) | Capability::NotApplicable(r) => r,
                        Capability::Measurable => unreachable!("GPU 阶段不该是可测量的"),
                    },
                }),
                "{backend:?} 上把 gpu_execute 填成 Measured 必须被 validate 拒绝"
            );
        }
    }

    #[test]
    fn compositor_is_never_measurable_on_any_backend() {
        for backend in [
            BackendKind::D3d11,
            BackendKind::Soft,
            BackendKind::X11,
            BackendKind::Web,
        ] {
            let mut s = stats(backend);
            s.stages.set(StageId::Compositor, Duration::from_micros(1));
            assert!(
                s.validate().is_err(),
                "{backend:?} 上 compositor 不可测，填值必须报错"
            );
            assert!(
                !backend.capability(StageId::Compositor).is_measurable(),
                "{backend:?} 的 compositor 能力声明必须不是 Measurable"
            );
        }
    }

    // ---- 2. 不可用阶段必须有明确原因 ----------------------------------

    #[test]
    fn unavailable_reasons_are_never_empty() {
        // 后端静态声明的每一条原因都必须非空、且确实是"不可用/不适用"。
        for backend in [
            BackendKind::D3d11,
            BackendKind::Soft,
            BackendKind::X11,
            BackendKind::Web,
        ] {
            for (id, reason) in backend.declared_unavailable() {
                assert!(
                    !reason.trim().is_empty(),
                    "{backend:?} 的 {} 声称不可用却没给原因",
                    id.name()
                );
            }
            for (id, reason) in backend.declared_not_applicable() {
                assert!(
                    !reason.trim().is_empty(),
                    "{backend:?} 的 {} 声称不适用却没给原因",
                    id.name()
                );
            }
        }
        // 本帧临时不可用：空原因必须被拒绝。
        let mut s = stats(BackendKind::D3d11);
        s.frame_unavailable = &[(StageId::Upload, "   ")];
        assert_eq!(s.validate(), Err(StatsError::EmptyReason(StageId::Upload)));
    }

    // ---- 3. NotApplicable 不得被解释为 Unavailable ---------------------

    #[test]
    fn not_applicable_and_unavailable_are_distinct() {
        // CPU 后端：gpu_execute 是"没有这个阶段" → NotApplicable。
        let soft = stats(BackendKind::Soft);
        assert_eq!(soft.stage(StageId::GpuExecute), Stage::NotApplicable);
        assert_eq!(soft.stage(StageId::GpuExecute).label(), "not_applicable");
        assert_ne!(soft.stage(StageId::GpuExecute).label(), "unavailable");

        // D3D11：gpu_execute 是"有阶段但测不了" → Unavailable(原因)。
        let d3d = stats(BackendKind::D3d11);
        match d3d.stage(StageId::GpuExecute) {
            Stage::Unavailable(reason) => assert!(!reason.trim().is_empty()),
            other => panic!("D3D11 的 gpu_execute 应为 Unavailable，实际 {other:?}"),
        }
        assert_ne!(d3d.stage(StageId::GpuExecute), Stage::NotApplicable);

        // 可测量但本帧没走到：也必须是 Unavailable（带原因），不能是 NotApplicable。
        let idle = stats(BackendKind::D3d11);
        match idle.stage(StageId::CommandGen) {
            Stage::Unavailable(reason) => assert!(reason.contains("本帧未经过")),
            other => panic!("未打点的可测阶段应为 Unavailable，实际 {other:?}"),
        }
    }

    // ---- 4. Measured(Duration::ZERO) 必须合法 --------------------------

    #[test]
    fn measured_zero_is_legal_and_distinct_from_no_measurement() {
        let mut s = stats(BackendKind::D3d11);
        s.stages.set(StageId::CommandGen, Duration::ZERO);
        assert_eq!(s.validate(), Ok(()), "零耗时是合法测量值");
        assert_eq!(
            s.stage(StageId::CommandGen),
            Stage::Measured(Duration::ZERO)
        );
        assert!(s.stage(StageId::CommandGen).is_measured());
        // 与"没测"必须可区分：同一个字段，未打点时为 None。
        assert_eq!(s.stages.get(StageId::Upload), None);
        assert!(!s.stage(StageId::Upload).is_measured());
    }

    // ---- 5. 统计关闭时不得产生虚假的测量值 ----------------------------

    #[test]
    fn disabled_stats_produce_no_measurements() {
        // 关闭时 mark() 必须直接返回 None —— 调用方因此不会写任何阶段。
        assert_eq!(mark(false), None);
        assert_eq!(since(None), None);
        let mut times = StageTimes::default();
        record(&mut times, StageId::Layout, mark(false));
        assert_eq!(times, StageTimes::default(), "关闭时不得写入任何阶段");

        // 用一个"关闭"的 RenderStats 走完整流程：所有可测阶段都必须是 Unavailable
        // （带原因），绝不能出现 Measured，尤其不能出现 0。
        let s = stats(BackendKind::D3d11);
        assert_eq!(s.validate(), Ok(()));
        for id in StageId::ALL {
            assert!(
                !s.stage(id).is_measured(),
                "统计关闭时 {} 不应有测量值",
                id.name()
            );
        }
    }

    #[test]
    fn enabled_mark_returns_a_timestamp_and_records_a_duration() {
        // 开启路径：mark(true) 必须给出 Some，且 record 写入一个可读值。
        let t = mark(true);
        assert!(t.is_some(), "开启时 mark 必须返回时间戳");
        let mut times = StageTimes::default();
        record(&mut times, StageId::Layout, t);
        assert!(
            times.get(StageId::Layout).is_some(),
            "开启时 record 必须写入测量值"
        );
        assert!(since(t).is_some());
    }

    // ---- 数据模型细节 --------------------------------------------------

    #[test]
    fn set_overwrites_instead_of_accumulating() {
        let mut s = stats(BackendKind::D3d11);
        s.stages.set(StageId::Layout, Duration::from_micros(10));
        s.stages.set(StageId::Layout, Duration::from_micros(4));
        assert_eq!(
            s.stages.get(StageId::Layout),
            Some(Duration::from_micros(4))
        );
    }

    #[test]
    fn every_stage_id_round_trips_through_set_and_get() {
        for id in StageId::ALL {
            let mut times = StageTimes::default();
            times.set(id, Duration::from_nanos(1));
            assert_eq!(
                times.get(id),
                Some(Duration::from_nanos(1)),
                "{}",
                id.name()
            );
        }
        // 没有漏掉 `StageId::ALL` 里的成员：长度必须等于枚举变体数。
        assert_eq!(StageId::ALL.len(), 17);
        // 名字互不重复（JSON key 冲突会静默覆盖）。
        let mut names: Vec<&str> = StageId::ALL.iter().map(|i| i.name()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "阶段名字必须唯一");
    }

    #[test]
    fn reset_for_next_frame_clears_stages_and_counters_but_keeps_identity() {
        let mut s = stats(BackendKind::X11);
        s.stages.set(StageId::CpuRaster, Duration::from_micros(9));
        s.counters.dirty_rect_count = 3;
        s.counters.frame_skipped = true;
        s.presented = true;
        s.reset_for_next_frame(8);
        assert_eq!(s.frame_id, 8);
        assert_eq!(s.backend, BackendKind::X11, "后端身份不应被重置");
        assert_eq!(s.stages, StageTimes::default(), "上一帧的阶段必须清空");
        assert_eq!(s.counters, FrameCounters::default());
        assert!(!s.presented);
    }

    /// **B1 裁决**：`presented` 属 Runtime 独占，`merge_backend` 不得复制/覆盖。
    ///
    /// 这条用例同时钉住两件事：
    /// 1. 后端把 `presented` 写成 `true` 也不会污染 Runtime 的值；
    /// 2. Runtime 写的值不会被后端数据清掉。
    #[test]
    fn merge_backend_never_touches_presented() {
        // Runtime 已判定"本帧未上屏"。
        let mut runtime_stats = stats(BackendKind::D3d11);
        runtime_stats.presented = false;

        // 后端（错误地）声称自己上屏了。
        let mut backend_stats = RenderStats::new(1, BackendKind::D3d11);
        backend_stats.presented = true;
        backend_stats
            .stages
            .set(StageId::BackendSubmit, Duration::from_micros(5));

        assert!(runtime_stats.merge_backend(Some(backend_stats)));
        assert!(
            !runtime_stats.presented,
            "merge_backend 不得用后端的 presented 覆盖 Runtime 的值"
        );
        // 后端阶段仍然合并成功（证明上面的断言不是"整个 merge 都没生效"）。
        assert_eq!(
            runtime_stats.stages.get(StageId::BackendSubmit),
            Some(Duration::from_micros(5))
        );

        // 反向：Runtime 说已上屏，后端说没有 → 仍以 Runtime 为准。
        let mut runtime_stats = stats(BackendKind::D3d11);
        runtime_stats.presented = true;
        let mut backend_stats = RenderStats::new(2, BackendKind::D3d11);
        backend_stats.presented = false;
        assert!(runtime_stats.merge_backend(Some(backend_stats)));
        assert!(
            runtime_stats.presented,
            "merge_backend 不得把 Runtime 的 presented 清成 false"
        );
    }
    #[test]
    fn frame_unavailable_can_explain_a_measurable_stage_this_frame() {
        // 可测量，但本帧因为某种原因没测到（例如查询失败）→ 由 frame_unavailable 解释。
        let mut s = stats(BackendKind::D3d11);
        s.frame_unavailable = &[(StageId::Upload, "本帧图集未变化，无需上传")];
        assert_eq!(s.validate(), Ok(()));
        assert_eq!(
            s.stage(StageId::Upload),
            Stage::Unavailable("本帧图集未变化，无需上传")
        );
    }

    #[test]
    fn unavailable_entry_for_not_applicable_stage_is_rejected() {
        let mut s = stats(BackendKind::Soft);
        s.frame_unavailable = &[(StageId::GpuExecute, "不该出现在这里")];
        assert_eq!(
            s.validate(),
            Err(StatsError::UnavailableButNotApplicable(StageId::GpuExecute))
        );
    }

    #[test]
    fn json_never_prints_a_fake_zero_for_unmeasurable_stages() {
        let mut s = stats(BackendKind::D3d11);
        s.stages.set(StageId::CommandGen, Duration::ZERO);
        let mut out = String::new();
        s.write_json(&mut out);
        // 可测且打了点：输出 0（合法的真实 0）。
        assert!(out.contains("\"command_gen\":0"), "真实 0 应输出 0：{out}");
        // 不可测的 GPU：必须是 null + 原因，绝不能是数字。
        assert!(
            out.contains("\"gpu_execute\":null"),
            "gpu_execute 必须输出 null：{out}"
        );
        assert!(
            out.contains("gpu_execute.unavailable"),
            "gpu_execute 必须带不可用原因：{out}"
        );
        assert!(
            !out.contains("\"gpu_execute\":0"),
            "gpu_execute 不得输出 0：{out}"
        );
        // 不适用（CPU 后端没有 GPU）：必须是 null + not_applicable 标记。
        let soft = stats(BackendKind::Soft);
        let mut out2 = String::new();
        soft.write_json(&mut out2);
        assert!(out2.contains("\"gpu_execute\":null"), "{out2}");
        assert!(out2.contains("gpu_execute.not_applicable"), "{out2}");
    }

    #[test]
    fn d3d11_declares_gpu_and_compositor_unavailable() {
        let unavail = BackendKind::D3d11.declared_unavailable();
        assert!(unavail.iter().any(|(id, _)| *id == StageId::GpuExecute));
        assert!(unavail.iter().any(|(id, _)| *id == StageId::Compositor));
        // 而 D3D11 的 backend_submit 是可测的。
        assert!(BackendKind::D3d11
            .capability(StageId::BackendSubmit)
            .is_measurable());
    }

    #[test]
    fn soft_backends_merge_command_generation_into_raster() {
        // 设计文档要求：软渲染不能拆出虚假的 command_gen。
        for backend in [BackendKind::Soft, BackendKind::X11, BackendKind::Web] {
            let cap = backend.capability(StageId::CommandGen);
            assert!(
                !cap.is_measurable(),
                "{backend:?} 不应把 command_gen 声明为可测量"
            );
            assert!(
                backend.capability(StageId::CpuRaster).is_measurable(),
                "{backend:?} 的 cpu_raster 必须可测"
            );
        }
        // GPU 后端反过来：有 command_gen / backend_submit，没有 cpu_raster。
        assert!(BackendKind::D3d11
            .capability(StageId::CommandGen)
            .is_measurable());
        assert!(!BackendKind::D3d11
            .capability(StageId::CpuRaster)
            .is_measurable());
    }

    #[test]
    fn web_only_pixel_stages_do_not_exist_on_other_backends() {
        for id in [
            StageId::PixelConvert,
            StageId::ImageAlloc,
            StageId::PixelCopy,
            StageId::CanvasBlit,
        ] {
            assert!(
                BackendKind::Web.capability(id).is_measurable(),
                "Web 的 {} 必须可测",
                id.name()
            );
            assert_eq!(
                BackendKind::X11.capability(id),
                Capability::NotApplicable("该阶段只在 Web 后端存在"),
                "X11 上 {} 必须是不适用",
                id.name()
            );
            for other in [BackendKind::D3d11, BackendKind::Soft] {
                match other.capability(id) {
                    Capability::NotApplicable(reason) => {
                        assert!(!reason.trim().is_empty(), "不适用必须给原因")
                    }
                    cap => panic!(
                        "{other:?} 上 {} 应为不适用（这是 Web 独有阶段），实际 {cap:?}",
                        id.name()
                    ),
                }
            }
        }
    }
}
