//! **标脏必须外扩墨迹溢出量**：控件 rect 只保证布局空间，不保证装得下墨迹。
//!
//! 背景（审查 P1-5）：下伸字（g/y/j）与圆字母 overshoot 会画到 rect 外 1~5px。
//! 那些像素不在脏区里，局部重绘（scissor）时不会被清掉 ——
//! `snapshot_harness::invalidate_rect_partial_equals_full` 当初就是靠**手工把测试脏区
//! 加宽 4px** 才变绿的，根因一直没修。
//!
//! 现在 `UiTree::layout()` 从 `metrics.ink_bounds()` 求出墨迹溢出量（`ink_depth - line_height`），
//! 所有按 rect 标脏的路径统一外扩。这条用例无 GPU：只检查脏区几何。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle,
    UiTree, Widget,
};

/// 假度量：行高 16，但墨迹深度 20（多出 4px 的下伸/overshoot）。
struct InkMetrics {
    scale: f32,
}

impl TextMetrics for InkMetrics {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn ink_bounds(&self, _s: &TextStyle) -> (f32, f32) {
        (0.0, 20.0)
    }
    fn scale(&self) -> f32 {
        self.scale
    }
}

/// 不覆盖 `ink_bounds` 的度量：默认实现 = `(0, line_height)` → 溢出 0。
struct PlainMetrics;

impl TextMetrics for PlainMetrics {
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

struct Box2 {
    rect: Rect,
    hits: Rc<Cell<u32>>,
}

impl Widget for Box2 {
    fn id(&self) -> &str {
        "probe"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(200.0, 100.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
        self.hits.set(self.hits.get() + 1);
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 空渲染上下文：`render()` 会把脏区清空，方便断言"我们标的那一块"。
struct Sink;

impl TextMetrics for Sink {
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

fn tree_with<M: TextMetrics + 'static>(metrics: M) -> UiTree {
    let mut t = UiTree::new(Box::new(Box2 {
        rect: Rect::ZERO,
        hits: Rc::new(Cell::new(0)),
    }));
    t.layout(Size::new(400.0, 300.0), &metrics);
    // 布局会留下整窗脏区；先渲染一次清空，再只标我们关心的那一块
    t.render(&mut Sink);
    assert!(t.dirty().is_empty(), "前置：脏区应已清空");
    t
}

#[test]
fn dirty_rect_is_expanded_by_ink_overflow() {
    let mut t = tree_with(InkMetrics { scale: 1.0 });
    t.invalidate_rect(Rect::new(100.0, 100.0, 40.0, 20.0));
    let rects = t.dirty().rects();
    assert_eq!(rects.len(), 1, "应只有一块脏区：{rects:?}");
    let r = rects[0];
    // 溢出 4px 大于最小边距 2px，四边各扩 4 → (96,96,48,28)
    assert_eq!(
        (r.x, r.y, r.w, r.h),
        (96.0, 96.0, 48.0, 28.0),
        "脏区应按墨迹溢出量外扩 4px"
    );
}

#[test]
fn minimum_margin_applies_even_without_font_overflow() {
    let mut t = tree_with(PlainMetrics);
    t.invalidate_rect(Rect::new(100.0, 100.0, 40.0, 20.0));
    let rects = t.dirty().rects();
    assert_eq!(rects.len(), 1);
    let r = rects[0];
    // 应用字体的名义度量常常"刚好等于行高"（SimSun 实测 gap=0），
    // 而真实墨迹仍会越过 1~2px（overshoot + 抗锯齿），所以框架兜一个 2px 最小边距。
    assert_eq!(
        (r.x, r.y, r.w, r.h),
        (98.0, 98.0, 44.0, 24.0),
        "即使度量没有溢出，也必须保留 2px 最小安全边距"
    );
}
