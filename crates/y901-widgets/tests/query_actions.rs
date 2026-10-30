//! 方向 C：ActionSpec 类型校验 + Query 查询语言。

use y901_agent::{handle_call, ApiCall, ApiError, Policy};
use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, InputKind, Json, Pos, Rect, RenderCtx,
    Size, TextMetrics, TextStyle, UiTree, Widget,
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

/// 15 个节点的树：10 个按钮（其中 2 个含 Save）+ 5 个输入框。
fn big_tree() -> UiTree {
    let mut root = Flex::column("root");
    for i in 0..10 {
        let label = if i < 2 {
            format!("Save {i}")
        } else {
            format!("Btn {i}")
        };
        root = root.push(Box::new(
            Button::new(format!("b{i}"), label).flex_item(FlexItem::height(20.0)),
        ));
    }
    for i in 0..5 {
        root = root.push(Box::new(
            TextInput::new(format!("t{i}")).flex_item(FlexItem::height(24.0)),
        ));
    }
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 600.0), &Mono);
    tree
}

// ---------- 1. ActionSpec ----------

#[test]
fn button_click_is_inputkind_none_and_textinput_set_text_is_text() {
    let tree = big_tree();

    let btn = tree.find("b0").expect("button");
    let click = btn
        .semantics()
        .actions
        .iter()
        .find(|a| a.name == "click")
        .cloned()
        .expect("button 应声明 click");
    assert_eq!(click.input, InputKind::None);

    let input = tree.find("t0").expect("input");
    let set_text = input
        .semantics()
        .actions
        .iter()
        .find(|a| a.name == "set_text")
        .cloned()
        .expect("input 应声明 set_text");
    assert_eq!(set_text.input, InputKind::Text);

    // 语义树里也应带类型
    let snap = tree.snapshot_json().to_string();
    assert!(
        snap.contains("\"input\":\"text\""),
        "语义树应暴露参数类型：{snap}"
    );
}

#[test]
fn invoke_with_wrong_argument_type_is_rejected() {
    let mut tree = big_tree();
    let policy = Policy::default();

    // set_text 需要 text(string)，传 number 应 400
    let err = handle_call(
        &mut tree,
        &policy,
        ApiCall::Invoke {
            id: "t0".into(),
            action: "set_text".into(),
            args: Json::obj().set("text", 42u32),
            if_rev: None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, ApiError::BadRequest(_)),
        "类型不符应 400，实际 {err:?}"
    );

    // 传 string 应通过
    assert!(handle_call(
        &mut tree,
        &policy,
        ApiCall::Invoke {
            id: "t0".into(),
            action: "set_text".into(),
            args: Json::obj().set("text", "hello"),
            if_rev: None,
        }
    )
    .is_ok());

    // click 不需要参数，传 text 也应被拒
    let err = handle_call(
        &mut tree,
        &policy,
        ApiCall::Invoke {
            id: "b0".into(),
            action: "click".into(),
            args: Json::obj().set("text", "x"),
            if_rev: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
}

#[test]
fn unimplemented_action_still_rejected() {
    let mut tree = big_tree();
    let err = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Invoke {
            id: "b0".into(),
            action: "explode".into(),
            args: Json::obj(),
            if_rev: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
}

// ---------- 2. Query ----------

#[test]
fn query_filters_by_role_and_label() {
    let mut tree = big_tree();
    let out = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query {
            q: r#"role=button & label~="Save""#.into(),
        },
    )
    .unwrap();

    let text = out.to_string();
    assert!(
        text.contains("\"count\":2"),
        "应命中 2 个 Save 按钮：{text}"
    );
    assert!(text.contains("\"b0\"") && text.contains("\"b1\""));
    assert!(!text.contains("\"b2\""), "非 Save 按钮不应出现");
    assert!(!text.contains("\"t0\""), "输入框不应出现");
}

#[test]
fn query_limit_truncates_results() {
    let mut tree = big_tree();
    let out = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query {
            q: "role=button & limit=2".into(),
        },
    )
    .unwrap();
    let text = out.to_string();
    assert!(text.contains("\"count\":2"), "limit 应生效：{text}");
}

#[test]
fn query_returns_compact_nodes_not_full_tree() {
    let mut tree = big_tree();
    let out = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query { q: "id=b3".into() },
    )
    .unwrap();
    let text = out.to_string();
    assert!(text.contains("\"id\":\"b3\""));
    assert!(text.contains("\"revision\""));
    assert!(text.contains("\"rect\""));
    assert!(
        !text.contains("\"children\""),
        "查询结果不应带整树 children：{text}"
    );
}

#[test]
fn query_with_bad_syntax_is_bad_request() {
    let mut tree = big_tree();
    let err = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query { q: "nope=1".into() },
    )
    .unwrap_err();
    assert!(matches!(err, ApiError::BadRequest(_)));
}

// ---------- 3. 查询只返回暴露节点 ----------

struct Hidden {
    rect: Rect,
}

impl Widget for Hidden {
    fn id(&self) -> &str {
        "hidden"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(50.0, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        // 默认封禁
        y901_ui::Semantics::default().label("hidden-node")
    }
}

#[test]
fn query_skips_unexposed_nodes() {
    let root = Flex::column("root").push(Box::new(Hidden { rect: Rect::ZERO }));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 100.0), &Mono);

    let out = handle_call(
        &mut tree,
        &Policy::default(),
        ApiCall::Query {
            q: r#"label~="hidden""#.into(),
        },
    )
    .unwrap();
    assert!(
        out.to_string().contains("\"count\":0"),
        "未暴露节点不得出现在查询结果里"
    );
    let _ = Pos::new(0.0, 0.0);
}
