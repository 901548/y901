//! Widget trait 与三类上下文。
//!
//! 框架的全部"平台假设"都收敛在 `TextMetrics` 和 `RenderCtx` 两个 trait 里：
//! 框架自己不知道 D3D11、DirectWrite、Win32，也不知道任何具体业务。
//! 应用层提供实现，组件只依赖 trait。

use crate::color::{Color, TextStyle};
use crate::event::{EventResult, UiEvent};
use crate::geometry::{Edges, Pos, Rect, Size};

/// 绘制/命中层级。
///
/// 子节点继承父节点的层级，`Modal` 会盖住整个 `Base`。
/// 绘制顺序固定为 `Base → Overlay → Modal`；命中顺序相反。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// 普通界面。
    #[default]
    Base = 0,
    /// 浮层（下拉菜单、提示）：视觉在上，但点外部可以穿透。
    Overlay = 1,
    /// 模态（对话框）：点外部会被吞掉，焦点被限制在子树内。
    Modal = 2,
}

impl Layer {
    pub fn rank(self) -> u8 {
        self as u8
    }
}
/// 组件的语义角色。给 Agent 看的那一层。
///
/// **不是枚举**：框架只提供一组约定常量（panel / button / text …），
/// 业务角色（例如终端、文件树）由组件库或应用自己定义：
///
/// ```ignore
/// pub const ROLE_TIMELINE: Role = Role("timeline");
/// ```
///
/// 这样框架永远不会因为"多了一种业务"而需要改代码。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Role(pub &'static str);

impl Role {
    pub const PANEL: Role = Role("panel");
    pub const WINDOW: Role = Role("window");
    pub const TEXT: Role = Role("text");
    pub const BUTTON: Role = Role("button");
    pub const TEXT_INPUT: Role = Role("text_input");
    pub const LIST: Role = Role("list");
    pub const LIST_ITEM: Role = Role("list_item");
    pub const SCROLLBAR: Role = Role("scrollbar");

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl Default for Role {
    fn default() -> Self {
        Role::PANEL
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// 主轴方向。只有这两种——够了，也是刻意保持简单。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Axis {
    #[default]
    Horizontal,
    Vertical,
}

/// 单个维度的尺寸约束。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dimension {
    /// 由内容决定。
    #[default]
    Auto,
    /// 固定逻辑像素。
    Px(f32),
    /// 占父容器剩余空间的权重（flex-grow）。
    Flex(f32),
    /// 父容器尺寸的百分比。
    Percent(f32),
}

impl Dimension {
    pub fn resolve(&self, available: f32, content: f32) -> f32 {
        match *self {
            Dimension::Auto => content,
            Dimension::Px(v) => v,
            Dimension::Flex(_) => content,
            Dimension::Percent(p) => available * (p / 100.0),
        }
    }

    pub fn flex_factor(&self) -> Option<f32> {
        match *self {
            Dimension::Flex(f) if f > 0.0 => Some(f),
            _ => None,
        }
    }

    pub fn is_flex(&self) -> bool {
        matches!(self, Dimension::Flex(f) if *f > 0.0)
    }
}

/// 组件在父容器里的伸缩参数。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlexItem {
    pub width: Dimension,
    pub height: Dimension,
}

impl FlexItem {
    pub const fn fill() -> Self {
        Self {
            width: Dimension::Flex(1.0),
            height: Dimension::Flex(1.0),
        }
    }

    /// 固定尺寸。任一轴传 0.0 表示"该轴不强制"，交给内容或父容器决定。
    pub const fn fixed(w: f32, h: f32) -> Self {
        Self {
            width: if w > 0.0 { Dimension::Px(w) } else { Dimension::Auto },
            height: if h > 0.0 { Dimension::Px(h) } else { Dimension::Auto },
        }
    }

    pub const fn width(w: f32) -> Self {
        Self {
            width: Dimension::Px(w),
            height: Dimension::Auto,
        }
    }

    pub const fn height(h: f32) -> Self {
        Self {
            width: Dimension::Auto,
            height: Dimension::Px(h),
        }
    }

    pub const fn grow(factor: f32) -> Self {
        Self {
            width: Dimension::Flex(factor),
            height: Dimension::Flex(factor),
        }
    }
}

/// 轴掩码：用来表达"哪个方向无界"。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Axis2(u8);

impl Axis2 {
    pub const NONE: Axis2 = Axis2(0);
    pub const H: Axis2 = Axis2(1);
    pub const V: Axis2 = Axis2(2);
    pub const BOTH: Axis2 = Axis2(3);

    pub const fn contains(self, other: Axis2) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Axis2) -> Axis2 {
        Axis2(self.0 | other.0)
    }

    pub const fn any(self) -> bool {
        self.0 != 0
    }
}

/// 布局约束：父节点给子节点的可用空间上下限。
///
/// `unbounded` 显式标记哪个方向"没有上限"（ScrollView 的内容测量）。
/// 关键约定：**`measure` 返回的永远是有限的内容尺寸**，约束里的 `INFINITY`
/// 只表示"你可以任意高/宽"，绝不允许出现在返回值里。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Constraints {
    pub max: Size,
    pub min: Size,
    pub unbounded: Axis2,
}

impl Constraints {
    pub const fn loose(max: Size) -> Self {
        Self {
            max,
            min: Size::ZERO,
            unbounded: Axis2::NONE,
        }
    }

    pub const fn tight(size: Size) -> Self {
        Self {
            max: size,
            min: size,
            unbounded: Axis2::NONE,
        }
    }

    /// 主轴无界：ScrollView 用它测量"内容到底多高/多宽"。
    ///
    /// `cross` 是交叉轴的有限上限。返回值里 `max` 对应轴是 `INFINITY`，
    /// 但 `constrain()` 会把它夹成内容尺寸，永不外泄。
    pub fn unbounded_main(axis: crate::Axis, cross: Size) -> Self {
        match axis {
            crate::Axis::Vertical => Self {
                max: Size::new(cross.w, f32::INFINITY),
                min: Size::ZERO,
                unbounded: Axis2::V,
            },
            crate::Axis::Horizontal => Self {
                max: Size::new(f32::INFINITY, cross.h),
                min: Size::ZERO,
                unbounded: Axis2::H,
            },
        }
    }

    pub fn is_unbounded(&self, axis: crate::Axis) -> bool {
        match axis {
            crate::Axis::Horizontal => self.unbounded.contains(Axis2::H),
            crate::Axis::Vertical => self.unbounded.contains(Axis2::V),
        }
    }

    /// 把内容尺寸夹进约束。
    ///
    /// 注意：`min` 可能是 0，`max` 可能是 ∞ —— 结果是**内容尺寸**，
    /// 不会因为 `∞` 而产生 `inf` / `NaN`。
    pub fn constrain(&self, size: Size) -> Size {
        // 先保证输入有限：万一组件的 measure 算飞了，这里兜底成 0。
        let safe = Size::new(
            if size.w.is_finite() { size.w } else { 0.0 },
            if size.h.is_finite() { size.h } else { 0.0 },
        );
        let clamped = safe.max(self.min).min(self.max);
        Size::new(
            if clamped.w.is_finite() { clamped.w } else { 0.0 },
            if clamped.h.is_finite() { clamped.h } else { 0.0 },
        )
    }
}

/// 文本度量：布局阶段需要的全部字体信息。
pub trait TextMetrics {
    /// 单行文本的墨迹外框尺寸。
    fn measure_text(&self, text: &str, style: &TextStyle) -> Size;
    /// 行高（基线间距）。
    fn line_height(&self, style: &TextStyle) -> f32;
    /// DPI 缩放，1.0 = 100%。
    fn scale(&self) -> f32;
}

/// 渲染上下文：框架唯一的出图接口。
///
/// 后端实现它（D3D11 / 软件 / 测试用的记录器），组件只往里画。
pub trait RenderCtx: TextMetrics {
    /// 视口尺寸（逻辑像素）。
    fn size(&self) -> Size;

    fn fill_rect(&mut self, rect: Rect, color: Color);

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);

    /// 在 `origin`（左上角）画一行文本，返回前进宽度。
    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32;

    /// 裁剪栈：终端/列表滚动需要。
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);
}

/// 事件上下文：组件通过它向框架提请"焦点/重绘/重排"。
///
/// 刻意不借用树的字段：只有值拷贝，组件无法从事件里偷改树结构。
pub struct EventCtx {
    /// 当前焦点节点 id（快照）。
    pub focus: Option<String>,
    /// 组件请求把焦点移交给谁（分发结束后由框架统一执行）。
    pub focus_request: Option<String>,
    pub redraw: bool,
    pub relayout: bool,
    pub viewport: Size,
    /// 本次分发中组件产生的意图。框架只负责搬运，不解释。
    pub commands: Vec<crate::command::Command>,

    /// 当前处于哪个路由阶段。
    pub phase: crate::event::EventPhase,
    /// 本次事件命中/捕获的目标节点 id。
    pub target: String,
    /// 置 true 后，框架立刻停止后续阶段（捕获拦截 / 冒泡截断）。
    pub stop_propagation: bool,
    /// 置 true 时，即使返回 `Handled` 也继续冒泡（罕见，需显式请求）。
    pub propagate: bool,
    /// 指针捕获请求，分发结束后由框架落实。
    pub pointer_capture: Option<crate::event::PointerCapture>,
    /// 焦点守卫：在处理 `Focus` 事件时置 true 可以拒绝这次焦点变更。
    pub focus_vetoed: bool,
}

impl EventCtx {
    pub fn new(focus: Option<String>, viewport: Size) -> Self {
        Self {
            focus,
            focus_request: None,
            redraw: false,
            relayout: false,
            viewport,
            commands: Vec::new(),
            phase: crate::event::EventPhase::Target,
            target: String::new(),
            stop_propagation: false,
            propagate: false,
            pointer_capture: None,
            focus_vetoed: false,
        }
    }

    /// 拒绝本次焦点变更（只能在 `UiEvent::Focus` 的处理中调用）。
    pub fn veto_focus(&mut self) {
        self.focus_vetoed = true;
    }

    /// 立刻终止后续阶段（Capture 里拦事件、Bubble 里截断都用它）。
    pub fn stop_propagation(&mut self) {
        self.stop_propagation = true;
    }

    /// 即使 `Handled` 也继续冒泡。
    pub fn allow_propagation(&mut self) {
        self.propagate = true;
    }

    /// 请求指针捕获：直到 `release_pointer_capture` 之前，
    /// 所有 `MouseMove` / `MouseUp` 都强制派发给这个节点（无视坐标）。
    pub fn set_pointer_capture(&mut self, id: impl Into<String>) {
        self.pointer_capture = Some(crate::event::PointerCapture::Capture(id.into()));
    }

    /// 释放指针捕获。
    pub fn release_pointer_capture(&mut self) {
        self.pointer_capture = Some(crate::event::PointerCapture::Release);
    }

    /// 当前是否处于目标阶段。
    pub fn is_target(&self) -> bool {
        self.phase == crate::event::EventPhase::Target
    }

    /// 请应用层做一件事。框架不理解 `name` 的含义。
    pub fn emit_app(&mut self, name: impl Into<String>, args: crate::json::Json) {
        self.commands
            .push(crate::command::Command::app(name, args));
    }

    /// 请某个节点执行一个动作（组件不需要认识对方）。
    pub fn emit_node(
        &mut self,
        target: impl Into<String>,
        action: impl Into<String>,
        args: crate::json::Json,
    ) {
        self.commands
            .push(crate::command::Command::node(target, action, args));
    }

    /// 直接投递一条命令。
    pub fn emit(&mut self, cmd: crate::command::Command) {
        self.commands.push(cmd);
    }

    pub fn request_focus(&mut self, id: impl Into<String>) {
        self.focus_request = Some(id.into());
    }

    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }

    pub fn request_relayout(&mut self) {
        self.relayout = true;
        self.redraw = true;
    }

    pub fn is_focused(&self, id: &str) -> bool {
        self.focus.as_deref() == Some(id)
    }

    /// 把组件的返回值与已累积的标志合并。
    pub fn finish(&mut self, result: EventResult) -> EventResult {
        let mut out = result;
        if self.relayout {
            out = out.merge(EventResult::Relayout);
        } else if self.redraw {
            out = out.merge(EventResult::Redraw);
        }
        out
    }
}

/// 语义描述：给 Agent 读的节点信息。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Semantics {
    pub label: String,
    pub value: String,
    /// 该节点支持的动作名，例如 ["click", "set_text"]。
    pub actions: Vec<String>,
    /// 额外键值（终端会填 cols/rows/scroll_offset）。
    pub props: Vec<(String, String)>,
    /// 结构化状态：Agent 直接消费（文本、光标、禁用…）。
    pub state: crate::json::Json,
}

impl Semantics {
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }

    /// 写一个结构化状态字段（保留类型：数字还是数字、布尔还是布尔）。
    pub fn state(mut self, key: impl Into<String>, value: impl Into<crate::json::Json>) -> Self {
        let key = key.into();
        self.state = self.state.clone().set(&key, value.into());
        self
    }

    pub fn action(mut self, name: impl Into<String>) -> Self {
        self.actions.push(name.into());
        self
    }

    pub fn prop(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.props.push((key.into(), value.into()));
        self
    }
}

/// 组件。
///
/// 生命周期：`measure` → `arrange` → `render`；事件随时来。
/// 只实现这一个 trait，就能被框架当普通节点对待。
///
/// 布局是**框架驱动**的：容器在 `measure`/`arrange` 里通过
/// [`crate::layout::LayoutCache`] 访问子节点，缓存、去重、脏标记都由框架负责。
pub trait Widget {
    /// 树内唯一 id，例如 "panel.list"。
    fn id(&self) -> &str;

    fn role(&self) -> Role {
        Role::PANEL
    }

    /// 自己在父容器里的伸缩参数。
    fn flex(&self) -> FlexItem {
        FlexItem::default()
    }

    /// 绘制/命中层级。子节点继承父节点的层级（取最大值）。
    fn layer(&self) -> Layer {
        Layer::Base
    }

    /// 是否可以获得键盘焦点（Tab 遍历、Modal 自动聚焦都用它）。
    fn focusable(&self) -> bool {
        false
    }

    /// 语义版本：状态变化时 +1，供 RPC 增量推送快速判断。
    /// 返回 0 表示不维护，框架退回对 state JSON 做指纹哈希。
    fn revision(&self) -> u64 {
        0
    }

    /// 子节点（容器才有）。
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }

    /// 可变子节点。返回 `None` 表示叶子组件。
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        None
    }

    /// 内容版本号：**内在尺寸**可能变化时 +1（文本变了、行数变了…）。
    ///
    /// 框架用它判断 `measure` 结果能否复用。默认 0 = 永不变化。
    /// 注意：只影响尺寸的状态才需要 bump；纯颜色/悬停态不要 bump，否则白白失效。
    fn layout_version(&self) -> u64 {
        0
    }

    /// 上报期望尺寸。父容器据此做 flex 分配。
    ///
    /// 容器内部应通过 `cache.measure(child, metrics, constraints)` 测量子节点，
    /// 而不是直接调用 `child.measure(...)` —— 否则会绕过缓存。
    fn measure(&mut self, metrics: &dyn TextMetrics, constraints: Constraints, cache: &mut crate::layout::LayoutCache) -> Size;

    /// 父容器算好绝对矩形后回传。
    ///
    /// 容器要在这里继续给子节点排布（框架提供 `layout::flex_arrange`），
    /// 叶子只缓存 rect。递归由容器驱动，框架不预设"子节点怎么摆"。
    fn arrange(&mut self, rect: Rect, metrics: &dyn TextMetrics, cache: &mut crate::layout::LayoutCache);

    /// 自己当前的矩形（`arrange` 之后有效）。
    fn rect(&self) -> Rect;

    /// 事件。返回 `Ignored` 表示不关心，让框架继续找别人。
    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    /// 出图。只允许通过 `RenderCtx`。
    fn render(&mut self, _ctx: &mut dyn RenderCtx);

    /// 语义信息。
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }

    /// 内边距：容器用它把子节点内缩。
    fn padding(&self) -> Edges {
        Edges::ZERO
    }

    /// 本帧是否必须全量重绘（例如全屏遮罩、动画）。
    ///
    /// 返回 false 时，框架会用 `needs_paint` 决定是否跳过整棵子树。
    fn always_paints(&self) -> bool {
        false
    }

    /// 脏区剪枝：与脏区不相交就可以整棵子树跳过。
    fn needs_paint(&self, dirty: &crate::tree::DirtyRegion) -> bool {
        self.always_paints() || dirty.intersects(self.rect())
    }
}
