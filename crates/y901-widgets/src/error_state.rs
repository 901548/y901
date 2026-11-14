//! ErrorState：`EmptyState` 的错误版（红色图标 + 红色标题 + 重试按钮）。
//!
//! 为什么和 EmptyState 分开而不是加一个 `kind` 参数：
//! 两者的**语义**不同（"没有数据" vs "出错了，可以重试"），颜色规则也不同
//! （错误态必须让用户一眼看出"出事了"）。合成一个组件会让调用方每次都要传 kind。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

/// 内圈直径（从图标直径派生，避免两个磁数各自漂移）。
const INNER_D: f32 = super::state_layout::ICON_D * 0.42;

/// 错误状态占位。
pub struct ErrorState {
    id: String,
    title: String,
    description: Option<String>,
    action: Option<(String, y901_ui::Command)>,
    rect: Rect,
    item: FlexItem,
    /// DPI 缩放（`measure` 里唯一写入；未测过时 1.0）。
    scale: f32,
    action_rect: Rect,
}

impl ErrorState {
    /// 用标题创建（例如"加载失败"）。
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            action: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            scale: 1.0,
            action_rect: Rect::ZERO,
        }
    }

    /// 追加错误详情。
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// 追加"重试"之类的操作按钮。
    pub fn action(mut self, label: impl Into<String>, cmd: y901_ui::Command) -> Self {
        self.action = Some((label.into(), cmd));
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 期望高度：图标 + 标题 +（说明）+（按钮）。
    fn content_height(&self) -> f32 {
        super::state_layout::content_height(self.scale, self.description.is_some(), self.action.is_some())
    }
}

impl Widget for ErrorState {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        c.constrain(Size::new(c.max.w, self.content_height()))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        self.action_rect = super::state_layout::action_rect(rect, self.scale);
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if let UiEvent::MouseUp { pos, .. } = event {
            if self.action.is_some() && self.action_rect.contains(*pos) {
                if let Some((_, cmd)) = self.action.as_ref() {
                    ctx.emit_command(crate::command_with_args(cmd, y901_ui::json::Json::Null));
                }
                return EventResult::Redraw;
            }
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        // 先拷颜色：`ctx.theme()` 持有 `&ctx`，后续绘制要 `&mut ctx`。
        let error_red = ctx.theme().error_color();
        let (muted, accent) = {
            let theme = ctx.theme();
            (
                theme.muted_text_color(),
                theme.style(Role::BUTTON, State::Pressed).bg,
            )
        };

        let mut y = self.rect.y + 8.0;
        let cx = self.rect.x + self.rect.w * 0.5;
        // 错误红圆形占位（半透明底 + 实心内圈）
        ctx.fill_round_rect(
            Rect::new(cx - super::state_layout::ICON_D * self.scale * 0.5, y, super::state_layout::ICON_D * self.scale, super::state_layout::ICON_D * self.scale),
            Color::rgba(error_red.r, error_red.g, error_red.b, 0.18),
            (super::state_layout::ICON_D * self.scale) * 0.5,
        );
        ctx.fill_round_rect(
            Rect::new(
                cx - INNER_D * self.scale * 0.5,
                y + super::state_layout::ICON_D * self.scale * 0.5 - INNER_D * self.scale * 0.5,
                INNER_D * self.scale,
                INNER_D * self.scale,
            ),
            error_red,
            INNER_D * self.scale * 0.5,
        );
        y += (super::state_layout::ICON_D + 16.0) * self.scale;

        // 标题：错误红
        let title_style = TextStyle::fg(error_red);
        let tw = ctx.measure_text(&self.title, &title_style).w;
        ctx.draw_text(Pos::new(cx - tw * 0.5, y), &self.title, &title_style);
        y += 24.0 * self.scale;

        // 说明：弱化色
        if let Some(desc) = self.description.as_ref() {
            y += 8.0 * self.scale;
            let style = TextStyle::fg(muted);
            let dw = ctx.measure_text(desc, &style).w;
            ctx.draw_text(Pos::new(cx - dw * 0.5, y), desc, &style);
        }

        // 重试按钮
        if let Some((label, _)) = self.action.as_ref() {
            ctx.fill_round_rect(self.action_rect, accent, 6.0);
            let style = TextStyle::fg(Color::WHITE);
            let lw = ctx.measure_text(label, &style).w;
            let ly = self.action_rect.y + (super::state_layout::ACTION_H * self.scale - ctx.line_height(&style)) * 0.5;
            ctx.draw_text(
                Pos::new(self.action_rect.x + (self.action_rect.w - lw) * 0.5, ly),
                label,
                &style,
            );
        }
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(true)
            .label(format!("error {}", self.id))
            .value(self.title.clone())
            .prop("kind", "error")
            .action("retry");
        if let Some(d) = self.description.as_ref() {
            s = s.prop("description", d.clone());
        }
        s
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
    fn height_grows_with_optional_parts() {
        let mut a = ErrorState::new("e", "加载失败");
        let mut b = ErrorState::new("e", "加载失败")
            .description("连接超时")
            .action("确定", y901_ui::Command::app("error_state:activate", y901_ui::json::Json::Null));
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(400.0, 400.0));
        assert!(b.measure(&Metrics, c, &mut cache).h > a.measure(&Metrics, c, &mut cache).h);
    }

    #[test]
    fn retry_only_fires_inside_its_rect() {
        
        let mut e = ErrorState::new("e", "t").action("确定", y901_ui::Command::app("error_state:activate", y901_ui::json::Json::Null));
        let mut cache = LayoutCache::new();
        e.arrange(Rect::new(0.0, 0.0, 400.0, 200.0), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 200.0));
        e.event(&mut ctx, &UiEvent::MouseUp { pos: Pos::new(4.0, 4.0), button: y901_ui::MouseButton::Left });
        assert!(ctx.commands.is_empty());
        let r = e.action_rect;
        e.event(&mut ctx, &UiEvent::MouseUp { pos: Pos::new(r.x + 4.0, r.y + 4.0), button: y901_ui::MouseButton::Left });
        assert_eq!(ctx.commands.len(), 1);
    }
}