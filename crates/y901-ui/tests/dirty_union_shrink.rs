//! 控件收缩时的脏区必须覆盖"旧矩形 ∪ 新矩形"。
//!
//! 背景：局部重绘（scissor）只重画脏区。如果控件从 100×100 缩到 50×50 时
//! 只标记新矩形，屏幕上 50..100 那圈旧内容永远不会被覆盖 → 残影。
//!
//! 这个测试不依赖 GPU：它检查 `UiTree::layout` 在发现 rect 变化时，
//! 是否把**并集**加进了 `DirtyRegion`。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiTree, Widget,
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

/// 尺寸由外部 `Rc<Cell>` 控制；`gen` 变化时让布局缓存失效。
struct Shrinker {
    w: Rc<Cell<f32>>,
    h: Rc<Cell<f32>>,
    gen: Rc<Cell<u64>>,
    rect: Rect,
}

impl Widget for Shrinker {
    fn id(&self) -> &str {
        "probe"
    }
    fn layout_version(&self) -> u64 {
        self.gen.get()
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(self.w.get(), self.h.get()))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, y901_ui::Color::WHITE);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
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
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
fn shrink_marks_union_of_old_and_new_rect() {
    let w = Rc::new(Cell::new(100.0));
    let h = Rc::new(Cell::new(100.0));
    let gen = Rc::new(Cell::new(1u64));

    let root = Flex::column("root").push(Box::new(Shrinker {
        w: w.clone(),
        h: h.clone(),
        gen: gen.clone(),
        rect: Rect::ZERO,
    }));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 前置：初始矩形是 100×100（Flex 在交叉轴上会撑满宽度，只校验高度）
    let r0 = tree.rect_of("probe").expect("probe 必须存在");
    assert_eq!(r0.h, 100.0, "初始高度应为 100");

    // 清空脏区
    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty(), "前置：脏区已清空");

    // 缩小到 50×50
    w.set(50.0);
    h.set(50.0);
    gen.set(2);
    tree.mark_layout_dirty("probe");
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let r1 = tree.rect_of("probe").expect("probe 必须存在");
    assert_eq!(r1.h, 50.0, "缩小后高度应为 50，实际 {r1:?}");

    // 关键断言：脏区必须覆盖**旧**矩形（100×100），否则 50..100 留下残影
    assert!(!tree.dirty().is_empty(), "布局后必须产生脏区");
    let bounds = tree.dirty().bounds().expect("应有脏区包围盒");
    assert!(
        bounds.bottom() >= 100.0,
        "脏区必须覆盖旧矩形的高度（100），否则下半截留残影；实际 {bounds:?}"
    );
    // 且必须覆盖新矩形
    assert!(
        bounds.bottom() >= 50.0,
        "脏区必须覆盖新矩形，实际 {bounds:?}"
    );
}

#[test]
fn unchanged_layout_marks_nothing() {
    let w = Rc::new(Cell::new(80.0));
    let h = Rc::new(Cell::new(40.0));
    let gen = Rc::new(Cell::new(1u64));

    let root = Flex::column("root").push(Box::new(Shrinker {
        w: w.clone(),
        h: h.clone(),
        gen: gen.clone(),
        rect: Rect::ZERO,
    }));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty());

    // 强制重排但尺寸没变：不应凭空产生脏区
    tree.mark_layout_dirty("probe");
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(
        tree.dirty().is_empty(),
        "矩形没变就不该标脏，实际 {:?}",
        tree.dirty().bounds()
    );
}

#[test]
fn grow_also_marks_union() {
    let w = Rc::new(Cell::new(50.0));
    let h = Rc::new(Cell::new(50.0));
    let gen = Rc::new(Cell::new(1u64));

    let root = Flex::column("root").push(Box::new(Shrinker {
        w: w.clone(),
        h: h.clone(),
        gen: gen.clone(),
        rect: Rect::ZERO,
    }));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.render(&mut Sink);

    w.set(120.0);
    h.set(90.0);
    gen.set(2);
    tree.mark_layout_dirty("probe");
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let bounds = tree.dirty().bounds().expect("应有脏区");
    assert!(bounds.right() >= 120.0 && bounds.bottom() >= 90.0, "变大也要覆盖新区域");
}
