//! Badge：小徽标（胶囊标签 / 纯圆点）。
//!
//! 为什么需要它：列表项旁边"未读 3"、"beta"、"失败"这类短标记非常常见，
//! 没有它就只能拼 TextLabel + 背景，颜色语义（成功/警告/错误）还得自己定。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

/// 徽标语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeKind {
    /// 中性灰。
    Default,
    /// 强调（蓝）。
    Accent,
    /// 成功（绿）。
    Success,
    /// 警告（黄）。
    Warning,
    /// 错误（红）。
    Error,
}

impl BadgeKind {
    /// 底色：Accent 取主题按钮按下态，其余从语义色压暗 0.62。
    fn bg(self, theme: &y901_ui::Theme) -> Option<Color> {
        let dim = |c: y901_ui::Color| y901_ui::Color::rgba(c.r * 0.62, c.g * 0.62, c.b * 0.62, 1.0);
        match self {
            BadgeKind::Default => None,
            BadgeKind::Accent => Some(theme.style(y901_ui::Role::BUTTON, y901_ui::State::Pressed).bg),
            BadgeKind::Success => Some(dim(theme.success_color())),
            BadgeKind::Warning => Some(dim(theme.warning_color())),
            BadgeKind::Error => Some(dim(theme.error_color())),
        }
    }

    /// 文字色：彩色底用白字；`Default` 用主题正文色
    ///（否则 LIGHT 主题下浅底上白字看不清）。
    fn fg(self, theme: &y901_ui::Theme) -> Color {
        match self {
            BadgeKind::Default => theme.style(y901_ui::Role::TEXT, y901_ui::State::Normal).fg,
            _ => Color::WHITE,
        }
    }

    /// 语义名。
    pub fn as_str(self) -> &'static str {
        match self {
            BadgeKind::Default => "default",
            BadgeKind::Accent => "accent",
            BadgeKind::Success => "success",
            BadgeKind::Warning => "warning",
            BadgeKind::Error => "error",
        }
    }
}

/// 文字高度（12px 字 + 上下 2px 内边距）。
const BADGE_H: f32 = 18.0;
/// 左右内边距。
const PAD_X: f32 = 8.0;
/// 纯圆点直径。
const DOT_D: f32 = 8.0;

/// 小徽标。
pub struct Badge {
    id: String,
    text: String,
    kind: BadgeKind,
    dot_only: bool,
    rect: Rect,
    item: FlexItem,
    /// 可选样式覆盖（仅容器型表面）。
    style: Option<crate::style::WidgetStyle>,
    /// DPI 缩放（`measure` 里唯一写入；未测过时 1.0）。
    scale: f32,
}

impl Badge {
    /// 带文字的徽标。
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            kind: BadgeKind::Default,
            dot_only: false,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            style: None,
            scale: 1.0,
        }
    }

    /// 只有圆点、没有文字。
    pub fn dot(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: String::new(),
            kind: BadgeKind::Default,
            dot_only: true,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            style: None,
            scale: 1.0,
        }
    }

    /// 设置语义类型。
    pub fn kind(mut self, kind: BadgeKind) -> Self {
        self.kind = kind;
        self
    }

    /// **覆盖容器型表面**（背景 / 描边 / 圆角）。
    ///
    /// 注意：语义色（如 switch 轨道、error 红、empty 灰）**不**由它覆盖 ——
    /// 那些走 `Theme`，否则会破坏 DARK/LIGHT 自适应。
    pub fn style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }
}

impl Widget for Badge {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::TEXT
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        if self.dot_only {
            return c.constrain(Size::new(DOT_D * self.scale, DOT_D * self.scale));
        }
        let style = TextStyle::fg(Color::WHITE);
        let tw = m.measure_text(&self.text, &style).w;
        c.constrain(Size::new(tw + PAD_X * 2.0 * self.scale, BADGE_H * self.scale))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let fallback = {
            let theme = ctx.theme();
            theme.style(Role::PANEL, State::Hovered).bg
        };
        let bg = self.style.and_then(|s| s.bg).or_else(|| self.kind.bg(ctx.theme())).unwrap_or(fallback);
        let radius = if self.dot_only {
            self.rect.w.min(self.rect.h) * 0.5
        } else {
            match self.style.map(|s| s.radius) {
                Some(r) if r > 0.0 => r,
                _ => self.rect.h * 0.5,
            }
        };
        ctx.fill_round_rect(self.rect, bg, radius);
        if self.dot_only {
            return;
        }
        let style = TextStyle::fg(self.kind.fg(ctx.theme()));
        let tw = ctx.measure_text(&self.text, &style).w;
        let tx = self.rect.x + (self.rect.w - tw) * 0.5;
        let ty = self.rect.y + (self.rect.h - ctx.line_height(&style)) * 0.5;
        ctx.draw_text(Pos::new(tx, ty), &self.text, &style);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("badge {}", self.id))
            .value(self.text.clone())
            .prop("kind", self.kind.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    #[test]
    fn text_badge_is_wider_than_dot() {
        let mut a = Badge::new("a", "beta");
        let mut b = Badge::dot("b");
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(200.0, 100.0));
        let sa = a.measure(&Metrics, c, &mut cache);
        let sb = b.measure(&Metrics, c, &mut cache);
        assert!(sa.w > sb.w && sb.w == DOT_D, "{} vs {}", sa.w, sb.w);
        assert_eq!(sa.h, BADGE_H);
    }
}