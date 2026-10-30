//! TextInput 的密码模式 / 占位提示 + Dialog::set_size。

use y901_ui::{
    Color, Constraints, EventCtx, Json, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, UiTree, Widget,
};
use y901_widgets::{Dialog, TextInput};

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

/// 记录所有 draw_text 的文本。
#[derive(Default)]
struct Rec {
    texts: Vec<String>,
}

impl TextMetrics for Rec {
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

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(400.0, 200.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, t: &str, s: &TextStyle) -> f32 {
        self.texts.push(t.to_string());
        self.measure_text(t, s).w
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

fn arranged(ti: &mut TextInput) {
    let mut cache = y901_ui::layout::LayoutCache::new();
    ti.measure(
        &Mono,
        Constraints::loose(Size::new(240.0, 24.0)),
        &mut cache,
    );
    ti.arrange(Rect::new(0.0, 0.0, 240.0, 24.0), &Mono, &mut cache);
}

/// 给输入框打上焦点（组件内部的 focused 只由框架的 Focus 事件设置）。
fn focus(ti: &mut TextInput) {
    let mut ctx = EventCtx::new(None, Size::new(240.0, 24.0));
    let _ = ti.event(&mut ctx, &UiEvent::Focus(true));
}

fn render_texts(ti: &mut TextInput) -> Vec<String> {
    let mut rec = Rec::default();
    ti.render(&mut rec);
    rec.texts
}

// ---------- 密码模式 ----------

#[test]
fn password_masks_rendered_text() {
    let mut ti = TextInput::new("pw").password(true);
    ti.set_text("secret");
    arranged(&mut ti);

    let texts = render_texts(&mut ti);
    let joined = texts.join("");
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("secret") || t.contains('s') && t.len() > 1),
        "渲染不得出现原文：{texts:?}"
    );
    assert_eq!(joined, "******", "6 个字符应画 6 个 *，实际 {texts:?}");
}

#[test]
fn password_masks_spaces_too() {
    let mut ti = TextInput::new("pw").password(true);
    ti.set_text("a b"); // 含空格
    arranged(&mut ti);

    let joined = render_texts(&mut ti).join("");
    assert_eq!(
        joined, "***",
        "空格也必须画 *（否则长度不可见）：{joined:?}"
    );
}

#[test]
fn password_masks_semantics() {
    let mut ti = TextInput::new("pw").password(true);
    ti.set_text("secret");
    let sem = ti.semantics();

    assert_eq!(sem.label, "password", "label 不能暴露明文");
    assert_eq!(sem.value, "******", "value 必须是掩码");
    assert_eq!(
        sem.state.get("text").and_then(|v| v.as_str()),
        Some("******"),
        "state.text 必须是掩码"
    );
    assert!(sem.sensitive, "密码框必须标记 sensitive");
    // 光标/锚点/长度保留（长度已通过 * 数暴露，用户接受）
    assert!(sem.props.iter().any(|(k, _)| k == "cursor"));
    assert!(sem.props.iter().any(|(k, _)| k == "len"));
    assert!(
        !format!("{:?}", sem).contains("secret"),
        "整个语义里不能出现原文"
    );
}

#[test]
fn password_preserves_interaction() {
    let mut ti = TextInput::new("pw").password(true);
    ti.set_text("secret");

    // 原文仍可从 text() 读到（Host 提交给 SSH / 保存时用）
    assert_eq!(ti.text(), "secret");
    assert_eq!(ti.cursor(), 6);

    // 光标移动 / 选区仍然工作
    focus(&mut ti);
    let mut ctx = EventCtx::new(None, Size::new(240.0, 24.0));
    let _ = ti.event(
        &mut ctx,
        &UiEvent::Key {
            key: y901_ui::Key::Left,
            modifiers: y901_ui::Modifiers::default(),
        },
    );
    assert_eq!(ti.cursor(), 5, "密码模式下 Left 仍然移动光标");
}

#[test]
fn password_masks_ime_preedit() {
    let mut ti = TextInput::new("pw").password(true);
    arranged(&mut ti);

    // 直接喂 IME 预编辑（不走 PTY）：Compose 事件（需要焦点才会进入 preedit）
    focus(&mut ti);
    let mut ctx = EventCtx::new(None, Size::new(240.0, 24.0));
    let _ = ti.event(&mut ctx, &UiEvent::Compose("中文".to_string()));

    let texts = render_texts(&mut ti);
    assert!(
        !texts.iter().any(|t| t.contains('中') || t.contains('文')),
        "IME 预编辑不得暴露明文：{texts:?}"
    );
    assert!(
        texts.iter().all(|t| t == "*"),
        "预编辑应逐字画 *，实际 {texts:?}"
    );
}

#[test]
fn non_password_semantics_unchanged() {
    let mut ti = TextInput::new("plain");
    ti.set_text("hello");
    let sem = ti.semantics();
    assert_eq!(sem.label, "hello");
    assert_eq!(sem.value, "hello");
    assert_eq!(
        sem.state.get("text").and_then(|v| v.as_str()),
        Some("hello")
    );
    assert!(!sem.sensitive);
}

// ---------- 占位提示 ----------

#[test]
fn placeholder_renders_when_empty() {
    let mut ti = TextInput::new("ph").placeholder("请输入用户名");
    arranged(&mut ti);

    let texts = render_texts(&mut ti);
    assert!(
        texts.iter().any(|t| t == "请输入用户名"),
        "空文本时应画占位提示，实际 {texts:?}"
    );
}

#[test]
fn placeholder_not_rendered_when_has_text() {
    let mut ti = TextInput::new("ph").placeholder("请输入用户名");
    ti.set_text("abc");
    arranged(&mut ti);

    let texts = render_texts(&mut ti);
    assert!(
        !texts.iter().any(|t| t == "请输入用户名"),
        "有内容时不得画占位提示，实际 {texts:?}"
    );
}

#[test]
fn placeholder_not_rendered_during_ime_composition() {
    let mut ti = TextInput::new("ph").placeholder("请输入用户名");
    arranged(&mut ti);
    focus(&mut ti);
    let mut ctx = EventCtx::new(None, Size::new(240.0, 24.0));
    let _ = ti.event(&mut ctx, &UiEvent::Compose("中".to_string()));

    let texts = render_texts(&mut ti);
    assert!(
        !texts.iter().any(|t| t == "请输入用户名"),
        "IME 组合中已有内容，不应再画占位提示：{texts:?}"
    );
}

#[test]
fn placeholder_not_in_semantics() {
    let ti = TextInput::new("ph").placeholder("请输入用户名");
    let sem: Semantics = ti.semantics();
    assert!(!sem.label.contains("请输入"), "占位提示不进语义 label");
    assert!(!sem.value.contains("请输入"));
}

// ---------- Dialog::set_size ----------

fn arranged_dialog(d: &mut Dialog, root: Size) {
    let mut cache = y901_ui::layout::LayoutCache::new();
    d.measure(&Mono, Constraints::loose(root), &mut cache);
    d.arrange(Rect::new(0.0, 0.0, root.w, root.h), &Mono, &mut cache);
}

#[test]
fn dialog_set_size_changes_arrange() {
    let mut d = Dialog::new("dlg", "T");
    d.set_open(true);
    arranged_dialog(&mut d, Size::new(1200.0, 900.0));
    assert_eq!(
        d.panel_rect().size(),
        Size::new(420.0, 220.0),
        "默认面板必须是 420×220"
    );

    d.set_size(600.0, 400.0);
    arranged_dialog(&mut d, Size::new(1200.0, 900.0));
    assert_eq!(d.panel_rect().w, 600.0, "set_size 后宽度必须生效");
    assert_eq!(d.panel_rect().h, 400.0, "set_size 后高度必须生效");

    // 面板必须居中（1200×900 的窗口里，(1200-600)/2 = 300）
    assert_eq!(d.panel_rect().x, 300.0);
    assert_eq!(d.panel_rect().y, 250.0);
}
#[test]
fn dialog_set_size_clamps() {
    let mut d = Dialog::new("dlg", "T");
    d.set_open(true);
    d.set_size(10.0, 10.0); // 低于下限
    arranged_dialog(&mut d, Size::new(2000.0, 2000.0));
    // 下限 200×150 仍应正常渲染（不 panic、不产生负矩形）
    let mut rec = Rec::default();
    d.render(&mut rec);
    assert!(rec.texts.iter().any(|t| t == "T"));

    d.set_size(9999.0, 9999.0); // 高于上限
    arranged_dialog(&mut d, Size::new(2000.0, 2000.0));
    let mut rec2 = Rec::default();
    d.render(&mut rec2);
    assert!(rec2.texts.iter().any(|t| t == "T"));
}

// 保持 Json import 有用（Compose 之外的动作路径）
#[test]
fn dialog_close_still_works_after_set_size() {
    let mut d = Dialog::new("dlg", "T");
    d.set_size(500.0, 300.0);
    d.set_open(true);
    let mut ctx = EventCtx::new(None, Size::new(1200.0, 900.0));
    let r = d.event(
        &mut ctx,
        &UiEvent::Action {
            name: "close".into(),
            args: Json::obj(),
        },
    );
    assert!(r.needs_relayout());
    assert!(!d.is_open());
    let _ = UiTree::new(Box::new(TextInput::new("x")));
}
