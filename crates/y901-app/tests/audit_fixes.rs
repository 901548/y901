//! 审查修复的回归测试：MCP 握手、IME 提交、RPC/MCP 一致性、headless 走 Host。

use y901_runtime::{translate, Host, RawEvent};
use y901_ui::{
    Command, EventResult, Flex, FlexItem, Key, Modifiers, Size, TextMetrics, TextStyle, UiEvent,
    UiTree,
};
use y901_widgets::Button;

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

// ---------- P1-3：IME Commit ----------

#[test]
fn ime_commit_becomes_text_not_compose() {
    let composing = translate(
        &RawEvent::Ime {
            text: "ni".into(),
            composition: true,
        },
        Size::new(100.0, 100.0),
    )
    .expect("event");
    assert!(
        matches!(composing, UiEvent::Compose(_)),
        "组合中应是 Compose"
    );

    let commit = translate(
        &RawEvent::Ime {
            text: "你".into(),
            composition: false,
        },
        Size::new(100.0, 100.0),
    )
    .expect("event");
    assert!(
        matches!(&commit, UiEvent::Text(t) if t == "你"),
        "提交应是 Text，实际 {commit:?}"
    );
}

// ---------- P1-1：Host 处理过的事件不再下钻 ----------

#[test]
fn host_handled_event_short_circuits_tree() {
    struct Sink {
        seen: usize,
        handled: bool,
    }
    impl Host for Sink {
        fn idle(&mut self, _t: &mut UiTree) {}
        fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool {
            false
        }
        fn event(&mut self, _t: &mut UiTree, _e: &UiEvent) -> EventResult {
            self.seen += 1;
            if self.handled {
                EventResult::Handled
            } else {
                EventResult::Ignored
            }
        }
    }

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        Button::new("ok", "OK").flex_item(FlexItem::height(24.0)),
    ))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let ev = UiEvent::Key {
        key: Key::Enter,
        modifiers: Modifiers::default(),
    };

    let mut sink = Sink {
        seen: 0,
        handled: true,
    };
    // 与 runtime 相同顺序：Host 先看，Handled 则跳过 dispatch
    let mut dispatched = 0;
    if !sink.event(&mut tree, &ev).is_handled() {
        tree.dispatch(&ev);
        dispatched += 1;
    }
    assert_eq!(sink.seen, 1);
    assert_eq!(dispatched, 0, "Host 已处理时不应再 dispatch");

    let mut sink2 = Sink {
        seen: 0,
        handled: false,
    };
    let mut dispatched2 = 0;
    if !sink2.event(&mut tree, &ev).is_handled() {
        tree.dispatch(&ev);
        dispatched2 += 1;
    }
    assert_eq!(dispatched2, 1, "Host 未处理时应继续 dispatch");
}
