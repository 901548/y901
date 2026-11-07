//! 阶段 0 能力：浮层关闭请求 + hover 事件冒泡。
//!
//! 说明：容器统一用 `Vec<Box<dyn Widget>>` 保存子节点（树依赖 `children_mut()` 建索引），
//! 单子节点的"包装"也走 Vec —— 早期版本返回 `None` 导致 `hit_test` 找不到任何节点。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, MouseButton, Pos, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};

struct Metrics;

impl TextMetrics for Metrics {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 记录事件的叶子控件；可在 MouseDown 时请求关闭某个浮层。
struct Leaf {
    id: String,
    rect: Rect,
    log: Rc<RefCell<Vec<String>>>,
    close_on_down: Option<String>,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(50.0, 50.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::MouseEnter => {
                self.log.borrow_mut().push(format!("{}:enter", self.id));
                EventResult::Handled
            }
            UiEvent::MouseLeave => {
                self.log.borrow_mut().push(format!("{}:leave", self.id));
                EventResult::Handled
            }
            UiEvent::MouseDown { .. } => {
                if let Some(id) = self.close_on_down.clone() {
                    ctx.request_close_popup(id);
                }
                self.log.borrow_mut().push(format!("{}:down", self.id));
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 容器：把子节点按 100×100 铺满自身矩形（单子节点场景）。
struct Wrap {
    id: String,
    rect: Rect,
    children: Vec<Box<dyn Widget>>,
}

impl Wrap {
    fn new(id: &str, child: Box<dyn Widget>) -> Self {
        Self {
            id: id.to_string(),
            rect: Rect::ZERO,
            children: vec![child],
        }
    }
}

impl Widget for Wrap {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(100.0, 100.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        for ch in self.children.iter_mut() {
            ch.arrange(rect, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn leaf(id: &str, log: &Rc<RefCell<Vec<String>>>) -> Leaf {
    Leaf {
        id: id.to_string(),
        rect: Rect::ZERO,
        log: log.clone(),
        close_on_down: None,
    }
}

#[test]
fn hover_enter_reaches_the_hit_node() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new(Box::new(Wrap::new("outer", Box::new(leaf("inner", &log)))));
    tree.layout(Size::new(200.0, 200.0), &Metrics);

    tree.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(20.0, 20.0),
    });
    let entries = log.borrow().clone();
    assert!(
        entries.iter().any(|e| e == "inner:enter"),
        "命中节点应收到 enter，实际 {entries:?}"
    );
}

#[test]
fn request_close_popup_closes_the_open_popup() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new(Box::new(Wrap::new(
        "outer",
        Box::new(Leaf {
            id: "inner".into(),
            rect: Rect::ZERO,
            log: log.clone(),
            close_on_down: Some("tip".into()),
        }),
    )));
    tree.layout(Size::new(200.0, 200.0), &Metrics);

    assert!(
        tree.open_popup(
            Box::new(Leaf {
                id: "tip".into(),
                rect: Rect::ZERO,
                log: Rc::default(),
                close_on_down: None,
            }),
            Rect::new(10.0, 10.0, 20.0, 20.0)
        ),
        "浮层应打开"
    );
    assert_eq!(tree.popups().len(), 1);

    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(20.0, 20.0),
        button: MouseButton::Left,
    });
    assert_eq!(
        tree.popups().len(),
        0,
        "request_close_popup 应在分发结束后真正关闭浮层"
    );
}

#[test]
fn closing_an_unknown_popup_is_harmless() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new(Box::new(Wrap::new(
        "outer",
        Box::new(Leaf {
            id: "inner".into(),
            rect: Rect::ZERO,
            log: log.clone(),
            close_on_down: Some("does-not-exist".into()),
        }),
    )));
    tree.layout(Size::new(200.0, 200.0), &Metrics);
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(20.0, 20.0),
        button: MouseButton::Left,
    });
    assert_eq!(tree.popups().len(), 0, "关闭不存在的浮层不应 panic");
}

/// 祖先记录器：容器自己实现 event 转发（把收到的 Enter/Leave 记下来）。
struct RecordingWrap {
    id: String,
    rect: Rect,
    log: Rc<RefCell<Vec<String>>>,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for RecordingWrap {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(100.0, 100.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        for ch in self.children.iter_mut() {
            ch.arrange(rect, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::MouseEnter => {
                self.log.borrow_mut().push(format!("{}:enter", self.id));
                EventResult::Handled
            }
            UiEvent::MouseLeave => {
                self.log.borrow_mut().push(format!("{}:leave", self.id));
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn ancestors_receive_enter_and_leave() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let inner_log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new(Box::new(RecordingWrap {
        id: "outer".into(),
        rect: Rect::ZERO,
        log: log.clone(),
        children: vec![Box::new(leaf("inner", &inner_log))],
    }));
    tree.layout(Size::new(200.0, 200.0), &Metrics);

    // 进入 inner：inner 与 outer 都应收到 enter
    tree.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(20.0, 20.0),
    });
    let events = log.borrow().clone();
    assert!(
        events.iter().any(|e| e == "outer:enter"),
        "祖先应收到 enter（冒泡），实际 {events:?}"
    );
    assert!(
        inner_log.borrow().iter().any(|e| e == "inner:enter"),
        "命中节点也应收到 enter"
    );

    // 移出窗口：两者都应收到 leave
    tree.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(900.0, 900.0),
    });
    let events = log.borrow().clone();
    assert!(
        events.iter().any(|e| e == "outer:leave"),
        "祖先应收到 leave，实际 {events:?}"
    );
}
