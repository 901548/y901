//! hover 跟踪：MouseEnter / MouseLeave 的派发语义 + `is_hovered` 快照。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, MouseButton, Pos, Rect, RenderCtx, Semantics, Size,
    TextMetrics, UiEvent, UiTree, Widget,
};

/// 记录自己收到了哪些事件（按名字）。
struct Recorder {
    id: String,
    rect: Rect,
    log: Rc<RefCell<Vec<String>>>,
}

impl Recorder {
    fn new(id: &str, log: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            id: id.to_string(),
            rect: Rect::ZERO,
            log,
        }
    }
}

impl Widget for Recorder {
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
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        let name = match e {
            UiEvent::MouseEnter => "enter",
            UiEvent::MouseLeave => "leave",
            UiEvent::MouseMove { .. } => "move",
            UiEvent::MouseDown { .. } => "down",
            _ => return EventResult::Ignored,
        };
        self.log.borrow_mut().push(format!("{}:{name}", self.id));
        EventResult::Handled
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 简易容器：把两个 Recorder 横排（各 50 宽）。
struct Pair {
    rect: Rect,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for Pair {
    fn id(&self) -> &str {
        "pair"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(100.0, 50.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        for (i, ch) in self.children.iter_mut().enumerate() {
            ch.arrange(
                Rect::new(rect.x + i as f32 * 50.0, rect.y, 50.0, rect.h),
                m,
                cache,
            );
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

fn tree() -> (UiTree, Rc<RefCell<Vec<String>>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let pair = Pair {
        rect: Rect::ZERO,
        children: vec![
            Box::new(Recorder::new("a", log.clone())),
            Box::new(Recorder::new("b", log.clone())),
        ],
    };
    let mut t = UiTree::new(Box::new(pair));
    t.layout(Size::new(200.0, 100.0), &Dummy);
    (t, log)
}

/// 只用于 layout 的度量实现（不画字）。
struct Dummy;

impl TextMetrics for Dummy {
    fn measure_text(&self, text: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn mv(x: f32) -> UiEvent {
    UiEvent::MouseMove {
        pos: Pos::new(x, 20.0),
    }
}

#[test]
fn entering_a_node_fires_mouse_enter() {
    let (mut t, log) = tree();
    t.dispatch(&mv(10.0));
    // 注意：MouseMove 也会派给命中节点，所以日志里 enter 后跟着一条 move。
    // 这里只管 enter/leave 的顺序，不管 move 的数量。
    let enters: Vec<String> = log
        .borrow()
        .iter()
        .filter(|e| !e.ends_with(":move"))
        .cloned()
        .collect();
    assert_eq!(enters, vec!["a:enter"], "进入 A 应触发一次 enter");
}

#[test]
fn leaving_fires_mouse_leave_for_the_old_node() {
    let (mut t, log) = tree();
    t.dispatch(&mv(10.0)); // 进入 a
    t.dispatch(&mv(60.0)); // 移到 b
    let events: Vec<String> = log
        .borrow()
        .iter()
        .filter(|e| !e.ends_with(":move"))
        .cloned()
        .collect();
    assert_eq!(
        events,
        vec!["a:enter", "a:leave", "b:enter"],
        "A → B 应依次是 A 的 leave 与 B 的 enter"
    );
}

#[test]
fn moving_inside_the_same_node_only_sends_moves() {
    let (mut t, log) = tree();
    t.dispatch(&mv(10.0));
    log.borrow_mut().clear();
    t.dispatch(&mv(20.0));
    t.dispatch(&mv(30.0));
    assert!(
        log.borrow().iter().all(|e| e.ends_with(":move")),
        "同一节点内移动不应有 enter/leave，实际 {:?}",
        log.borrow()
    );
}

#[test]
fn leaving_the_window_clears_hover() {
    let (mut t, log) = tree();
    t.dispatch(&mv(10.0));
    assert_eq!(t.hovered(), Some("a"));
    // 移到窗口外（没有节点命中）
    t.dispatch(&mv(500.0));
    assert_eq!(t.hovered(), None, "移出所有节点后 hovered 应为 None");
    assert!(
        log.borrow().iter().any(|e| e == "a:leave"),
        "移出时应给 A 发 leave"
    );
}

#[test]
fn removing_the_hovered_node_clears_it_without_panicking() {
    let (mut t, log) = tree();
    t.dispatch(&mv(10.0));
    assert_eq!(t.hovered(), Some("a"));
    assert!(t.remove_node("a"), "应能移除 A");
    assert_eq!(t.hovered(), None, "被移除的节点不能继续是 hovered");
    // 再移动不应 panic
    t.dispatch(&mv(20.0));
    let _ = log;
}

#[test]
fn hover_snapshot_reaches_render_ctx() {
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;

    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    let mut soft = SoftBackend::new_headless(64, 64, Box::new(engine), cell).expect("soft");

    let (mut t, _log) = tree();
    t.dispatch(&mv(10.0));
    t.invalidate_all();
    t.render(&mut soft);
    // 渲染后，后端应记得 hovered 是 "a"
    use y901_ui::RenderCtx as _;
    assert!(soft.is_hovered("a"), "渲染注入后 a 应处于 hovered");
    assert!(!soft.is_hovered("b"));
    let _ = MouseButton::Left; // 保持导入被使用
}
