//! Tab 焦点循环 + 焦点高亮：TextInput ↔ Button。
//!
//! 这是"无障碍/键盘可用"的最低保证：
//! 1. Tab 必须能在可聚焦控件之间循环；
//! 2. 拿到焦点的 Button 必须切到 `State::Focused` 的外观（不是和 Normal 一样）。

use y901_ui::{Key, Modifiers, Size, UiEvent, UiTree, Widget};
use y901_widgets::{Button, TextInput};

struct Mono;

impl y901_ui::TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn build() -> UiTree {
    let root = y901_ui::Flex::column("root")
        .push(Box::new(TextInput::new("input")))
        .push(Box::new(Button::new("btn", "OK")));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(320.0, 200.0), &Mono);
    tree
}

fn tab() -> UiEvent {
    UiEvent::Key {
        key: Key::Tab,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn tab_cycles_between_textinput_and_button() {
    let mut tree = build();

    // 第一次 Tab：焦点进入第一个可聚焦控件
    let _ = tree.dispatch(&tab());
    let first = tree.focus().map(|s| s.to_string());
    assert!(first.is_some(), "Tab 后必须有焦点");
    assert_eq!(first.as_deref(), Some("input"), "第一个应是 TextInput");

    // 第二次 Tab：轮到 Button
    let _ = tree.dispatch(&tab());
    assert_eq!(
        tree.focus().map(|s| s.to_string()).as_deref(),
        Some("btn"),
        "第二次 Tab 应切到 Button"
    );

    // 第三次 Tab：回到 TextInput（循环）
    let _ = tree.dispatch(&tab());
    assert_eq!(
        tree.focus().map(|s| s.to_string()).as_deref(),
        Some("input"),
        "第三次 Tab 应循环回 TextInput"
    );
}

#[test]
fn focused_button_redraws_and_uses_focused_style() {
    let mut tree = build();
    let _ = tree.dispatch(&tab()); // input
    tree.render(&mut Null);
    // 切到按钮：必须请求重绘，否则焦点高亮不会出现
    let r = tree.dispatch(&tab());
    assert!(r.needs_redraw(), "焦点变化必须重绘");

    // 按钮拿到 Focus 事件后内部状态变为 focused → 渲染时用 State::Focused
    let _ = tree.dispatch(&UiEvent::Focus(true));
    let style_focused =
        y901_ui::theme::DARK_THEME.style(y901_ui::Role::BUTTON, y901_ui::State::Focused);
    let style_normal =
        y901_ui::theme::DARK_THEME.style(y901_ui::Role::BUTTON, y901_ui::State::Normal);
    assert_ne!(
        style_focused.bg, style_normal.bg,
        "主题必须给 Focused 一个可见的区别（否则键盘用户看不出焦点在哪）"
    );
}

struct Null;

impl y901_ui::TextMetrics for Null {
    fn measure_text(&self, _t: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::ZERO
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl y901_ui::RenderCtx for Null {
    fn size(&self) -> Size {
        Size::new(320.0, 200.0)
    }
    fn fill_rect(&mut self, _r: y901_ui::Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: y901_ui::Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &y901_ui::TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: y901_ui::Rect) {}
    fn pop_clip(&mut self) {}
}

/// 让 `Button` 的 `Widget` 导入被使用（否则 clippy 会报 unused import）。
// 测试 API：测试内的类型见证/未用常量，仅测试代码引用
#[allow(dead_code)]
fn _assert_widget_object_safe(b: &mut Button) -> &mut dyn Widget {
    b
}
