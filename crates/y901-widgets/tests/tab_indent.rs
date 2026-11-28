//! `TextArea` 的 Tab 缩进（PLAN 批次 5 的最后一个未做项）。
//!
//! 关键设计点，三条都在这里钉住：
//!
//! 1. **默认不接管 Tab**：`Tab` 仍然是框架的焦点遍历 —— 键盘用户在大界面里移动的主要手段
//!    不能被组件随便吃掉；
//! 2. 只有显式 `tab_indents(true)` 才插缩进，且**插空格**（对齐到下一个制表位），
//!    不是插 `\t`（`TextBuffer` 逐字符量宽，`\t` 会画歪）；
//! 3. **`Ctrl+Tab` 永远是焦点遍历**（框架保证，不看 `wants_tab`）—— 这是"组件吃掉了 Tab"
//!    时唯一的键盘出路，没有它就是键盘陷阱。

use y901_ui::{Flex, Key, Modifiers, Size, TextMetrics, TextStyle, UiEvent, UiTree};
use y901_widgets::{Button, TextArea};

struct Mono;
impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        let w: f32 = t
            .chars()
            .map(|c| if (c as u32) >= 0x2E80 { 16.0 } else { 8.0 })
            .sum();
        Size::new(w, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn tree_with(area: TextArea) -> UiTree {
    let root = Flex::row("root")
        .push(Box::new(area))
        .push(Box::new(Button::new("btn", "after")));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 200.0), &Mono);
    tree.set_focus(Some("ta".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    tree.layout(Size::new(400.0, 200.0), &Mono);
    tree
}

fn keys(tree: &mut UiTree, key: Key, shift: bool, ctrl: bool) {
    tree.dispatch(&UiEvent::Key {
        key,
        modifiers: Modifiers {
            shift,
            ctrl,
            ..Default::default()
        },
    });
}

fn text(tree: &mut UiTree) -> String {
    tree.node_as_mut::<TextArea>("ta")
        .map(|w| w.text().to_string())
        .unwrap_or_default()
}

fn cursor(tree: &mut UiTree) -> usize {
    tree.node_as_mut::<TextArea>("ta")
        .map(|w| w.cursor())
        .unwrap_or(0)
}

fn focus(tree: &UiTree) -> Option<String> {
    tree.focus().map(|s| s.to_string())
}

/// 默认行为不变：**Tab 切换焦点**，不会往正文里塞空格。
#[test]
fn tab_moves_focus_by_default() {
    let mut tree = tree_with(TextArea::new("ta").with_text("abc"));
    assert_eq!(focus(&tree).as_deref(), Some("ta"));
    keys(&mut tree, Key::Tab, false, false);
    assert_eq!(text(&mut tree), "abc", "默认 Tab 不得改动正文");
    assert_ne!(
        focus(&tree).as_deref(),
        Some("ta"),
        "默认 Tab 必须把焦点移走（这是键盘导航的基本盘）"
    );
}

/// 开启后：Tab 插到**下一个制表位**（4 个空格制表位），Shift+Tab 反缩进。
#[test]
fn tab_indents_to_the_next_tab_stop_and_shift_tab_outdents() {
    let mut tree = tree_with(TextArea::new("ta").tab_indents(true).with_text(""));
    // 行首：插满 4 个空格
    keys(&mut tree, Key::Tab, false, false);
    assert_eq!(text(&mut tree), "    ", "行首 Tab 应补齐 4 个空格");
    assert_eq!(cursor(&mut tree), 4);

    // 再按 Tab：已对齐到制表位 → 再来 4 个
    keys(&mut tree, Key::Tab, false, false);
    assert_eq!(text(&mut tree), "        ", "已对齐后 Tab 应再加一个制表位");

    // Shift+Tab：删掉最多 4 个空格
    keys(&mut tree, Key::Tab, true, false);
    assert_eq!(text(&mut tree), "    ", "Shift+Tab 应删掉一个制表位的空格");

    // 反缩进只吃空格：光标前是字母时不动
    let mut tree = tree_with(TextArea::new("ta").tab_indents(true).with_text("ab"));
    let before = cursor(&mut tree);
    keys(&mut tree, Key::Tab, true, false);
    assert_eq!(
        cursor(&mut tree),
        before,
        "光标前不是空格时 Shift+Tab 不应动"
    );
    assert_eq!(text(&mut tree), "ab");
}

/// 制表位是**按显示列**算的：从第 2 列按 Tab 只补 2 个空格（不是 4 个）。
#[test]
fn tab_stop_is_measured_in_display_columns() {
    let mut tree = tree_with(TextArea::new("ta").tab_indents(true).with_text("ab"));
    assert_eq!(cursor(&mut tree), 2);
    keys(&mut tree, Key::Tab, false, false);
    assert_eq!(text(&mut tree), "ab  ", "第 2 列按 Tab 只补 2 个空格");
    assert_eq!(cursor(&mut tree), 4, "插完应对齐到第 4 列");

    // 宽字占 2 列：一个中文 + Tab 应补 2 个空格（1+2=3 → 补到 4）
    let mut tree = tree_with(TextArea::new("ta").tab_indents(true).with_text("中"));
    let c0 = cursor(&mut tree);
    assert_eq!(c0, 3, "前置：光标在 中 之后");
    keys(&mut tree, Key::Tab, false, false);
    assert_eq!(
        text(&mut tree),
        "中  ",
        "宽字占 2 列时，从第 2 列补 2 个空格才对（按字符数算会补 4 个）"
    );
}

/// **`Ctrl+Tab` 必须永远是焦点遍历**（键盘陷阱的逃生口）。
#[test]
fn ctrl_tab_escapes_even_when_tab_is_swallowed() {
    let mut tree = tree_with(TextArea::new("ta").tab_indents(true).with_text(""));
    keys(&mut tree, Key::Tab, false, false);
    assert_eq!(text(&mut tree), "    ", "Tab 先插了缩进");
    // Ctrl+Tab：不能被缩进吃掉
    keys(&mut tree, Key::Tab, false, true);
    assert_eq!(
        text(&mut tree),
        "    ",
        "Ctrl+Tab 不得改动正文（它属于焦点遍历）"
    );
    assert_ne!(
        focus(&tree).as_deref(),
        Some("ta"),
        "开启 Tab 缩进后，Ctrl+Tab 仍是唯一的键盘出路 —— 必须切走焦点"
    );
}
