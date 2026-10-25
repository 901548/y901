//! 语义 API：AI 只发意图（点谁、输入什么），坐标与事件由网关伪造。
//!
//! 四道闸（顺序即优先级）：
//! 1. **能力作用域**：节点必须 `exposed_to_ai`，否则 403；
//! 2. **版本锁**：带 `if_rev` 时必须与当前 `revision` 一致，否则 409；
//! 3. **动作合法性**：`invoke` 的动作必须在 `semantics().actions` 里；
//! 4. **敏感脱敏**：`sensitive` 节点在返回的 JSON 里 value 一律 `***`。

use y901_ui::{Json, MouseButton, Pos, Rect, UiEvent, UiTree};

/// 网关能执行的全部语义动作。
#[derive(Clone, Debug, PartialEq)]
pub enum ApiCall {
    /// 列出（过滤 + 脱敏后的）语义树。
    Tree,
    /// 按查询语言筛选，返回精简节点列表（id/role/label/rect/revision）。
    Query { q: String },
    /// 点击节点：网关自动换算中心点并伪造 MouseDown + MouseUp。
    Click { id: String, if_rev: Option<u64> },
    /// 向节点输入文本（等价于聚焦后键入）。
    Type {
        id: String,
        text: String,
        if_rev: Option<u64>,
    },
    /// 向节点滚动（正数向下）。
    Scroll {
        id: String,
        delta: f32,
        if_rev: Option<u64>,
    },
    /// 触发组件声明的动作。
    Invoke {
        id: String,
        action: String,
        args: Json,
        if_rev: Option<u64>,
    },
}

impl ApiCall {
    pub fn id(&self) -> Option<&str> {
        match self {
            ApiCall::Tree | ApiCall::Query { .. } => None,
            ApiCall::Click { id, .. }
            | ApiCall::Type { id, .. }
            | ApiCall::Scroll { id, .. }
            | ApiCall::Invoke { id, .. } => Some(id),
        }
    }

    pub fn if_rev(&self) -> Option<u64> {
        match self {
            ApiCall::Tree | ApiCall::Query { .. } => None,
            ApiCall::Click { if_rev, .. }
            | ApiCall::Type { if_rev, .. }
            | ApiCall::Scroll { if_rev, .. }
            | ApiCall::Invoke { if_rev, .. } => *if_rev,
        }
    }
}

/// 网关返回的错误。`status` 直接映射 HTTP 状态码。
#[derive(Clone, Debug, PartialEq)]
pub enum ApiError {
    /// 401：缺 Token / Token 错
    Unauthorized,
    /// 403：节点存在但未对 AI 开放
    Forbidden { id: String },
    /// 404：节点不存在
    NotFound { id: String },
    /// 409：状态已过期（人或别的 Agent 改过）
    Conflict { expected: u64, actual: u64 },
    /// 400：请求本身不合法（未知动作、参数缺失）
    BadRequest(String),
}

impl ApiError {
    pub fn status(&self) -> u16 {
        match self {
            ApiError::Unauthorized => 401,
            ApiError::Forbidden { .. } => 403,
            ApiError::NotFound { .. } => 404,
            ApiError::Conflict { .. } => 409,
            ApiError::BadRequest(_) => 400,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ApiError::Unauthorized => "unauthorized",
            ApiError::Forbidden { .. } => "forbidden",
            ApiError::NotFound { .. } => "not_found",
            ApiError::Conflict { .. } => "conflict",
            ApiError::BadRequest(_) => "bad_request",
        }
    }

    pub fn to_json(&self) -> Json {
        let mut obj = Json::obj()
            .set("ok", false)
            .set("code", self.code())
            .set("status", self.status());
        match self {
            // Forbidden / NotFound 都不回显 id：避免用错误码探测节点是否存在
            ApiError::Forbidden { .. } | ApiError::NotFound { .. } => {}
            ApiError::BadRequest(msg) => obj = obj.set("message", msg.as_str()),
            ApiError::Conflict { expected, actual } => {
                obj = obj
                    .set("expected_rev", *expected)
                    .set("actual_rev", *actual);
            }
            ApiError::Unauthorized => {}
        }
        obj
    }
}

/// 网关策略：由应用注入，决定"给谁看什么"。
#[derive(Clone, Debug)]
pub struct Policy {
    /// 期望的 Bearer Token。空字符串表示不校验（仅测试用）。
    pub token: String,
    /// 是否把 `sensitive` 节点的 value 打码。
    pub redact: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            token: String::new(),
            redact: true,
        }
    }
}

impl Policy {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            redact: true,
        }
    }

    /// 校验裸 Token（MCP 的 `params.token`）。空配置表示不校验。
    pub fn authorize_token(&self, presented: Option<&str>) -> Result<(), ApiError> {
        if self.token.is_empty() {
            return Ok(());
        }
        match presented {
            Some(t) if t == self.token => Ok(()),
            _ => Err(ApiError::Unauthorized),
        }
    }

    /// 校验 `Authorization` 头的值（形如 `Bearer xxx`）。
    pub fn authorize(&self, header: Option<&str>) -> Result<(), ApiError> {
        if self.token.is_empty() {
            return Ok(()); // 未配置 token：视为本地可信
        }
        let Some(raw) = header else {
            return Err(ApiError::Unauthorized);
        };
        let presented = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "))
            .unwrap_or(raw)
            .trim();
        if presented == self.token {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}

/// 执行一次语义调用。**纯函数**：不碰网络，方便单测。
pub fn handle_call(tree: &mut UiTree, policy: &Policy, call: ApiCall) -> Result<Json, ApiError> {
    match call {
        ApiCall::Tree => Ok(public_tree(tree, policy)),
        ApiCall::Query { q } => {
            let parsed = y901_ui::Query::parse(&q).map_err(ApiError::BadRequest)?;
            Ok(query_result(tree, policy, &parsed))
        }
        ApiCall::Click { id, if_rev } => {
            let (rect, node_rev) = locate(tree, &id)?;
            check_rev(if_rev, node_rev)?;
            let center = Pos::new(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
            let mut results = Vec::new();
            for ev in [
                UiEvent::MouseMove { pos: center },
                UiEvent::MouseDown {
                    pos: center,
                    button: MouseButton::Left,
                },
                UiEvent::MouseUp {
                    pos: center,
                    button: MouseButton::Left,
                },
            ] {
                let r = tree.dispatch_to(&id, &ev);
                results.push(r.is_handled());
            }
            Ok(Json::obj()
                .set("ok", true)
                .set("action", "click")
                .set("id", id.clone())
                .set("handled", results.iter().any(|v| *v))
                .set("rev", revision_of(tree, &id)))
        }
        ApiCall::Type { id, text, if_rev } => {
            let (_, node_rev) = locate(tree, &id)?;
            check_rev(if_rev, node_rev)?;
            let handled = tree
                .dispatch_to(&id, &UiEvent::Text(text.clone()))
                .is_handled();
            Ok(Json::obj()
                .set("ok", true)
                .set("action", "type")
                .set("id", id.clone())
                .set("handled", handled)
                .set("rev", revision_of(tree, &id)))
        }
        ApiCall::Scroll { id, delta, if_rev } => {
            let (rect, node_rev) = locate(tree, &id)?;
            check_rev(if_rev, node_rev)?;
            let pos = Pos::new(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5);
            let handled = tree
                .dispatch_to(&id, &UiEvent::Wheel { pos, delta })
                .is_handled();
            Ok(Json::obj()
                .set("ok", true)
                .set("action", "scroll")
                .set("id", id.clone())
                .set("handled", handled)
                .set("rev", revision_of(tree, &id)))
        }
        ApiCall::Invoke {
            id,
            action,
            args,
            if_rev,
        } => {
            let (_, node_rev) = locate(tree, &id)?;
            check_rev(if_rev, node_rev)?;
            // 动作必须在组件声明的列表里，避免"猜 API"
            let allowed = tree
                .find(&id)
                .map(|n| n.semantics().actions.iter().any(|a| a.name == action))
                .unwrap_or(false);
            if !allowed {
                return Err(ApiError::BadRequest(format!(
                    "action `{action}` is not declared by `{id}`"
                )));
            }
            // 参数类型必须与 ActionSpec.input 匹配
            if let Some(node) = tree.find(&id) {
                if let Some(spec) = node.semantics().actions.iter().find(|a| a.name == action) {
                    let value = args.get(spec.input.arg_name()).cloned().unwrap_or(Json::Null);
                    // 无参动作不接受任何多余参数
                    let extra = matches!(spec.input, y901_ui::InputKind::None)
                        && matches!(&args, Json::Obj(map) if !map.is_empty());
                    if extra || !spec.input.accepts(&value) {
                        return Err(ApiError::BadRequest(format!(
                            "action `{action}` expects {:?} argument `{}`",
                            spec.input,
                            spec.input.arg_name()
                        )));
                    }
                }
            }
            let handled = tree
                .dispatch_to(&id, &UiEvent::Action { name: action.clone(), args })
                .is_handled();
            // 声明了却没处理 = 契约违约，不能返回成功让 Agent 以为生效了
            if !handled {
                return Err(ApiError::BadRequest(format!(
                    "action `{action}` not handled by `{id}`"
                )));
            }
            Ok(Json::obj()
                .set("ok", true)
                .set("action", action)
                .set("id", id.clone())
                .set("handled", handled)
                .set("rev", revision_of(tree, &id)))
        }
    }
}

/// 取节点可见区与 revision，并做能力作用域检查。
fn locate(tree: &UiTree, id: &str) -> Result<(Rect, u64), ApiError> {
    let Some(node) = tree.find(id) else {
        return Err(ApiError::NotFound {
            id: id.to_string(),
        });
    };
    let sem = node.semantics();
    if !sem.exposed_to_ai {
        return Err(ApiError::Forbidden {
            id: id.to_string(),
        });
    }
    let rect = tree.rect_of(id).unwrap_or_else(|| node.rect());
    if rect.is_empty() {
        // 还没布局：拒绝而不是点 (0,0)
        return Err(ApiError::BadRequest(format!("node `{id}` has no geometry")));
    }
    // P0-1：必须返回**有效** revision（组件自报 + 框架计数器），
    // 与 get_tree / query / node_resource 的输出保持一致，否则 if_rev 恒 409。
    Ok((rect, tree.effective_revision(id)))
}

/// 节点对 Agent 的有效 revision（组件自身 + 框架计数器）。
///
/// 先 `refresh_semantics()`：动作刚发生时语义面还没被下一帧 layout 检查过，
/// 这里补一次，保证返回给 Agent 的 rev 是**动作之后**的值。
fn revision_of(tree: &mut UiTree, id: &str) -> u64 {
    tree.refresh_semantics();
    tree.effective_revision(id)
}

fn check_rev(expected: Option<u64>, actual: u64) -> Result<(), ApiError> {
    match expected {
        Some(want) if want != actual => Err(ApiError::Conflict {
            expected: want,
            actual,
        }),
        _ => Ok(()),
    }
}

/// 给 AI 看的树：过滤作用域 + 脱敏。
fn public_tree(tree: &UiTree, policy: &Policy) -> Json {
    fn walk(node: &dyn y901_ui::Widget, tree: &UiTree, policy: &Policy) -> Option<Json> {
        let sem = node.semantics();
        if !sem.exposed_to_ai {
            return None;
        }
        let rect = node.rect();
        let merged = redact_node(&sem, policy);
        let children: Vec<Json> = node
            .children()
            .iter()
            .filter_map(|c| walk(c.as_ref(), tree, policy))
            .collect();
        Some(
            Json::obj()
                .set("id", node.id())
                .set("role", node.role().as_str())
                .set(
                    "rect",
                    Json::obj()
                        .set("x", rect.x)
                        .set("y", rect.y)
                        .set("w", rect.w)
                        .set("h", rect.h),
                )
                .set("revision", tree.effective_revision(node.id()))
                // 顶层 focused：框架拥有（来自 focus_path），组件不得设置
                .set("focused", tree.focus() == Some(node.id()))
                .set("sensitive", merged.get("sensitive").cloned().unwrap_or(Json::Bool(false)))
                // disabled 是 Agent 的可见语义（"现在点不了"），必须输出；
                // 后续 revision 的哈希范围与这里保持一一对应。
                .set("disabled", merged.get("disabled").cloned().unwrap_or(Json::Bool(false)))
                .set("label", merged.get("label").cloned().unwrap_or(Json::Null))
                .set("value", merged.get("value").cloned().unwrap_or(Json::Null))
                .set("actions", Json::Arr(
                    sem.actions.iter().map(|a| a.to_json()).collect()
                ))
                .set("state", merged.get("state").cloned().unwrap_or(Json::Null))
                .set("children", Json::Arr(children)),
        )
    }

    walk(tree.root(), tree, policy).unwrap_or_else(Json::obj)
}

/// 统一的节点脱敏：把 `Semantics` 转成给 AI 看的 JSON。
///
/// `public_tree` 和 `resources/read ui://node/{id}` 都走这里，
/// 避免两套逻辑漂移（曾经单节点路径漏掉了 state 脱敏）。
pub fn redact_node(sem: &y901_ui::Semantics, policy: &Policy) -> Json {
    let sensitive = policy.redact && sem.sensitive;
    let value = if sensitive {
        "***".to_string()
    } else {
        sem.value.clone()
    };
    // 敏感节点：state 的每个键都打码，避免从结构化字段反推内容
    let state = if sensitive {
        let mut masked = Json::obj();
        if let Json::Obj(map) = &sem.state {
            for (k, _) in map {
                masked = masked.set(k.as_str(), "***");
            }
        }
        masked
    } else {
        sem.state.clone()
    };
    let label = if sensitive {
        "***".to_string()
    } else {
        sem.label.clone()
    };
    Json::obj()
        .set("label", label)
        .set("value", value)
        .set(
            "actions",
            Json::Arr(sem.actions.iter().map(|a| a.to_json()).collect()),
        )
        .set("state", state)
        .set("sensitive", sem.sensitive)
        .set("disabled", sem.disabled)
}

/// 查询结果：只返回定位所需字段，避免整树 JSON。
fn query_result(tree: &UiTree, policy: &Policy, q: &y901_ui::Query) -> Json {
    let ids = y901_ui::query_nodes(tree.root(), q);
    let mut items = Vec::new();
    for id in ids {
        let Some(node) = tree.find(&id) else { continue };
        let sem = node.semantics();
        if !sem.exposed_to_ai {
            continue;
        }
        let rect = tree.rect_of(&id).unwrap_or_else(|| node.rect());
        let redacted = redact_node(&sem, policy);
        items.push(
            Json::obj()
                .set("id", id)
                .set("role", node.role().as_str())
                .set("label", redacted.get("label").cloned().unwrap_or(Json::Null))
                .set("revision", tree.effective_revision(node.id()))
                // 与 public_tree 对齐：disabled / focused 都属于 Agent 可操作性的输入
                .set("disabled", redacted.get("disabled").cloned().unwrap_or(Json::Bool(false)))
                .set("focused", tree.focus() == Some(node.id()))
                .set(
                    "rect",
                    Json::obj()
                        .set("x", rect.x)
                        .set("y", rect.y)
                        .set("w", rect.w)
                        .set("h", rect.h),
                ),
        );
    }
    Json::obj()
        .set("count", items.len() as u32)
        .set("items", Json::Arr(items))
}

/// **统一入口**：RPC 与 MCP 都走这里。
///
/// 方法名同时接受短名（`query`）与工具名（`ui_query`），
/// 这样 TCP 协议与 MCP 工具表可以各自命名，而执行逻辑只有一份。
pub fn dispatch_request(
    tree: &mut UiTree,
    policy: &Policy,
    method: &str,
    params: &Json,
) -> Result<Json, ApiError> {
    let call = match method {
        "get_tree" | "tree" | "ui_tree" => ApiCall::Tree,
        "query" | "ui_query" => ApiCall::Query {
            q: arg_str(params, "q")?,
        },
        "click" | "ui_click" => ApiCall::Click {
            id: arg_str(params, "id")?,
            if_rev: arg_u64(params, "if_rev"),
        },
        "type" | "ui_type_text" => ApiCall::Type {
            id: arg_str(params, "id")?,
            text: arg_str(params, "text")?,
            if_rev: arg_u64(params, "if_rev"),
        },
        "scroll" | "ui_scroll" => ApiCall::Scroll {
            id: arg_str(params, "id")?,
            delta: params.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
            if_rev: arg_u64(params, "if_rev"),
        },
        // 原生事件直投（RPC 的 dispatch 与 MCP 的底层入口共用）
        "dispatch" => {
            let id = arg_str(params, "id")?;
            let raw = params.get("event").cloned().unwrap_or(Json::Null);
            let ev = crate::rpc::RpcEvent::from_json(&raw)
                .map_err(|e| ApiError::BadRequest(format!("bad event: {e}")))?;
            // P0-2：dispatch 也必须过安全闸（能力 + 版本 + 几何），
            // 否则 Agent 可以绕过 exposed/if_rev 向任意节点注入事件。
            let (_rect, rev) = locate(tree, &id)?;
            check_rev(arg_u64(params, "if_rev"), rev)?;
            let handled = tree.dispatch_to(&id, &ev.to_ui_event()).is_handled();
            return Ok(Json::obj().set("ok", handled).set("id", id));
        }
        // 焦点 / 订阅：同样是网关职责，host 只做转发（P0-2 统一入口）。
        "focus" => {
            let id = arg_str(params, "id")?;
            // 能力闸 + 几何（locate 顺带拒绝未布局节点）
            let _ = locate(tree, &id)?;
            if tree.set_focus(Some(id.clone())) {
                return Ok(Json::obj().set("ok", true).set("id", id));
            }
            // set_focus 返回 false = 被 veto 或节点不可聚焦
            return Err(ApiError::BadRequest(format!("focus vetoed: `{id}`")));
        }
        "subscribe" => {
            let id = arg_str(params, "id")?;
            locate(tree, &id)?;
            // interval 校验：负数/非数值 → 默认；超过 1 小时 → 400（防止误传毫秒/秒混淆）
            let interval_ms = match params.get("interval_ms").and_then(|v| v.as_f64()) {
                None => 100.0,
                Some(v) if !(0.0..=3_600_000.0).contains(&v) => {
                    return Err(ApiError::BadRequest(format!(
                        "interval_ms 必须在 [0, 3600000]，实际 {v}"
                    )))
                }
                Some(v) => v,
            };
            if !tree.subscribe(&id, std::time::Duration::from_millis(interval_ms as u64)) {
                return Err(ApiError::NotFound { id });
            }
            return Ok(Json::obj().set("subscribed", id));
        }
        "unsubscribe" => {
            // 取消订阅不涉及信息泄露，**不过能力闸**（否则 Agent 无法清理已隐藏节点的订阅）
            let id = arg_str(params, "id")?;
            tree.unsubscribe(&id);
            return Ok(Json::obj().set("unsubscribed", id));
        }
        "invoke" | "ui_invoke" => ApiCall::Invoke {
            id: arg_str(params, "id")?,
            action: arg_str(params, "action")?,
            args: params.get("args").cloned().unwrap_or_else(Json::obj),
            if_rev: arg_u64(params, "if_rev"),
        },
        other => {
            return Err(ApiError::BadRequest(format!("unknown method: {other}")));
        }
    };
    handle_call(tree, policy, call)
}

fn arg_str(params: &Json, key: &str) -> Result<String, ApiError> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ApiError::BadRequest(format!("missing argument: {key}")))
}

fn arg_u64(params: &Json, key: &str) -> Option<u64> {
    params.get(key).and_then(|v| v.as_f64()).map(|f| f as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::{Constraints, EventCtx, EventResult, FlexItem, RenderCtx, Semantics, Size, TextStyle, TextMetrics, Widget};

    struct Mono;
    impl TextMetrics for Mono {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.len() as f32 * 8.0, 16.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            16.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    /// 记录收到的事件，可配置 exposed / sensitive。
    struct Probe {
        id: String,
        exposed: bool,
        sensitive: bool,
        rect: Rect,
        got: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
        rev: u64,
        children: Vec<Box<dyn Widget>>,
    }

    impl Widget for Probe {
        fn id(&self) -> &str {
            &self.id
        }
        fn flex(&self) -> FlexItem {
            FlexItem::fill()
        }
        fn children(&self) -> &[Box<dyn Widget>] {
            &self.children
        }
        fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
            Some(&mut self.children)
        }
        fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
            for k in self.children.iter_mut() {
                let _ = k;
            }
            c.constrain(Size::new(100.0, 40.0))
        }
        fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
            self.rect = r;
            for k in self.children.iter_mut() {
                k.arrange(r, m, cache);
            }
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn revision(&self) -> u64 {
            self.rev
        }
        fn event(&mut self, _ctx: &mut EventCtx, ev: &UiEvent) -> EventResult {
            let tag = match ev {
                UiEvent::MouseDown { .. } => "mouse_down",
                UiEvent::MouseUp { .. } => "mouse_up",
                UiEvent::Text(_) => "text",
                UiEvent::Wheel { .. } => "wheel",
                UiEvent::Action { .. } => "action",
                _ => "other",
            };
            self.got.borrow_mut().push(tag);
            EventResult::Handled
        }
        fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
                .label(&self.id)
                .value("secret-value")
                .action("activate")
                .state("n", 1u32)
                .exposed(self.exposed)
                .sensitive(self.sensitive)
        }
    }

    fn tree_with(exposed: bool, sensitive: bool, rev: u64) -> (UiTree, std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>) {
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let probe = Probe {
            id: "btn".into(),
            exposed,
            sensitive,
            rect: Rect::ZERO,
            got: got.clone(),
            rev,
            children: Vec::new(),
        };
        let root = y901_ui::Flex::column("root").push(Box::new(probe));
        let mut tree = UiTree::new(Box::new(root));
        tree.layout(Size::new(200.0, 100.0), &Mono);
        (tree, got)
    }

    #[test]
    fn click_dispatches_mouse_down_and_up() {
        let (mut tree, got) = tree_with(true, false, 7);
        let policy = Policy::default();
        let r = handle_call(&mut tree, &policy, ApiCall::Click { id: "btn".into(), if_rev: None });
        assert!(r.is_ok());
        let events = got.borrow().clone();
        assert!(events.contains(&"mouse_down"), "应伪造 MouseDown: {events:?}");
        assert!(events.contains(&"mouse_up"), "应伪造 MouseUp: {events:?}");
    }

    #[test]
    fn type_and_scroll_are_translated() {
        let (mut tree, got) = tree_with(true, false, 7);
        let policy = Policy::default();
        handle_call(&mut tree, &policy, ApiCall::Type { id: "btn".into(), text: "hi".into(), if_rev: None }).unwrap();
        handle_call(&mut tree, &policy, ApiCall::Scroll { id: "btn".into(), delta: -120.0, if_rev: None }).unwrap();
        let events = got.borrow().clone();
        assert!(events.contains(&"text"));
        assert!(events.contains(&"wheel"));
    }

    #[test]
    fn unexposed_node_is_forbidden() {
        let (mut tree, got) = tree_with(false, false, 1);
        let err = handle_call(&mut tree, &Policy::default(), ApiCall::Click { id: "btn".into(), if_rev: None })
            .unwrap_err();
        assert_eq!(err, ApiError::Forbidden { id: "btn".into() });
        assert_eq!(err.status(), 403);
        assert!(got.borrow().is_empty(), "被拒绝时不得派发任何事件");
    }

    #[test]
    fn stale_revision_is_conflict() {
        let (mut tree, got) = tree_with(true, false, 9);
        let err = handle_call(
            &mut tree,
            &Policy::default(),
            ApiCall::Click { id: "btn".into(), if_rev: Some(3) },
        )
        .unwrap_err();
        assert_eq!(err, ApiError::Conflict { expected: 3, actual: 9 });
        assert_eq!(err.status(), 409);
        assert!(got.borrow().is_empty());

        // 正确的 revision 放行
        assert!(handle_call(
            &mut tree,
            &Policy::default(),
            ApiCall::Click { id: "btn".into(), if_rev: Some(9) }
        )
        .is_ok());
    }

    #[test]
    fn unknown_action_is_rejected() {
        let (mut tree, _) = tree_with(true, false, 1);
        let err = handle_call(
            &mut tree,
            &Policy::default(),
            ApiCall::Invoke {
                id: "btn".into(),
                action: "explode".into(),
                args: Json::obj(),
                if_rev: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn sensitive_value_is_redacted_in_tree() {
        let (tree, _) = tree_with(true, true, 1);
        let view = public_tree(&tree, &Policy { token: String::new(), redact: true });
        let text = view.to_string();
        assert!(text.contains("***"), "敏感值必须打码：{text}");
        assert!(!text.contains("secret-value"), "原始值不得泄漏");
    }

    #[test]
    fn token_is_checked() {
        let policy = Policy::with_token("s3cret");
        assert_eq!(policy.authorize(None), Err(ApiError::Unauthorized));
        assert_eq!(policy.authorize(Some("Bearer nope")), Err(ApiError::Unauthorized));
        assert!(policy.authorize(Some("Bearer s3cret")).is_ok());
        // 未配置 token 时不校验
        assert!(Policy::default().authorize(None).is_ok());
    }
}
