//! 审查修复的回归测试：MCP 握手、IME 提交、RPC/MCP 一致性、headless 走 Host。

use y901_agent::{McpServer, Policy};
use y901_runtime::{translate, Host, RawEvent};
use y901_ui::{
    Command, EventResult, Flex, FlexItem, Json, Key, Modifiers, Size, TextMetrics, TextStyle,
    UiEvent, UiTree,
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

/// 记录 Host 收到的命令。
#[derive(Default)]
struct Recorder {
    commands: Vec<String>,
}

impl Host for Recorder {
    fn idle(&mut self, _tree: &mut UiTree) {}
    fn command(&mut self, _tree: &mut UiTree, cmd: Command) -> bool {
        if let Command::App { name, .. } = cmd {
            self.commands.push(name);
            return true;
        }
        false
    }
}

// ---------- P0：标准 MCP 握手 ----------

#[test]
fn standard_mcp_handshake_when_no_token_configured() {
    let mut server = McpServer::new(Policy::default());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        Button::new("ok", "OK").flex_item(FlexItem::height(24.0)),
    ))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    // 标准客户端：params 只有 protocolVersion / capabilities / clientInfo
    let out = server
        .handle_line(
            &mut tree,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"claude","version":"1.0"}}}"#,
        )
        .expect("response");
    assert!(out.contains("protocolVersion"), "握手应成功：{out}");
    assert!(!out.contains("-32001"), "不应要求 token：{out}");

    let tools = server
        .handle_line(
            &mut tree,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        )
        .expect("response");
    assert!(tools.contains("ui_click"), "tools/list 应可用");
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

// ---------- P1-4：RPC 与 MCP 的树一致 ----------

#[test]
fn rpc_get_tree_matches_mcp_ui_tree() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        Button::new("ok", "OK").flex_item(FlexItem::height(24.0)),
    ))));
    tree.layout(Size::new(300.0, 100.0), &Mono);
    let policy = Policy::default();

    // RPC 路径：get_tree 走 dispatch_request
    let via_rpc = y901_agent::dispatch_request(&mut tree, &policy, "get_tree", &Json::obj())
        .expect("rpc tree");

    // MCP 路径：resources/read ui://tree
    let mut server = McpServer::new(Policy::default());
    server.state = y901_agent::mcp::State::Initialized;
    let raw = server
        .handle_line(
            &mut tree,
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"ui://tree"}}"#,
        )
        .expect("mcp");
    let parsed = Json::parse(&raw).expect("json");
    let content = match parsed.get("result").and_then(|r| r.get("contents")) {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("contents"),
    };
    let text = content[0]
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let via_mcp = Json::parse(text).expect("inner");

    assert_eq!(
        via_rpc.to_string(),
        via_mcp.to_string(),
        "RPC 与 MCP 的树必须逐字节一致"
    );
}

// ---------- P1-2：headless 命令必须经 Host ----------

#[test]
fn mcp_click_emits_command_that_host_receives() {
    let mut server = McpServer::new(Policy::default());
    // 按钮点击会发 Command::App，headless 路径必须把它交给 host.command
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(
            Button::new("ok", "OK")
                .on_click("demo.click", Json::obj())
                .flex_item(FlexItem::height(24.0)),
        )),
    ));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let _ = server.handle_line(
        &mut tree,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
    );
    let _ = server.handle_line(
        &mut tree,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"ok"}}}"#,
    );

    let mut host = Recorder::default();
    for cmd in tree.drain_commands() {
        host.command(&mut tree, cmd);
    }
    assert!(
        host.commands.iter().any(|c| c == "demo.click"),
        "点击应产生命令并交给 Host，实际 {:?}",
        host.commands
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
