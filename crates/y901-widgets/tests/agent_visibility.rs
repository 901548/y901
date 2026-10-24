//! Agent 可见的 disabled 字段：三处输出必须一致。
//!
//! 这是 revision 实现的前置：**哈希范围必须等于 Agent 可见范围**。
//! 若 `disabled` 不可见却被哈希，Agent 会收到"状态变了但看不出来"的 409。

use y901_agent::{handle_call, ApiCall, McpServer, Policy};
use y901_ui::{
    Flex, FlexItem, Json, Size, TextStyle, TextMetrics, UiTree,
};
use y901_widgets::{Button, TextInput};

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

fn tree_with(disabled: bool) -> UiTree {
    let btn = Button::new("ok", "OK")
        .disabled(disabled)
        .flex_item(FlexItem::height(24.0));
    let input = TextInput::new("field")
        .disabled(disabled)
        .flex_item(FlexItem::height(24.0));
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(btn)).push(Box::new(input)),
    ));
    tree.layout(Size::new(300.0, 120.0), &Mono);
    tree
}

/// 从 `public_tree` 的输出里取某节点的字段。
fn field_of(view: &Json, id: &str, key: &str) -> Option<Json> {
    fn find<'a>(node: &'a Json, id: &str) -> Option<&'a Json> {
        if node.get("id").and_then(|v| v.as_str()) == Some(id) {
            return Some(node);
        }
        match node.get("children") {
            Some(Json::Arr(kids)) => kids.iter().find_map(|k| find(k, id)),
            _ => None,
        }
    }
    find(view, id).and_then(|n| n.get(key).cloned())
}

#[test]
fn get_tree_reports_disabled_true() {
    let mut tree = tree_with(true);
    let view = handle_call(&mut tree, &Policy::default(), ApiCall::Tree).expect("tree");

    assert_eq!(
        field_of(&view, "ok", "disabled"),
        Some(Json::Bool(true)),
        "disabled 的按钮必须在 get_tree 里可见：{}",
        view.to_string()
    );
    assert_eq!(field_of(&view, "field", "disabled"), Some(Json::Bool(true)));
}

#[test]
fn get_tree_reports_disabled_false() {
    let mut tree = tree_with(false);
    let view = handle_call(&mut tree, &Policy::default(), ApiCall::Tree).expect("tree");
    assert_eq!(field_of(&view, "ok", "disabled"), Some(Json::Bool(false)));
}

#[test]
fn query_result_reports_disabled() {
    let mut tree = tree_with(true);
    let out = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query {
            q: "id=ok".into(),
        },
    )
    .expect("query");
    let items = match out.get("items") {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("items"),
    };
    assert_eq!(
        items[0].get("disabled"),
        Some(&Json::Bool(true)),
        "query 结果也必须带 disabled：{}",
        items[0].to_string()
    );
}

#[test]
fn node_resource_reports_disabled() {
    let mut tree = tree_with(true);
    let mut server = McpServer::new(Policy::default());
    server.state = y901_agent::mcp::State::Initialized;

    let raw = server
        .handle_line(
            &mut tree,
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"ui://node/ok"}}"#,
        )
        .expect("response");
    assert!(
        raw.contains("\"disabled\":true") || raw.contains("\\\"disabled\\\":true"),
        "单节点资源也必须带 disabled：{raw}"
    );
}

#[test]
fn disabled_matches_component_state() {
    // 反向验证：组件确实是 disabled，输出也跟着变——不是恒真字段
    let mut on = tree_with(true);
    let mut off = tree_with(false);
    let a = handle_call(&mut on, &Policy::default(), ApiCall::Tree).unwrap();
    let b = handle_call(&mut off, &Policy::default(), ApiCall::Tree).unwrap();
    assert_ne!(
        field_of(&a, "ok", "disabled"),
        field_of(&b, "ok", "disabled"),
        "两种状态输出必须不同，否则字段是常量"
    );
}

/// disabled 只允许出现在顶层；state 里若出现就是双写，会让 Agent 困惑、
/// 也会让 revision 的哈希范围含糊。此测试防止它被加回来。
#[test]
fn semantics_disabled_is_top_level_only() {
    let tree = tree_with(true);
    let btn = tree.find("ok").expect("button");
    let sem = btn.semantics();
    assert!(sem.disabled, "顶层 disabled 必须为 true");

    if let y901_ui::Json::Obj(map) = &sem.state {
        assert!(
            !map.iter().any(|(k, _)| k == "disabled"),
            "state 里不应再出现 disabled，实际 {:?}",
            map.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
        );
    }

    // TextInput 同样规则
    let input = tree.find("field").expect("input");
    let sem2 = input.semantics();
    assert!(sem2.disabled);
    if let y901_ui::Json::Obj(map) = &sem2.state {
        assert!(
            !map.iter().any(|(k, _)| k == "disabled"),
            "TextInput 的 state 里也不应出现 disabled"
        );
    }
}