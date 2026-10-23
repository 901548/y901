//! 浮层组件：`Menu`（Overlay）与 `Dialog`（Modal）。
//!
//! 都设计成**全屏宿主**：应用把它们放进 `Stack`（所有子节点同尺寸），
//! 于是它们的 rect 就是整个视口 —— 这样"点外部关闭/吞掉"才有几何依据。

use y901_ui::{
    Command, Constraints, Edges, EventCtx, EventResult, FlexItem, Json, Layer, Pos, Rect, RenderCtx,
    Role, Semantics, Size, State, TextStyle, TextMetrics, UiEvent, Widget,
};

/// 菜单项高度。
const ITEM_H: f32 = 26.0;
/// 菜单面板内边距。
const PAD: f32 = 6.0;

pub struct Menu {
    id: String,
    items: Vec<String>,
    /// 触发时发给应用层的命令名。
    action: String,
    open: bool,
    /// 全屏 rect（宿主给的）。
    rect: Rect,
    /// 面板 rect（内容区）。
    panel: Rect,
    hover: Option<usize>,
    highlighted: usize,
    item: FlexItem,
}

impl Menu {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            action: "menu.select".to_string(),
            open: false,
            rect: Rect::ZERO,
            panel: Rect::ZERO,
            hover: None,
            highlighted: 0,
            item: FlexItem::default(),
        }
    }

    pub fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    pub fn action(mut self, name: impl Into<String>) -> Self {
        self.action = name.into();
        self
    }

    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = 0;
    }

    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    pub fn close(&mut self) {
        self.open = false;
        self.hover = None;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    fn item_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.panel.x,
            self.panel.y + PAD + index as f32 * ITEM_H,
            self.panel.w,
            ITEM_H,
        )
    }
}

impl Widget for Menu {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("menu")
    }

    fn layer(&self) -> Layer {
        Layer::Overlay
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(c.max)
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = rect;
        let w = 220.0_f32.min(rect.w - 16.0);
        let h = PAD * 2.0 + self.items.len() as f32 * ITEM_H;
        self.panel = Rect::new(rect.x + 16.0, rect.y + 16.0, w, h);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn clip(&self) -> Option<Rect> {
        Some(self.rect)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if !self.open {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::MouseMove { pos } => {
                let hover = (0..self.items.len()).find(|i| self.item_rect(*i).contains(*pos));
                if hover != self.hover {
                    self.hover = hover;
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::MouseDown { pos, .. } => {
                if self.panel.contains(*pos) {
                    if let Some(i) = (0..self.items.len()).find(|i| self.item_rect(*i).contains(*pos)) {
                        ctx.emit_app(
                            self.action.clone(),
                            Json::obj()
                                .set("index", i as u32)
                                .set("label", self.items[i].clone()),
                        );
                    }
                }
                // 无论点哪里，菜单都关闭；并吞掉这次点击，避免穿透到底层
                self.close();
                ctx.stop_propagation();
                EventResult::Redraw
            }
            UiEvent::Key {
                key: y901_ui::Key::Escape,
                ..
            } => {
                self.close();
                EventResult::Redraw
            }
            UiEvent::Key { key, .. } if !self.items.is_empty() => match key {
                y901_ui::Key::Down => {
                    self.highlighted = (self.highlighted + 1) % self.items.len();
                    EventResult::Redraw
                }
                y901_ui::Key::Up => {
                    self.highlighted = (self.highlighted + self.items.len() - 1) % self.items.len();
                    EventResult::Redraw
                }
                y901_ui::Key::Enter => {
                    let i = self.highlighted.min(self.items.len() - 1);
                    ctx.emit_app(
                        self.action.clone(),
                        Json::obj()
                            .set("index", i as u32)
                            .set("label", self.items[i].clone()),
                    );
                    self.close();
                    EventResult::Redraw
                }
                y901_ui::Key::Escape => {
                    self.close();
                    EventResult::Redraw
                }
                _ => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if !self.open {
            return;
        }
        // 半透明遮罩：视觉上把注意力收到菜单上
        ctx.fill_rect(self.rect, y901_ui::Color::rgba(0.0, 0.0, 0.0, 0.25));

        let panel_style = ctx.theme().style(Role("panel"), State::Normal);
        ctx.fill_rect(self.panel, panel_style.bg);
        ctx.stroke_rect(self.panel, y901_ui::Color::rgba(0.35, 0.38, 0.45, 1.0), 1.0);

        for (i, label) in self.items.iter().enumerate() {
            let r = self.item_rect(i);
            if self.hover == Some(i) || self.highlighted == i {
                let hover = ctx.theme().style(Role::LIST_ITEM, State::Hovered);
                ctx.fill_rect(r, hover.bg);
            }
            let fg = ctx.theme().style(Role::LIST_ITEM, State::Normal).fg;
            ctx.draw_text(
                Pos::new(r.x + 10.0, r.y + 4.0),
                label,
                &TextStyle::fg(fg),
            );
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label("menu")
            .action("select")
            .state("open", self.open)
            .state("items", self.items.len() as u32)
    }
}

/// 模态对话框：吃掉所有外部点击，焦点由框架自动移入。
pub struct Dialog {
    id: String,
    title: String,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    panel: Rect,
    open: bool,
    highlighted: usize,
    item: FlexItem,
}

impl Dialog {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            children: Vec::new(),
            rect: Rect::ZERO,
            panel: Rect::ZERO,
            open: false,
            highlighted: 0,
            item: FlexItem::default(),
        }
    }

    pub fn push(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }

    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = 0;
    }

    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

impl Widget for Dialog {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("dialog")
    }

    fn layer(&self) -> Layer {
        Layer::Modal
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(c.max)
    }

    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = rect;
        let w = 420.0_f32.min(rect.w - 40.0);
        let h = 220.0_f32.min(rect.h - 40.0);
        self.panel = Rect::new(
            rect.x + (rect.w - w) * 0.5,
            rect.y + (rect.h - h) * 0.5,
            w,
            h,
        );

        let inner = self.panel.inset(Edges::all(16.0));
        let body = Rect::new(inner.x, inner.y + 28.0, inner.w, (inner.h - 28.0).max(0.0));
        if let Some(first) = self.children.first_mut() {
            first.arrange(body, m, cache);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if !self.open {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::MouseDown { .. } | UiEvent::MouseMove { .. } | UiEvent::Wheel { .. } => {
                // 面板内的点击交给子节点（框架已经先派发过），面板外一律吞掉
                ctx.stop_propagation();
                EventResult::Handled
            }
            UiEvent::Key {
                key: y901_ui::Key::Escape,
                ..
            } => {
                self.close();
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if !self.open {
            return;
        }
        ctx.fill_rect(self.rect, y901_ui::Color::rgba(0.0, 0.0, 0.0, 0.55));
        let panel_style = ctx.theme().style(Role("panel"), State::Normal);
        ctx.fill_rect(self.panel, panel_style.bg);
        ctx.stroke_rect(self.panel, y901_ui::Color::rgba(0.40, 0.44, 0.52, 1.0), 1.0);
        ctx.draw_text(
            Pos::new(self.panel.x + 16.0, self.panel.y + 10.0),
            &self.title,
            &TextStyle::fg(panel_style.fg),
        );
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(self.title.clone())
            .action("close")
            .state("open", self.open)
    }
}

/// 便捷构造：菜单项命令。
pub fn menu_command(menu: &str, open: bool) -> Command {
    Command::node(menu, if open { "open" } else { "close" }, Json::obj())
}