//! 光标链路回归：组件必须真的声明光标，树必须按位置查询。

use y901_ui::{Cursor, Flex, FlexItem, Pos, Size, TextMetrics, TextStyle, UiTree};
use y901_widgets::{Button, SplitView, TextInput};

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

#[test]
fn text_input_declares_ibeam_cursor() {
    let input = TextInput::new("q")
        .with_text("hello")
        .flex_item(FlexItem::fill());

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(input))));
    tree.layout(Size::new(300.0, 60.0), &Mono);

    let r = tree.rect_of("q").expect("input rect");
    let center = Pos::new(r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(
        tree.cursor_at(center),
        Cursor::Text,
        "输入框上应该是 I 型光标"
    );
    // 输入框外应该是默认
    assert_eq!(
        tree.cursor_at(Pos::new(r.x + r.w + 50.0, 5.0)),
        Cursor::Default
    );
}

#[test]
fn button_declares_pointer_cursor() {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(Button::new("ok", "OK"))),
    ));
    tree.layout(Size::new(300.0, 60.0), &Mono);

    let r = tree.rect_of("ok").expect("button rect");
    assert_eq!(
        tree.cursor_at(Pos::new(r.x + 5.0, r.y + 5.0)),
        Cursor::Pointer,
        "按钮上应该是手型光标"
    );
}

#[test]
fn splitter_cursor_comes_from_position_not_state() {
    let split = SplitView::horizontal(
        "split",
        Box::new(Flex::column("left")),
        Box::new(Flex::column("right")),
    )
    .ratio(0.5);

    let mut tree = UiTree::new(Box::new(split));
    tree.layout(Size::new(400.0, 200.0), &Mono);

    // 分隔条在 x=200；未发生任何 MouseMove
    assert_eq!(
        tree.cursor_at(Pos::new(200.5, 100.0)),
        Cursor::ResizeHorizontal,
        "cursor_at 必须按位置判定，而不是只看 hover 状态"
    );
    assert_eq!(tree.cursor_at(Pos::new(50.0, 100.0)), Cursor::Default);
}

#[test]
fn cursor_respects_clip_of_scroll_container() {
    // 滚动容器外的位置不应返回容器内组件的光标
    let input = TextInput::new("in").flex_item(FlexItem::height(400.0));
    let sv = y901_widgets::ScrollView::new("sv", Box::new(input)).flex_item(FlexItem::fill());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(sv))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let r = tree.rect_of("in").expect("input");
    // 输入框下半部分在视口外（内容 400 > 视口 100）
    let outside = Pos::new(r.x + 10.0, 300.0);
    assert_eq!(
        tree.cursor_at(outside),
        Cursor::Default,
        "视口外不应返回 I 型光标"
    );
}
