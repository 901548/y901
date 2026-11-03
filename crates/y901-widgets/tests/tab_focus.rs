//! P0-4：Tab 的处理归属。
//!
//! 规则：焦点组件声明 `wants_tab()` 时，Tab 交给它（终端补全）；
//! 否则由框架用来切换焦点。

use y901_ui::{
    EventResult, Flex, FlexItem, Key, Modifiers, Size, TextMetrics, TextStyle, UiEvent, UiTree,
};
use y901_widgets::Button;

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


/// 声明"我要自己处理 Tab"的通用探针。
///
/// 之前用 TerminalWidget 充当这个角色；终端搬到 `y901-extras` 之后，
/// 这里验证的是**框架机制**：谁声明 `wants_tab()`，Tab 就归谁。
struct TabConsumer {
    id: &'static str,
    rect: y901_ui::Rect,
    hits: std::rc::Rc<std::cell::Cell<u32>>,
}

impl y901_ui::Widget for TabConsumer {
    fn id(&self) -> &str {
        self.id
    }
    fn wants_tab(&self) -> bool {
        true
    }
    fn focusable(&self) -> bool {
        true
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: y901_ui::Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(c.max)
    }
    fn arrange(
        &mut self,
        r: y901_ui::Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> y901_ui::Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut y901_ui::EventCtx, e: &UiEvent) -> EventResult {
        if matches!(e, UiEvent::Key { key: Key::Tab, .. }) {
            self.hits.set(self.hits.get() + 1);
            return EventResult::Handled;
        }
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

fn tree() -> UiTree {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(
                {
                    let _ = FlexItem::height(200.0);
                    TabConsumer {
                        id: "term",
                        rect: y901_ui::Rect::ZERO,
                        hits: std::rc::Rc::new(std::cell::Cell::new(0)),
                    }
                },
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
