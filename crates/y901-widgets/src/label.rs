//! 官方组件：文本标签。
//!
//! 只做一件事：把一段文字画出来。它是"如何写一个 Widget"的最小样本。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, FlexItem, Pos, Rect, RenderCtx, Role, Semantics,
    Size, TextStyle, TextMetrics, UiEvent, Widget,
};

pub struct TextLabel {
    id: String,
    text: String,
    color: Color,
    item: FlexItem,
    rect: Rect,
}

impl TextLabel {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            color: Color::WHITE,
            item: FlexItem::default(),
            rect: Rect::ZERO,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Widget for TextLabel {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::TEXT
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let style = TextStyle::fg(self.color);
        c.constrain(Size::new(metrics.measure_text(&self.text, &style).w, metrics.line_height(&style)))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _metrics: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        // 对外暴露一个 set_text 动作，谁调都行，组件不关心调用者
        if let UiEvent::Action { name, args } = event {
            if name == "set_text" {
                if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                    self.text = text.to_string();
                    return EventResult::Relayout;
                }
            }
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.draw_text(
            Pos::new(self.rect.x, self.rect.y),
            &self.text,
            &TextStyle::fg(self.color),
        );
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(self.text.clone())
            .value(self.text.clone())
            .action("set_text")
    }
}
