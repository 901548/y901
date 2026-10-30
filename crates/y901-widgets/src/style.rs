//! 内置控件的样式覆盖（Theme / Style Injection）。
//!
//! 设计目标：
//! 1. **零分配**：`WidgetStyle` 全是 `Copy` 的小字段，组件按值持有；
//!    设置样式不产生任何堆分配（`Option<WidgetStyle>` = 一个可空结构体）。
//! 2. **不破坏默认外观**：`None` 时行为与之前完全一致（走主题）。
//! 3. **不引入业务语义**：这里只有颜色/圆角/描边这些**绘制参数**，
//!    框架不认识"按钮长什么样才算对"，只负责把参数交给渲染。

use y901_ui::Color;

/// 单个控件的可选样式覆盖。所有字段都是"可选/零值即不覆盖"。
///
/// 用法：
/// ```
/// use y901_widgets::{Button, WidgetStyle};
/// use y901_ui::Color;
///
/// let style = WidgetStyle::default()
///     .gradient(
///         Color::rgba(0.1, 0.5, 0.9, 1.0),
///         Color::rgba(0.5, 0.2, 0.9, 1.0),
///         (0.0, 1.0),
///     )
///     .radius(10.0)
///     .focus_border(Color::WHITE, 2.0);
/// let btn = Button::new("ok", "OK").style(style);
/// # let _ = btn;
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WidgetStyle {
    /// 背景起点色。`None` = 用主题色。
    pub bg: Option<Color>,
    /// 背景终点色。`Some` 时背景按 [`WidgetStyle::gradient_dir`] 走线性渐变。
    pub bg_to: Option<Color>,
    /// 渐变方向（像素空间向量）。默认 `(0, 1)` = 上→下。
    pub gradient_dir: (f32, f32),
    /// 圆角半径（像素）。`0` = 直角。
    pub radius: f32,
    /// 常驻描边色。
    pub border: Option<Color>,
    /// 常驻描边宽度。
    pub border_width: f32,
    /// 焦点描边色（键盘焦点时额外画一圈）。
    pub focus_border: Option<Color>,
    /// 焦点描边宽度。
    pub focus_border_width: f32,
    /// 滚动条轨道色。
    pub bar_track: Option<Color>,
    /// 滚动条滑块色。
    pub bar_thumb: Option<Color>,
    /// 滚动条宽度（像素，`0` = 用默认）。
    pub bar_width: f32,
    /// 滚动条圆角。
    pub bar_radius: f32,
}

impl WidgetStyle {
    /// 背景纯色。
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// 背景渐变（起点 + 终点 + 方向）。
    pub fn gradient(mut self, from: Color, to: Color, dir: (f32, f32)) -> Self {
        self.bg = Some(from);
        self.bg_to = Some(to);
        self.gradient_dir = dir;
        self
    }

    /// 圆角半径。
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// 常驻描边。
    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.border = Some(color);
        self.border_width = width;
        self
    }

    /// 焦点描边（键盘焦点时额外一圈）。
    pub fn focus_border(mut self, color: Color, width: f32) -> Self {
        self.focus_border = Some(color);
        self.focus_border_width = width;
        self
    }

    /// 滚动条样式。
    pub fn scrollbar(mut self, track: Color, thumb: Color, width: f32, radius: f32) -> Self {
        self.bar_track = Some(track);
        self.bar_thumb = Some(thumb);
        self.bar_width = width;
        self.bar_radius = radius;
        self
    }

    /// 是否配置了背景（纯色或渐变）。
    pub fn has_bg(&self) -> bool {
        self.bg.is_some()
    }

    /// 画背景：有渐变色走渐变（且带圆角），否则纯色。
    ///
    /// 输入：`ctx`、目标矩形。副作用：提交 1 个绘制原语。
    pub fn paint_bg(&self, ctx: &mut dyn y901_ui::RenderCtx, rect: y901_ui::Rect) {
        if let Some(from) = self.bg {
            match self.bg_to {
                Some(to) => ctx.fill_gradient_round_rect(
                    rect,
                    from,
                    to,
                    self.gradient_dir,
                    self.radius,
                ),
                None => ctx.fill_round_rect(rect, from, self.radius),
            }
        }
    }

    /// 画常驻描边（如果有）。
    pub fn paint_border(&self, ctx: &mut dyn y901_ui::RenderCtx, rect: y901_ui::Rect) {
        if let Some(color) = self.border {
            if self.border_width > 0.0 {
                ctx.stroke_round_rect(rect, color, self.border_width, self.radius);
            }
        }
    }

    /// 画焦点描边（如果有）。
    pub fn paint_focus(&self, ctx: &mut dyn y901_ui::RenderCtx, rect: y901_ui::Rect, focused: bool) {
        if !focused {
            return;
        }
        if let Some(color) = self.focus_border {
            let w = self.focus_border_width.max(1.0);
            ctx.stroke_round_rect(rect, color, w, self.radius);
        }
    }
}
