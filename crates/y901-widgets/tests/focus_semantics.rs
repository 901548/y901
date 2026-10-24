//! `focused` 的语义：框架拥有、顶层可见、跨接口一致、可订阅。
//!
//! 设计决策（见 FRAMEWORK_GUIDE §14）：
//! - 顶层 `focused` 由 `UiTree.focus_path` 注入，组件不得设置；
//! - 组件的内部 `focused` 字段只用于渲染与按键门控，**不进 `state`**；
//! - 订阅 patch 需要覆盖 value / state / disabled / focused / label。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use y901_agent::{handle_call, ApiCall, McpServer, Policy};
use y901_ui::{
    Command, Constraints, EventCtx, EventResult, Flex, FlexItem, Json, Pos, Rect, RenderCtx,
    Semantics, Size, TextStyle, TextMetrics, UiEvent, UiTree, Widget,
};
use y901_widgets::{Button, Dialog, TerminalWidget, TextInput};

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

fn find_node<'a>(node: &'a Json, id: &str) -> Option<&'a Json> {
    if node.get("id").and_then(|v| v.as_str()) == Some(id) {
        return Some(node);
    }
    match node.get("children") {
        Some(Json::Arr(k)) => k.iter().find_map(|c| find_node(c, id)),
        _ => None,
    }
}

fn get_field(node: &Json, key: &str) -> Option<Json> {
    node.get(key).cloned()
}

fn tree_view(tree: &mut UiTree) -> Json {
    handle_call(tree, &Policy::default(), ApiCall::Tree).expect("tree")
}

// ---------- 1. 顶层 focused 反映框架焦点 ----------

#[test]
fn top_level_focused_reflects_framework_focus() {
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
    assert!(tree.set_focus(Some("btn".to_string())));

    let view = tree_view(&mut tree);
    let btn = find_node(&view, "btn").expect("btn");
    let field_node = find_node(&view, "field").expect("field");

    assert_eq!(
        get_field(btn, "focused"),
        Some(Json::Bool(true)),
        "焦点在 btn 上时，btn 的顶层 focused 必须为 true：{}",
        btn.to_string()
    );
    assert_eq!(
        get_field(field_node, "focused"),
        Some(Json::Bool(false)),
        "非焦点节点必须是 false"
    );

    // 切走后 btn 应变 false
    assert!(tree.set_focus(Some("field".to_string())));
    let view = tree_view(&mut tree);
    assert_eq!(
        get_field(find_node(&view, "btn").unwrap(), "focused"),
        Some(Json::Bool(false))
    );
    assert_eq!(
        get_field(find_node(&view, "field").unwrap(), "focused"),
        Some(Json::Bool(true))
    );
}

// ---------- 2. 三接口一致 ----------

#[test]
fn focused_consistent_across_three_interfaces() {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(
            TextInput::new("field").flex_item(FlexItem::height(24.0)),
        )),
    ));
    tree.layout(Size::new(300.0, 100.0), &Mono);
    assert!(tree.set_focus(Some("field".to_string())));

    // get_tree
    let view = tree_view(&mut tree);
    let from_tree = get_field(find_node(&view, "field").unwrap(), "focused");

    // query
    let q = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query {
            q: "id=field".into(),
        },
    )
    .expect("query");
    let items = match q.get("items") {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("items"),
    };
    let from_query = get_field(&items[0], "focused");

    // MCP 单节点资源
    let mut server = McpServer::new(Policy::default());
    server.state = y901_agent::mcp::State::Initialized;
    let raw = server
        .handle_line(
            &mut tree,
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"ui://node/field"}}"#,
        )
        .expect("response");
    let parsed = Json::parse(&raw).unwrap();
    let contents = match parsed.get("result").and_then(|r| r.get("contents")) {
        Some(Json::Arr(a)) => a.clone(),
        _ => panic!("contents"),
    };
    let inner = Json::parse(
        contents[0]
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
    )
    .unwrap();
    let from_resource = get_field(&inner, "focused");

    assert_eq!(from_tree, Some(Json::Bool(true)), "get_tree 应带顶层 focused");
    assert_eq!(
        from_tree, from_query,
        "get_tree 与 query 的 focused 必须一致"
    );
    assert_eq!(
        from_query, from_resource,
        "query 与 node_resource 的 focused 必须一致"
    );
}

// ---------- 3. state.focused 已移除 ----------

#[test]
fn state_focused_is_removed() {
    let input = TextInput::new("field");
    let sem = input.semantics();
    if let Json::Obj(map) = &sem.state {
        assert!(
            !map.iter().any(|(k, _)| k == "focused"),
            "TextInput 的 state 不应再有 focused（应由框架注入顶层）：{:?}",
            map.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
        );
    }
}

// ---------- 4. Terminal 也有顶层 focused ----------

#[test]
fn terminal_also_has_top_level_focused() {
    let term = TerminalWidget::new("term", 40, 10).flex_item(FlexItem::height(60.0));
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(term))));
    tree.layout(Size::new(400.0, 200.0), &Mono);
    assert!(tree.set_focus(Some("term".to_string())));

    let view = tree_view(&mut tree);
    let node = find_node(&view, "term").expect("term");
    assert_eq!(
        get_field(node, "focused"),
        Some(Json::Bool(true)),
        "Terminal 也应获得顶层 focused：{}",
        node.to_string()
    );
}

// ---------- 5. 焦点请求被拒时组件不分叉 ----------

#[test]
fn text_input_click_only_sets_focused_after_focus_event() {
    // Modal 打开后焦点被限制在 Modal 子树内：外部 TextInput 的 request_focus 会被拒
    let mut dlg = Dialog::new("dlg", "T").push(Box::new(
        Button::new("inner", "OK").flex_item(FlexItem::height(24.0)),
    ));
    dlg.open();

    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(dlg))
            .push(Box::new(
                TextInput::new("field").flex_item(FlexItem::height(24.0)),
            )),
    ));
    tree.layout(Size::new(300.0, 200.0), &Mono);

    // Modal 自动聚焦到 inner
    assert_eq!(tree.focus(), Some("inner"), "前置：焦点应被 Modal 捕获");

    // 直接向 Modal 外的 TextInput 投递点击（绕过命中测试，模拟"点在不可达处"）
    let rect = tree.rect_of("field").expect("field rect");
    tree.dispatch_to(
        "field",
        &UiEvent::MouseDown {
            pos: Pos::new(rect.x + 5.0, rect.y + 5.0),
            button: y901_ui::MouseButton::Left,
        },
    );

    // 焦点请求被 Modal 作用域拒绝：
    assert_eq!(tree.focus(), Some("inner"), "焦点不应被 Modal 外的节点抢走");
    let input = tree.find("field").expect("field");
    // TextInput 暴露内部状态用于断言
    let focused = input.semantics().props.iter().any(|(k, v)| k == "focused" && v == "true");
    assert!(
        !focused,
        "request_focus 被拒后，组件的内部 focused 必须仍是 false（不能与框架分叉）"
    );
}

// ---------- 6/7/8. 订阅 patch 覆盖 focused / disabled / label ----------

/// 可手动改 label / disabled 的探针。
struct Probe {
    id: String,
    label: String,
    disabled: bool,
    rect: Rect,
    log: Rc<RefCell<Vec<String>>>,
}

impl Widget for Probe {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 24.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::Action { name, args } if name == "set_label" => {
                self.label = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                self.log.borrow_mut().push("label".into());
                EventResult::Redraw
            }
            UiEvent::Action { name, args } if name == "set_disabled" => {
                self.disabled = args
                    .get("flag")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                self.log.borrow_mut().push("disabled".into());
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.label.clone())
            .disabled(self.disabled)
            .action_spec("set_label", y901_ui::InputKind::Text)
            .action_spec("set_disabled", y901_ui::InputKind::Bool)
    }
}

fn probe_tree() -> (UiTree, Rc<RefCell<Vec<String>>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let probe = Probe {
        id: "p".into(),
        label: "A".into(),
        disabled: false,
        rect: Rect::ZERO,
        log: log.clone(),
    };
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(probe))));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    (tree, log)
}

fn patches_after(tree: &mut UiTree, f: impl FnOnce(&mut UiTree)) -> Vec<Json> {
    assert!(tree.subscribe("p", Duration::from_millis(0)));
    // 建立基线（订阅时已记录），再触发变化
    f(tree);
    tree.layout(Size::new(200.0, 100.0), &Mono);
    tree.poll_subscriptions(Instant::now())
}

#[test]
fn subscribe_patches_label_change() {
    let (mut tree, _log) = probe_tree();
    let patches = patches_after(&mut tree, |t| {
        t.dispatch_to(
            "p",
            &UiEvent::Action {
                name: "set_label".into(),
                args: Json::obj().set("text", "B".to_string()),
            },
        );
    });
    assert_eq!(patches.len(), 1, "应产生一条推送，实际 {patches:?}");
    let patch = patches[0].get("patch").expect("patch");
    assert_eq!(
        patch.get("label").and_then(|v| v.as_str()),
        Some("B"),
        "patch 必须包含 label：{}",
        patch.to_string()
    );
}

#[test]
fn subscribe_patches_disabled_change() {
    let (mut tree, _log) = probe_tree();
    let patches = patches_after(&mut tree, |t| {
        t.dispatch_to(
            "p",
            &UiEvent::Action {
                name: "set_disabled".into(),
                args: Json::obj().set("flag", true),
            },
        );
    });
    assert_eq!(patches.len(), 1, "应产生一条推送，实际 {patches:?}");
    let patch = patches[0].get("patch").expect("patch");
    assert_eq!(
        patch.get("disabled"),
        Some(&Json::Bool(true)),
        "patch 必须包含 disabled：{}",
        patch.to_string()
    );
}

#[test]
fn subscribe_patches_focused_change() {
    let (mut tree, _log) = probe_tree();
    let patches = patches_after(&mut tree, |t| {
        assert!(t.set_focus(Some("p".to_string())));
    });
    assert_eq!(patches.len(), 1, "焦点变化应产生一条推送，实际 {patches:?}");
    let patch = patches[0].get("patch").expect("patch");
    assert_eq!(
        patch.get("focused"),
        Some(&Json::Bool(true)),
        "patch 必须包含 focused：{}",
        patch.to_string()
    );
}

/// 防止未使用告警：`Cell`/`Command` 是探针将来扩展要用的。
#[allow(dead_code)]
fn _unused(_: Rc<Cell<usize>>, _: Option<Command>) {}
