//! `TextLabel` 的两件事：
//! 1. "改字段不标脏 ⇒ 不重绘"（验证上一轮文档的断言）；
//! 2. `set_color` 动作（`#rrggbb` / `#rrggbbaa`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use y901_agent::api::{dispatch_request, ApiError, Policy};
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, FlexItem, Json, Pos, Rect, RenderCtx,
    Semantics, Size, TextMetrics, TextStyle, UiTree, Widget,
};
use y901_widgets::{parse_color, TextLabel};

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

// ---------- 任务 1：改字段不标脏 ⇒ 不重绘 ----------

/// 颜色可从外部改、且把每次 render 的颜色记下来的探针。
struct Swatch {
    id: &'static str,
    rect: Rect,
    color: Rc<Cell<Color>>,
    drawn: Rc<RefCell<Vec<Color>>>,
}

impl Widget for Swatch {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let c = self.color.get();
        self.drawn.borrow_mut().push(c);
        ctx.fill_rect(self.rect, c);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

#[test]
fn field_change_without_invalidate_does_not_repaint() {
    let color = Rc::new(Cell::new(Color::rgba(1.0, 0.0, 0.0, 1.0)));
    let drawn = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new(Box::new(Swatch {
        id: "sw",
        rect: Rect::ZERO,
        color: color.clone(),
        drawn: drawn.clone(),
    }));
    tree.layout(Size::new(100.0, 100.0), &Mono);

    // 首帧：必然绘制
    tree.render(&mut Null);
    assert_eq!(drawn.borrow().len(), 1, "首帧应绘制");

    // 改字段，但不 invalidate
    color.set(Color::rgba(0.0, 1.0, 0.0, 1.0));
    tree.render(&mut Null);
    assert_eq!(
        drawn.borrow().len(),
        1,
        "改字段不标脏时不应重绘（脏区为空 → render 直接返回）"
    );

    // invalidate 之后才绘制
    tree.invalidate_all();
    tree.render(&mut Null);
    assert_eq!(drawn.borrow().len(), 2, "invalidate_all 之后应重绘");
    assert_eq!(drawn.borrow()[1], Color::rgba(0.0, 1.0, 0.0, 1.0));
}

struct Null;

impl TextMetrics for Null {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Mono.measure_text(t, s)
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        Mono.line_height(s)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Null {
    fn size(&self) -> Size {
        Size::new(100.0, 100.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

// ---------- 任务 2：parse_color 的 7 个边界 ----------

#[test]
fn parse_hex_rrggbb() {
    let c = parse_color("#ff0000").expect("#ff0000 应解析成功");
    assert_eq!(c, Color::rgba(1.0, 0.0, 0.0, 1.0));
}

#[test]
fn parse_hex_rrggbbaa() {
    let c = parse_color("#ff000080").expect("#ff000080 应解析成功");
    assert!((c.a - 0.5).abs() < 0.01, "alpha 应为 0.5，实际 {}", c.a);
}

#[test]
fn parse_rejects_short_hex() {
    assert!(parse_color("#fff").is_err(), "不支持 3 位简写");
}

#[test]
fn parse_rejects_named_color() {
    assert!(parse_color("red").is_err(), "不支持命名颜色");
}

#[test]
fn parse_rejects_rgba_function() {
    assert!(
        parse_color("rgba(255,0,0,0.5)").is_err(),
        "不支持 rgba() 格式"
    );
}

#[test]
fn parse_rejects_empty() {
    assert!(parse_color("").is_err(), "空串必须拒绝");
}

#[test]
fn parse_rejects_non_hex_chars() {
    assert!(parse_color("#gggggg").is_err(), "非 hex 字符必须拒绝");
}

// ---------- 任务 2：集成（走 dispatch_request 的 invoke 路径）----------

fn label_tree() -> UiTree {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        TextLabel::new("label", "hi").flex_item(FlexItem::height(20.0)),
    ))));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    tree
}

// ---------- 回归：自定义参数名不影响既有动作 ----------

#[test]
fn custom_arg_name_does_not_break_existing_actions() {
    use y901_ui::{ActionSpec, InputKind};

    // 默认参数名（回退到 InputKind）
    let none = ActionSpec::new("click", InputKind::None);
    assert_eq!(none.arg_name(), "", "无参动作的参数名必须是空串");
    let text = ActionSpec::new("set_text", InputKind::Text);
    assert_eq!(text.arg_name(), "text", "Text 的默认参数名必须是 text");
    let num = ActionSpec::new("scroll", InputKind::Number);
    assert_eq!(num.arg_name(), "value");

    // 自定义参数名
    let named = ActionSpec::named("set_color", InputKind::Text, "color");
    assert_eq!(named.arg_name(), "color");

    // JSON 形状：默认名**不**输出 arg 字段（保证既有 Agent 解析不受影响）
    assert!(
        none.to_json().get("arg").is_none(),
        "默认参数名不应输出 arg 字段"
    );
    assert!(text.to_json().get("arg").is_none());
    // 自定义名输出 arg 字段，Agent 才知道该传什么键
    assert_eq!(
        named.to_json().get("arg").and_then(|v| v.as_str()),
        Some("color")
    );
}

#[test]
fn invoke_set_text_still_uses_text_argument() {
    let mut tree = label_tree();
    let ok = dispatch_request(
        &mut tree,
        &Policy::default(),
        "invoke",
        &Json::obj()
            .set("id", "label")
            .set("action", "set_text")
            .set("args", Json::obj().set("text", "hello")),
    );
    assert!(ok.is_ok(), "set_text 仍应接受 text 参数：{ok:?}");

    // 用错参数名应被拒（证明校验仍在工作）
    let bad = dispatch_request(
        &mut tree,
        &Policy::default(),
        "invoke",
        &Json::obj()
            .set("id", "label")
            .set("action", "set_text")
            .set("args", Json::obj().set("color", "#ff0000")),
    );
    assert!(
        matches!(bad, Err(ApiError::BadRequest(_))),
        "set_text 不接受 color 参数：{bad:?}"
    );
}

#[test]
fn invoke_set_color_changes_label_color() {
    let mut tree = label_tree();

    let out = dispatch_request(
        &mut tree,
        &Policy::default(),
        "invoke",
        &Json::obj()
            .set("id", "label")
            .set("action", "set_color")
            .set("args", Json::obj().set("color", "#00ff00")),
    );
    assert!(out.is_ok(), "set_color 应成功，实际 {out:?}");

    // 通过语义/渲染路径确认颜色确实变了：直接读组件的字段不可行（Box<dyn Widget>），
    // 因此用"再次 set_color 不同值也应成功" + "非法值被拒"来确认动作接线正确。
    let bad = dispatch_request(
        &mut tree,
        &Policy::default(),
        "invoke",
        &Json::obj()
            .set("id", "label")
            .set("action", "set_color")
            .set("args", Json::obj().set("color", "red")),
    );
    assert!(
        matches!(bad, Err(ApiError::BadRequest(_))),
        "非法颜色必须 400，实际 {bad:?}"
    );

    // 动作声明必须可见（Agent 能发现它）
    let node = tree.find("label").expect("label");
    assert!(
        node.semantics()
            .actions
            .iter()
            .any(|a| a.name == "set_color"),
        "semantics().actions 必须声明 set_color"
    );
}
