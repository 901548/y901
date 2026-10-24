//! 统一 dispatcher：RPC 与 MCP 必须返回一致结果；role 由框架注入。

use y901_agent::{dispatch_request, McpServer, Policy};
use y901_ui::{
    Flex, FlexItem, Json, Size, TextStyle, TextMetrics, UiTree,
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

fn tree() -> UiTree {
    let mut root = Flex::column("root");
    for (id, label) in [("action.save", "Save"), ("action.cancel", "Cancel"), ("action.quit", "Quit")] {
        root = root.push(Box::new(
            Button::new(id, label).flex_item(FlexItem::height(24.0)),
        ));
    }
    let mut t = UiTree::new(Box::new(root));
    t.layout(Size::new(300.0, 200.0), &Mono);
    t
}

#[test]
fn rpc_and_mcp_query_return_the_same_payload() {
    let mut tree = tree();
    let policy = Policy::default();
    let q = r#"role=button & label~="a" & limit=2"#;

    // 统一入口（RPC 走这条）
    let via_rpc = dispatch_request(
        &mut tree,
        &policy,
        "query",
        &Json::obj().set("q", q),
    )
    .expect("rpc query");

    // MCP 走 tools/call ui_query
    let mut server = McpServer::new(Policy::default());
    server.state = y901_agent::mcp::State::Initialized;
    let line = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"ui_query","arguments":{{"q":{}}}}}}}"#,
        Json::Str(q.to_string())
    );
    let raw = server.handle_line(&mut tree, &line).expect("mcp response");
    let parsed = Json::parse(&raw).expect("json");
    let content = match parsed.get("result").and_then(|r| r.get("content")) {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("content"),
    };
    let text = content[0].get("text").and_then(|v| v.as_str()).unwrap_or("");
    let via_mcp = Json::parse(text).expect("inner json");

    assert_eq!(
        via_rpc.to_string(),
        via_mcp.to_string(),
        "RPC 与 MCP 的查询结果必须逐字节一致"
    );
    assert!(via_rpc.to_string().contains("\"count\":2"));
}

#[test]
fn role_is_injected_by_framework_not_by_component() {
    let mut tree = tree();

    // 组件自己从不填 role
    let node = tree.find("action.save").expect("button");
    assert_eq!(
        node.semantics().role.0,
        "",
        "组件不应手填 role，应由框架注入"
    );

    // 但 query 能按 role 命中
    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "query",
        &Json::obj().set("q", "role=button"),
    )
    .expect("query");
    assert!(
        out.to_string().contains("\"count\":3"),
        "按 role 查询应命中 3 个按钮：{}",
        out.to_string()
    );
}

#[test]
fn id_convention_is_queryable() {
    let mut tree = tree();
    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "query",
        &Json::obj().set("q", r#"id="action.save""#),
    )
    .expect("query");
    assert!(out.to_string().contains("\"count\":1"));
}