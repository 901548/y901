//! 同一节点的 revision 必须**跨接口一致**。
//!
//! 三处输出：`get_tree`（public_tree）、`query`（query_result）、
//! MCP 单节点资源（node_resource）。任何一处用「组件自报」而另一处用
//! 「框架有效值」，都会让 Agent 在不同接口看到不同 revision —— 乐观锁直接失效。

use y901_agent::{handle_call, ApiCall, McpServer, Policy};
use y901_ui::{Flex, FlexItem, Json, Size, TextMetrics, TextStyle, UiTree};
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

fn tree() -> UiTree {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(
                Button::new("btn", "OK").flex_item(FlexItem::height(24.0)),
            ))
            .push(Box::new(
                TextInput::new("field").flex_item(FlexItem::height(24.0)),
            )),
    ));
    tree.layout(Size::new(300.0, 120.0), &Mono);
    tree
}

fn find_node<'a>(node: &'a Json, id: &str) -> Option<&'a Json> {
    if node.get("id").and_then(|v| v.as_str()) == Some(id) {
        return Some(node);
    }
    match node.get("children") {
        Some(Json::Arr(k)) => k.iter().find_map(|c| find_node(c, id)),
        _ => None,
    }
}

fn rev(node: &Json) -> u64 {
    node.get("revision")
        .and_then(|v| v.as_f64())
        .map(|f| f as u64)
        .expect("revision 字段")
}

fn revisions_of(tree: &mut UiTree, id: &str) -> (u64, u64, u64) {
    // 1) get_tree
    let view = handle_call(tree, &Policy::default(), ApiCall::Tree).expect("tree");
    let from_tree = rev(find_node(&view, id).expect("node in tree"));

    // 2) query
    let q = handle_call(
        tree,
        &Policy::default(),
        ApiCall::Query {
            q: format!("id={id}"),
        },
    )
    .expect("query");
    let items = match q.get("items") {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("items"),
    };
    let from_query = rev(&items[0]);

    // 3) MCP ui://node/{id}
    let mut server = McpServer::new(Policy::default());
    server.state = y901_agent::mcp::State::Initialized;
    let raw = server
        .handle_line(
            tree,
            &format!(
                r#"{{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{{"uri":"ui://node/{id}"}}}}"#
            ),
        )
        .expect("response");
    let parsed = Json::parse(&raw).expect("json");
    let contents = match parsed.get("result").and_then(|r| r.get("contents")) {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("contents"),
    };
    let inner_text = contents[0]
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let inner = Json::parse(inner_text).expect("inner json");
    let from_resource = rev(&inner);

    (from_tree, from_query, from_resource)
}

#[test]
fn all_three_interfaces_report_the_same_revision() {
    let mut tree = tree();
    let (a, b, c) = revisions_of(&mut tree, "btn");
    assert_eq!(a, b, "get_tree 与 query 的 revision 必须一致：{a} vs {b}");
    assert_eq!(
        b, c,
        "query 与 node_resource 的 revision 必须一致：{b} vs {c}"
    );

    let (a2, b2, c2) = revisions_of(&mut tree, "field");
    assert_eq!(a2, b2);
    assert_eq!(b2, c2);
}

#[test]
fn revision_is_nonzero_after_a_state_change_across_all_interfaces() {
    let mut tree = tree();
    let before = revisions_of(&mut tree, "field").0;

    handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Invoke {
            id: "field".into(),
            action: "set_text".into(),
            args: Json::obj().set("text", "hello"),
            if_rev: None,
        },
    )
    .expect("invoke");

    let (a, b, c) = revisions_of(&mut tree, "field");
    assert!(a > before, "状态变化后 revision 必须增长（{before} → {a}）");
    assert_eq!(a, b);
    assert_eq!(b, c);
}
