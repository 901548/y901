//! 广播事件（`Tick`）的重绘粒度：只标"真的要求重绘"的节点矩形，不整窗重绘。

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiEvent, UiTree, Widget,
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

/// 可以被"点亮"的探针：`on` 为真时对 Tick 返回 Redraw。
struct Blinker {
    id: &'static str,
    on: bool,
    rect: Rect,
}

impl Widget for Blinker {
    fn id(&self) -> &str {
        self.id
    }

    /// 订阅 Tick：`Tick` 广播现在只发给订阅者（`wants_tick`）。
    fn wants_tick(&self) -> bool {
        true
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::Tick if self.on => EventResult::Redraw,
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn tree_with(on: bool) -> UiTree {
    let root = Flex::column("root")
        .push(Box::new(Blinker {
            id: "blink",
            on,
            rect: Rect::ZERO,
        }))
        .push(Box::new(
            Flex::column("other").flex_item(FlexItem::height(100.0)),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.invalidate_all();
    tree
}

#[test]
fn tick_redraw_marks_only_responding_node() {
    let mut tree = tree_with(true);
    // 清空脏区（渲染一次）
    struct Null;
    impl TextMetrics for Null {
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
    impl RenderCtx for Null {
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
    tree.render(&mut Null);
    assert!(tree.dirty().is_empty(), "前置：脏区已清空");

    let r = tree.dispatch(&UiEvent::Tick);
    assert!(r.needs_redraw(), "Blinker 应返回 Redraw");

    assert!(!tree.dirty().is_empty(), "必须标脏");
    assert!(
        !tree.dirty().is_full(),
        "广播事件的重绘不能是整窗（之前 Tick 会 invalidate_all）"
    );
    assert_eq!(tree.dirty().len(), 1, "只标记响应节点那一块");
}

#[test]
fn tick_without_responders_leaves_dirty_empty() {
    let mut tree = tree_with(false);
    // 清空脏区
    tree.invalidate_all();
    let mut sink = Sink;
    tree.render(&mut sink);
    assert!(tree.dirty().is_empty());

    let r = tree.dispatch(&UiEvent::Tick);
    assert!(!r.needs_redraw());
    assert!(tree.dirty().is_empty(), "没人要求重绘就不该有脏区");
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
