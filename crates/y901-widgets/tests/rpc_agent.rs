//! Agent 视角的端到端测试：真实 TCP、真实 RPC、真实组件。
//!
//! 走完一条完整链路：
//!   get_tree 找坐标 → dispatch 鼠标按下 → invoke 全选 → dispatch 键盘输入 → 再 get_tree 验证
//!
//! 全程不碰任何内部结构，只用 Agent 能看到的协议。

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::channel;
use std::time::Duration;

use y901_agent::{Pending, RpcEvent, RpcRequest};
use y901_ui::{Flex, FlexItem, Json, Pos, UiEvent, UiTree};
use y901_widgets::TextInput;

// ---------- 极简等宽度量 ----------

struct Mono;

impl y901_ui::TextMetrics for Mono {
    fn measure_text(&self, text: &str, _style: &y901_ui::TextStyle) -> y901_ui::Size {
        let mut w = 0.0;
        for ch in text.chars() {
            w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
        }
        y901_ui::Size::new(w, 16.0)
    }
    fn line_height(&self, _style: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

// ---------- 工具 ----------

/// 在语义树 JSON 里按 id 深度优先查找节点。
fn find_node<'a>(node: &'a Json, id: &str) -> Option<&'a Json> {
    if node.get("id").and_then(|v| v.as_str()) == Some(id) {
        return Some(node);
    }
    if let Some(Json::Arr(children)) = node.get("children") {
        for child in children {
            if let Some(found) = find_node(child, id) {
                return Some(found);
            }
        }
    }
    None
}

fn num(node: &Json, key: &str) -> f64 {
    node.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn text_of(node: &Json, key: &str) -> String {
    node.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

/// UI 线程侧：处理一条 RPC 请求。
fn handle(tree: &mut UiTree, pending: Pending) {
    let result = match &pending.request {
        RpcRequest::GetTree => Ok(tree.snapshot_json()),
        RpcRequest::Ping => Ok(Json::obj().set("pong", true)),
        RpcRequest::Dispatch { node, event } => {
            let ev = event.to_ui_event();
            if tree.dispatch_to(node, &ev).is_handled() {
                Ok(Json::obj().set("ok", true))
            } else {
                Err(format!("node ignored event: {node}"))
            }
        }
        RpcRequest::Invoke {
            node,
            action,
            args,
        } => {
            let ev = UiEvent::Action {
                name: action.clone(),
                args: args.clone(),
            };
            if tree.dispatch_to(node, &ev).is_handled() {
                Ok(Json::obj().set("ok", true))
            } else {
                Err(format!("node ignored action: {action}"))
            }
        }
        RpcRequest::Focus { node } => {
            if tree.set_focus(Some(node.clone())) {
                Ok(Json::obj().set("focused", node.clone()))
            } else {
                Err(format!("cannot focus: {node}"))
            }
        }
        RpcRequest::Subscribe { node, interval_ms } => {
            if tree.subscribe(node, std::time::Duration::from_millis(*interval_ms)) {
                Ok(Json::obj())
            } else {
                Err(format!("unknown node: {node}"))
            }
        }
        RpcRequest::Unsubscribe { node } => {
            tree.unsubscribe(node);
            Ok(Json::obj())
        }
        RpcRequest::Unknown { method } => Err(format!("unknown method: {method}")),
    };
    y901_agent::rpc::reply_to(pending, result);
}

// ---------- 测试 ----------

#[test]
fn agent_drives_text_input_over_rpc() {
    // 组件树：root(column) → input
    let root = Flex::column("root").push(Box::new(
        TextInput::new("input")
            .with_text("preexisting")
            .flex_item(FlexItem::fill()),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(y901_ui::Size::new(400.0, 200.0), &Mono);

    let (tx, rx) = channel();
    let (addr, _hub) = y901_agent::rpc::spawn("127.0.0.1:0", tx).expect("bind rpc");

    // ---- Agent 侧 ----
    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(addr).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        let mut call = |line: &str| -> Json {
            writeln!(writer, "{line}").unwrap();
            writer.flush().unwrap();
            let mut buf = String::new();
            reader.read_line(&mut buf).unwrap();
            Json::parse(&buf).expect("valid json")
        };

        // 1) 找输入框坐标
        let tree_resp = call(r#"{"id":1,"method":"get_tree"}"#);
        let root = tree_resp.get("result").expect("result");
        let input = find_node(root, "input").expect("input node");
        let rect = input.get("rect").expect("rect");
        let (x, y, w, h) = (
            num(rect, "x"),
            num(rect, "y"),
            num(rect, "w"),
            num(rect, "h"),
        );
        assert!(w > 0.0 && h > 0.0, "输入框应有真实尺寸");

        // 2) 在输入框内按下鼠标（padding 8）
        let cx = x + 8.0 + 4.0;
        let cy = y + h / 2.0;
        let r2 = call(&format!(
            r#"{{"id":2,"method":"dispatch","params":{{"id":"input","event":{{"type":"mouse_down","x":{cx},"y":{cy}}}}}}}"#
        ));
        assert_eq!(r2.get("ok").and_then(|v| v.as_bool()), Some(true), "mouse_down 失败: {}", r2.to_string());

        // 3) 全选
        let r3 = call(
            r#"{"id":3,"method":"invoke","params":{"id":"input","action":"select_all"}}"#,
        );
        assert_eq!(r3.get("ok").and_then(|v| v.as_bool()), Some(true));

        // 4) 输入 "Agent"（替换选区）
        let r4 = call(
            r#"{"id":4,"method":"dispatch","params":{"id":"input","event":{"type":"text","text":"Agent"}}}"#,
        );
        assert_eq!(r4.get("ok").and_then(|v| v.as_bool()), Some(true));

        // 5) 再次读取，验证结构化 state
        let r5 = call(r#"{"id":5,"method":"get_tree"}"#);
        let input2 = find_node(r5.get("result").expect("result"), "input").expect("input node");
        let state = input2.get("state").expect("state 字段");
        (
            text_of(state, "text"),
            text_of(input2, "value"),
            num(state, "caret"),
        )
    });

    // ---- UI 线程侧：处理 5 条请求 ----
    for _ in 0..5 {
        let pending = rx.recv_timeout(Duration::from_secs(3)).expect("request");
        handle(&mut tree, pending);
    }

    let (state_text, value, caret) = client.join().unwrap();
    assert_eq!(state_text, "Agent", "state.text 应被 Agent 改写");
    assert_eq!(value, "Agent", "value 同步反映内容");
    assert_eq!(caret, 5.0, "光标应停在末尾");
}

#[test]
fn dispatch_reports_unhandled_nodes() {
    let root = Flex::column("root").push(Box::new(
        TextInput::new("input").flex_item(FlexItem::fill()),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(y901_ui::Size::new(200.0, 100.0), &Mono);

    // 未聚焦时文本事件被忽略 → RPC 返回错误而不是假装成功
    let (tx, rx) = channel();
    let (addr, _hub) = y901_agent::rpc::spawn("127.0.0.1:0", tx).expect("bind rpc");

    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(addr).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        writeln!(
            writer,
            r#"{{"id":1,"method":"dispatch","params":{{"id":"input","event":{{"type":"text","text":"x"}}}}}}"#
        )
        .unwrap();
        writer.flush().unwrap();
        let mut buf = String::new();
        reader.read_line(&mut buf).unwrap();
        Json::parse(&buf).expect("json")
    });

    handle(&mut tree, rx.recv_timeout(Duration::from_secs(3)).expect("req"));
    let resp = client.join().unwrap();
    assert_eq!(resp.get("ok").and_then(|v| v.as_bool()), Some(false));

    // 组件状态没被偷偷改动
    let snap = tree.snapshot_json();
    let input = find_node(&snap, "input").unwrap();
    assert_eq!(text_of(input.get("state").unwrap(), "text"), "");
}

#[test]
fn rpc_event_parsing_covers_mouse_and_wheel() {
    let ev = RpcEvent::from_json(&Json::parse(
        r#"{"type":"mouse_down","x":12,"y":34,"button":"left"}"#,
    )
    .unwrap())
    .unwrap();
    match ev.to_ui_event() {
        UiEvent::MouseDown { pos, .. } => assert_eq!(pos, Pos::new(12.0, 34.0)),
        other => panic!("unexpected {other:?}"),
    }

    let wheel = RpcEvent::from_json(&Json::parse(r#"{"type":"wheel","x":1,"y":2,"delta":120}"#).unwrap())
        .unwrap();
    assert!(matches!(wheel.to_ui_event(), UiEvent::Wheel { delta, .. } if delta == 120.0));

    assert!(RpcEvent::from_json(&Json::parse(r#"{"type":"bogus"}"#).unwrap()).is_err());
}

#[test]
fn agent_subscribes_and_gets_incremental_patch() {
    let root = Flex::column("root").push(Box::new(
        TextInput::new("input")
            .with_text("abc")
            .flex_item(FlexItem::fill()),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(y901_ui::Size::new(300.0, 100.0), &Mono);

    let (tx, rx) = channel();
    let (addr, hub) = y901_agent::rpc::spawn("127.0.0.1:0", tx).expect("bind rpc");

    // Agent：订阅后等两条推送
    let client = std::thread::spawn(move || {
        let stream = std::net::TcpStream::connect(addr).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        writeln!(
            writer,
            "{{\"id\":1,\"method\":\"subscribe\",\"params\":{{\"id\":\"input\",\"interval_ms\":0}}}}"
        )
        .unwrap();
        writer.flush().unwrap();
        let mut ack = String::new();
        reader.read_line(&mut ack).unwrap();
        let ack = Json::parse(&ack).unwrap();
        assert_eq!(ack.get("ok").and_then(|v| v.as_bool()), Some(true));

        let mut pushes = Vec::new();
        for _ in 0..2 {
            let mut buf = String::new();
            reader.read_line(&mut buf).unwrap();
            pushes.push(Json::parse(&buf).expect("push json"));
        }
        pushes
    });

    // UI：处理订阅
    let pending = rx.recv_timeout(Duration::from_secs(3)).expect("subscribe");
    handle(&mut tree, pending);

    // 第一次变化：聚焦 + 输入
    tree.dispatch_to("input", &UiEvent::Text("".into())); // 未聚焦，会被忽略
    tree.dispatch_to(
        "input",
        &UiEvent::Action {
            name: "focus".into(),
            args: Json::obj(),
        },
    );
    tree.dispatch_to("input", &UiEvent::Text("X".into()));

    for ev in tree.poll_subscriptions(std::time::Instant::now()) {
        hub.send(&ev.to_string());
    }

    // 第二次变化：只改文本
    tree.dispatch_to("input", &UiEvent::Text("Y".into()));
    for ev in tree.poll_subscriptions(std::time::Instant::now()) {
        hub.send(&ev.to_string());
    }

    let pushes = client.join().unwrap();

    // 第一条：focus 与文本都变了
    let first = &pushes[0];
    assert_eq!(first.get("event").and_then(|v| v.as_str()), Some("state_changed"));
    assert_eq!(first.get("id").and_then(|v| v.as_str()), Some("input"));
    let patch1 = first.get("patch").expect("patch");
    assert!(patch1.get("text").is_some());
    assert!(patch1.get("focused").is_some());

    // 第二条：只应包含变化的字段（text / caret），不该重发 focused
    let patch2 = pushes[1].get("patch").expect("patch");
    assert_eq!(text_of(patch2, "text"), "abcXY");
    assert!(
        patch2.get("focused").is_none(),
        "未变化的字段不应出现在 patch 里: {}",
        patch2.to_string()
    );
    assert!(patch2.get("text").is_some());
}

#[test]
fn subscription_throttle_coalesces_rapid_changes() {
    let root = Flex::column("root").push(Box::new(
        TextInput::new("input")
            .with_text("")
            .flex_item(FlexItem::fill()),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(y901_ui::Size::new(300.0, 100.0), &Mono);
    tree.set_focus(Some("input".to_string()));

    // 100ms 节流
    assert!(tree.subscribe("input", Duration::from_millis(100)));

    // 连续输入 5 个字符（同一窗口内）
    for ch in ["a", "b", "c", "d", "e"] {
        tree.dispatch_to("input", &UiEvent::Text(ch.to_string()));
    }

    // 窗口未到：一条都不发
    let early = tree.poll_subscriptions(std::time::Instant::now());
    assert!(early.is_empty(), "节流窗口内不应推送");

    // 窗口到了：合并成一条，patch 里是最终状态
    let late = tree.poll_subscriptions(
        std::time::Instant::now() + Duration::from_millis(150),
    );
    assert_eq!(late.len(), 1, "5 次输入应合并成 1 条推送");
    let patch = late[0].get("patch").expect("patch");
    assert_eq!(text_of(patch, "text"), "abcde");
}