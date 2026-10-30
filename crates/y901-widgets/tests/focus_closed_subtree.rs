//! 关闭的子树不参与焦点遍历（与"不画、不命中、不可聚焦"三条不变量对齐）。
//!
//! 放在 widgets 测试里是因为要用真实的 `Dialog` / `TextInput`；
//! `y901-ui` 不能反向依赖 `y901-widgets`。

use y901_ui::{Flex, FlexItem, Key, Modifiers, Size, TextMetrics, TextStyle, UiEvent, UiTree};
use y901_widgets::{Button, Dialog, TextInput};

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

/// ```text
/// root (Flex)
/// ├── dlg (Dialog, Modal)
/// │   ├── i1 (TextInput)
/// │   └── i2 (TextInput)
/// └── btn (Button)
/// ```
fn tree_with_dialog(open: bool) -> UiTree {
    let mut dlg = Dialog::new("dlg", "Rename");
    dlg.push_child(Box::new(
        TextInput::new("i1").flex_item(FlexItem::height(24.0)),
    ));
    dlg.push_child(Box::new(
        TextInput::new("i2").flex_item(FlexItem::height(24.0)),
    ));
    dlg.set_open(open);

    // Dialog 自己的 `flex_item` 是构建器（消费 self），这里先设好再装箱
    let root = Flex::column("root").push(Box::new(dlg)).push(Box::new(
        Button::new("btn", "OK").flex_item(FlexItem::height(32.0)),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree
}

/// 把焦点顺序走一遍，收集经过的节点 id。
fn focus_cycle(tree: &mut UiTree) -> Vec<String> {
    let n = tree.focus_order_len();
    let mut out = Vec::new();
    let _ = tree.set_focus(None);
    for _ in 0..n {
        let _ = tree.focus_next(1);
        if let Some(id) = tree.focus() {
            out.push(id.to_string());
        }
    }
    out
}

#[test]
fn closed_dialog_children_are_not_focusable() {
    let mut tree = tree_with_dialog(false);

    assert_eq!(
        tree.focus_order_len(),
        1,
        "关闭的 Dialog 里的两个 TextInput 不应进入焦点顺序（只留外层 Button）"
    );
    let cycle = focus_cycle(&mut tree);
    assert_eq!(cycle, vec!["btn".to_string()], "Tab 只能到达 btn");
}

#[test]
fn open_dialog_children_are_focusable() {
    let mut tree = tree_with_dialog(true);

    assert_eq!(
        tree.focus_order_len(),
        2,
        "打开 Dialog 后焦点被限制在 Modal 子树内（两个 TextInput）"
    );
    let mut cycle = focus_cycle(&mut tree);
    cycle.sort();
    assert_eq!(
        cycle,
        vec!["i1".to_string(), "i2".to_string()],
        "Tab 应只在对话框内的两个输入框之间循环"
    );
}

#[test]
fn open_close_dialog_updates_focus_order() {
    let mut tree = tree_with_dialog(true);
    assert_eq!(tree.focus_order_len(), 2, "打开时两个输入框可聚焦");

    // 关闭对话框（走真实路径：Action{"close"} 返回 Relayout）
    let r = tree.dispatch_to(
        "dlg",
        &UiEvent::Action {
            name: "close".into(),
            args: y901_ui::Json::obj(),
        },
    );
    assert!(r.needs_relayout(), "close 必须触发重排，实际 {r:?}");
    tree.invalidate_all();
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert_eq!(
        tree.focus_order_len(),
        1,
        "关闭后焦点顺序必须重建（只剩外层 Button）"
    );
    let cycle = focus_cycle(&mut tree);
    assert_eq!(cycle, vec!["btn".to_string()]);
}

#[test]
fn closed_dialog_is_skipped_by_tab_from_outside() {
    // 更贴近真实路径：焦点在按钮上按 Tab，不能跳进关闭的对话框
    let mut tree = tree_with_dialog(false);
    let _ = tree.set_focus(Some("btn".to_string()));
    assert_eq!(tree.focus(), Some("btn"));

    for _ in 0..4 {
        let _ = tree.dispatch(&UiEvent::Key {
            key: Key::Tab,
            modifiers: Modifiers::default(),
        });
        assert_eq!(
            tree.focus(),
            Some("btn"),
            "只有一个可聚焦节点时，Tab 必须停在原地（不能进关闭的 Dialog）"
        );
    }
}

#[test]
fn dialog_supports_downcast_for_runtime_updates() {
    // Host 改运行中的 Dialog 只能走 find_mut + as_any_mut（见 Widget::as_any_mut）
    let mut tree = tree_with_dialog(false);
    let dlg = tree
        .find_mut("dlg")
        .expect("dlg 在树里")
        .as_any_mut()
        .and_then(|a| a.downcast_mut::<Dialog>())
        .expect("Dialog 必须支持 downcast");
    dlg.set_title("Delete file?");
    dlg.set_buttons(vec![("Cancel".to_string(), "cancel".to_string())]);
    assert_eq!(dlg.children().len(), 2);
}
