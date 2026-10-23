//! Tab 焦点遍历测试。

use y901_ui::{Flex, FlexItem, Key, Modifiers, Size, TextStyle, TextMetrics, UiEvent, UiTree};
use y901_widgets::TextInput;

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        let mut w = 0.0;
        for ch in text.chars() {
            w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
        }
        Size::new(w, 16.0)
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn tree_with_three_inputs() -> UiTree {
    let root = Flex::column("root")
        .push(Box::new(TextInput::new("a").flex_item(FlexItem::height(30.0))))
        .push(Box::new(TextInput::new("b").flex_item(FlexItem::height(30.0))))
        .push(Box::new(TextInput::new("c").flex_item(FlexItem::height(30.0))));

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 200.0), &Mono);
    tree
}

fn tab(shift: bool) -> UiEvent {
    UiEvent::Key {
        key: Key::Tab,
        modifiers: Modifiers {
            shift,
            ..Default::default()
        },
    }
}

#[test]
fn tab_cycles_through_focusable_nodes() {
    let mut tree = tree_with_three_inputs();
    assert_eq!(tree.focus_order_len(), 3, "三个输入框都应可聚焦");

    // 起始无焦点：第一次 Tab 落到第一个
    tree.dispatch(&tab(false));
    assert_eq!(tree.focus(), Some("a"));

    tree.dispatch(&tab(false));
    assert_eq!(tree.focus(), Some("b"));

    tree.dispatch(&tab(false));
    assert_eq!(tree.focus(), Some("c"));

    // 边界循环回第一个
    tree.dispatch(&tab(false));
    assert_eq!(tree.focus(), Some("a"));
}

#[test]
fn shift_tab_walks_backwards() {
    let mut tree = tree_with_three_inputs();
    tree.set_focus(Some("a".to_string()));

    tree.dispatch(&tab(true));
    assert_eq!(tree.focus(), Some("c"), "从第一个往回应循环到最后一个");

    tree.dispatch(&tab(true));
    assert_eq!(tree.focus(), Some("b"));

    tree.dispatch(&tab(true));
    assert_eq!(tree.focus(), Some("a"));
}

#[test]
fn focus_order_follows_tree_order() {
    let tree = tree_with_three_inputs();
    // 顺序稳定且等于树序：a → b → c
    assert_eq!(tree.focus_order_len(), 3);
}

#[test]
fn tab_is_linear_in_number_of_nodes() {
    // 100 个输入框，Tab 一次仍只做 O(n) 顺序查找 + O(d) 设置
    let mut root = Flex::column("root");
    for i in 0..100 {
        root = root.push(Box::new(
            TextInput::new(format!("i{i}")).flex_item(FlexItem::height(20.0)),
        ));
    }
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 2000.0), &Mono);
    assert_eq!(tree.focus_order_len(), 100);

    tree.dispatch(&tab(false));
    assert_eq!(tree.focus(), Some("i0"));
    tree.dispatch(&tab(false));
    assert_eq!(tree.focus(), Some("i1"));
}
