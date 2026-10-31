//! Overlay 层（浮层）：
//! 1. 挂在 Overlay 上的浮层**不会被父级裁切**（滚动容器裁切不影响它）；
//! 2. 浮层拥有最高事件优先级：命中浮层 → 派给它；点在外部 → 自动关闭。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Modifiers, Pos, Rect, RenderCtx, Role, Semantics,
    Size, TextStyle, TextMetrics, UiEvent, UiTree, Widget,
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

/// 记录每次 fill_rect 时的裁切栈顶部，用来证明"浮层没被父级裁切"。
#[derive(Clone, Default)]
struct Clips {
    /// (绘制矩形, 当时的裁切矩形)
    fills: Rc<RefCell<Vec<(Rect, Rect)>>>,
}

struct Rec {
    clips: Vec<Rect>,
    out: Clips,
}

impl TextMetrics for Rec {
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

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, r: Rect, _c: Color) {
        let clip = *self.clips.last().expect("必有基座裁切");
        self.out.fills.borrow_mut().push((r, clip));
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, r: Rect) {
        let c = match self.clips.last() {
            Some(outer) => r.intersect(*outer),
            None => r,
        };
        self.clips.push(c);
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
}

/// 可见区很小的滚动容器（模拟 Demo 的 FileTable）。
struct ClippedBox {
    rect: Rect,
    kid: Box<dyn Widget>,
}

impl Widget for ClippedBox {
    fn id(&self) -> &str {
        "clipped"
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        std::slice::from_ref(&self.kid)
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        None
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, c.max.h))
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        self.kid.arrange(r, m, cache);
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn clip(&self) -> Option<Rect> {
        // 裁切到 100x80：浮层要是被它裁就会变成 100x80 以内
        Some(Rect::new(self.rect.x, self.rect.y, 100.0, 80.0))
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::rgba(0.1, 0.1, 0.12, 1.0));
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn build_tree() -> UiTree {
    let root = ClippedBox {
        rect: Rect::ZERO,
        kid: Box::new(y901_ui::Flex::column("inner")),
    };
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree
}

#[test]
fn popup_is_not_clipped_by_parent() {
    let mut tree = build_tree();

    // 浮层摆在视口右下角：完全在父容器裁切区（100x80）之外
    tree.open_popup(
        Box::new(PaintProbe::new("tip", Rect::new(300.0, 200.0, 80.0, 40.0))),
        Rect::new(300.0, 190.0, 10.0, 10.0),
    );
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let rect = tree.popup_rect("tip").expect("浮层应有矩形");
    assert!(rect.w > 0.0);

    tree.invalidate_all();
    let out = Clips::default();
    let mut ctx = Rec {
        clips: vec![Rect::new(0.0, 0.0, 400.0, 300.0)],
        out: out.clone(),
    };
    tree.render(&mut ctx);

    let fills = out.fills.borrow();
    let (painted, clip) = fills
        .iter()
        .find(|(r, _)| r.x >= 300.0)
        .expect("浮层的矩形应被画出来");
    assert!(
        clip.w >= 400.0,
        "浮层应在视口裁切下绘制（裁切 {clip:?}），而不是父容器的 100x80"
    );
    let _ = painted;
}

#[test]
fn popup_flips_when_no_space_below() {
    let mut tree = build_tree();
    tree.open_popup(
        Box::new(PaintProbe::new("tip2", Rect::ZERO)),
        Rect::new(10.0, 280.0, 20.0, 16.0),
    );
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let r = tree.popup_rect("tip2").expect("浮层");
    assert!(r.bottom() <= 300.0, "不能超出视口底部：{r:?}");
}

#[test]
fn click_inside_popup_routes_to_it() {
    struct Counter(Rc<std::cell::Cell<u32>>);
    impl Widget for Counter {
        fn id(&self) -> &str {
            "pop"
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _c: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(120.0, 60.0))
        }
        fn arrange(
            &mut self,
            r: Rect,
            _m: &dyn TextMetrics,
            _c: &mut y901_ui::layout::LayoutCache,
        ) {
            self.0.set(self.0.get());
            let _ = r;
        }
        fn rect(&self) -> Rect {
            Rect::ZERO
        }
        fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
            if matches!(e, UiEvent::MouseDown { .. }) {
                self.0.set(self.0.get() + 1);
                return EventResult::Handled;
            }
            EventResult::Ignored
        }
        fn render(&mut self, _c: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    let hits = Rc::new(std::cell::Cell::new(0));
    let mut tree = build_tree();
    tree.open_popup(
        Box::new(Counter(hits.clone())),
        Rect::new(200.0, 100.0, 10.0, 10.0),
    );
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let r = tree.popup_rect("pop").expect("popup");

    let res = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: y901_ui::MouseButton::Left,
    });
    assert!(res.is_handled(), "浮层内部点击应被它接管");
    assert_eq!(hits.get(), 1);
    assert!(tree.has_popup(), "点在内部不能关闭浮层");
}

#[test]
fn click_outside_closes_popup() {
    let mut tree = build_tree();
    tree.open_popup(
        Box::new(PaintProbe::new("pop2", Rect::ZERO)),
        Rect::new(200.0, 100.0, 10.0, 10.0),
    );
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.has_popup());

    let res = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(5.0, 5.0),
        button: y901_ui::MouseButton::Left,
    });
    assert!(res.is_handled(), "点外部应被消耗");
    assert!(!tree.has_popup(), "点外部应关闭浮层");
}

#[test]
fn closing_popup_marks_its_area_dirty() {
    let mut tree = build_tree();
    tree.open_popup(
        Box::new(PaintProbe::new("p3", Rect::ZERO)),
        Rect::new(50.0, 50.0, 10.0, 10.0),
    );
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.render(&mut Sink);
    let rect = tree.popup_rect("p3").expect("popup");
    assert!(tree.close_popup("p3"));
    let b = tree.dirty().bounds().expect("关闭应标脏");
    assert!(
        b.intersect(rect).w > 0.0,
        "标脏应覆盖浮层原位置：浮层 {rect:?} / 脏区 {b:?}"
    );
}

/// 一个只会画自己矩形的探针控件。
struct PaintProbe {
    id: &'static str,
    rect: Rect,
}

impl PaintProbe {
    fn new(id: &'static str, rect: Rect) -> Self {
        Self { id, rect }
    }
}

impl Widget for PaintProbe {
    fn id(&self) -> &str {
        self.id
    }
    fn role(&self) -> Role {
        Role("popup")
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let s = if self.rect.is_empty() {
            Size::new(80.0, 40.0)
        } else {
            Size::new(self.rect.w, self.rect.h)
        };
        c.constrain(s)
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        if self.rect.is_empty() {
            self.rect = r;
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(
            Rect::new(ctx.size().w - 90.0, 200.0, 80.0, 40.0),
            Color::rgba(0.9, 0.8, 0.2, 1.0),
        );
    }
    fn semantics(&self) -> Semantics {
        let _ = Modifiers::default();
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
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}
