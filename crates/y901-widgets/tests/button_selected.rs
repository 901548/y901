//! `Button::selected`：按钮组单选的持久选中态。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Json, Pos, Rect, RenderCtx, Semantics, Size, State,
    TextStyle, TextMetrics, Theme, UiEvent, Widget,
};
use y901_widgets::Button;

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

/// 记录 fill_rect 颜色 + 主题可注入。
struct Rec {
    theme: Theme,
    fills: Vec<Color>,
}

impl TextMetrics for Rec {
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

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(400.0, 200.0)
    }
    fn fill_rect(&mut self, _r: Rect, c: Color) {
        self.fills.push(c);
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, t: &str, s: &TextStyle) -> f32 {
        self.measure_text(t, s).w
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
    fn theme(&self) -> &Theme {
        &self.theme
    }
}

fn render(b: &mut Button) -> Vec<Color> {
    let mut cache = y901_ui::layout::LayoutCache::new();
    b.measure(&Mono, Constraints::loose(Size::new(200.0, 40.0)), &mut cache);
    b.arrange(Rect::new(0.0, 0.0, 120.0, 32.0), &Mono, &mut cache);
    let mut rec = Rec {
        theme: Theme::DARK,
        fills: Vec::new(),
    };
    b.render(&mut rec);
    rec.fills
}

fn color_of(state: State) -> Color {
    Theme::DARK.style(y901_ui::Role::BUTTON, state).bg
}

#[test]
fn button_selected_renders_with_selected_state() {
    let mut plain = Button::new("b1", "暗");
    let normal = render(&mut plain);

    let mut sel = Button::new("b2", "暗").selected(true);
    let selected = render(&mut sel);

    assert_eq!(normal.len(), 1, "按钮只画一个矩形");
    assert_eq!(
        selected[0],
        color_of(State::Selected),
        "选中态必须用 State::Selected 的配色"
    );
    assert_ne!(selected[0], normal[0], "选中态与普通态必须不同");
}

#[test]
fn button_selected_overrides_hovered_and_pressed() {
    // 选中 + 悬停：仍是 Selected（Selected 优先级高于 Hovered）
    let mut b = Button::new("b", "亮").selected(true);
    let _ = b.event(
        &mut EventCtx::new(None, Size::new(400.0, 200.0)),
        &UiEvent::MouseMove {
            pos: Pos::new(10.0, 10.0),
        },
    );
    // 需要先 arrange 才能命中
    let mut cache = y901_ui::layout::LayoutCache::new();
    b.arrange(Rect::new(0.0, 0.0, 120.0, 32.0), &Mono, &mut cache);
    let _ = b.event(
        &mut EventCtx::new(None, Size::new(400.0, 200.0)),
        &UiEvent::MouseMove {
            pos: Pos::new(10.0, 10.0),
        },
    );

    let mut rec = Rec {
        theme: Theme::DARK,
        fills: Vec::new(),
    };
    b.render(&mut rec);
    assert_eq!(
        rec.fills[0],
        color_of(State::Selected),
        "Selected 必须压过 Hovered"
    );

    // 选中 + 按下：仍是 Selected
    let _ = b.event(
        &mut EventCtx::new(None, Size::new(400.0, 200.0)),
        &UiEvent::MouseDown {
            pos: Pos::new(10.0, 10.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    let mut rec2 = Rec {
        theme: Theme::DARK,
        fills: Vec::new(),
    };
    b.render(&mut rec2);
    assert_eq!(
        rec2.fills[0],
        color_of(State::Selected),
        "Selected 必须压过 Pressed"
    );
}

#[test]
fn button_disabled_overrides_selected() {
    // 禁用优先级最高：即使 selected(true) 也画 Disabled
    let mut b = Button::new("b", "x").selected(true).disabled(true);
    let fills = render(&mut b);
    assert_eq!(fills[0], color_of(State::Disabled));
}

#[test]
fn button_set_selected_updates_semantics() {
    let mut b = Button::new("b", "暗");
    assert_eq!(
        b.semantics().state.get("selected"),
        Some(&Json::Bool(false)),
        "默认未选中"
    );

    b.set_selected(true);
    let sem: Semantics = b.semantics();
    assert_eq!(
        sem.state.get("selected"),
        Some(&Json::Bool(true)),
        "set_selected 之后语义必须更新"
    );
}

#[test]
fn button_set_selected_does_not_auto_invalidate() {
    // set_selected 只改字段：UiTree 的脏区不受影响，调用方必须自己 invalidate。
    use y901_ui::{Flex, FlexItem, UiTree};
    let root = Flex::column("root").push(Box::new(
        Button::new("b", "暗").flex_item(FlexItem::height(32.0)),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    tree.invalidate_all();

    let mut rec = Rec {
        theme: Theme::DARK,
        fills: Vec::new(),
    };
    tree.render(&mut rec);
    assert!(tree.dirty().is_empty(), "渲染后脏区应清空");

    // 改字段
    tree.find_mut("b")
        .and_then(|w| w.as_any_mut())
        .and_then(|a| a.downcast_mut::<Button>())
        .expect("Button 需要 as_any_mut")
        .set_selected(true);

    assert!(
        tree.dirty().is_empty(),
        "set_selected 不应自动标脏（与 set_title/set_buttons 一致）"
    );

    // 调用方补标脏后才重绘
    tree.invalidate("b");
    assert!(!tree.dirty().is_empty());
}

#[test]
fn theme_selected_color_is_between_normal_and_pressed() {
    // 派生色：比 Normal 深、比 Pressed 浅（DARK 主题下 Normal 比 Pressed 浅）
    let normal = color_of(State::Normal);
    let selected = color_of(State::Selected);
    let pressed = color_of(State::Pressed);

    assert_ne!(selected, normal);
    assert_ne!(selected, pressed);
    // 逐通道：selected 应落在 normal 与 pressed 之间
    for (a, s, b) in [
        (normal.r, selected.r, pressed.r),
        (normal.g, selected.g, pressed.g),
        (normal.b, selected.b, pressed.b),
    ] {
        let lo = a.min(b) - 1e-4;
        let hi = a.max(b) + 1e-4;
        assert!(
            s >= lo && s <= hi,
            "Selected 通道值 {s} 应落在 [{lo}, {hi}] 之间"
        );
    }
}

#[test]
fn event_result_still_handled_when_selected() {
    let mut b = Button::new("b", "x").selected(true);
    let mut cache = y901_ui::layout::LayoutCache::new();
    b.arrange(Rect::new(0.0, 0.0, 120.0, 32.0), &Mono, &mut cache);
    let mut ctx = EventCtx::new(None, Size::new(400.0, 200.0));
    let r: EventResult = b.event(
        &mut ctx,
        &UiEvent::MouseDown {
            pos: Pos::new(10.0, 10.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert!(r != EventResult::Ignored, "选中态不影响点击响应");
}
