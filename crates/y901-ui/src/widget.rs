//! Widget trait 与三类上下文。
//!
//! 框架的全部"平台假设"都收敛在 `TextMetrics` 和 `RenderCtx` 两个 trait 里：
//! 框架自己不知道 D3D11、DirectWrite、Win32，也不知道任何具体业务。
//! 应用层提供实现，组件只依赖 trait。

use crate::color::{Color, TextStyle};
use crate::event::{EventResult, UiEvent};
use crate::geometry::{Edges, Pos, Rect, Size};

/// 一个"请求挂浮层"的工厂：组件在事件里只提交工厂，
/// 真正构建 + 挂载由框架在 `finish_dispatch` 统一做。
///
/// 为什么是工厂而不是现成 widget：事件处理期间组件只有 `&mut EventCtx`，
/// 拿不到 `UiTree`；工厂在分发结束后才调用，避开借用冲突，
/// 也避免了"弹出失败但已经构造了一半"的浪费。
pub struct PopupRequest {
    /// 上层提交的工厂（只调一次）。
    pub factory: Box<dyn FnOnce() -> Box<dyn Widget>>,
    /// 触发控件的屏幕矩形。
    pub anchor: Rect,
    /// 是否模态（阻塞底层事件）。
    pub modal: bool,
}

/// 鼠标光标形状。框架只声明"想要什么"，具体映射到平台由应用层负责。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Default,
    /// 可点击（按钮、链接）
    Pointer,
    /// 文本输入
    Text,
    /// 左右拖拽（竖向分割条）
    ResizeHorizontal,
    /// 上下拖拽（横向分割条）
    ResizeVertical,
    /// 抓取/拖动中
    Grab,
}

/// 挂载上下文：组件在 `on_mount` 里订阅响应式数据时用。
pub struct MountCtx {
    /// 本节点 id（订阅时用作 owner）。
    pub id: String,
    /// 共享失效队列：Signal 变化时往里推 id。
    pub invalidation: crate::signal::Invalidation,
}

impl MountCtx {
    pub fn new(id: impl Into<String>, invalidation: crate::signal::Invalidation) -> Self {
        Self {
            id: id.into(),
            invalidation,
        }
    }
}
/// 组件的语义角色。给 Agent 看的那一层。
///
/// **不是枚举**：框架只提供一组约定常量（panel / button / text …），
/// 业务角色（例如终端、文件树）由组件库或应用自己定义：
///
/// ```
/// use y901_ui::Role;
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
            width: if w > 0.0 {
                Dimension::Px(w)
            } else {
                Dimension::Auto
            },
            height: if h > 0.0 {
                Dimension::Px(h)
            } else {
                Dimension::Auto
            },
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
            if clamped.w.is_finite() {
                clamped.w
            } else {
                0.0
            },
            if clamped.h.is_finite() {
                clamped.h
            } else {
                0.0
            },
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
/// 计算一个点在渐变方向上的归一化投影 `t`（0..1）。
///
/// 与 GPU 着色器口径一致：四角投影求区间，再归一化。
fn gradient_t(rect: Rect, dir: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = dir;
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return 0.0;
    }
    let proj = |px: f32, py: f32| ((px - rect.x) * dx + (py - rect.y) * dy) / len2;
    let corners = [
        proj(rect.x, rect.y),
        proj(rect.right(), rect.y),
        proj(rect.x, rect.bottom()),
        proj(rect.right(), rect.bottom()),
    ];
    let t_min = corners.iter().cloned().fold(f32::MAX, f32::min);
    let t_max = corners.iter().cloned().fold(f32::MIN, f32::max);
    let span = t_max - t_min;
    if span <= f32::EPSILON {
        return 0.0;
    }
    ((proj(x, y) - t_min) / span).clamp(0.0, 1.0)
}

/// 渐变背景的条带数（默认实现用）。
///
/// 为什么是常量而不是参数：条带数按像素切会让 draw 次数随控件高度线性增长，
/// 32 条在 200~1000px 高度上都足够平滑，且开销固定。
pub const GRADIENT_STEPS: usize = 32;

/// 圆角抗锯齿的竖直超采样数（每个像素行内部再切几份）。
pub const AA_SAMPLES: usize = 4;

pub trait RenderCtx: TextMetrics {
    /// 某个动画 key 的当前进度（0..=1）。
    ///
    /// 默认返回 `1.0`："没有动画 → 就是终态"。这是现有控件不改一行也不受影响的原因。
    fn animation(&self, _key: u64) -> f32 {
        1.0
    }

    /// 注入本帧的动画快照（由 `UiTree::render` 调用）。
    ///
    /// 为什么是"注入快照"而不是传 `&UiTree`：`RenderCtx` 由各后端实现，
    /// 它不应认识 UiTree（否则 UI 层与后端反向耦合）。
    fn set_animations(&mut self, _snapshot: &crate::animation::AnimSnapshot) {}


    /// 视口尺寸（逻辑像素）。
    fn size(&self) -> Size;

    fn fill_rect(&mut self, rect: Rect, color: Color);

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);

    /// 在 `origin`（左上角）画一行文本，返回前进宽度。
    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32;

    /// 绘制单个字符，返回实际占用宽度（像素）。
    ///
    /// 默认实现用**栈上** `[u8; 4]` 编码成 UTF-8 后转调 [`RenderCtx::draw_text`]，
    /// 零堆分配。终端这类"每格一个字符"的场景必须用它，否则每格一次 `String` 分配
    /// （80×24 = 每帧 1920 次）。
    ///
    /// 后端可以覆盖（例如直接查字形缓存），但默认实现对所有后端都够用。
    fn draw_char(&mut self, pos: Pos, ch: char, style: &TextStyle) -> f32 {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        self.draw_text(pos, s, style)
    }

    /// 圆角矩形填充。
    ///
    /// 输入：`rect`（外接矩形）、`color`（含 alpha）、`radius`（像素，自动夹到半宽/半高）。
    /// 副作用：在 `rect` 内（含裁剪）绘制。
    /// 默认实现：逐扫描线用 [`RenderCtx::fill_rect`] 拼出圆角——对任何后端都可用，
    /// 不需要新增 GPU 图元；`radius <= 0` 时退化成普通矩形。
    /// 性能：中间部分合并成一整块，只有上下各 `radius` 行逐行绘制。
    fn fill_round_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
        if r <= 0.5 {
            self.fill_rect(rect, color);
            return;
        }
        self.fill_rect(
            Rect::new(rect.x, rect.y + r, rect.w, rect.h - 2.0 * r),
            color,
        );

        // 上下各 r 行：逐扫描线画，并在左右边界做**覆盖率混合**（Coverage AA）。
        // 把每个像素行在竖直方向再切 AA_SAMPLES 份，各自代入圆方程求
        // 内缩量，取平均；平均内缩的小数部分就是边界像素被覆盖的比例。
        let rows = r.ceil() as i32;
        for i in 0..rows {
            let mut inset_sum = 0.0f32;
            for k in 0..AA_SAMPLES {
                let dy = r - (i as f32 + (k as f32 + 0.5) / AA_SAMPLES as f32);
                inset_sum += r - (r * r - dy * dy).max(0.0).sqrt();
            }
            let inset = inset_sum / AA_SAMPLES as f32;
            self.fill_round_row(rect, rect.y + i as f32, inset, color);
            self.fill_round_row(rect, rect.bottom() - 1.0 - i as f32, inset, color);
        }
    }

    /// 圆角 + 线性渐变（"苹果风按钮"底座）。
    ///
    /// 输入：`rect`、`from`/`to`、`dir`（任意方向向量）、`radius`。
    /// 默认实现（CPU 兜底）：按圆角逐行算左右内缩，
    /// 在每一行里用水平条带近似渐变（对竖直渐变是精确的，对斜向是近似）。
    /// GPU 后端应覆盖为 SDF + 渐变单实例。
    fn fill_gradient_round_rect(
        &mut self,
        rect: Rect,
        from: Color,
        to: Color,
        dir: (f32, f32),
        radius: f32,
    ) {
        let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
        if r <= 0.5 {
            self.fill_gradient_rect_dir(rect, from, to, dir);
            return;
        }
        // 中间整块（不受圆角影响）：直接用渐变
        self.fill_gradient_rect_dir(
            Rect::new(rect.x, rect.y + r, rect.w, rect.h - 2.0 * r),
            from,
            to,
            dir,
        );
        // 上下圆角行：每行只画 [x0, x1] 范围内的渐变条带
        let steps = (r.ceil() as i32).max(1);
        for i in 0..steps {
            let dy = r - (i as f32 + 0.5);
            let inset = r - (r * r - dy * dy).max(0.0).sqrt();
            let row = Rect::new(rect.x + inset, rect.y + i as f32, rect.w - 2.0 * inset, 1.0);
            // 1px 高的行里按 x 切 8 条（CPU 兜底的精度折衷）
            let n = 8;
            for k in 0..n {
                let t0 = k as f32 / n as f32;
                let t1 = (k + 1) as f32 / n as f32;
                let x0 = row.x + row.w * t0;
                let x1 = row.x + row.w * t1;
                // 用整个矩形的投影算 t（与 GPU 口径一致）
                let t = gradient_t(rect, dir, (x0 + x1) * 0.5, row.y + 0.5);
                self.fill_rect(Rect::new(x0, row.y, x1 - x0, 1.0), from.lerp(to, t));
            }
            let y = rect.bottom() - 1.0 - i as f32;
            let row = Rect::new(rect.x + inset, y, rect.w - 2.0 * inset, 1.0);
            for k in 0..n {
                let t0 = k as f32 / n as f32;
                let t1 = (k + 1) as f32 / n as f32;
                let x0 = row.x + row.w * t0;
                let x1 = row.x + row.w * t1;
                let t = gradient_t(rect, dir, (x0 + x1) * 0.5, y + 0.5);
                self.fill_rect(Rect::new(x0, y, x1 - x0, 1.0), from.lerp(to, t));
            }
        }
    }

    /// 圆角描边（只画轮廓，不填内部）。
    ///
    /// 输入：`rect`、`color`、`width`（描边宽度，居中于边界）、`radius`。
    /// 默认实现（CPU 兜底）：先画外圆角再用背景色挖内部是不行的（会覆盖下层），
    /// 所以这里抽象为"上下左右四条带 + 四个圆角环"：直边用 `fill_rect`，
    /// 圆角用逐行弦长差异画出带宽度的弧。后端（D3D）应覆盖为 SDF 单实例。
    fn stroke_round_rect(&mut self, rect: Rect, color: Color, width: f32, radius: f32) {
        let w = width.max(0.0);
        if w <= 0.0 || rect.is_empty() {
            return;
        }
        let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
        if r <= 0.5 {
            self.stroke_rect(rect, color, w);
            return;
        }
        // 直边（上下各一条、左右两条只到圆角起点）
        self.fill_rect(Rect::new(rect.x + r, rect.y, rect.w - 2.0 * r, w), color);
        self.fill_rect(
            Rect::new(rect.x + r, rect.bottom() - w, rect.w - 2.0 * r, w),
            color,
        );
        self.fill_rect(Rect::new(rect.x, rect.y + r, w, rect.h - 2.0 * r), color);
        self.fill_rect(
            Rect::new(rect.right() - w, rect.y + r, w, rect.h - 2.0 * r),
            color,
        );
        // 四个圆角：逐行画出弦长差（外弧 - 内弧）
        let steps = (r.ceil() as i32).max(1);
        for i in 0..steps {
            let dy = r - (i as f32 + 0.5);
            let outer = (r * r - dy * dy).max(0.0).sqrt();
            let inner_r = (r - w).max(0.0);
            let inner = if dy.abs() <= inner_r {
                (inner_r * inner_r - dy * dy).max(0.0).sqrt()
            } else {
                0.0
            };
            let band = (outer - inner).max(0.0);
            if band <= 0.0 {
                continue;
            }
            // 左上 / 右上
            self.fill_rect(
                Rect::new(rect.x + r - outer, rect.y + i as f32, band, 1.0),
                color,
            );
            self.fill_rect(
                Rect::new(rect.right() - r + inner, rect.y + i as f32, band, 1.0),
                color,
            );
            // 左下 / 右下
            let y = rect.bottom() - 1.0 - i as f32;
            self.fill_rect(Rect::new(rect.x + r - outer, y, band, 1.0), color);
            self.fill_rect(Rect::new(rect.right() - r + inner, y, band, 1.0), color);
        }
    }

    /// 圆角单行：左/右边界像素按覆盖率给 alpha，中间实色。
    fn fill_round_row(&mut self, rect: Rect, y: f32, inset: f32, color: Color) {
        let x0 = rect.x + inset;
        let x1 = rect.right() - inset;
        if x1 <= x0 {
            return;
        }
        let left_px = x0.floor();
        let right_px = x1.ceil();
        let left_frac = (x0 - left_px).clamp(0.0, 1.0);
        let right_frac = (right_px - x1).clamp(0.0, 1.0);

        if left_frac > 0.001 {
            self.fill_rect(
                Rect::new(left_px, y, 1.0, 1.0),
                color.with_alpha(color.a * (1.0 - left_frac)),
            );
        }
        if right_frac > 0.001 {
            self.fill_rect(
                Rect::new(right_px - 1.0, y, 1.0, 1.0),
                color.with_alpha(color.a * (1.0 - right_frac)),
            );
        }
        let body_x0 = left_px + if left_frac > 0.001 { 1.0 } else { 0.0 };
        let body_x1 = right_px - if right_frac > 0.001 { 1.0 } else { 0.0 };
        if body_x1 > body_x0 {
            self.fill_rect(Rect::new(body_x0, y, body_x1 - body_x0, 1.0), color);
        }
    }

    /// 全方向线性渐变：`dir = (dx, dy)`（任意向量，不需归一化）。
    ///
    /// 默认实现：沿主导轴切 1px 条带，每条按其中心在 `dir` 上的投影取色。
    /// 对轴向渐变是精确的；对斜向渐变是“沿主导轴逐行/逐列变色”的近似
    /// （`fill_rect` 只能画轴对齐矩形，真正的任意角渐变需要 GPU 图元）。
    fn fill_gradient_rect_dir(&mut self, rect: Rect, from: Color, to: Color, dir: (f32, f32)) {
        if rect.is_empty() || (from.a <= 0.0 && to.a <= 0.0) {
            return;
        }
        let (dx, dy) = dir;
        let len2 = dx * dx + dy * dy;
        if len2 <= f32::EPSILON {
            self.fill_rect(rect, from);
            return;
        }
        let proj = |x: f32, y: f32| -> f32 { ((x - rect.x) * dx + (y - rect.y) * dy) / len2 };
        let corners = [
            proj(rect.x, rect.y),
            proj(rect.right(), rect.y),
            proj(rect.x, rect.bottom()),
            proj(rect.right(), rect.bottom()),
        ];
        let t_min = corners.iter().cloned().fold(f32::MAX, f32::min);
        let t_max = corners.iter().cloned().fold(f32::MIN, f32::max);
        let span = t_max - t_min;
        if span <= f32::EPSILON {
            self.fill_rect(rect, from);
            return;
        }

        let vertical = dx.abs() >= dy.abs();
        let extent = if vertical { rect.w } else { rect.h };
        let count = (extent.round() as i32).clamp(1, 512);
        let steps = (GRADIENT_STEPS as i32).max(1);
        // 颜色按 steps 档量化：既保证平滑，又避免每个 1px 条带都算一次 lerp
        let _ = steps;
        for i in 0..count {
            let a0 = i as f32 / count as f32;
            let a1 = (i + 1) as f32 / count as f32;
            let t = if vertical {
                let cx = rect.x + rect.w * (a0 + a1) * 0.5;
                ((proj(cx, rect.y + rect.h * 0.5) - t_min) / span).clamp(0.0, 1.0)
            } else {
                let cy = rect.y + rect.h * (a0 + a1) * 0.5;
                ((proj(rect.x + rect.w * 0.5, cy) - t_min) / span).clamp(0.0, 1.0)
            };
            let c = from.lerp(to, t);
            if vertical {
                let x0 = (rect.w * a0).round();
                let x1 = (rect.w * a1).round();
                if x1 > x0 {
                    self.fill_rect(Rect::new(rect.x + x0, rect.y, x1 - x0, rect.h), c);
                }
            } else {
                let y0 = (rect.h * a0).round();
                let y1 = (rect.h * a1).round();
                if y1 > y0 {
                    self.fill_rect(Rect::new(rect.x, rect.y + y0, rect.w, y1 - y0), c);
                }
            }
        }
    }

    /// 一维渐变（兼容旧接口）：等价于 dir = (1,0) 或 (0,1)。
    fn fill_gradient_rect(&mut self, rect: Rect, from: Color, to: Color, horizontal: bool) {
        let dir = if horizontal { (1.0, 0.0) } else { (0.0, 1.0) };
        self.fill_gradient_rect_dir(rect, from, to, dir);
    }

    /// 裁剪栈：滚动容器由框架自动压栈，组件不需要手动调用。
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);

    /// 当前主题。默认返回内置暗色主题，应用可注入自己的。
    fn theme(&self) -> &crate::theme::Theme {
        &crate::theme::DARK_THEME
    }
}

/// 事件上下文：组件通过它向框架提请"焦点/重绘/重排"。
///
/// 刻意不借用树的字段：只有值拷贝，组件无法从事件里偷改树结构。
pub struct EventCtx {
    /// 当前焦点节点 id（快照）。
    pub focus: Option<String>,
    /// 组件请求把焦点移交给谁（分发结束后由框架统一执行）。
    pub focus_request: Option<String>,
    #[doc(hidden)]
    pub redraw: bool,
    #[doc(hidden)]
    pub relayout: bool,
    pub viewport: Size,
    /// 本次分发中组件产生的意图。框架只负责搬运，不解释。
    #[doc(hidden)]
    pub commands: Vec<crate::command::Command>,

    /// 当前处于哪个路由阶段。
    pub phase: crate::event::EventPhase,
    /// 本次事件命中/捕获的目标节点 id。
    pub target: String,
    /// 置 true 后，框架立刻停止后续阶段（捕获拦截 / 冒泡截断）。
    pub stop_propagation: bool,
    /// 本次分发里**真正要求重排**的节点 id（框架内部记账，组件不要读写）。
    ///
    /// 用途：`Relayout` 的脏区要标在"请求者"身上，而不是命中目标身上
    /// （滚动时命中目标是行，请求者是外层滚动容器）。
    #[doc(hidden)]
    pub relayout_from: Option<String>,
    /// 置 true 时，即使返回 `Handled` 也继续冒泡（罕见，需显式请求）。
    #[doc(hidden)]
    pub propagate: bool,
    /// 指针捕获请求，分发结束后由框架落实。
    pub pointer_capture: Option<crate::event::PointerCapture>,
    /// 焦点守卫：在处理 `Focus` 事件时置 true 可以拒绝这次焦点变更。
    #[doc(hidden)]
    pub focus_vetoed: bool,
    /// 请求把 IME 候选窗钉在某个屏幕坐标（组件算好光标位置后填）。
    #[doc(hidden)]
    pub ime_candidate: Option<Pos>,
    /// 本次分发中组件提交的浮层请求（分发结束后由 `UiTree` 统一挂载）。
    #[doc(hidden)]
    pub popup_requests: Vec<PopupRequest>,
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
            relayout_from: None,
            propagate: false,
            pointer_capture: None,
            focus_vetoed: false,
            ime_candidate: None,
            popup_requests: Vec::new(),
        }
    }

    /// 请求把 IME 候选窗钉到这个屏幕坐标（由应用层落实）。
    pub fn set_ime_candidate(&mut self, pos: Pos) {
        self.ime_candidate = Some(pos);
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
        self.commands.push(crate::command::Command::app(name, args));
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
    /// 请求打开一个浮层（非模态）。
    ///
    /// 输入：`factory`（分发结束后才调用）、`anchor`（触发控件屏幕矩形）。
    /// 副作用：追加一条请求并标记重绘。
    pub fn request_popup(
        &mut self,
        factory: Box<dyn FnOnce() -> Box<dyn Widget>>,
        anchor: Rect,
    ) {
        self.popup_requests.push(PopupRequest {
            factory,
            anchor,
            modal: false,
        });
        self.redraw = true;
    }

    /// 请求打开一个**模态**浮层（阻塞底层事件）。
    pub fn request_modal_popup(
        &mut self,
        factory: Box<dyn FnOnce() -> Box<dyn Widget>>,
        anchor: Rect,
    ) {
        self.popup_requests.push(PopupRequest {
            factory,
            anchor,
            modal: true,
        });
        self.redraw = true;
    }

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

/// 动作参数的形态。网关据此做类型校验，Agent 据此知道该传什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    None,
    Text,
    Number,
    Bool,
}

impl InputKind {
    /// 参数名（约定：每个动作最多一个入参）。
    pub fn arg_name(self) -> &'static str {
        match self {
            InputKind::None => "",
            InputKind::Text => "text",
            InputKind::Number => "value",
            InputKind::Bool => "flag",
        }
    }

    /// 校验一个 JSON 值是否匹配。
    pub fn accepts(self, value: &crate::json::Json) -> bool {
        use crate::json::Json;
        match self {
            InputKind::None => matches!(value, Json::Null),
            InputKind::Text => matches!(value, Json::Str(_)),
            InputKind::Number => matches!(value, Json::Num(_)),
            InputKind::Bool => matches!(value, Json::Bool(_)),
        }
    }
}

/// 一个可被 Agent 调用的动作。
#[derive(Clone, Debug, PartialEq)]
pub struct ActionSpec {
    pub name: String,
    pub input: InputKind,
    /// 自定义参数名；`None` 时回退到 [`InputKind::arg_name`]（text/value/flag）。
    ///
    /// 用于参数名不落俗套的动作（例如 `set_color` 的参数叫 `color` 而非 `text`）。
    pub arg_name: Option<&'static str>,
}

impl ActionSpec {
    /// 用默认参数名（由 `InputKind` 决定）构造。
    pub fn new(name: impl Into<String>, input: InputKind) -> Self {
        Self {
            name: name.into(),
            input,
            arg_name: None,
        }
    }

    /// 用自定义参数名构造（参数名固定为 `'static str`，避免运行期分配）。
    pub fn named(name: impl Into<String>, input: InputKind, arg_name: &'static str) -> Self {
        Self {
            name: name.into(),
            input,
            arg_name: Some(arg_name),
        }
    }

    /// 网关从 `args` 里取参数时用的键名。
    ///
    /// 返回：自定义名（若有），否则 `input.arg_name()`。
    pub fn arg_name(&self) -> &str {
        self.arg_name.unwrap_or_else(|| self.input.arg_name())
    }

    /// 语义树里的紧凑表示：`{"name":"click","input":"none"}`。
    ///
    /// `arg_name` 为 `Some` 时额外输出 `"arg"` 字段（Agent 据此知道参数名）；
    /// 默认参数名的动作**不输出**该字段——保持既有 JSON 形状不变。
    pub fn to_json(&self) -> crate::json::Json {
        let obj = crate::json::Json::obj().set("name", self.name.clone()).set(
            "input",
            match self.input {
                InputKind::None => "none",
                InputKind::Text => "text",
                InputKind::Number => "number",
                InputKind::Bool => "bool",
            },
        );
        match self.arg_name {
            Some(name) => obj.set("arg", name),
            None => obj,
        }
    }
}
/// 语义描述：给 Agent 读的节点信息。
#[derive(Clone, Debug, PartialEq)]
pub struct Semantics {
    pub label: String,
    pub value: String,
    /// 角色（query 用；由框架在遍历时从 Widget::role() 自动注入）。
    pub role: Role,
    /// 该节点支持的动作名，例如 ["click", "set_text"]。
    pub actions: Vec<ActionSpec>,
    /// 额外键值（终端会填 cols/rows/scroll_offset）。
    pub props: Vec<(String, String)>,
    /// 结构化状态：Agent 直接消费（文本、光标、禁用…）。
    /// 是否被禁用（不参与 Tab 遍历，也不接收事件）。
    pub disabled: bool,
    /// 是否对 AI 开放（网关据此过滤，false 的节点 AI 不可见也不可操作）。
    pub exposed_to_ai: bool,
    /// 是否敏感（网关返回 JSON 时把 value 打码）。
    pub sensitive: bool,
    /// 是否持有键盘焦点。**框架拥有**：由 `UiTree::focus_path` 注入，
    /// 组件不得自行设置（组件自己的 focused 字段只用于渲染/按键门控）。
    pub focused: bool,
    /// 结构化状态：Agent 直接消费。
    pub state: crate::json::Json,
}

impl Default for Semantics {
    fn default() -> Self {
        Self {
            label: String::new(),
            value: String::new(),
            role: Role(""),
            actions: Vec::new(),
            props: Vec::new(),
            disabled: false,
            // 默认封禁：组件必须显式 .exposed(true) 才对 AI 可见
            exposed_to_ai: false,
            sensitive: false,
            focused: false,
            state: crate::json::Json::Null,
        }
    }
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

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 是否向 AI 暴露这个节点（网关据此过滤）。
    pub fn exposed(mut self, exposed: bool) -> Self {
        self.exposed_to_ai = exposed;
        self
    }

    /// 标记敏感：网关返回 JSON 时会把 value 打码。
    pub fn sensitive(mut self, sensitive: bool) -> Self {
        self.sensitive = sensitive;
        self
    }

    /// 写一个结构化状态字段（保留类型：数字还是数字、布尔还是布尔）。
    pub fn state(mut self, key: impl Into<String>, value: impl Into<crate::json::Json>) -> Self {
        let key = key.into();
        self.state = self.state.clone().set(&key, value.into());
        self
    }

    /// 声明一个动作（无参数）。
    pub fn action(mut self, name: impl Into<String>) -> Self {
        self.actions.push(ActionSpec::new(name, InputKind::None));
        self
    }

    /// 声明一个带参数类型的动作（参数名由 `input` 决定：text / value / flag）。
    pub fn action_spec(mut self, name: impl Into<String>, input: InputKind) -> Self {
        self.actions.push(ActionSpec::new(name, input));
        self
    }

    /// 声明一个**参数名自定义**的带参动作。
    ///
    /// 输入：动作名、参数类型、参数键名（`'static`，不分配）。
    /// 返回：自身。
    /// 用在哪：参数名不落俗套时（如 `set_color` 用 `color` 而不是 `text`）。
    /// 影响：网关按这个键从 `args` 取值，`to_json` 会额外输出 `"arg"` 字段。
    pub fn action_spec_named(
        mut self,
        name: impl Into<String>,
        input: InputKind,
        arg_name: &'static str,
    ) -> Self {
        self.actions.push(ActionSpec::named(name, input, arg_name));
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
    /// 树内唯一 id，例如 `"panel.list"`。
    ///
    /// 返回：稳定字符串引用（框架会大量比较它，别每次返回新分配的 `String`）。
    /// 约束：**同一棵树内必须唯一**——重复 id 会让 `add_node` 拒绝插入，
    /// 也会让 `find` / `dispatch_to` / 语义查询指向错误的节点。
    /// 一旦挂载就不应改变（`step` 会按 id 缓存矩形与 revision）。
    fn id(&self) -> &str;

    /// 语义角色：给 Agent 与 `query` 用的分类标签。
    ///
    /// 返回：`Role`（不是枚举，业务可自定义常量）。默认 `PANEL`。
    /// 副作用：无。`role` 会参与语义面哈希，改动它会让 revision +1。
    fn role(&self) -> Role {
        Role::PANEL
    }

    /// 自己在父容器里的伸缩参数（宽/高的 `Dimension`）。
    ///
    /// 返回：`FlexItem`；默认 `Auto`（由内容决定）。
    /// 副作用：无。容器在 `measure`/`arrange` 里读取它决定分配，**不写回**组件。
    fn flex(&self) -> FlexItem {
        FlexItem::default()
    }


    /// 是否可以获得键盘焦点（Tab 遍历、Modal 自动聚焦都用它）。
    ///
    /// 返回：`true` 表示愿意接收焦点。默认 `false`。
    /// 约束：返回 true **不等于**当前持有焦点——焦点由 `UiTree::focus_path` 决定，
    /// 组件应通过 `UiEvent::Focus(bool)` 得知自己拿到/失去焦点。
    fn focusable(&self) -> bool {
        false
    }

    /// 语义版本：组件自己维护的单调计数器，状态变化时 +1。
    ///
    /// 返回：当前版本；`0` 表示组件不自己维护。
    /// 副作用：无。框架的 `effective_revision = 本值 + 框架计数器`，
    /// 因此返回 0 只是"框架计数器独立生效"，**不是**关闭版本锁。
    fn revision(&self) -> u64 {
        0
    }

    /// 挂载：申请资源（订阅 Signal、启动后台任务）。
    ///
    /// 输入：`ctx.id`（本节点 id）、`ctx.invalidation`（共享失效队列）。
    /// 时机：树构建时与 `add_node` / `insert_node` 插入后立即调用，**先于首次布局**。
    /// 副作用：允许任意（这是唯一可以"申请资源"的地方）。
    /// 失败：无返回值；资源申请失败请自己 `eprintln!` 并降级，不要 panic。
    fn on_mount(&mut self, _ctx: &mut MountCtx) {}

    /// 卸载：释放资源（取消订阅、关闭 PTY）。
    ///
    /// 时机：`remove_node` 时对整棵被摘掉的子树**自底向上**调用，随后节点被 drop。
    /// 副作用：允许任意，但**不得**再访问树（此时树已不包含该节点）。
    /// 注意：关闭浮层（`is_open=false`）**不会**触发卸载——只是不画不命中。
    fn on_unmount(&mut self) {}

    /// 是否处于已打开状态。关闭的浮层不参与绘制、命中与索引。
    ///
    /// 返回：`true` 默认（普通节点永远"打开"）；浮层组件按自身状态返回。
    /// 副作用：无（框架把它当纯查询）。
    /// 重要：翻转它必须返回 `EventResult::Relayout`，否则索引不会重建，
    /// 关闭的节点会残留在 `paint_order` 里（既占绘制又可能被误命中）。
    fn is_open(&self) -> bool {
        true
    }

    /// 是否自己消费 Tab 键（而不是让框架切换焦点）。
    ///
    /// 返回：`true` = Tab 会派发给本组件（终端用它做补全）；默认 `false`。
    /// 副作用：无。框架在 `UiTree::dispatch` 里先问焦点节点，`true` 才放行。
    /// 注意：只有**当前焦点**组件会被询问，非焦点组件的返回值无意义。
    fn wants_tab(&self) -> bool {
        false
    }

    /// 布局缓存键（可选）。返回 `Some` 时框架直接用它，避免对 `id()` 做字符串哈希。
    ///
    /// 返回：`Some(key)` = 调用方保证在同一 widget 生命周期内稳定且全局唯一；
    /// `None` = 框架退回 `id()` 的 FNV-1a 哈希。
    /// 用途：容器（`Flex`/`Stack`）在构造时算一次，`measure`/`arrange` 就不再碰字符串。
    /// 注意：不要用会随布局变化的值（尺寸、约束）做键，否则缓存会失效或串味。
    fn cache_key(&self) -> Option<u64> {
        None
    }

    /// 是否订阅 `UiEvent::Tick`（每帧心跳）。
    ///
    /// 返回：`true` = 广播 `Tick` 时调用本组件的 `event`；默认 `false`。
    /// 副作用：无。框架据此把 `Tick` 广播从"遍历全树"降为"只发给订阅者"。
    /// 用途：需要周期性拉取外设数据的组件（终端读 PTY、动画）。
    /// 注意：不订阅就永远收不到 `Tick`——需要心跳的组件必须覆写它。
    fn wants_tick(&self) -> bool {
        false
    }

    /// 是否处于"模态"状态（模态时焦点与点击都被限制在该子树内）。
    ///
    /// 返回：`true` = 此时它是模态层；默认 `false`。
    /// 它描述的是"是否阻塞底层"（交互语义），
    /// `is_modal` 描述"是否阻塞底层"（交互语义）。两者本来被混为一谈，
    /// 导致想要"画在上层但不阻塞"的浯层无法表达。
    fn is_modal(&self) -> bool {
        false
    }

    /// 是否被禁用。禁用节点会被排除在 Tab 遍历之外。
    ///
    /// 返回：`true` 表示不接收键盘焦点、不参与 Tab。
    /// 副作用：无。注意它**不**阻止鼠标事件——组件应在 `event` 里自己判 `disabled`。
    /// 与 `semantics().disabled` 不一致会导致 Agent 看到的状态与行为不符。
    fn is_disabled(&self) -> bool {
        false
    }

    /// 裁剪矩形（与 `rect()` 同一坐标系）。子节点只能落在这一区域内。
    ///
    /// 返回：`Some(rect)` 表示裁剪到该矩形（滚动容器声明视口）；`None` 不裁剪。
    /// 副作用：无。框架在渲染时自动 `push_clip` / `pop_clip`，组件无需手写。
    /// 注意：命中测试用 `rect ∩ clip` —— 看不见的区域点不到。
    fn clip(&self) -> Option<Rect> {
        None
    }

    /// 向下转型：容器需要就地修改子节点（虚拟化行复用）时用。
    ///
    /// 返回：`Some(self as &mut dyn Any)` 才能被 `downcast_mut`；默认 `None`（不支持）。
    /// 副作用：返回的是可变借用，调用方拿到的期间不能再借用树。
    /// 失败：`None` 表示该组件不支持向下转型，调用方应回退到"重建节点"。
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        None
    }

    /// 鼠标悬停时的光标形状（静态声明）。
    ///
    /// 返回：`Cursor`，由应用层映射到平台（框架不认识 `IDC_*`）。
    /// 副作用：无；但框会在**每次 MouseMove** 都问一次（实现要够快）。
    fn cursor(&self) -> Cursor {
        Cursor::Default
    }

    /// 按指针位置计算光标形状（需要几何判断的组件覆写它）。
    ///
    /// 输入：`pos` 为屏幕坐标（同一坐标系下的指针位置）。
    /// 返回：该位置对应的光标；默认退回 `cursor()`。
    /// 副作用：无。仅当节点被命中时调用。
    fn cursor_at(&self, _pos: Pos) -> Cursor {
        self.cursor()
    }

    /// 子节点（容器才有）。
    ///
    /// 返回：只读切片；默认空切片（叶子组件）。
    /// 副作用：无。框架用它建索引、算焦点顺序、递归查询语义。
    /// 约束：返回的顺序就是**树序**（影响绘制先后与 Tab 顺序）。
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }

    /// 可变子节点。返回 `None` 表示叶子组件。
    ///
    /// 返回：`Some(&mut Vec<..>)` 才能被 `add_node` / `insert_node` / `remove_node`
    /// 修改；默认 `None`（叶子）。
    /// 副作用：调用方拿到可变借用期间不能再借用树。
    /// 注意：直接改这个 Vec **不会**触发布局失效——请走 `UiTree` 的增删接口。
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        None
    }

    /// 内容版本号：**内在尺寸**可能变化时 +1（文本变了、行数变了…）。
    ///
    /// 返回：当前版本；默认 0 表示"尺寸永不变化"。
    /// 副作用：无。框架用它判断 `measure` 结果能否复用。
    /// 约束：只有**影响尺寸**的状态才 bump；纯颜色/悬停态不要 bump，
    /// 否则每帧都会白白失效重算。
    fn layout_version(&self) -> u64 {
        0
    }

    /// 上报期望尺寸。父容器据此做 flex 分配。
    ///
    /// 输入：`metrics`（字体度量）、`constraints`（父给的最小/最大尺寸）、`cache`（布局缓存）。
    /// 返回：受 `constraints` 约束后的尺寸。
    /// 副作用：允许缓存内部测量结果（通常由 `cache` 代劳），**不得**修改树结构。
    /// 约束：容器必须用 `cache.measure(child, …)` 而不是 `child.measure(…)`，
    /// 否则会绕过缓存与去重，大树上退化成指数级重复测量。
    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
        cache: &mut crate::layout::LayoutCache,
    ) -> Size;

    /// 父容器算好绝对矩形后回传。
    ///
    /// 输入：`rect`（父坐标系里的绝对矩形）、`metrics`、`cache`。
    /// 返回：无。
    /// 副作用：**应当**把 `rect` 存下来（`rect()` 后续要靠它），容器还要给子节点排布。
    /// 约束：递归由容器驱动（`layout::flex_arrange`），框架不预设子节点怎么摆。
    fn arrange(
        &mut self,
        rect: Rect,
        metrics: &dyn TextMetrics,
        cache: &mut crate::layout::LayoutCache,
    );

    /// 自己当前的矩形（`arrange` 之后有效）。
    ///
    /// 返回：父坐标系矩形；布局前是 `Rect::ZERO`（因此 `rect_of` 会拒绝未布局的节点）。
    /// 副作用：无（框架把它当纯查询，会高频调用）。
    fn rect(&self) -> Rect;

    /// 事件。返回 `Ignored` 表示不关心，让框架继续找别人。
    ///
    /// 输入：`ctx`（可提请焦点/捕获/重绘/发命令）、`event`（三阶段之一）。
    /// 返回：`EventResult`；`Handled` 阻止继续冒泡，`Redraw`/`Relayout` 追加标志。
    /// 副作用：允许改自身状态（这是唯一的状态变更入口之一，另一个是 `on_mount`）。
    /// 约束：**Capture 阶段只拦截，不要在这里做响应**——否则响应会执行两次。
    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    /// 出图。只允许通过 `RenderCtx`。
    ///
    /// 输入：`ctx`（`fill_rect` / `draw_text` / `draw_char` / 裁剪栈 / 主题）。
    /// 返回：无。
    /// 副作用：**不得有任何副作用**——不能改自身状态、不能 dispatch、不能读写外部可变状态。
    /// 原因：revision 的哈希时机在 `layout()` 之前，`render` 里改的状态不会被捕获。
    fn render(&mut self, _ctx: &mut dyn RenderCtx);

    /// 语义信息。给 Agent 看的**唯一**来源。
    ///
    /// 返回：`Semantics`；默认 `exposed_to_ai = false`（不给 AI 用）。
    /// 副作用：无（但会被高频调用：查询、哈希、推送）。
    /// 约束：会参与语义面哈希（label/value/state/disabled/sensitive/exposed/focused），
    /// 因此**不要**把悬停、动画帧等瞬时状态放进去，否则 revision 会被淹没。
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }

    /// 内边距：容器用它把子节点内缩。
    ///
    /// 返回：`Edges`（逻辑像素）；默认 `ZERO`。
    /// 副作用：无。只影响容器的排布，不改变自身矩形。
    fn padding(&self) -> Edges {
        Edges::ZERO
    }

    /// 本帧是否必须全量重绘（例如全屏遮罩、动画）。
    ///
    /// 返回：`true` 表示忽略脏区、整棵子树都画；默认 `false`。
    /// 副作用：无。返回 true 会让脏区剪枝失效，**只给真正每帧都变的节点用**。
    fn always_paints(&self) -> bool {
        false
    }

    /// 脏区剪枝：与脏区不相交就可以整棵子树跳过。
    ///
    /// 输入：当前帧脏区。返回：是否需要绘制本节点。
    /// 副作用：无。默认实现是 `always_paints() || dirty.intersects(rect())`。
    /// 注意：节点矩形为空（未布局）时 `intersects` 为 false → 整棵跳过，
    /// 这是"未布局不绘制"的机制保证。
    fn needs_paint(&self, dirty: &crate::tree::DirtyRegion) -> bool {
        self.always_paints() || dirty.intersects(self.rect())
    }
}
