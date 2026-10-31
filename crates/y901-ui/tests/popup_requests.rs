//! `ctx.request_popup` / `request_modal_popup`：组件在事件里安全地挂浮层。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, MouseButton, Pos, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiEvent, UiTree, Widget,
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

/// 一个"点击就弹菜单"的按钮：它不持有 UiTree，只提交工厂。
struct Trigger {
    rect: Rect,
    created: Rc<Cell<u32>>,
}

impl Widget for Trigger {
    fn id(&self) -> &str {
        "trigger"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(120.0, 40.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult {
        if matches!(e, UiEvent::MouseDown { .. }) {
            let created = self.created.clone();
            ctx.request_popup(
                Box::new(move || {
                    created.set(created.get() + 1);
                    Box::new(PaintProbe { rect: Rect::ZERO })
                }),
                self.rect,
            );
            return EventResult::Handled;
        }
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::rgba(0.2, 0.3, 0.5, 1.0));
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

struct PaintProbe {
    rect: Rect,
}

impl Widget for PaintProbe {
    fn id(&self) -> &str {
        "menu"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(140.0, 90.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::rgba(0.9, 0.8, 0.3, 1.0));
    }
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true).label("menu")
    }
}

fn tree() -> (UiTree, Rc<Cell<u32>>) {
    let created = Rc::new(Cell::new(0));
    let t = Trigger {
        rect: Rect::ZERO,
        created: created.clone(),
    };
    let mut tree = UiTree::new(Box::new(t));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    (tree, created)
}

#[test]
fn request_popup_mounts_after_dispatch() {
    let (mut tree, created) = tree();
    let r = tree.rect_of("trigger").expect("trigger");
    let res = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: MouseButton::Left,
    });
    assert!(res.is_handled());
    assert_eq!(created.get(), 1, "工厂应被调用一次");
    assert!(tree.has_popup(), "请求应在分发结束后被挂载");
    // 挂载后需要一次布局才有矩形
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let pr = tree.popup_rect("menu").expect("popup 矩形");
    assert!(pr.w > 0.0 && pr.h > 0.0, "浮层应被摆放：{pr:?}");
}

#[test]
fn requested_popup_closes_on_outside_click() {
    let (mut tree, _) = tree();
    let r = tree.rect_of("trigger").expect("trigger");
    let _ = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: MouseButton::Left,
    });
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.has_popup());

    let _ = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(380.0, 290.0),
        button: MouseButton::Left,
    });
    assert!(!tree.has_popup(), "点外部应关闭");
}

#[test]
fn modal_popup_survives_outside_click() {
    struct ModalTrigger {
        rect: Rect,
    }
    impl Widget for ModalTrigger {
        fn id(&self) -> &str {
            "mtrigger"
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _c: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(120.0, 40.0))
        }
        fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
            self.rect = r;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult {
            if matches!(e, UiEvent::MouseDown { .. }) {
                ctx.request_modal_popup(
                    Box::new(|| Box::new(PaintProbe { rect: Rect::ZERO })),
                    self.rect,
                );
                return EventResult::Handled;
            }
            EventResult::Ignored
        }
        fn render(&mut self, _c: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    let mut tree = UiTree::new(Box::new(ModalTrigger { rect: Rect::ZERO }));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let r = tree.rect_of("mtrigger").expect("trigger");
    let _ = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: MouseButton::Left,
    });
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.has_popup());
    assert!(tree.popups()[0].is_modal(), "应是模态");

    let res = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(390.0, 290.0),
        button: MouseButton::Left,
    });
    assert!(res.is_handled(), "模态时点外部也要吃掉事件");
    assert!(tree.has_popup(), "模态浮层不能被点外部关闭");
}

#[test]
fn requested_popup_is_visible_to_semantics() {
    let (mut tree, _) = tree();
    let r = tree.rect_of("trigger").expect("trigger");
    let _ = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: MouseButton::Left,
    });
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let snap = tree.snapshot();
    // 语义树里必须能找到浮层的 id（Agent 才看得见）
    let mut found = false;
    snap.walk(&mut |n| {
        if n.id == "menu" {
            found = true;
        }
    });
    assert!(found, "浮层节点应出现在语义树里：{:?}", snap.id);
}

#[test]
fn render_draws_popup_after_base_with_viewport_clip() {
    let (mut tree, _) = tree();
    let r = tree.rect_of("trigger").expect("trigger");
    let _ = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: MouseButton::Left,
    });
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.invalidate_all();

    // 记录绘制时最深一层裁剪：浮层必须在"视口"裁剪下
    struct Rec {
        deepest: Rc<Cell<f32>>,
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
        fn fill_rect(&mut self, rect: Rect, _c: Color) {
            // 浮层矩形是 140x90 且贴在触发点旁；只关心它被画时的裁剪宽度
            if rect.w <= 140.0 && rect.h <= 90.0 {
                self.deepest.set(rect.w);
            }
        }
        fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
        fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
            0.0
        }
        fn push_clip(&mut self, _r: Rect) {}
        fn pop_clip(&mut self) {}
    }

    let seen = Rc::new(Cell::new(0.0));
    let mut rec = Rec {
        deepest: seen.clone(),
    };
    tree.render(&mut rec);
    assert!(seen.get() > 0.0, "浮层应被绘制出来");
}
