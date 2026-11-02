//! TreeView 键盘导航：**真实事件**驱动（`UiTree::dispatch`），不是直接调方法。
//!
//! 早前我判断"EventCtx 不可构造所以键盘测不了"——这是错的：
//! Tabs 的测试已经证明"先点击拿到焦点，再 `tree.dispatch(&Key)`"可以走通整条路由。
//! 本文件用同样的方式验证 ArrowUp/Down/Left/Right。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Key, Modifiers, MouseButton, Pos, Size, UiEvent, UiTree};
use y901_widgets::tree_view::{TreeItem, TreeView};

const W: u32 = 240;
const H: u32 = 200;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

/// 两棵树：`父`（展开，含 A/B）+ `别的`。
fn tree_with(selected: Option<Vec<usize>>) -> UiTree {
    let roots = vec![
        TreeItem::branch(
            "father",
            vec![TreeItem::leaf("child-a"), TreeItem::leaf("child-b")],
        ),
        TreeItem::leaf("other"),
    ];
    let mut tv = TreeView::new("t", roots);
    if let Some(p) = selected {
        tv.select_path(&p);
    }
    let mut t = UiTree::new(Box::new(tv));
    t.layout(Size::new(W as f32, H as f32), &soft());
    t
}

fn key(k: Key) -> UiEvent {
    UiEvent::Key {
        key: k,
        modifiers: Modifiers::default(),
    }
}

/// 点第 `row` 行（行高 24，从 y=0 起）。
fn click_row(t: &mut UiTree, row: usize) {
    let y = row as f32 * 24.0 + 12.0;
    t.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(80.0, y),
        button: MouseButton::Left,
    });
}

fn selected(t: &mut UiTree) -> Option<Vec<usize>> {
    t.node_as_mut::<TreeView>("t")
        .and_then(|v| v.selected().cloned())
}

fn visible_len(t: &mut UiTree) -> usize {
    t.node_as_mut::<TreeView>("t")
        .map(|v| v.visible_len())
        .unwrap_or(0)
}

#[test]
fn arrow_down_moves_selection_to_the_next_row() {
    let mut t = tree_with(Some(vec![0]));
    click_row(&mut t, 0); // 拿到焦点
    assert_eq!(selected(&mut t), Some(vec![0]));

    t.dispatch(&key(Key::Down));
    assert_eq!(selected(&mut t), Some(vec![0, 0]), "应移到第一个子项");

    t.dispatch(&key(Key::Down));
    assert_eq!(selected(&mut t), Some(vec![0, 1]), "应移到第二个子项");

    t.dispatch(&key(Key::Down));
    assert_eq!(selected(&mut t), Some(vec![1]), "应跨到下一个根节点");

    // 已是最后一行：再按不越界（不循环）
    t.dispatch(&key(Key::Down));
    assert_eq!(selected(&mut t), Some(vec![1]), "末行再向下应停在原地");
}

#[test]
fn arrow_up_moves_selection_back() {
    let mut t = tree_with(Some(vec![1]));
    click_row(&mut t, 3); // 点最后一行拿焦点
    assert_eq!(selected(&mut t), Some(vec![1]));

    t.dispatch(&key(Key::Up));
    assert_eq!(selected(&mut t), Some(vec![0, 1]));

    t.dispatch(&key(Key::Up));
    assert_eq!(selected(&mut t), Some(vec![0, 0]));

    t.dispatch(&key(Key::Up));
    assert_eq!(selected(&mut t), Some(vec![0]), "应移回父节点");

    // 首行再按不越界
    t.dispatch(&key(Key::Up));
    assert_eq!(selected(&mut t), Some(vec![0]), "首行再向上应停在原地");
}

#[test]
fn arrow_left_collapses_then_jumps_to_parent() {
    // 标准文件管理器语义：已展开 → 折叠；已折叠 → 跳到父节点。
    let mut t = tree_with(Some(vec![0, 0]));
    click_row(&mut t, 1); // 选中第一个子项并拿焦点

    let before = visible_len(&mut t);
    assert_eq!(before, 4, "父 + 2 子 + 别的 = 4 行");

    // 选中子项时按 Left：子项自身不可折叠 → 跳到父节点
    t.dispatch(&key(Key::Left));
    assert_eq!(selected(&mut t), Some(vec![0]), "子项上按 Left 应跳到父节点");

    // 父节点上按 Left：已展开 → 折叠
    t.dispatch(&key(Key::Left));
    assert_eq!(visible_len(&mut t), 2, "折叠后应只剩 父 + 别的");
    assert_eq!(selected(&mut t), Some(vec![0]), "折叠不该改变选中项");
}

#[test]
fn arrow_right_expands_then_enters_first_child() {
    let mut t = tree_with(Some(vec![1])); // 根节点 "别的"（叶子）
    click_row(&mut t, 0); // 选中父节点

    // 先折叠父节点
    t.dispatch(&key(Key::Left));
    assert_eq!(visible_len(&mut t), 2);

    // Right：已折叠 → 展开
    t.dispatch(&key(Key::Right));
    assert_eq!(visible_len(&mut t), 4, "Right 应展开父节点");

    // Right：已展开 → 进入首个子项
    t.dispatch(&key(Key::Right));
    assert_eq!(selected(&mut t), Some(vec![0, 0]), "Right 应进入首个子项");
}

#[test]
fn keyboard_without_focus_does_nothing() {
    let mut t = tree_with(Some(vec![0]));
    // 不点击、不给焦点：键盘事件不应改变选中
    t.dispatch(&key(Key::Down));
    assert_eq!(selected(&mut t), Some(vec![0]), "无焦点时不应响应键盘");
}
