//! 布局失效的传播：深节点变脏必须让整棵树重算，不能被缓存跳过。
//!
//! 这两个测试固化本轮修的两个结论：
//! 1. `mark_dirty` 沿父链向上传播 → root 不再 fresh → `layout()` 不跳过；
//! 2. `dispatch_to` 返回 Relayout 时 `finish_dispatch` 必须标记脏区并强制重算。

use y901_ui::{
    Constraints, EventCtx, EventResult, Json, Pos, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiEvent, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::new(10.0, 10.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        10.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 容器：可变子节点 + 收到 `bump` 动作时返回 Relayout。
struct Depth {
    id: &'static str,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
}

impl Depth {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            children: Vec::new(),
            rect: Rect::ZERO,
        }
    }
    fn push(mut self, c: Box<dyn Widget>) -> Self {
        self.children.push(c);
        self
    }
}

impl Widget for Depth {
    fn id(&self) -> &str {
        self.id
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let mut h = 0.0;
        for child in self.children.iter_mut() {
            let s = cache.measure(child.as_mut(), m, c);
            h += s.h;
        }
        c.constrain(Size::new(c.max.w, h))
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        let mut y = r.y;
        for child in self.children.iter_mut() {
            let s = child.measure(m, Constraints::loose(r.size()), cache);
            child.arrange(Rect::new(r.x, y, r.w, s.h), m, cache);
            y += s.h;
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::Action { name, .. } if name == "bump" => EventResult::Relayout,
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true).action("bump")
    }
}

fn leaf(id: &'static str) -> Box<dyn Widget> {
    Box::new(Depth::new(id))
}

/// 三层：root → mid → leaf
fn deep_tree() -> UiTree {
    let mid = Depth::new("mid").push(leaf("leaf"));
    let root = Depth::new("root").push(Box::new(mid));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 200.0), &Mono);
    tree
}

/// 1. 缓存基线：没有变化时第二次 layout 必须被跳过。
#[test]
fn second_layout_is_skipped_when_nothing_changed() {
    let mut tree = deep_tree();
    tree.layout(Size::new(200.0, 200.0), &Mono);
    assert!(
        tree.layout_stats().skipped,
        "没有任何变化时第二次 layout 应被跳过（否则缓存失效）"
    );
}

/// 2. 深层节点被标脏 ⇒ root 不再 fresh ⇒ layout 不跳过。
///
/// `add_node` 会把「父节点」标脏；父节点在第二层，root 必须因此失效。
#[test]
fn dirty_deep_parent_propagates_to_root() {
    let mut tree = deep_tree();
    tree.layout(Size::new(200.0, 200.0), &Mono);
    assert!(tree.layout_stats().skipped, "前置：此刻应可跳过");

    // 往 mid（深度 1）挂一个新子节点
    assert!(tree.add_node("mid", leaf("extra")));

    tree.layout(Size::new(200.0, 200.0), &Mono);
    assert!(
        !tree.layout_stats().skipped,
        "深节点变脏后 root 必须失效，layout 不能被跳过"
    );
    assert!(
        tree.rect_of("extra").is_some(),
        "新节点必须被真正布局（有 rect）"
    );
}

/// 3. dispatch_to 返回 Relayout ⇒ 立刻产生非空脏区，并强制下一次 layout 重算。
#[test]
fn dispatch_to_relayout_marks_dirty_and_forces_layout() {
    let mut tree = deep_tree();
    tree.render(&mut Null);
    tree.layout(Size::new(200.0, 200.0), &Mono);
    assert!(tree.dirty().is_empty(), "渲染后脏区应为空");
    assert!(tree.layout_stats().skipped, "前置：此刻应可跳过");

    let result = tree.dispatch_to(
        "leaf",
        &UiEvent::Action {
            name: "bump".into(),
            args: Json::obj(),
        },
    );
    assert!(
        result.needs_relayout(),
        "组件返回 Relayout，派发结果必须保留该标志"
    );
    assert!(
        !tree.dirty().is_empty(),
        "Relayout 必须立刻标记脏区（finish_dispatch 的责任）"
    );

    tree.layout(Size::new(200.0, 200.0), &Mono);
    assert!(
        !tree.layout_stats().skipped,
        "Relayout 之后 layout 不能被跳过"
    );
}

/// 4. 同一入口的负样例：普通按键事件不应产生脏区。
#[test]
fn non_relayout_event_does_not_dirty() {
    let mut tree = deep_tree();
    tree.render(&mut Null);
    assert!(tree.dirty().is_empty());

    let result = tree.dispatch_to(
        "leaf",
        &UiEvent::Key {
            key: y901_ui::Key::Char('a'),
            modifiers: Default::default(),
        },
    );
    assert!(!result.needs_relayout());
    assert!(
        tree.dirty().is_empty(),
        "Ignored 事件不应产生脏区，实际 {:?}",
        tree.dirty()
    );
}

/// 什么都不画的后端。
struct Null;

impl TextMetrics for Null {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Mono.measure_text(t, s)
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        Mono.line_height(s)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Null {
    fn size(&self) -> Size {
        Size::new(200.0, 200.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}
