//! 官方组件：按钮。
//!
//! 它**不知道**点击之后会发生什么：
//! 只把一条 `Command::App` 交出去，由应用层决定语义。
//! 这样 Button 永远不会去认识 Terminal、FileTree 或任何别的组件。

use y901_ui::{
    Color, Command, Constraints, EventCtx, EventResult, FlexItem, Json, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

pub struct Button {
    id: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
    label: String,
    /// 点击时发给应用层的命令名；`args` 原样透传。
    command: String,
    args: Json,
    item: FlexItem,
    rect: Rect,
    disabled: bool,
    hovered: bool,
    pressed: bool,
    /// 持久选中态（按钮组单选用）。
    selected: bool,
    /// 是否拥有键盘焦点（Tab 切过来时为 true）。
    focused: bool,
    /// 外部注入的样式（`None` = 走主题）。按值持有，零分配。
    style: Option<crate::style::WidgetStyle>,
}

impl Button {
    /// 新建按钮：`id` 树内唯一，`label` 是显示文字。
    ///
    /// 默认点击发送 `click` 命令（无参数），可用 `on_click` 覆盖。
    /// 副作用：无。
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            id,
            key,
            label: label.into(),
            command: "click".to_string(),
            args: Json::obj(),
            item: FlexItem::default(),
            rect: Rect::ZERO,
            disabled: false,
            hovered: false,
            pressed: false,
            selected: false,
            focused: false,
            style: None,
        }
    }

    /// 点击时发什么命令 —— 由使用方指定，按钮不解释。
    pub fn on_click(mut self, command: impl Into<String>, args: Json) -> Self {
        self.command = command.into();
        self.args = args;
        self
    }

    /// 选中态（构建器）：用于"按钮组单选"（主题切换、模式选择等）。
    ///
    /// 返回：自身。副作用：无（只改字段，渲染时映射到 `State::Selected`）。
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self
    }

    /// 运行时切换选中态。
    ///
    /// 副作用：只改字段——**不自动标脏**（组件碰不到 `UiTree`）。
    /// 调用方随后必须 `tree.invalidate(id)`。
    pub fn set_selected(&mut self, on: bool) {
        self.selected = on;
    }

    /// 禁用按钮：不参与 Tab 遍历，点击不再发命令。
    ///
    /// 返回：自身。副作用：语义里 `disabled=true`（Agent 可见）；
    /// 视觉上使用 `State::Disabled` 的配色。
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 设置 flex 伸缩参数（宽/高策略）。
    ///
    /// 返回：自身。副作用：无（只在父容器测量时被读取）。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    fn inside(&self, pos: Pos) -> bool {
        self.rect.contains(pos)
    }

    /// 注入样式（覆盖背景/圆角/描边/焦点圈）。
    ///
    /// 输入：样式值（`Copy`）。返回：`self`（链式）。
    /// 注意：不会自动标脏——调用方需 `tree.invalidate(id)`。
    pub fn style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// 运行时更换样式（`None` = 恢复主题）。
    pub fn set_style(&mut self, style: Option<crate::style::WidgetStyle>) -> bool {
        if self.style == style {
            return false;
        }
        self.style = style;
        true
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

    /// 向下转型：`Host` 运行时调 `set_selected` 的唯一路径
    /// （`UiTree` 不暴露具体类型，只能 `find_mut(id).as_any_mut()`）。
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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
            UiEvent::Focus(focused) => {
                if self.focused == *focused {
                    EventResult::Ignored
                } else {
                    self.focused = *focused;
                    EventResult::Redraw
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 颜色全部来自主题：组件不持有任何业务色
        // 优先级：Disabled > Selected > Pressed > Hovered > Normal。
        // Selected 是**持久**状态（"当前选中的主题"），必须始终可见；
        // Pressed/Hovered 是瞬时反馈，不覆盖它。
        let state = if self.disabled {
            State::Disabled
        } else if self.selected {
            State::Selected
        } else if self.pressed {
            State::Pressed
        } else if self.hovered {
            State::Hovered
        } else if self.focused {
            // 键盘焦点：比 Normal 明显，但不盖按下/选中的语义
            State::Focused
        } else {
            State::Normal
        };
        let s = ctx.theme().style(Role::BUTTON, state);
        // 外部样式优先：有注入就用注入的，否则回到主题。
        match self.style {
            Some(style) if style.has_bg() => {
                style.paint_bg(ctx, self.rect);
                style.paint_border(ctx, self.rect);
                style.paint_focus(ctx, self.rect, self.focused);
            }
            _ => {
                ctx.fill_rect(self.rect, s.bg);
                // 主题路径下的键盘焦点：用 fg 色画一圈（无样式注入时也能看出焦点）
                if self.focused && state == State::Focused {
                    ctx.stroke_round_rect(self.rect, s.fg, 2.0, 0.0);
                }
            }
        }

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
            // disabled 是**顶层语义字段**，不进 state：
            // 两处重复会让 Agent 困惑"哪个是真的"，也会让哈希范围含糊。
            .disabled(self.disabled)
            .prop("hovered", self.hovered.to_string())
            .prop("pressed", self.pressed.to_string())
            // 选中态是**持久状态**：放 state（结构化），供 Agent 判断
            // "当前是哪个主题/模式"。hovered/pressed 是瞬时的，留在 prop。
            .state("selected", self.selected)
    }
}

/// 方便构造 App 命令的辅助函数。
pub fn app_command(name: &str) -> Command {
    Command::app(name, Json::obj())
}
