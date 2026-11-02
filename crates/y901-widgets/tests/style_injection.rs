//! 样式注入：内置控件可以被外部主题覆盖（背景/圆角/描边/焦点圈/滚动条）。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, Modifiers, Pos, Rect, RenderCtx, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};
use y901_widgets::{Button, TextInput, WidgetStyle};

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

/// 记录绘制调用（fill / stroke），用来断言"样式真的生效"。
/// 渐变调用记录：(矩形, 方向)。
type GradientCalls = Rc<RefCell<Vec<(Rect, (f32, f32))>>>;

#[derive(Clone, Default)]
struct Calls {
    fills: Rc<RefCell<Vec<Rect>>>,
    strokes: Rc<RefCell<Vec<(Rect, Color, f32)>>>,
    gradients: GradientCalls,
}

struct Rec {
    calls: Calls,
    clips: Vec<Rect>,
}

impl Calls {
    fn new() -> Self {
        Self::default()
    }
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
        Size::new(320.0, 200.0)
    }
    fn fill_rect(&mut self, r: Rect, _c: Color) {
        self.calls.fills.borrow_mut().push(r);
    }
    fn stroke_rect(&mut self, r: Rect, c: Color, w: f32) {
        self.calls.strokes.borrow_mut().push((r, c, w));
    }
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, r: Rect) {
        let c = match self.clips.last() {
            Some(outer) => r.intersect(*outer),
            None => r,
        };
        self.clips.push(c);
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }

    // 覆盖圆角/渐变，让"注入的样式走了新原语"这件事可被观察
    fn fill_round_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        assert!(radius > 0.0, "圆角样式应带半径");
        self.calls.fills.borrow_mut().push(rect);
        let _ = color;
    }

    fn fill_gradient_round_rect(
        &mut self,
        rect: Rect,
        _from: Color,
        _to: Color,
        dir: (f32, f32),
        _radius: f32,
    ) {
        self.calls.gradients.borrow_mut().push((rect, dir));
    }

    fn stroke_round_rect(&mut self, rect: Rect, color: Color, width: f32, _radius: f32) {
        self.calls.strokes.borrow_mut().push((rect, color, width));
    }
}

fn tree_with(widget: Box<dyn Widget>) -> UiTree {
    let root = Flex::column("root").push(widget);
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(320.0, 200.0), &Mono);
    tree
}

fn rec(calls: &Calls) -> Rec {
    Rec {
        calls: calls.clone(),
        clips: vec![Rect::new(0.0, 0.0, 320.0, 200.0)],
    }
}

#[test]
fn button_style_injection_draws_gradient_and_focus_ring() {
    let style = WidgetStyle::default()
        .gradient(
            Color::rgba(0.10, 0.50, 0.90, 1.0),
            Color::rgba(0.50, 0.20, 0.90, 1.0),
            (1.0, 0.0),
        )
        .radius(10.0)
        .focus_border(Color::WHITE, 2.0);

    let calls = Calls::new();
    let mut tree = tree_with(Box::new(Button::new("btn", "OK").style(style)));
    tree.render(&mut rec(&calls));

    // 背景走的是"圆角渐变"单实例
    assert_eq!(
        calls.gradients.borrow().len(),
        1,
        "注入渐变样式后应提交 1 个圆角渐变"
    );
    // 没焦点 → 没有焦点圈
    assert_eq!(calls.strokes.borrow().len(), 0, "无焦点时不应画焦点圈");

    // 给焦点后再渲染：必须出现焦点描边（stroke_round_rect）
    let calls2 = Calls::new();
    tree.set_focus(Some("btn".to_string()));
    let _ = tree.dispatch(&UiEvent::Focus(true));
    // `set_focus` 可能已经把 Focus 事件发过了（第二次会返回 Ignored）；
    // 渲染是按脏区剪枝的，所以这里必须显式标脏才能触发重绘。
    tree.invalidate("btn");
    tree.render(&mut rec(&calls2));
    assert_eq!(
        calls2.strokes.borrow().len(),
        1,
        "有焦点时必须画 1 圈焦点描边"
    );
    let (rect, _c, w) = calls2.strokes.borrow()[0];
    assert!((w - 2.0).abs() < 1e-6, "焦点描边宽度应来自注入样式");
    assert!(rect.w > 0.0 && rect.h > 0.0, "描边应落在按钮矩形上");
}

#[test]
fn text_input_style_injection_replaces_theme_background() {
    let style = WidgetStyle::default()
        .bg(Color::rgba(0.05, 0.05, 0.08, 1.0))
        .radius(8.0)
        .border(Color::rgba(0.3, 0.3, 0.4, 1.0), 1.0);

    let calls = Calls::new();
    let mut tree = tree_with(Box::new(TextInput::new("input").widget_style(style)));
    tree.render(&mut rec(&calls));

    assert_eq!(calls.fills.borrow().len(), 1, "纯色圆角背景应画 1 次");
    assert_eq!(calls.strokes.borrow().len(), 1, "常驻描边应画 1 次");
}

#[test]
fn style_injection_keeps_dirty_region_local() {
    let mut tree = tree_with(Box::new(
        Button::new("btn", "OK").style(WidgetStyle::default().bg(Color::WHITE).radius(6.0)),
    ));
    tree.render(&mut rec(&Calls::new()));
    assert!(tree.dirty().is_empty(), "前置：脏区已清空");

    // 运行时换样式（调用方负责标脏）
    let mut changed = false;
    tree.with_node_mut("btn", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(b) = any.downcast_mut::<Button>() {
                changed = b.set_style(Some(
                    WidgetStyle::default()
                        .bg(Color::rgba(1.0, 0.0, 0.0, 1.0))
                        .radius(12.0),
                ));
            }
        }
    });
    assert!(changed, "样式应真的变了");
    tree.invalidate("btn");

    // 脏区必须只覆盖按钮，而不是整窗
    assert!(!tree.dirty().is_full(), "换样式不应把整窗标脏");
    let bounds = tree.dirty().bounds().expect("应有脏区");
    assert!(
        bounds.w <= 320.0 && bounds.h <= 64.0,
        "脏区应局限在按钮附近，实际 {bounds:?}"
    );
}

#[test]
fn set_style_with_same_value_reports_no_change() {
    let style = WidgetStyle::default().bg(Color::WHITE);
    let mut btn = Button::new("b", "B").style(style);
    assert!(!btn.set_style(Some(style)), "同样的样式不应报告变化");
    assert!(btn.set_style(None), "清空样式应报告变化");
}

#[test]
fn dialog_panel_style_is_applied() {
    let style = WidgetStyle::default()
        .gradient(Color::BLACK, Color::WHITE, (0.0, 1.0))
        .radius(16.0);
    let mut dlg = y901_widgets::Dialog::new("dlg", "Title");
    dlg.set_open(true);
    assert!(dlg.set_panel_style(Some(style)));

    let calls = Calls::new();
    let mut tree = tree_with(Box::new(dlg));
    tree.invalidate_all();
    tree.render(&mut rec(&calls));
    assert_eq!(
        calls.gradients.borrow().len(),
        1,
        "Dialog 面板应使用注入的圆角渐变"
    );
}

#[test]
fn dialog_button_style_is_applied() {
    let button = WidgetStyle::default()
        .gradient(Color::BLACK, Color::WHITE, (0.0, 1.0))
        .radius(9.0)
        .border(Color::WHITE, 1.0);
    let mut dlg =
        y901_widgets::Dialog::new("dlg", "Title").buttons(vec![("Cancel", "cancel"), ("OK", "ok")]);
    dlg.set_open(true);
    assert!(dlg.set_button_style(Some(button)));

    let calls = Calls::new();
    let mut tree = tree_with(Box::new(dlg));
    tree.invalidate_all();
    tree.render(&mut rec(&calls));
    assert_eq!(
        calls.gradients.borrow().len(),
        2,
        "Dialog 内部两个按钮都应使用注入的渐变"
    );
    assert!(
        calls.strokes.borrow().len() >= 2,
        "Dialog 按钮应画出注入的描边"
    );
}

#[test]
fn scrollbar_style_is_applied() {
    use y901_widgets::ScrollView;
    // ScrollView::new(id, child)：内容用一列 40 行文字撑高，触发滚动条
    let mut column = y901_ui::Flex::column("content");
    for i in 0..40 {
        column = column.push(Box::new(y901_widgets::TextLabel::new(
            format!("row{i}"),
            format!("row {i}"),
        )));
    }
    let mut sv = ScrollView::new("sv", Box::new(column));
    assert!(sv.set_bar_style(Some(WidgetStyle::default().scrollbar(
        Color::BLACK,
        Color::WHITE,
        6.0,
        3.0
    ))));

    let calls = Calls::new();
    let mut tree = tree_with(Box::new(sv));
    tree.invalidate_all();
    tree.render(&mut rec(&calls));
    // 轨道 + 滑块都用圆角画（fill_round_rect）
    assert!(!calls.fills.borrow().is_empty(), "滚动条应画出轨道/滑块");
}

/// 让 `EventCtx`/`EventResult` 等导入被使用（否则 clippy 报 unused）。
// 测试 API：测试内的类型见证/未用常量，仅测试代码引用
#[allow(dead_code)]
fn _unused(_c: &mut EventCtx, _r: EventResult, _m: Modifiers, _c2: Constraints) {}

#[test]
fn dialog_can_be_used_as_a_popup_without_double_scrim() {
    // 作为 popup 内容时，遮罩已由浮层层统一画，Dialog 自己不应再画一层
    let mut dlg = y901_widgets::Dialog::new("dlg2", "Title");
    dlg.set_open(true);
    assert!(dlg.set_scrim(false), "应能关闭遮罩");

    let calls = Calls::new();
    let mut tree = tree_with(Box::new(dlg));
    tree.invalidate_all();
    tree.render(&mut rec(&calls));

    // 没了遮罩：不应出现"全屏黑底"那么大的填充
    let big = calls
        .fills
        .borrow()
        .iter()
        .any(|r| r.w >= 300.0 && r.h >= 200.0);
    assert!(!big, "关闭遮罩后不应再有满屏填充：{:?}", calls.fills.borrow());
}

#[test]
fn menu_works_as_popup_without_layer() {
    use y901_widgets::Menu;
    // Menu 不再声明 `layer()`：作为浮层挂载时，
    // 裁切 / 命中 / 命令派发 / "选完就关"全部由浮层层提供。
    let mut menu = Menu::new("m").items(vec!["a".into(), "b".into()]);
    menu.open();

    let mut tree = UiTree::new(Box::new(y901_ui::Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.open_popup(Box::new(menu), Rect::new(10.0, 10.0, 100.0, 24.0)));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.has_popup(), "应已挂载");

    // 拿到面板矩形（命中就看它）
    let panel = {
        let mut p = Rect::ZERO;
        tree.with_popup_mut("m", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(menu) = any.downcast_mut::<Menu>() {
                    p = menu.panel_rect();
                }
            }
        });
        p
    };
    assert!(panel.w > 0.0, "面板应有矩形：{panel:?}");

    // 点第一项：应派发 App 命令（index/label），且菜单自己关闭
    // 面板内部布局：PAD=6，项高 26 → 第一项在 panel.y + 6..32
    let _ = tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(panel.x + 10.0, panel.y + 6.0 + 13.0),
        button: y901_ui::MouseButton::Left,
    });
    let cmds = tree.drain_commands();
    assert!(
        cmds.iter()
            .any(|c| matches!(c, y901_ui::Command::App { name, .. } if name == "menu.select")),
        "点菜单项应派发 menu.select，实际 {cmds:?}"
    );
    // Menu 自己 `close()` 了（`is_open() == false`）→ 浮层层自动撞掉
    assert!(!tree.has_popup(), "菜单选完自己关了，浮层也应消失");
}

#[test]
fn dialog_runs_as_modal_popup() {
    use y901_ui::{Key, Modifiers, MouseButton};
    use y901_widgets::Dialog;

    // 触发器：点一下就通过 `ctx.request_modal_popup` 弹 Dialog
    struct Trigger {
        rect: Rect,
    }
    impl y901_ui::Widget for Trigger {
        fn id(&self) -> &str {
            "open"
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _c: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(120.0, 40.0))
        }
        fn arrange(
            &mut self,
            r: Rect,
            _m: &dyn TextMetrics,
            _c: &mut y901_ui::layout::LayoutCache,
        ) {
            self.rect = r;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, ctx: &mut EventCtx, e: &y901_ui::UiEvent) -> EventResult {
            if matches!(e, y901_ui::UiEvent::MouseDown { .. }) {
                let mut dlg = Dialog::new("confirm", "删除？");
                dlg.set_open(true);
                dlg.set_scrim(false); // 遮罩由浮层层画
                dlg.set_buttons(vec![
                    ("ok".into(), "OK".into()),
                    ("cancel".into(), "Cancel".into()),
                ]);
                ctx.request_modal_popup(Box::new(move || Box::new(dlg)), self.rect);
                return EventResult::Handled;
            }
            EventResult::Ignored
        }
        fn render(&mut self, _c: &mut dyn RenderCtx) {}
        fn semantics(&self) -> y901_ui::Semantics {
            y901_ui::Semantics::default()
        }
    }

    let mut tree = UiTree::new(Box::new(Trigger { rect: Rect::ZERO }));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    let r = tree.rect_of("open").expect("trigger");

    // 点开
    let _ = tree.dispatch(&y901_ui::UiEvent::MouseDown {
        pos: Pos::new(r.x + 5.0, r.y + 5.0),
        button: MouseButton::Left,
    });
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.has_popup(), "Dialog 应已作为浯层挂载");
    assert!(tree.popups()[0].is_modal(), "Dialog 应是模态浯层");

    // Tab 循环在 Dialog 内部（不逃到底层 trigger）
    for _ in 0..4 {
        let _ = tree.dispatch(&y901_ui::UiEvent::Key {
            key: Key::Tab,
            modifiers: Modifiers::default(),
        });
        if let Some(f) = tree.focus() {
            assert_ne!(f, "open", "焦点不应逃回底层触发器");
        }
    }

    // 点外部：模态不关闭
    let _ = tree.dispatch(&y901_ui::UiEvent::MouseDown {
        pos: Pos::new(390.0, 290.0),
        button: MouseButton::Left,
    });
    assert!(tree.has_popup(), "模态 Dialog 不应被点外部关闭");

    // Escape 关闭
    let _ = tree.dispatch(&y901_ui::UiEvent::Key {
        key: Key::Escape,
        modifiers: Modifiers::default(),
    });
    assert!(!tree.has_popup(), "Escape 应关闭 Dialog");
}
