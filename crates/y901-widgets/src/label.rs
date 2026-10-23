//! 官方组件：文本标签。
//!
//! 只做一件事：把一段文字画出来。它是"如何写一个 Widget"的最小样本。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, FlexItem, MountCtx, Pos, Rect, RenderCtx, Role,
    Semantics, Size, TextStyle, TextMetrics, UiEvent, Widget,
};

pub struct TextLabel {
    id: String,
    text: String,
    /// 可选的响应式来源：挂载后由 Signal 驱动文本。
    signal: Option<y901_ui::Signal<String>>,
    color: Color,
    item: FlexItem,
    rect: Rect,
}

impl TextLabel {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            signal: None,
            color: Color::WHITE,
            item: FlexItem::default(),
            rect: Rect::ZERO,
        }
    }

    /// 绑定一个 Signal<String>：值变化 → 框架自动标脏重绘。
    pub fn bind(mut self, signal: y901_ui::Signal<String>) -> Self {
        self.text = signal.get();
        self.signal = Some(signal);
        self
    }

    /// 当前应显示的文本（优先取 Signal）。
    fn current_text(&self) -> String {
        match &self.signal {
            Some(sig) => sig.get(),
            None => self.text.clone(),
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

    pub fn text(&self) -> String {
        self.current_text()
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
        c.constrain(Size::new(metrics.measure_text(&self.current_text(), &style).w, metrics.line_height(&style)))
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
            &self.current_text(),
            &TextStyle::fg(self.color),
        );
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(self.current_text())
            .value(self.current_text())
            .action("set_text")
    }

    fn on_mount(&mut self, ctx: &mut MountCtx) {
        // bind() 之后无需手动订阅：挂载时自动接上失效队列
        if let Some(sig) = &self.signal {
            sig.subscribe(ctx.id.clone(), ctx.invalidation.clone());
        }
    }

    fn on_unmount(&mut self) {
        if let Some(sig) = &self.signal {
            sig.unsubscribe(&self.id);
        }
    }
}

impl TextLabel {
    /// 用 Signal 的当前值刷新快照文本。
    pub fn refresh(&mut self) {
        if let Some(sig) = &self.signal {
            self.text = sig.get();
        }
    }
}