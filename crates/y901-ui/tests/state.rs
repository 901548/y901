//! `State<T>`：改值即标脏（不依赖 `EventCtx`）。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::state::State;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, Pos, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 一个把数值画成色块的探针；它**自己不会**请求重绘。
struct CellBox {
    id: &'static str,
    value: Rc<Cell<u32>>,
    rect: Rect,
}

impl Widget for CellBox {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(100.0, 40.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let t = self.value.get() as f32 / 10.0;
        ctx.fill_rect(self.rect, Color::rgba(t, 0.2, 0.4, 1.0));
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn tree() -> (UiTree, Rc<Cell<u32>>) {
    let value = Rc::new(Cell::new(0));
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(CellBox {
        id: "counter",
        value: value.clone(),
        rect: Rect::ZERO,
    }))));
    tree.layout(Size::new(300.0, 200.0), &Mono);
    (tree, value)
}

#[test]
fn state_change_marks_owner_dirty() {
    let (mut tree, value) = tree();
    tree.invalidate_all();
    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty(), "前提：刚画完应无脏区");

    let state: State<u32> = tree.use_state("counter", 0);
    // 业务侧改数据：没有 ctx、没有手动 invalidate
    value.set(7);
    assert!(state.set(7), "值变了，应返回 true");
    let n = tree.drain_invalidation();
    assert!(n >= 1, "应至少标脏一个节点，实际 {n}");

    eprintln!(
        "[diag] n={n} full={} rects={} rect_of={:?}",
        tree.dirty().is_full(),
        tree.dirty().rects().len(),
        tree.rect_of("counter")
    );
    let b = tree.dirty().bounds().expect("应有脏区");
    let r = tree.rect_of("counter").expect("counter");
    assert!(
        b.intersect(r).w > 0.0,
        "脏区应覆盖 counter：脏区 {b:?} / 控件 {r:?}"
    );
    assert!(!tree.dirty().is_full(), "只标该控件，不应退化成整窗");
}

#[test]
fn state_same_value_does_not_dirty() {
    let (mut tree, _value) = tree();
    let state: State<u32> = tree.use_state("counter", 5);
    tree.invalidate_all();
    tree.render(&mut Sink);

    assert!(!state.set(5), "值相同不应通知");
    assert_eq!(tree.drain_invalidation(), 0);
    assert!(tree.dirty().is_empty(), "值没变就不该有脏区");
}

#[test]
fn state_update_in_place() {
    let (mut tree, _v) = tree();
    let state: State<u32> = tree.use_state("counter", 1);
    state.update(|v| *v += 41);
    assert_eq!(state.get(), 42);
    assert_eq!(state.owner(), "counter");
    assert!(tree.drain_invalidation() >= 1, "就地修改也要标脏");
}

#[test]
fn state_can_also_invalidate_other_node() {
    let (mut tree, _v) = tree();
    let a: State<u32> = tree.use_state("counter", 0);
    // 变更时也把 root 标脏（例：容器需要重排）
    a.also_invalidate("root", tree.invalidation_queue_clone());
    a.set(3);
    let n = tree.drain_invalidation();
    assert!(n >= 2, "两个订阅者都应标脏，实际 {n}");
}

struct Sink;

impl TextMetrics for Sink {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::ZERO
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Sink {
    fn size(&self) -> Size {
        Size::new(300.0, 200.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}
