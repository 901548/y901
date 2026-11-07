//! 收尾：RPC 每请求鉴权 + ScrollView 视口外不可命中（走真实 dispatch）。

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Pos, Rect, RenderCtx, Size, TextMetrics,
    TextStyle, UiEvent, UiTree, Widget,
};

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

// ---------- 2. ScrollView 视口外不可命中 ----------

struct Tracked {
    id: String,
    rect: Rect,
    clicks: std::rc::Rc<std::cell::RefCell<usize>>,
}

impl Widget for Tracked {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 40.0))
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
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::MouseDown { .. } => {
                *self.clicks.borrow_mut() += 1;
                EventResult::Handled
            }
            // 其余事件放行，让滚轮冒泡到 ScrollView
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default().exposed(true)
    }
}

fn spacer(id: &str) -> Box<dyn Widget> {
    Box::new(Tracked {
        id: id.to_string(),
        rect: Rect::ZERO,
        clicks: std::rc::Rc::new(std::cell::RefCell::new(0)),
    })
}

#[test]
fn scrollview_child_outside_viewport_is_not_clickable_until_scrolled() {
    let clicks = std::rc::Rc::new(std::cell::RefCell::new(0usize));

    // 内容 400px：5 个 40px 行 + 目标按钮在最后
    let mut column = Flex::column("content");
    for i in 0..4 {
        column = column.push(spacer(&format!("pad{i}")));
    }
    column = column.push(Box::new(Tracked {
        id: "target".into(),
        rect: Rect::ZERO,
        clicks: clicks.clone(),
    }));

    let sv = y901_widgets::ScrollView::new("sv", Box::new(column)).flex_item(FlexItem::fill());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(sv))));
    // 视口 200 高，内容 200（4×40 + 40）
    tree.layout(Size::new(300.0, 200.0), &Mono);

    // 目标在 y=160..200，仍在视口内 —— 先把它推出视口
    // 再加高内容：直接把视口压到 120
    tree.layout(Size::new(300.0, 120.0), &Mono);

    let screen = Pos::new(50.0, 180.0); // 目标顶部 160 之后的位置，已在视口(0..120)外

    // 真实 dispatch：视口外的点击不应到达 target
    tree.dispatch(&UiEvent::MouseDown {
        pos: screen,
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(*clicks.borrow(), 0, "视口外的子节点不应收到点击");

    // 滚动使 target 进入视口
    tree.dispatch(&UiEvent::Wheel {
        pos: Pos::new(50.0, 60.0),
        delta: -480.0,
        mods: y901_ui::Modifiers::default(),
    });
    tree.layout(Size::new(300.0, 120.0), &Mono);

    let target = tree.rect_of("target").expect("target");
    let center = Pos::new(target.x + 20.0, target.y + 20.0);
    assert!(
        target.y >= 0.0 && target.y < 120.0,
        "滚动后目标应进入视口：{target:?}"
    );

    tree.dispatch(&UiEvent::MouseDown {
        pos: center,
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(*clicks.borrow(), 1, "进入视口后应能点中");
}
