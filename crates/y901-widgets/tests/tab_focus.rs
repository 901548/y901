//! P0-4：Tab 的处理归属。
//!
//! 规则：焦点组件声明 `wants_tab()` 时，Tab 交给它（终端补全）；
//! 否则由框架用来切换焦点。

use y901_ui::{
    EventResult, Flex, FlexItem, Key, Modifiers, Size, TextMetrics, TextStyle, UiEvent, UiTree,
};
use y901_widgets::{Button, TerminalWidget};

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

fn tree() -> UiTree {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(
                TerminalWidget::new("term", 80, 24).flex_item(FlexItem::height(200.0)),
            ))
            .push(Box::new(
                Button::new("btn", "OK").flex_item(FlexItem::height(40.0)),
            )),
    ));
    tree.layout(Size::new(400.0, 400.0), &Mono);
    tree
}

fn tab() -> UiEvent {
    UiEvent::Key {
        key: Key::Tab,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn terminal_consumes_tab_instead_of_focus_switch() {
    let mut tree = tree();
    let _ = tree.set_focus(Some("term".to_string()));
    assert_eq!(tree.focus(), Some("term"), "前置：终端已聚焦");

    let result = tree.dispatch(&tab());

    assert_eq!(
        tree.focus(),
        Some("term"),
        "终端聚焦时 Tab 不应切换焦点（终端要用它做补全）"
    );
    assert!(
        result != EventResult::Ignored,
        "终端应当消费 Tab，实际 {result:?}"
    );
}

#[test]
fn tab_still_switches_focus_outside_terminal() {
    let mut tree = tree();
    let _ = tree.set_focus(Some("btn".to_string()));
    assert_eq!(tree.focus(), Some("btn"), "前置：按钮已聚焦");

    let _ = tree.dispatch(&tab());

    assert_ne!(
        tree.focus(),
        Some("btn"),
        "非终端节点聚焦时，Tab 必须照常切换焦点"
    );
}
