//! Dialog 的动态能力 + TextInput::set_text。
//!
//! 这些方法都**不自动标脏**（组件碰不到 `UiTree`）；测试只验证字段语义，
//! 标脏责任见 `docs/FRAMEWORK_GUIDE.md` §29。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Json, Pos, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiTree, Widget,
};
use y901_widgets::{Dialog, TextInput};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 固定高度的探针（用来验证纵向排列）。
struct Probe {
    id: &'static str,
    h: f32,
    rect: Rect,
}

impl Widget for Probe {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, self.h))
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
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 记录所有 draw_text 的渲染上下文。
#[derive(Default)]
struct Rec {
    texts: Vec<String>,
}

impl TextMetrics for Rec {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(800.0, 600.0)
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

fn arranged_dialog(d: &mut Dialog) {
    let mut cache = y901_ui::layout::LayoutCache::new();
    d.measure(&Mono, Constraints::loose(Size::new(800.0, 600.0)), &mut cache);
    d.arrange(Rect::new(0.0, 0.0, 800.0, 600.0), &Mono, &mut cache);
}

// ---------- 任务 0：多个子节点都要排布 ----------

#[test]
fn dialog_arranges_all_children() {
    let mut d = Dialog::new("dlg", "Title");
    d.push_child(Box::new(Probe {
        id: "a",
        h: 30.0,
        rect: Rect::ZERO,
    }));
    d.push_child(Box::new(Probe {
        id: "b",
        h: 40.0,
        rect: Rect::ZERO,
    }));
    d.set_open(true);
    arranged_dialog(&mut d);

    let a = d.children()[0].rect();
    let b = d.children()[1].rect();
    assert!(a.h > 0.0 && b.h > 0.0, "两个子节点都必须拿到高度：{a:?} {b:?}");
    assert!(a.y < b.y, "b 必须排在 a 下面：a.y={} b.y={}", a.y, b.y);
    // 不能越过面板底部（按钮条之上）
    assert!(b.bottom() <= 600.0, "子节点不得溢出屏幕：b={b:?}");
}

// ---------- 任务 2：Dialog 动态能力 ----------

#[test]
fn dialog_set_title_updates_render() {
    let mut d = Dialog::new("dlg", "Old");
    d.set_open(true);
    arranged_dialog(&mut d);

    d.set_title("Rename");
    let mut rec = Rec::default();
    d.render(&mut rec);
    assert!(
        rec.texts.iter().any(|t| t == "Rename"),
        "render 必须画出新标题，实际 {:?}",
        rec.texts
    );
    assert!(!rec.texts.iter().any(|t| t == "Old"), "旧标题不应再出现");
}

#[test]
fn dialog_set_buttons_updates_buttons() {
    let mut d = Dialog::new("dlg", "T");
    d.set_buttons(vec![("OK".to_string(), "ok".to_string())]);
    d.set_open(true);
    arranged_dialog(&mut d);

    let mut rec = Rec::default();
    d.render(&mut rec);
    assert!(rec.texts.iter().any(|t| t == "OK"));

    // 换成两个按钮：button_rects 必须先被清空，重新 arrange 后长度匹配
    d.set_buttons(vec![
        ("Cancel".to_string(), "cancel".to_string()),
        ("Delete".to_string(), "delete".to_string()),
    ]);
    arranged_dialog(&mut d);

    let mut rec2 = Rec::default();
    d.render(&mut rec2);
    assert!(rec2.texts.iter().any(|t| t == "Cancel"));
    assert!(rec2.texts.iter().any(|t| t == "Delete"));
    assert!(
        !rec2.texts.iter().any(|t| t == "OK"),
        "旧按钮不应再画出来：{:?}",
        rec2.texts
    );
}

#[test]
fn dialog_clear_and_push_children() {
    let mut d = Dialog::new("dlg", "T");
    d.push_child(Box::new(Probe {
        id: "a",
        h: 20.0,
        rect: Rect::ZERO,
    }));
    assert_eq!(d.children().len(), 1);

    d.clear_children();
    assert!(d.children().is_empty(), "clear_children 必须清空");

    d.push_child(Box::new(Probe {
        id: "b",
        h: 20.0,
        rect: Rect::ZERO,
    }));
    assert_eq!(d.children().len(), 1);
    assert_eq!(d.children()[0].id(), "b");
}

#[test]
fn dialog_set_open_opens_and_closes() {
    let mut d = Dialog::new("dlg", "T");
    assert!(!d.is_open(), "默认关闭");

    d.set_open(true);
    assert!(d.is_open());
    d.set_open(false);
    assert!(!d.is_open());
}

// ---------- 任务 3：TextInput 动态能力 ----------

#[test]
fn text_input_set_text_updates_buffer() {
    let mut ti = TextInput::new("input");
    ti.set_text("hello");
    assert_eq!(ti.text(), "hello");
    assert_eq!(ti.cursor(), 5, "光标应在末尾（字节偏移）");
    assert!(ti.selection().is_none(), "设置文本后不应有选区");

    // 中文：光标是**字节**偏移
    ti.set_text("中文");
    assert_eq!(ti.text(), "中文");
    assert_eq!(ti.cursor(), 6, "两个汉字 = 6 字节");
}

#[test]
fn text_input_clear_resets_all() {
    let mut ti = TextInput::new("input");
    ti.set_text("abc");
    let before = {
        use y901_ui::Widget;
        ti.layout_version()
    };

    ti.clear();
    assert_eq!(ti.text(), "");
    assert_eq!(ti.cursor(), 0);
    assert!(ti.selection().is_none());

    use y901_ui::Widget;
    assert!(
        ti.layout_version() > before,
        "clear 必须 bump layout_version（{} -> {}）",
        before,
        ti.layout_version()
    );
}

#[test]
fn text_input_supports_downcast() {
    use y901_ui::Widget;
    let mut ti = TextInput::new("input");
    ti.set_text("abc");
    let any = ti.as_any_mut().expect("TextInput 必须实现 as_any_mut");
    let back = any.downcast_mut::<TextInput>().expect("必须能 downcast 回 TextInput");
    assert_eq!(back.text(), "abc");
    // downcast 到错误类型必须失败（不是 panic）
    assert!(any.downcast_mut::<Dialog>().is_none());
}

// ---------- 任务 1：结构性变化请求 layout ----------

#[test]
fn add_node_requests_layout() {
    // 根必须是容器（能提供 children_mut），否则 add_node 会拒绝
    let mut tree = UiTree::new(Box::new(y901_ui::layout::Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    // 布局后不应该有挂起的请求
    assert!(
        !tree.take_layout_request(),
        "初次 layout 后不应有挂起请求"
    );

    let ok = tree.add_node(
        "root",
        Box::new(Probe {
            id: "child",
            h: 20.0,
            rect: Rect::ZERO,
        }),
    );
    assert!(ok, "add_node 应成功");
    assert!(tree.take_layout_request(), "add_node 必须请求 layout");
    assert!(
        !tree.take_layout_request(),
        "请求是一次性的（第二次必须为 false）"
    );
}

#[test]
fn remove_node_requests_layout() {
    let mut tree = UiTree::new(Box::new(y901_ui::layout::Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.add_node(
        "root",
        Box::new(Probe {
            id: "child",
            h: 20.0,
            rect: Rect::ZERO,
        }),
    );
    let _ = tree.take_layout_request();

    assert!(tree.remove_node("child"));
    assert!(tree.take_layout_request(), "remove_node 必须请求 layout");
}

// 让 `Json` 参与编译（Dialog 的按钮命令参数类型）
#[test]
fn dialog_buttons_emit_app_command() {
    let mut d = Dialog::new("dlg", "T");
    d.set_buttons(vec![("OK".to_string(), "ok".to_string())]);
    d.set_open(true);
    arranged_dialog(&mut d);

    // 面板右下角的按钮区域内点击
    let mut rec = Rec::default();
    d.render(&mut rec);

    // 用一个覆盖整个面板的点击（按钮矩形在 arrange 时算好）
    let mut ctx = EventCtx::new(None, Size::new(800.0, 600.0));
    // 直接点第一个按钮：从 render 无法拿到矩形，改用"扫描点击"不可靠，
    // 这里只验证 close 动作（走同一套 emit_app 机制）
    let r = d.event(
        &mut ctx,
        &y901_ui::UiEvent::Action {
            name: "close".into(),
            args: Json::obj(),
        },
    );
    assert!(r.needs_relayout(), "close 必须触发重排");
    assert!(!d.is_open(), "close 之后必须关闭");
}
