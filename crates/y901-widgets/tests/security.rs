//! 安全与契约回归：默认封禁、Token、脱敏 label、关闭浮层、ScrollView 子节点。

use std::cell::RefCell;
use std::rc::Rc;

use y901_agent::{McpServer, Policy};
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, FlexItem, Json, Pos, Rect, RenderCtx, Size,
    TextStyle, TextMetrics, UiEvent, UiTree, Widget,
};
use y901_widgets::{Button, Menu, ScrollView, TextLabel};

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

// ---------- 1. 默认封禁 + label 脱敏 ----------

struct Secret {
    rect: Rect,
    exposed: bool,
}

impl Widget for Secret {
    fn id(&self) -> &str {
        "secret"
    }
    fn flex(&self) -> FlexItem {
        FlexItem::fill()
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(Size::new(100.0, 40.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
            .label("top-secret-label")
            .value("top-secret-value")
            .exposed(self.exposed)
            .sensitive(true)
    }
}

#[test]
fn unexposed_node_is_invisible_to_agent_by_default() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(Secret {
        rect: Rect::ZERO,
        exposed: false,
    }))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let server = McpServer::new(Policy::default());
    let view = y901_agent::handle_call(&mut tree, server.policy(), y901_agent::ApiCall::Tree).unwrap();
    let text = view.to_string();
    assert!(!text.contains("secret"), "未暴露节点不得出现在树里：{text}");
}

#[test]
fn sensitive_label_is_redacted_too() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(Secret {
        rect: Rect::ZERO,
        exposed: true,
    }))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let view = y901_agent::handle_call(
        &mut tree,
        &Policy::default(),
        y901_agent::ApiCall::Tree,
    )
    .unwrap();
    let text = view.to_string();
    assert!(!text.contains("top-secret-label"), "label 也必须脱敏：{text}");
    assert!(!text.contains("top-secret-value"), "value 必须脱敏：{text}");
    assert!(text.contains("***"));
}

// ---------- 2. Token ----------

#[test]
fn standard_mcp_initialize_without_token_succeeds() {
    // 标准 MCP 客户端的 initialize 只带 protocolVersion / capabilities / clientInfo，
    // 不带任何 token。stdio 是进程边界，鉴权在启动时由 Y901_TOKEN 决定。
    let policy = Policy::with_token("s3cret");
    let mut server = McpServer::new(policy);
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(Button::new("ok", "OK")))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let out = server
        .handle_line(
            &mut tree,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"claude","version":"1.0"}}}"#,
        )
        .expect("response");
    assert!(
        out.contains("protocolVersion") && out.contains("serverInfo"),
        "标准 initialize 应成功：{out}"
    );

    // 握手后 tools/list 正常
    let tools = server
        .handle_line(&mut tree, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .expect("response");
    assert!(tools.contains("ui_click"), "tools/list 应可用：{tools}");
}

#[test]
fn mcp_rejects_bad_jsonrpc_version() {
    let mut server = McpServer::new(Policy::default());
    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let out = server
        .handle_line(&mut tree, r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#)
        .expect("response");
    assert!(out.contains("-32600"), "版本不对应拒绝：{out}");
}

// ---------- 3. 关闭的浮层不可命中 ----------

#[test]
fn closed_menu_is_not_hittable() {
    let mut menu = Menu::new("menu").items(vec!["A".to_string()]);
    // 不 open
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert!(tree.rect_of("menu").is_none(), "关闭的浮层不应进索引");
    assert_ne!(tree.hit_test(Pos::new(30.0, 40.0)).as_deref(), Some("menu"));

    // 打开后可见
    let mut menu = Menu::new("menu").items(vec!["A".to_string()]);
    menu.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.rect_of("menu").is_some(), "打开后应进索引");
}

// ---------- 4. ScrollView 子节点真的被渲染/可命中 ----------

struct Painted {
    id: String,
    rect: Rect,
    hits: Rc<RefCell<usize>>,
}

impl Widget for Painted {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(Size::new(c.max.w, 30.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::WHITE);
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        *self.hits.borrow_mut() += 1;
        EventResult::Handled
    }
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default().exposed(true)
    }
}

#[test]
fn scrollview_children_are_reachable_and_hittable() {
    let hits = Rc::new(RefCell::new(0usize));
    let inner = Painted {
        id: "inner".into(),
        rect: Rect::ZERO,
        hits: hits.clone(),
    };
    let sv = ScrollView::new("sv", Box::new(inner)).flex_item(FlexItem::fill());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(sv))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    // 子节点必须在索引里（说明 children_mut 不再返回 None）
    assert!(tree.rect_of("inner").is_some(), "ScrollView 子节点应可索引");
    assert_eq!(
        tree.hit_test(Pos::new(50.0, 10.0)).as_deref(),
        Some("inner")
    );

    tree.dispatch_to(
        "inner",
        &UiEvent::MouseDown {
            pos: Pos::new(50.0, 10.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert_eq!(*hits.borrow(), 1, "子节点应收到事件");
}

#[test]
fn label_bound_signal_auto_subscribes_on_tree_creation() {
    // 初始树（不是 add_node）也必须触发 on_mount
    let sig = y901_ui::Signal::new("a".to_string());
    let label = TextLabel::new("lb", "a").bind(sig.clone());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(label))));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    assert_eq!(sig.subscriber_count(), 1, "构造即挂载应自动订阅");
    sig.set("b".to_string());
    assert_eq!(tree.drain_invalidation(), 1);
}

#[test]
fn button_is_exposed_by_default_but_plain_widgets_are_not() {
    struct Plain;
    impl Widget for Plain {
        fn id(&self) -> &str {
            "plain"
        }
        fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
            c.constrain(Size::new(10.0, 10.0))
        }
        fn arrange(&mut self, _r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {}
        fn rect(&self) -> Rect {
            Rect::ZERO
        }
        fn render(&mut self, _c: &mut dyn RenderCtx) {}
        fn semantics(&self) -> y901_ui::Semantics {
            y901_ui::Semantics::default()
        }
    }

    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(Button::new("btn", "OK")))
            .push(Box::new(Plain)),
    ));
    tree.layout(Size::new(300.0, 100.0), &Mono);

    let view = y901_agent::handle_call(&mut tree, &Policy::default(), y901_agent::ApiCall::Tree)
        .unwrap();
    let text = view.to_string();
    assert!(text.contains("\"btn\""), "按钮默认对 AI 可见：{text}");
    assert!(!text.contains("\"plain\""), "普通组件默认封禁：{text}");
}

#[test]
fn json_roundtrip_helper() {
    let j = Json::parse(r#"{"a":1}"#).unwrap();
    assert!(j.to_string().contains("\"a\""));
}
