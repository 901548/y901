//! 四个 P0 的证伪测试（审计发现）。
//!
//! 全部走真实路由：`dispatch_request` / `handle_call` / `McpServer::handle_line`，
//! 不直接调组件内部方法。

use std::sync::Mutex;

use y901_agent::api::{dispatch_request, ApiError, Policy};
use y901_agent::mcp::McpServer;
use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Json, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiTree, Widget,
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

/// 对 AI 不可见的探针（exposed = false）。
struct Hidden {
    id: String,
    rect: Rect,
}

impl Widget for Hidden {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Handled
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        // 默认 exposed_to_ai = false
        Semantics::default().label("hidden")
    }
}

fn tree_with_button() -> UiTree {
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

/// 制造一次"框架计数器 > 0"：真实输入会改变语义面 -> bump。
fn bump_field(tree: &mut UiTree) {
    // TextInput 未聚焦时会忽略文本事件，先聚焦
    tree.set_focus(Some("field".to_string()));
    // 两次输入：第一次只建立语义基线（不 bump），第二次才真正产生计数器增量
    for text in ["x", "y"] {
        dispatch_request(
            tree,
            &Policy::default(),
            "type",
            &Json::obj().set("id", "field").set("text", text),
        )
        .expect("type");
    }
}

fn rev_of(tree: &mut UiTree, id: &str) -> u64 {
    let view = dispatch_request(tree, &Policy::default(), "get_tree", &Json::obj()).expect("tree");
    fn find<'a>(n: &'a Json, id: &str) -> Option<&'a Json> {
        if n.get("id").and_then(|v| v.as_str()) == Some(id) {
            return Some(n);
        }
        match n.get("children") {
            Some(Json::Arr(k)) => k.iter().find_map(|c| find(c, id)),
            _ => None,
        }
    }
    find(&view, id)
        .and_then(|n| n.get("revision"))
        .and_then(|v| v.as_f64())
        .map(|f| f as u64)
        .expect("revision")
}

// ---------- P0-1：own vs effective 不一致 ----------

#[test]
fn if_rev_from_get_tree_is_accepted_by_click() {
    let mut tree = tree_with_button();
    bump_field(&mut tree);
    let rev = rev_of(&mut tree, "field");
    assert!(rev > 0, "前置：输入后有效 revision 应 > 0，实际 {rev}");

    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "type",
        &Json::obj()
            .set("id", "field")
            .set("text", "y")
            .set("if_rev", rev),
    );
    assert!(
        out.is_ok(),
        "get_tree 给出的 rev 必须被后续动作接受，实际 {out:?}"
    );
}

// ---------- P0-2：dispatch 绕过安全闸 ----------

#[test]
fn dispatch_to_unexposed_node_is_forbidden() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(Hidden {
        id: "secret".into(),
        rect: Rect::ZERO,
    }))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "dispatch",
        &Json::obj()
            .set("id", "secret")
            .set("event", Json::obj().set("type", "key").set("key", "enter")),
    );
    assert!(
        matches!(out, Err(ApiError::Forbidden { .. })),
        "对未 exposed 节点 dispatch 必须 403，实际 {out:?}"
    );
}

#[test]
fn dispatch_with_stale_rev_is_conflict() {
    let mut tree = tree_with_button();
    bump_field(&mut tree);
    let rev = rev_of(&mut tree, "field");
    assert!(rev > 0);

    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "dispatch",
        &Json::obj()
            .set("id", "field")
            .set("if_rev", rev.saturating_sub(1))
            .set("event", Json::obj().set("type", "key").set("key", "enter")),
    );
    assert!(
        matches!(out, Err(ApiError::Conflict { .. })),
        "过期 if_rev 必须 409，实际 {out:?}"
    );
}

// ---------- P0-3：MCP 不校验 Token ----------

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn mcp_requires_token_when_configured() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // 先清掉，制造"未注入 Token"的场景
    std::env::remove_var("Y901_TOKEN");
    let policy = Policy {
        token: "s3cret".to_string(),
        redact: true,
    };
    let mut server = McpServer::new(policy.clone());
    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(10.0, 10.0), &Mono);

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let resp = server.handle_line(&mut tree, init).expect("response");
    assert!(
        resp.contains("401") || resp.contains("unauthorized"),
        "未注入 Token 时必须拒绝，实际 {resp}"
    );

    // 注入正确 Token 后应通过
    std::env::set_var("Y901_TOKEN", "s3cret");
    let mut server = McpServer::new(policy);
    let resp = server
        .handle_line(&mut tree, init)
        .expect("response");
    assert!(
        resp.contains("result"),
        "注入正确 Token 后应握手成功，实际 {resp}"
    );
    std::env::remove_var("Y901_TOKEN");
}

// ---------- P0-4：Key 事件 to_json/from_json 回环 ----------

#[test]
fn key_event_roundtrips_through_json() {
    use y901_agent::rpc::RpcEvent;
    use y901_ui::Key;

    for key in [
        Key::Enter,
        Key::Tab,
        Key::Backspace,
        Key::Escape,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
    ] {
        let ev = RpcEvent::Key(key);
        let json = ev.to_json();
        let back = RpcEvent::from_json(&json)
            .unwrap_or_else(|e| panic!("{key:?} 回环失败：{e}，json={json}"));
        assert_eq!(back, ev, "{key:?} 回环后不一致，json={json}");
    }
}

// ---------- P0-2 统一入口：focus / subscribe / unsubscribe ----------

fn tree_with_hidden() -> UiTree {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(Hidden {
            id: "secret".into(),
            rect: Rect::ZERO,
        })),
    ));
    tree.layout(Size::new(300.0, 100.0), &Mono);
    tree
}

#[test]
fn dispatch_request_focus_checks_exposed() {
    let mut tree = tree_with_hidden();
    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "focus",
        &Json::obj().set("id", "secret"),
    );
    assert!(
        matches!(out, Err(ApiError::Forbidden { .. })),
        "对未 exposed 节点 focus 必须 403，实际 {out:?}"
    );
}

#[test]
fn dispatch_request_subscribe_checks_exposed() {
    let mut tree = tree_with_hidden();
    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "subscribe",
        &Json::obj().set("id", "secret").set("interval_ms", 100),
    );
    assert!(
        matches!(out, Err(ApiError::Forbidden { .. })),
        "对未 exposed 节点 subscribe 必须 403，实际 {out:?}"
    );
}

#[test]
fn dispatch_request_subscribe_rejects_bad_interval() {
    let mut tree = with_visible_button();
    for bad in [-1.0f64, 4_000_000.0f64] {
        let out = dispatch_request(
            &mut tree,
            &Policy::default(),
            "subscribe",
            &Json::obj().set("id", "btn").set("interval_ms", bad),
        );
        assert!(
            matches!(out, Err(ApiError::BadRequest(_))),
            "interval_ms={bad} 必须 400，实际 {out:?}"
        );
    }
}

#[test]
fn dispatch_request_unsubscribe_no_locate_needed() {
    let mut tree = tree_with_hidden();
    // 未 exposed 也能取消订阅：这只是移除本地状态，不泄露信息
    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "unsubscribe",
        &Json::obj().set("id", "secret"),
    );
    assert!(out.is_ok(), "unsubscribe 不应走能力闸，实际 {out:?}");
}

#[test]
fn host_routes_focus_through_dispatch_request() {
    // 结构断言：host.rs 的 focus/subscribe/unsubscribe 三个分支必须统一转发
    let src = include_str!("../../../crates/y901-app/src/host.rs");
    for method in ["focus", "subscribe", "unsubscribe"] {
        let needle = format!("dispatch_request(tree, policy, \"{method}\"");
        assert!(
            src.contains(&needle),
            "host.rs 必须把 {method} 转发给 dispatch_request（找不到 `{needle}`）"
        );
    }
}

fn with_visible_button() -> UiTree {
    tree_with_button()
}