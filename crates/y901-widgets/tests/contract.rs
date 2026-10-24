//! 本轮修复的回归测试：动作契约、add_node 递归挂载、重复 id。

use std::cell::Cell;
use std::rc::Rc;

use y901_agent::{handle_call, ApiCall, ApiError, Policy};
use y901_ui::{
    Command, Flex, FlexItem, Json, Rect, Size, Signal, TextStyle, TextMetrics, UiEvent, UiTree,
};
use y901_widgets::{Button, Dialog, Menu, TextLabel};

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

// ---------- Menu select ----------

#[test]
fn menu_select_action_triggers_command_and_closes() {
    let mut menu = Menu::new("menu").items(vec!["Open".into(), "Save".into()]);
    menu.open();

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let handled = tree
        .dispatch_to(
            "menu",
            &UiEvent::Action {
                name: "select".into(),
                args: Json::obj().set("index", 1u32),
            },
        )
        .is_handled();
    assert!(handled, "select 应被处理");

    let cmds = tree.drain_commands();
    assert_eq!(cmds.len(), 1, "应发出一条命令，实际 {cmds:?}");
    match &cmds[0] {
        Command::App { name, args } => {
            assert_eq!(name, "menu.select");
            assert!(args.to_string().contains("\"index\":1"), "{}", args.to_string());
        }
        other => panic!("应为 App 命令：{other:?}"),
    }
    assert!(
        tree.snapshot_json().to_string().contains("\"open\":false"),
        "菜单应关闭"
    );
}

#[test]
fn menu_select_out_of_range_is_ignored() {
    let mut menu = Menu::new("menu").items(vec!["Only".into()]);
    menu.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let handled = tree
        .dispatch_to(
            "menu",
            &UiEvent::Action {
                name: "select".into(),
                args: Json::obj().set("index", 9u32),
            },
        )
        .is_handled();
    assert!(!handled, "越界索引不应被处理");
    assert!(tree.drain_commands().is_empty());
}

// ---------- Dialog close ----------

#[test]
fn dialog_close_action_closes_it() {
    let mut dlg = Dialog::new("dlg", "Info").buttons(vec![("OK", "ok")]);
    dlg.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(dlg))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert!(tree.snapshot_json().to_string().contains("\"open\":true"));
    let handled = tree
        .dispatch_to(
            "dlg",
            &UiEvent::Action {
                name: "close".into(),
                args: Json::obj(),
            },
        )
        .is_handled();
    assert!(handled, "close 应被处理");
    assert!(
        tree.snapshot_json().to_string().contains("\"open\":false"),
        "对话框应关闭"
    );
}

// ---------- 网关：未处理的动作必须报错 ----------

#[test]
fn gateway_errors_when_action_not_handled() {
    // Terminal 的 focus 声明已删除；这里用一个"声明了但 event 不处理"的场景：
    // Button 声明 click，但 Action("click") 并不是它的语义（它只响应 Action("click")？）
    // 用 Menu 的 select 传入非法 index：声明存在但组件返回 Ignored。
    let mut menu = Menu::new("menu").items(vec!["Only".into()]);
    menu.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let err = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Invoke {
            id: "menu".into(),
            action: "select".into(),
            args: Json::obj().set("index", 9u32),
            if_rev: None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, ApiError::BadRequest(_)),
        "未处理应返回 400，实际 {err:?}"
    );
}

#[test]
fn undeclared_action_is_rejected() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        Button::new("ok", "OK").flex_item(FlexItem::height(24.0)),
    ))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let err = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Invoke {
            id: "ok".into(),
            action: "explode".into(),
            args: Json::obj(),
            if_rev: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
}

// ---------- add_node 递归挂载 ----------

#[test]
fn add_node_mounts_whole_subtree() {
    let sig = Signal::new("a".to_string());

    // 复合容器：Flex 里包一个 bind(signal) 的标签
    let label = TextLabel::new("lbl", "a").bind(sig.clone());
    let panel = Flex::column("panel")
        .push(Box::new(label))
        .flex_item(FlexItem::height(40.0));

    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert!(tree.add_node("root", Box::new(panel)));
    assert_eq!(sig.subscriber_count(), 1, "子树内的标签应被挂载并订阅");

    sig.set("b".to_string());
    assert_eq!(tree.drain_invalidation(), 1, "Signal 变化应产生失效");
}

#[test]
fn add_node_rejects_duplicate_id() {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(
            Button::new("dup", "A").flex_item(FlexItem::height(24.0)),
        )),
    ));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let before = tree.snapshot_json().to_string();
    let ok = tree.add_node("root", Box::new(Button::new("dup", "B")));
    assert!(!ok, "重复 id 应被拒绝");
    assert_eq!(
        before,
        tree.snapshot_json().to_string(),
        "被拒绝后树结构不应变化"
    );
}

#[test]
fn insert_node_rejects_duplicate_id() {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(TextLabel::new("dup", "x"))),
    ));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let ok = tree.insert_node("root", 0, Box::new(TextLabel::new("dup", "y")));
    assert!(!ok);
    let _ = Rc::new(Cell::new(0));
    let _ = Rect::ZERO;
}
