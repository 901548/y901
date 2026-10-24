//! 关闭的浮层/节点不得绘制子树。
//!
//! 覆盖两条渲染路径：
//! - Modal/Overlay 走 `render_subtree`（延后回放）
//! - Base 走 `render_pass`（就地绘制）
//!
//! 把任意一个 `is_open()` 守卫删掉，对应测试立刻变红。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, Pos, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiTree, Widget,
};
use y901_widgets::Dialog;

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 只记录"被渲染了几次"的探针；`open` 决定它是否可见。
struct Counted {
    id: &'static str,
    rect: Rect,
    paints: Rc<Cell<usize>>,
    open: bool,
}

impl Widget for Counted {
    fn id(&self) -> &str {
        self.id
    }
    fn is_open(&self) -> bool {
        self.open
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 24.0))
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
        self.paints.set(self.paints.get() + 1);
        ctx.fill_rect(self.rect, Color::WHITE);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

/// 不做绘制、只统计裁剪栈平衡的后端。
struct Null {
    depth: i32,
}

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
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {
        self.depth += 1;
    }
    fn pop_clip(&mut self) {
        self.depth -= 1;
    }
}

fn null() -> Null {
    Null { depth: 0 }
}

/// 1. Modal 层（延后回放）：关闭时子节点一次都不画。
#[test]
fn closed_dialog_does_not_paint_its_body() {
    let paints = Rc::new(Cell::new(0));
    let body = Counted {
        id: "body",
        rect: Rect::ZERO,
        paints: paints.clone(),
        open: true,
    };

    let dlg = Dialog::new("dlg", "T").push(Box::new(body)); // 未 open()
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(dlg))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let mut ctx = null();
    tree.render(&mut ctx);

    assert_eq!(
        paints.get(),
        0,
        "关闭的 Dialog 不应绘制任何子节点（render_subtree 的 is_open 守卫）"
    );
    assert_eq!(ctx.depth, 0, "push/pop 必须配平");
}

/// 2. 打开后必须绘制（证明守卫没把正常路径也拦掉）。
#[test]
fn open_dialog_paints_its_body() {
    let paints = Rc::new(Cell::new(0));
    let body = Counted {
        id: "body",
        rect: Rect::ZERO,
        paints: paints.clone(),
        open: true,
    };

    let mut dlg = Dialog::new("dlg", "T").push(Box::new(body));
    dlg.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(dlg))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let mut ctx = null();
    tree.render(&mut ctx);

    assert!(
        paints.get() >= 1,
        "打开的 Dialog 必须绘制子节点，实际 {}",
        paints.get()
    );
    assert_eq!(ctx.depth, 0);
}

/// 3. Base 层（就地绘制）：is_open()==false 的节点也不画、不进索引。
#[test]
fn closed_base_widget_is_not_painted() {
    let paints = Rc::new(Cell::new(0));
    let node = Counted {
        id: "panel",
        rect: Rect::ZERO,
        paints: paints.clone(),
        open: false,
    };

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(node))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let mut ctx = null();
    tree.render(&mut ctx);

    assert_eq!(
        paints.get(),
        0,
        "关闭的 Base 节点不应绘制（render_pass 的 is_open 守卫）"
    );
    assert!(tree.rect_of("panel").is_none(), "关闭节点不应进索引");
}

/// 4. 关闭必须立刻停画并退出索引（is_open 变化 ⇒ Relayout ⇒ 重建）。
#[test]
fn closing_dialog_stops_painting_and_leaves_index() {
    let paints = Rc::new(Cell::new(0));
    let body = Counted {
        id: "body",
        rect: Rect::ZERO,
        paints: paints.clone(),
        open: true,
    };

    let mut dlg = Dialog::new("dlg", "T").push(Box::new(body));
    dlg.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(dlg))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let mut ctx = null();
    tree.render(&mut ctx);
    let painted_while_open = paints.get();
    assert!(painted_while_open >= 1, "打开时应绘制");
    assert!(tree.rect_of("dlg").is_some(), "打开时应进索引");

    let result = tree.dispatch_to(
        "dlg",
        &y901_ui::UiEvent::Action {
            name: "close".into(),
            args: y901_ui::Json::obj(),
        },
    );
    assert!(
        result.needs_relayout(),
        "关闭必须返回 Relayout，否则索引不会重建（实际 {result:?}）"
    );

    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.rect_of("dlg").is_none(), "关闭后必须退出索引");

    tree.render(&mut ctx);
    assert_eq!(
        paints.get(),
        painted_while_open,
        "关闭后不应再绘制子节点"
    );
}
