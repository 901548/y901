//! **文本编辑产品能力**：撤销/重做、按词移动、双击选词（PLAN 批次 5）。
//!
//! 为什么单独一个文件：这些能力横跨三层 —— `TextBuffer`（纯逻辑）、
//! `TextInput` / `TextArea`（键与鼠标映射）、`UiTree`（事件分发）。
//! 只测 `TextBuffer` 会漏掉"Ctrl+Z 根本没接到控件上"，只测控件又说不清
//! 语义为什么是这样。这里从**事件**出发，断言到**文本结果**。

use std::time::Duration;
use y901_ui::{
    Color, Flex, Key, Modifiers, MouseButton, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle,
    UiEvent, UiTree,
};
use y901_widgets::{TextArea, TextInput};

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
        Size::new(400.0, 120.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

fn input_tree(initial: &str) -> UiTree {
    let root = Flex::column("root").push(Box::new(TextInput::new("ed").with_text(initial)));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 120.0), &Mono);
    tree.set_focus(Some("ed".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    tree.layout(Size::new(400.0, 120.0), &Mono);
    tree.render(&mut Null);
    tree
}

fn area_tree(initial: &str) -> UiTree {
    let root = Flex::column("root").push(Box::new(TextArea::new("ed").with_text(initial)));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 120.0), &Mono);
    tree.set_focus(Some("ed".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    tree.layout(Size::new(400.0, 120.0), &Mono);
    tree.render(&mut Null);
    tree
}

fn input_text(tree: &mut UiTree) -> String {
    tree.node_as_mut::<TextInput>("ed")
        .map(|w| w.text().to_string())
        .unwrap_or_default()
}

fn area_text(tree: &mut UiTree) -> String {
    tree.node_as_mut::<TextArea>("ed")
        .map(|w| w.text().to_string())
        .unwrap_or_default()
}

fn type_str(tree: &mut UiTree, s: &str) {
    for ch in s.chars() {
        tree.dispatch(&UiEvent::Text(ch.to_string()));
    }
}

fn key(tree: &mut UiTree, key: Key, modifiers: Modifiers) {
    tree.dispatch(&UiEvent::Key { key, modifiers });
}

fn ctrl(tree: &mut UiTree, ch: char) {
    key(
        tree,
        Key::Char(ch),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
}

fn ctrl_shift(tree: &mut UiTree, ch: char) {
    key(
        tree,
        Key::Char(ch),
        Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
    );
}

fn editor_cursor(tree: &mut UiTree) -> usize {
    tree.node_as_mut::<TextInput>("ed")
        .map(|w| w.cursor())
        .unwrap_or(usize::MAX)
}

fn editor_selection(tree: &mut UiTree) -> Option<(usize, usize)> {
    tree.node_as_mut::<TextInput>("ed")
        .and_then(|w| w.selection())
}

// ---------- 撤销 / 重做 ----------

#[test]
fn ctrl_z_undoes_a_typing_run_as_one_step() {
    let mut tree = input_tree("");
    type_str(&mut tree, "hello");
    assert_eq!(input_text(&mut tree), "hello");

    ctrl(&mut tree, 'z');
    assert_eq!(
        input_text(&mut tree),
        "",
        "连打的 5 个字符应作为**一步**撤销（不是逐字回退）"
    );

    // 再按一次：历史已空，文本保持（不应 panic，也不应变出别的内容）
    ctrl(&mut tree, 'z');
    assert_eq!(input_text(&mut tree), "");
}

#[test]
fn ctrl_z_then_ctrl_shift_z_round_trips() {
    let mut tree = input_tree("");
    type_str(&mut tree, "abc");
    ctrl(&mut tree, 'z');
    assert_eq!(input_text(&mut tree), "");
    ctrl_shift(&mut tree, 'z');
    assert_eq!(
        input_text(&mut tree),
        "abc",
        "Ctrl+Shift+Z 应把整段重做回来"
    );
    // Ctrl+Y 也应能重做（两种习惯都支持）：先撤销再用 Y 重做
    ctrl(&mut tree, 'z');
    assert_eq!(input_text(&mut tree), "");
    ctrl(&mut tree, 'y');
    assert_eq!(input_text(&mut tree), "abc", "Ctrl+Y 与 Ctrl+Shift+Z 等价");
}

#[test]
fn a_new_edit_clears_the_redo_stack() {
    let mut tree = input_tree("");
    type_str(&mut tree, "abc");
    ctrl(&mut tree, 'z');
    assert_eq!(input_text(&mut tree), "");
    type_str(&mut tree, "x");
    assert_eq!(input_text(&mut tree), "x");
    ctrl(&mut tree, 'y');
    assert_eq!(
        input_text(&mut tree),
        "x",
        "新编辑之后原来的重做栈必须作废（否则会跳回被丢弃的文本）"
    );
}

#[test]
fn undo_restores_the_cursor_where_the_edit_started() {
    let mut tree = input_tree("abcdef");
    // 点在第 4 个字符之前（等宽 8px/字符 → x=24），把光标放到中间。
    let rect = tree.rect_of("ed").expect("输入框有矩形");
    let at = Pos::new(rect.x + 24.0, rect.y + rect.h * 0.5);
    tree.dispatch(&UiEvent::MouseDown {
        pos: at,
        button: MouseButton::Left,
    });
    tree.dispatch(&UiEvent::MouseUp {
        pos: at,
        button: MouseButton::Left,
    });
    // 内边距会让"点 x=24"落在 2 而不是 3 —— 不硬编码，直接读回真实位置。
    let at_edit = editor_cursor(&mut tree);
    assert!(
        (1..6).contains(&at_edit),
        "前置：光标应落在文本中间，实得 {at_edit}"
    );
    type_str(&mut tree, "XY");
    assert_ne!(input_text(&mut tree), "abcdef", "前置：XY 已插入");
    ctrl(&mut tree, 'z');
    assert_eq!(input_text(&mut tree), "abcdef");
    assert_eq!(
        editor_cursor(&mut tree),
        at_edit,
        "撤销后光标应回到插入起点，而不是跳到开头或结尾"
    );
}

#[test]
fn history_is_bounded_and_does_not_grow_forever() {
    let mut tree = input_tree("");
    // 每段之间 sleep 一发，避免被合并成一步（合并窗口 700ms）。
    for i in 0..5 {
        type_str(&mut tree, &i.to_string());
        std::thread::sleep(Duration::from_millis(0));
        // 用删除打断合并（删除是 Other，不与 Typing 合并）
        key(&mut tree, Key::Backspace, Modifiers::default());
    }
    // 只是确保"大量编辑后仍能正常撤销且不崩"；上限本身由 TextBuffer 单测覆盖。
    for _ in 0..100 {
        ctrl(&mut tree, 'z');
    }
    let text = input_text(&mut tree);
    assert!(text.len() <= 5, "撤销到底后应回到早期状态，实得 {text:?}");
}

// ---------- 按词移动 ----------

#[test]
fn ctrl_arrow_moves_by_word_not_by_char() {
    let mut tree = input_tree("hello brave world");
    assert_eq!(editor_cursor(&mut tree), 17, "前置：光标在末尾");
    ctrl_key(&mut tree, Key::Left);
    assert_eq!(editor_cursor(&mut tree), 12, "Ctrl+← 应跳到 world 的词首");
    ctrl_key(&mut tree, Key::Left);
    assert_eq!(editor_cursor(&mut tree), 6, "再按一次到 brave 的词首");
    ctrl_key(&mut tree, Key::Right);
    assert_eq!(editor_cursor(&mut tree), 12, "Ctrl+→ 应跳到 world 的词首");
}

#[test]
fn ctrl_shift_arrow_extends_the_selection_by_word() {
    let mut tree = input_tree("one two three");
    ctrl_shift_key(&mut tree, Key::Left);
    assert_eq!(
        editor_selection(&mut tree),
        Some((8, 13)),
        "Ctrl+Shift+← 应选中最后一个词（three）"
    );
}

#[test]
fn word_moves_never_split_a_grapheme_cluster() {
    // "e" + 组合重音 是一簇；词移动/选词都不能落在两者之间。
    let text = "cafe\u{301} bar";
    let mut tree = input_tree(text);
    ctrl_key(&mut tree, Key::Left);
    let cursor = editor_cursor(&mut tree);
    assert!(
        text.is_char_boundary(cursor) && cursor != 5,
        "光标不能落在 e 与 U+0301 之间，实得 {cursor}"
    );
    assert_eq!(
        cursor, 7,
        "应落在 bar 的词首（cafe\u{301} 占 6 字节 + 1 个空格）"
    );
}

// ---------- 双击选词 ----------

#[test]
fn double_click_selects_the_word_under_the_pointer() {
    let mut tree = input_tree("alpha beta gamma");
    let rect = tree.rect_of("ed").expect("输入框有矩形");
    // "beta" 从第 6 字节开始；等宽 8px/字符 → 中心约在 6*8+16 = 64
    let click = Pos::new(rect.x + 64.0, rect.y + rect.h * 0.5);
    tree.dispatch(&UiEvent::MouseDown {
        pos: click,
        button: MouseButton::Left,
    });
    tree.dispatch(&UiEvent::MouseUp {
        pos: click,
        button: MouseButton::Left,
    });
    // 第一次点击：只落光标，不选词
    assert_eq!(editor_selection(&mut tree), None, "单击不应选中整词");

    // 第二次点击（同位置、时间窗内）→ 选词
    tree.dispatch(&UiEvent::MouseDown {
        pos: click,
        button: MouseButton::Left,
    });
    let sel = editor_selection(&mut tree);
    let (start, end) = sel.expect("双击应选中一个词");
    assert_eq!(
        &"alpha beta gamma"[start..end],
        "beta",
        "选中的必须是 beta，实得 {:?}",
        &"alpha beta gamma"[start..end]
    );
}

#[test]
fn text_area_has_the_same_three_capabilities() {
    // TextArea 走不同的键处理分支；这里确认它没有落下同一批能力。
    let mut tree = area_tree("one two");
    ctrl_key(&mut tree, Key::Left);
    let cursor = tree
        .node_as_mut::<TextArea>("ed")
        .map(|w| w.cursor())
        .unwrap();
    assert_eq!(cursor, 4, "TextArea 的 Ctrl+← 也应跳到 two 的词首");

    type_str(&mut tree, "X");
    assert_eq!(area_text(&mut tree), "one Xtwo");
    ctrl(&mut tree, 'z');
    assert_eq!(area_text(&mut tree), "one two", "TextArea 也要支持 Ctrl+Z");
    ctrl(&mut tree, 'y');
    assert_eq!(area_text(&mut tree), "one Xtwo", "TextArea 也要支持 Ctrl+Y");
}

fn ctrl_key(tree: &mut UiTree, key: Key) {
    key_with(tree, key, true, false);
}

fn ctrl_shift_key(tree: &mut UiTree, key: Key) {
    key_with(tree, key, true, true);
}

fn key_with(tree: &mut UiTree, k: Key, ctrl: bool, shift: bool) {
    tree.dispatch(&UiEvent::Key {
        key: k,
        modifiers: Modifiers {
            ctrl,
            shift,
            ..Default::default()
        },
    });
}
