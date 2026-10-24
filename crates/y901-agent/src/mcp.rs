//! MCP（Model Context Protocol）Server：stdio + JSON-RPC 2.0。
//!
//! 零依赖：只用 `std::io` 与自带的 `Json`。
//!
//! 消息是**行分隔**的 JSON-RPC 2.0（MCP 的 stdio 传输规范）。
//! 这里只做协议编解码与工具映射，真正的执行复用 `api::handle_call`，
//! 因此脱敏、作用域、版本锁这些安全策略天然继承。

use std::io::{BufRead, Write};

use y901_ui::{Json, UiTree};

use crate::api::{handle_call, ApiCall, ApiError, Policy};

/// 协议版本：与 MCP 2024-11-05 规范对齐。
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// 握手状态机：未 initialize 之前，除 ping 外一律拒绝。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Uninitialized,
    Initialized,
}

pub struct McpServer {
    policy: Policy,
    pub state: State,
}

impl McpServer {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            state: State::Uninitialized,
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// 处理一行 JSON-RPC。返回 `None` 表示这是通知，不需要回复。
    pub fn handle_line(&mut self, tree: &mut UiTree, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }

        let Ok(msg) = Json::parse(line) else {
            return Some(error_response(Json::Null, -32700, "parse error"));
        };

        // JSON-RPC 2.0 版本字段必须存在且正确
        if msg.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
            let id = msg.get("id").cloned().unwrap_or(Json::Null);
            return Some(error_response(id, -32600, "invalid request: jsonrpc must be \"2.0\""));
        }

        let id = msg.get("id").cloned();
        let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
            return id.map(|i| error_response(i, -32600, "missing method"));
        };
        let params = msg.get("params").cloned().unwrap_or(Json::Null);

        // 通知：有 method 无 id
        if id.is_none() {
            // 未握手前不接受 initialized 通知
            if method == "notifications/initialized" && self.state == State::Initialized {
                // 已经是握手态（initialize 已推进），通知只作确认，无需额外状态
            }
            return None;
        }
        let id = id.unwrap();

        // Token：MCP 没有 HTTP 头，用 params.token 承载
        let presented = params.get("token").and_then(|v| v.as_str());
        if self.policy.authorize_token(presented).is_err() {
            return Some(error_response(id, -32001, "unauthorized: invalid or missing token"));
        }

        // 握手门：未初始化时只放行 initialize / ping
        if self.state == State::Uninitialized && method != "initialize" && method != "ping" {
            return Some(error_response(id, -32002, "Server not initialized"));
        }

        let result = match method {
            "initialize" => {
                // 回应 initialize 即视为已握手（initialized 通知只是补充确认）
                self.state = State::Initialized;
                Ok(initialize_result())
            }
            "ping" => Ok(Json::obj()),
            "tools/list" => Ok(Json::obj().set("tools", Json::Arr(tool_defs()))),
            "tools/call" => self.call_tool(tree, &params),
            "resources/list" => Ok(Json::obj().set("resources", Json::Arr(resource_defs()))),
            "resources/templates/list" => {
                Ok(Json::obj().set("resourceTemplates", Json::Arr(template_defs())))
            }
            "resources/read" => self.read_resource(tree, &params),
            _ => Err((-32601, format!("method not found: {method}"))),
        };

        Some(match result {
            Ok(value) => ok_response(id, value),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    // ---------- tools ----------

    fn call_tool(&mut self, tree: &mut UiTree, params: &Json) -> Result<Json, (i32, String)> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or((-32602, "tools/call requires name".to_string()))?;
        let args = params.get("arguments").cloned().unwrap_or_else(Json::obj);

        // 统一入口：RPC 与 MCP 共用同一份执行逻辑
        match crate::api::dispatch_request(tree, &self.policy, name, &args) {
            Ok(value) => Ok(tool_ok(&value)),
            Err(err) => Ok(tool_err(&err)),
        }
    }
    // ---------- resources ----------

    fn read_resource(&mut self, tree: &mut UiTree, params: &Json) -> Result<Json, (i32, String)> {
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or((-32602, "resources/read requires uri".to_string()))?;

        let text = if uri == "ui://tree" {
            match handle_call(tree, &self.policy, ApiCall::Tree) {
                Ok(json) => json.to_string(),
                Err(err) => return Ok(tool_err(&err)),
            }
        } else if let Some(id) = uri.strip_prefix("ui://node/") {
            match node_resource(tree, &self.policy, id) {
                Some(json) => json.to_string(),
                // 未暴露的节点：不区分「不存在」与「无权限」，避免回显 id 是否有效
                None => {
                    return Err((-32002, "resource not found".to_string()));
                }
            }
        } else {
            return Err((-32002, format!("unknown resource: {uri}")));
        };

        Ok(Json::obj().set(
            "contents",
            Json::Arr(vec![Json::obj()
                .set("uri", uri)
                .set("mimeType", "application/json")
                .set("text", text)]),
        ))
    }
}

// ---------- 定义 ----------

fn initialize_result() -> Json {
    Json::obj()
        .set("protocolVersion", PROTOCOL_VERSION)
        .set(
            "capabilities",
            Json::obj()
                .set("tools", Json::obj().set("listChanged", false))
                .set("resources", Json::obj().set("subscribe", false).set("listChanged", false)),
        )
        .set(
            "serverInfo",
            Json::obj()
                .set("name", "y901")
                .set("version", env!("CARGO_PKG_VERSION")),
        )
}

/// 四个工具 + 标准 JSON Schema。
fn tool_defs() -> Vec<Json> {
    vec![
        Json::obj()
            .set("name", "ui_click")
            .set(
                "description",
                "Click a UI node by id. The gateway resolves its rect and synthesizes mouse events.",
            )
            .set(
                "inputSchema",
                schema(
                    &[("id", "string", "节点 id（必须 exposed_to_ai=true）"),],
                    &["id"],
                    &[("if_rev", "number", "乐观锁：节点当前 revision，不匹配则冲突")],
                ),
            ),
        Json::obj()
            .set("name", "ui_type_text")
            .set("description", "Type text into a focused input node.")
            .set(
                "inputSchema",
                schema(
                    &[
                        ("id", "string", "节点 id"),
                        ("text", "string", "要输入的文本"),
                    ],
                    &["id", "text"],
                    &[("if_rev", "number", "乐观锁")],
                ),
            ),
        Json::obj()
            .set("name", "ui_scroll")
            .set("description", "Scroll a scrollable node. Positive delta scrolls down.")
            .set(
                "inputSchema",
                schema(
                    &[
                        ("id", "string", "节点 id"),
                        ("delta", "number", "滚动量（像素），正数向下"),
                    ],
                    &["id", "delta"],
                    &[("if_rev", "number", "乐观锁")],
                ),
            ),
        Json::obj()
            .set("name", "ui_query")
            .set(
                "description",
                "Query nodes by selector, e.g. `role=button & label~=\"Save\" & limit=5`. Returns id/role/label/rect/revision.",
            )
            .set(
                "inputSchema",
                schema(
                    &[("q", "string", "查询表达式")],
                    &["q"],
                    &[],
                ),
            ),
        Json::obj()
            .set("name", "ui_invoke")
            .set("description", "Invoke an action declared by a node's `actions` list.")
            .set(
                "inputSchema",
                schema(
                    &[
                        ("id", "string", "节点 id"),
                        ("action", "string", "动作名，必须在该节点的 actions 中"),
                        ("args", "object", "动作参数"),
                    ],
                    &["id", "action"],
                    &[("if_rev", "number", "乐观锁")],
                ),
            ),
    ]
}

/// 极小的 schema 构造器：required / optional 两组属性。
fn schema(
    required: &[(&str, &str, &str)],
    required_keys: &[&str],
    optional: &[(&str, &str, &str)],
) -> Json {
    let mut props = Json::obj();
    for (name, ty, desc) in required.iter().chain(optional.iter()) {
        props = props.set(
            *name,
            Json::obj()
                .set("type", *ty)
                .set("description", *desc),
        );
    }
    let mut obj = Json::obj()
        .set("type", "object")
        .set("properties", props)
        .set("additionalProperties", false);
    obj = obj.set(
        "required",
        Json::Arr(
            required_keys
                .iter()
                .map(|k| Json::Str((*k).to_string()))
                .collect(),
        ),
    );
    obj
}

fn template_defs() -> Vec<Json> {
    vec![Json::obj()
        .set("uriTemplate", "ui://node/{id}")
        .set("name", "ui_node")
        .set("description", "Single node details; use ui://node/<id>.")
        .set("mimeType", "application/json")]
}

fn resource_defs() -> Vec<Json> {
    vec![
        Json::obj()
            .set("uri", "ui://tree")
            .set("name", "ui_tree")
            .set(
                "description",
                "Full semantic tree (already filtered by exposed_to_ai and redacted).",
            )
            .set("mimeType", "application/json"),

    ]
}

/// 单个节点：只在 `exposed_to_ai` 时返回，否则一律 None。
fn node_resource(tree: &UiTree, policy: &Policy, id: &str) -> Option<Json> {
    let node = tree.find(id)?;
    if !node.semantics().exposed_to_ai {
        return None;
    }
    let sem = node.semantics();
    let rect = tree.rect_of(id).unwrap_or_else(|| node.rect());
    let merged = crate::api::redact_node(&sem, policy);
    Some(
        Json::obj()
            .set("id", id)
            .set("role", node.role().as_str())
            .set("revision", node.revision())
            .set(
                "rect",
                Json::obj()
                    .set("x", rect.x)
                    .set("y", rect.y)
                    .set("w", rect.w)
                    .set("h", rect.h),
            )
            .set("label", merged.get("label").cloned().unwrap_or(Json::Null))
            .set("value", merged.get("value").cloned().unwrap_or(Json::Null))
            .set("state", merged.get("state").cloned().unwrap_or(Json::Null))
            .set("actions", merged.get("actions").cloned().unwrap_or(Json::Null)),
    )
}
// ---------- 编解码 ----------

fn ok_response(id: Json, result: Json) -> String {
    Json::obj()
        .set("jsonrpc", "2.0")
        .set("id", id)
        .set("result", result)
        .to_string()
}

fn error_response(id: Json, code: i32, message: &str) -> String {
    Json::obj()
        .set("jsonrpc", "2.0")
        .set("id", id)
        .set(
            "error",
            Json::obj()
                .set("code", code)
                .set("message", message.to_string()),
        )
        .to_string()
}

/// 工具成功：MCP 要求 `content` 数组。
fn tool_ok(value: &Json) -> Json {
    Json::obj()
        .set(
            "content",
            Json::Arr(vec![Json::obj()
                .set("type", "text")
                .set("text", value.to_string())]),
        )
        .set("isError", false)
}

/// 工具失败：`isError: true` + 人类可读消息 + 机器可读 code。
fn tool_err(err: &ApiError) -> Json {
    Json::obj()
        .set(
            "content",
            Json::Arr(vec![Json::obj()
                .set("type", "text")
                .set("text", human_message(err))]),
        )
        .set("isError", true)
        .set("structuredContent", err.to_json())
}

fn human_message(err: &ApiError) -> String {
    match err {
        ApiError::Unauthorized => "unauthorized: missing or invalid token".to_string(),
        // 刻意与 NotFound 合并：否则 AI 可用 403/404 的差异探测节点是否存在
        ApiError::Forbidden { .. } | ApiError::NotFound { .. } => {
            "not found: no such node".to_string()
        }
        ApiError::Conflict { expected, actual } => format!(
            "conflict: state changed (expected rev {expected}, actual {actual}); re-read the tree and retry"
        ),
        ApiError::BadRequest(msg) => format!("bad request: {msg}"),
    }
}

// ---------- stdio 循环 ----------

/// 在 stdin/stdout 上跑 MCP 循环。
///
/// `exec` 负责把一行输入变成一行输出（通常转发给 UI 线程执行）。
pub fn run_stdio<F>(mut exec: F) -> std::io::Result<()>
where
    F: FnMut(&str) -> Option<String>,
{
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if let Some(response) = exec(&line) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::{Flex, FlexItem, Semantics, Size, TextStyle, TextMetrics, UiTree, Widget};

    struct Mono;
    impl TextMetrics for Mono {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.len() as f32 * 8.0, 16.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            20.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    struct Probe {
        id: String,
        exposed: bool,
        sensitive: bool,
        rect: y901_ui::Rect,
    }
    impl Widget for Probe {
        fn id(&self) -> &str {
            &self.id
        }
        fn measure(&mut self, _m: &dyn TextMetrics, c: y901_ui::Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
            c.constrain(Size::new(100.0, 40.0))
        }
        fn arrange(&mut self, r: y901_ui::Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
            self.rect = r;
        }
        fn rect(&self) -> y901_ui::Rect {
            self.rect
        }
        fn flex(&self) -> FlexItem {
            FlexItem::fill()
        }
        fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
        fn event(&mut self, _c: &mut y901_ui::EventCtx, _e: &y901_ui::UiEvent) -> y901_ui::EventResult {
            y901_ui::EventResult::Handled
        }
        fn semantics(&self) -> Semantics {
            Semantics::default()
                .label(&self.id)
                .value(if self.sensitive { "secret" } else { "public" })
                .exposed(self.exposed)
                .sensitive(self.sensitive)
        }
    }

    fn tree() -> UiTree {
        let root = Flex::column("root")
            .push(Box::new(Probe {
                id: "ok".into(),
                exposed: true,
                sensitive: false,
                rect: y901_ui::Rect::ZERO,
            }))
            .push(Box::new(Probe {
                id: "hidden".into(),
                exposed: false,
                sensitive: false,
                rect: y901_ui::Rect::ZERO,
            }))
            .push(Box::new(Probe {
                id: "vault".into(),
                exposed: true,
                sensitive: true,
                rect: y901_ui::Rect::ZERO,
            }));
        let mut t = UiTree::new(Box::new(root));
        t.layout(Size::new(400.0, 300.0), &Mono);
        t
    }

    /// 已握手的 server（除握手相关用例外，其余测试直接用它）。
    fn ready() -> McpServer {
        let mut s = McpServer::new(Policy::default());
        s.state = State::Initialized;
        s
    }

    fn call(server: &mut McpServer, tree: &mut UiTree, msg: &str) -> Json {
        Json::parse(&server.handle_line(tree, msg).expect("response")).expect("json")
    }

    #[test]
    fn initialize_returns_capabilities() {
        let mut s = ready();
        let mut t = tree();
        let r = call(&mut s, &mut t, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
        assert_eq!(r.get("jsonrpc").and_then(|v| v.as_str()), Some("2.0"));
        let result = r.get("result").expect("result");
        assert_eq!(
            result.get("protocolVersion").and_then(|v| v.as_str()),
            Some(PROTOCOL_VERSION)
        );
        assert!(result.get("capabilities").unwrap().get("tools").is_some());
        assert_eq!(
            result
                .get("serverInfo")
                .unwrap()
                .get("name")
                .and_then(|v| v.as_str()),
            Some("y901")
        );
    }

    #[test]
    fn initialized_notification_has_no_response() {
        let mut s = ready();
        let mut t = tree();
        let out = s.handle_line(&mut t, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        assert!(out.is_none(), "通知不应有响应");
        assert_eq!(s.state, State::Initialized);
    }

    #[test]
    fn tools_list_exposes_four_tools_with_schema() {
        let mut s = ready();
        let mut t = tree();
        let r = call(&mut s, &mut t, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = match r.get("result").unwrap().get("tools") {
            Some(Json::Arr(a)) => a.clone(),
            _ => panic!("tools 应为数组"),
        };
        assert_eq!(tools.len(), 5);
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(names, vec!["ui_click", "ui_type_text", "ui_scroll", "ui_query", "ui_invoke"]);

        let click = &tools[0].get("inputSchema").unwrap();
        assert_eq!(click.get("type").and_then(|v| v.as_str()), Some("object"));
        assert!(click.get("properties").unwrap().get("id").is_some());
    }

    #[test]
    fn tools_call_click_succeeds() {
        let mut s = ready();
        let mut t = tree();
        let r = call(
            &mut s,
            &mut t,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"ok"}}}"#,
        );
        let result = r.get("result").expect("result");
        assert_eq!(result.get("isError").and_then(|v| v.as_bool()), Some(false));
        let content = match result.get("content") {
            Some(Json::Arr(a)) => a.clone(),
            _ => panic!("content 应为数组"),
        };
        assert_eq!(content[0].get("type").and_then(|v| v.as_str()), Some("text"));
    }

    #[test]
    fn tools_call_on_hidden_node_is_error_without_leaking() {
        let mut s = ready();
        let mut t = tree();
        let r = call(
            &mut s,
            &mut t,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ui_click","arguments":{"id":"hidden"}}}"#,
        );
        let result = r.get("result").expect("result");
        assert_eq!(result.get("isError").and_then(|v| v.as_bool()), Some(true));
        let text = result.to_string();
        assert!(text.contains("forbidden"));
        assert!(!text.contains("\"hidden\""), "不应回显被隐藏节点的 id");
    }

    #[test]
    fn resources_read_returns_redacted_tree() {
        let mut s = ready();
        let mut t = tree();
        let r = call(
            &mut s,
            &mut t,
            r#"{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"uri":"ui://tree"}}"#,
        );
        let contents = match r.get("result").unwrap().get("contents") {
            Some(Json::Arr(a)) => a.clone(),
            _ => panic!("contents"),
        };
        let text = contents[0]
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        assert!(text.contains("\"ok\""), "可见节点应出现");
        assert!(!text.contains("\"hidden\""), "隐藏节点必须被过滤");
        assert!(!text.contains("secret"), "敏感值必须被脱敏");
        assert!(text.contains("***"));
    }

    #[test]
    fn unknown_method_returns_jsonrpc_error() {
        let mut s = ready();
        let mut t = tree();
        let r = call(&mut s, &mut t, r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#);
        let err = r.get("error").expect("error");
        assert_eq!(err.get("code").and_then(|v| v.as_f64()), Some(-32601.0));
        assert_eq!(r.get("id").and_then(|v| v.as_f64()), Some(9.0));
    }

    #[test]
    fn parse_error_is_reported() {
        let mut s = ready();
        let mut t = tree();
        let r = call(&mut s, &mut t, "{not json");
        assert_eq!(
            r.get("error").unwrap().get("code").and_then(|v| v.as_f64()),
            Some(-32700.0)
        );
    }
}
