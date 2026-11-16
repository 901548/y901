//! **键盘序列回归测试**：打字 / Backspace / 中文提交 / 焦点切换 的完整链路。
//!
//! 背景：真机上出现过"偶尔删不掉"（间歇性丢键）。DOM 层的事件路由已在
//! `y901-backend-web` 重构为 window 级统一监听（只看 `document.activeElement` 与
//! `event.isComposing`），本文件负责**框架层**的一半：给定事件序列，
//! `TextInput` 与 `UiTree` 必须做出正确反应。
//!
//! 这些测试不依赖 DOM，所以能在 CI 里稳定复现；它们同时是"以后别再退回去"的护栏。

use y901_ui::{Flex, Rect, RenderCtx, Size, TextMetrics, TextStyle, UiEvent, UiTree};
use y901_widgets::TextInput;

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        let w: f32 = text
            .chars()
            .map(|c| if (c as u32) >= 0x2E80 { 16.0 } else { 8.0 })
            .sum();
        Size::new(w, 16.0)
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
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
        Size::new(400.0, 100.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

/// 一棵只有一个输入框的树，焦点在它身上。
fn focused_input() -> UiTree {
    let root = Flex::column("root").push(Box::new(TextInput::new("kb-in")));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 100.0), &Mono);
    tree.set_focus(Some("kb-in".to_string()));
    // 让框架把 `Focus(true)` 投递给控件（focused 标志由事件驱动，不是自己设的）
    tree.dispatch(&UiEvent::Focus(true));
    tree.layout(Size::new(400.0, 100.0), &Mono);
    tree.render(&mut Null);
    tree
}

/// 读输入框当前的文本（用框架自带的按类型取节点接口）。
fn text_of(tree: &mut UiTree) -> String {
    tree.node_as_mut::<TextInput>("kb-in")
        .map(|input| input.text().to_string())
        .unwrap_or_default()
}

fn type_text(tree: &mut UiTree, s: &str) {
    for ch in s.chars() {
        tree.dispatch(&UiEvent::Text(ch.to_string()));
    }
}

fn press(tree: &mut UiTree, key: y901_ui::Key) {
    tree.dispatch(&UiEvent::Key {
        key,
        modifiers: y901_ui::Modifiers::default(),
    });
}

// ---- 1. 打字 + Backspace ----

#[test]
fn backspace_after_typing_clears_everything() {
    let mut tree = focused_input();
    type_text(&mut tree, "abc");
    assert_eq!(text_of(&mut tree), "abc", "前置：三个字符都已插入");

    for i in 0..3 {
        press(&mut tree, y901_ui::Key::Backspace);
        let expect = &"abc"[..2 - i];
        assert_eq!(text_of(&mut tree), expect, "第 {} 次 Backspace 后应为 {expect:?}", i + 1);
    }
    assert_eq!(text_of(&mut tree), "", "三次 Backspace 必须删光");
}

/// 连打 20 个 + 连按 20 次 Backspace（对应真机压力测试 a 场景）。
#[test]
fn twenty_chars_then_twenty_backspaces() {
    let mut tree = focused_input();
    type_text(&mut tree, "abcdefghijklmnopqrst");
    assert_eq!(text_of(&mut tree), "abcdefghijklmnopqrst");
    for _ in 0..20 {
        press(&mut tree, y901_ui::Key::Backspace);
    }
    assert_eq!(text_of(&mut tree), "", "20 次 Backspace 后必须为空（一次都不能丢）");
    // 再按一次：空串上 Backspace 不应 panic / 不应有负索引
    press(&mut tree, y901_ui::Key::Backspace);
    assert_eq!(text_of(&mut tree), "");
}

// ---- 2. 组字中/组字后的 Backspace ----

#[test]
fn backspace_works_after_composition_commit() {
    // 模拟中文提交：Compose("nihao") → Text("你好") → 再按 Backspace
    let mut tree = focused_input();
    tree.dispatch(&UiEvent::Compose("nihao".to_string()));
    tree.dispatch(&UiEvent::Text("你好".to_string()));
    assert_eq!(text_of(&mut tree), "你好", "前置：中文已提交");

    press(&mut tree, y901_ui::Key::Backspace);
    assert_eq!(text_of(&mut tree), "你", "组字结束后的第一个 Backspace 必须生效（回归点）");
    press(&mut tree, y901_ui::Key::Backspace);
    assert_eq!(text_of(&mut tree), "", "第二次 Backspace 也要生效");
}

/// 组字过程中按 Backspace：不应当删正文（那是输入法删拼音），提交后正文完好。
#[test]
fn backspace_during_composition_does_not_eat_committed_text() {
    let mut tree = focused_input();
    type_text(&mut tree, "ab");
    assert_eq!(text_of(&mut tree), "ab");

    // 进入组字，拼音阶段按 Backspace，然后正常提交
    tree.dispatch(&UiEvent::Compose("ni".to_string()));
    press(&mut tree, y901_ui::Key::Backspace);
    // 组字期间的 Backspace 只影响预编辑串（由输入法自己处理），正文不能被删
    assert!(
        text_of(&mut tree).ends_with("ab"),
        "组字中按 Backspace 不能删掉已提交的正文，实际: {:?}",
        text_of(&mut tree)
    );
    tree.dispatch(&UiEvent::Text("你".to_string()));
    assert_eq!(text_of(&mut tree), "ab你", "提交后的正文应完整");
}

// ---- 3. 焦点切换后再 Backspace ----

#[test]
fn backspace_survives_focus_loss_and_regain() {
    let mut tree = focused_input();
    type_text(&mut tree, "abc");

    // 焦点切走（点别处）再回来：控件必须重新收到 Focus(true) 才继续处理按键
    tree.set_focus(None);
    tree.dispatch(&UiEvent::Focus(false));
    tree.set_focus(Some("kb-in".to_string()));
    tree.dispatch(&UiEvent::Focus(true));

    press(&mut tree, y901_ui::Key::Backspace);
    assert_eq!(text_of(&mut tree), "ab", "焦点恢复后 Backspace 必须立即生效");
}

// ---- 4. 中文提交后立刻打英文 ----

#[test]
fn ascii_works_immediately_after_composition() {
    let mut tree = focused_input();
    tree.dispatch(&UiEvent::Compose("ni".to_string()));
    tree.dispatch(&UiEvent::Text("你".to_string()));
    // 提交后立刻输入英文（真实链路里走隐藏 input 的 `input` 事件）
    type_text(&mut tree, "ab");
    assert_eq!(text_of(&mut tree), "你ab", "中文提交后立刻打英文不能丢");
}
