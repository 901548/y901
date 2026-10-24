//! 官方组件：按钮。
//!
//! 它**不知道**点击之后会发生什么：
//! 只把一条 `Command::App` 交出去，由应用层决定语义。
//! 这样 Button 永远不会去认识 Terminal、FileTree 或任何别的组件。

use y901_ui::{
    Color, Command, Constraints, EventCtx, EventResult, FlexItem, Json, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextStyle, TextMetrics, UiEvent, Widget,
};


pub struct Button {
    id: String,
    label: String,
    /// 点击时发给应用层的命令名；`args` 原样透传。
    command: String,
    args: Json,
    item: FlexItem,
    rect: Rect,
    disabled: bool,
    hovered: bool,
    pressed: bool,
}

impl Button {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            command: "click".to_string(),
            args: Json::obj(),
            item: FlexItem::default(),
            rect: Rect::ZERO,
            disabled: false,
            hovered: false,
            pressed: false,
        }
    }

    /// 点击时发什么命令 —— 由使用方指定，按钮不解释。
    pub fn on_click(mut self, command: impl Into<String>, args: Json) -> Self {
        self.command = command.into();
        self.args = args;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    fn inside(&self, pos: Pos) -> bool {
        self.rect.contains(pos)
    }
}

impl Widget for Button {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::BUTTON
    }

    fn focusable(&self) -> bool {
        true
    }

    fn is_disabled(&self) -> bool {
        self.disabled
    }

    fn cursor(&self) -> y901_ui::Cursor {
        if self.disabled {
            y901_ui::Cursor::Default
        } else {
            y901_ui::Cursor::Pointer
        }
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
        let style = TextStyle::fg(Color::WHITE);
        let text = metrics.measure_text(&self.label, &style);
        // 左右各 12px 内边距
        c.constrain(Size::new(text.w + 24.0, metrics.line_height(&style) + 8.0))
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

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if self.disabled {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::MouseMove { pos } => {
                let was = self.hovered;
                self.hovered = self.inside(*pos);
                if was != self.hovered {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::MouseDown { pos, .. } if self.inside(*pos) => {
                self.pressed = true;
                ctx.request_focus(self.id.clone());
                EventResult::Redraw
            }
            UiEvent::MouseUp { pos, .. } => {
                let was_pressed = self.pressed;
                self.pressed = false;
                if was_pressed && self.inside(*pos) {
                    // 只发意图，不关心谁处理、处理什么
                    ctx.emit_app(self.command.clone(), self.args.clone());
                    EventResult::Redraw
                } else if was_pressed {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::Action { name, .. } if name == "click" => {
                ctx.emit_app(self.command.clone(), self.args.clone());
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 颜色全部来自主题：组件不持有任何业务色
        let state = if self.disabled {
            State::Disabled
        } else if self.pressed {
            State::Pressed
        } else if self.hovered {
            State::Hovered
        } else {
            State::Normal
        };
        let s = ctx.theme().style(Role::BUTTON, state);
        ctx.fill_rect(self.rect, s.bg);

        let style = TextStyle::fg(s.fg);
        let text_w = ctx.measure_text(&self.label, &style).w;
        let x = self.rect.x + (self.rect.w - text_w) * 0.5;
        let y = self.rect.y + (self.rect.h - ctx.line_height(&style)) * 0.5;
        ctx.draw_text(Pos::new(x, y), &self.label, &style);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.label.clone())
            .value(self.command.clone())
            .action("click")
            // disabled 必须进语义：Agent 据此判断"现在能不能点"，
            // 也是 revision 哈希覆盖的可见字段之一。
            .disabled(self.disabled)
            .state("disabled", self.disabled)
            .prop("hovered", self.hovered.to_string())
            .prop("pressed", self.pressed.to_string())
    }
}

/// 方便构造 App 命令的辅助函数。
pub fn app_command(name: &str) -> Command {
    Command::app(name, Json::obj())
}
