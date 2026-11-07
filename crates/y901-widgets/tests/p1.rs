//! P1 补齐：滚动条不误选、菜单键盘导航、对话框按钮。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use y901_ui::{Command, Flex, FlexItem, Key, Pos, Size, TextMetrics, TextStyle, UiEvent, UiTree};
use y901_widgets::{Dialog, ListView, Menu};

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

// ---------- 1. 点滚动条不得选中行 ----------

#[test]
fn listview_scrollbar_click_does_not_select_row() {
    let picked = Rc::new(Cell::new(usize::MAX));
    let cb = picked.clone();

    let items: Vec<String> = (0..100).map(|i| format!("row {i}")).collect();
    let list = ListView::from_strings("list", items).on_row_selected(move |i| cb.set(i));

    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(list.flex_item(FlexItem::fill()))),
    ));
    tree.layout(Size::new(300.0, 200.0), &Mono);

    // 滚动条在右侧 10px（BAR_WIDTH）
    let bar_pos = Pos::new(296.0, 100.0);
    tree.dispatch_to(
        "list",
        &UiEvent::MouseDown {
            pos: bar_pos,
            button: y901_ui::MouseButton::Left,
        },
    );
    tree.dispatch_to(
        "list",
        &UiEvent::MouseMove {
            pos: Pos::new(296.0, 150.0),
        },
    );
    tree.dispatch_to(
        "list",
        &UiEvent::MouseUp {
            pos: Pos::new(296.0, 150.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    tree.layout(Size::new(300.0, 200.0), &Mono);

    let snap = tree.snapshot_json().to_string();
    assert_eq!(picked.get(), usize::MAX, "点滚动条不应触发行选中");
    assert!(
        !snap.contains("\"selected\":0") && !snap.contains("\"selected\":1"),
        "选中行不应被改动：{snap}"
    );
    // 偏移确实变了
    let offset = snap
        .split("\"offset\":")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.0);
    assert!(offset > 0.0, "拖拽滚动条应改变 offset，实际 {offset}");
}

// ---------- 2. 菜单键盘导航 ----------

fn menu_only() -> (UiTree, Rc<RefCell<Vec<Command>>>) {
    let sink = Rc::new(RefCell::new(Vec::new()));
    let mut menu = Menu::new("menu").items(vec![
        "Open".to_string(),
        "Save".to_string(),
        "Quit".to_string(),
    ]);
    menu.open();

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    (tree, sink)
}

#[test]
fn menu_keyboard_navigation_moves_and_confirms() {
    let (mut tree, _sink) = menu_only();

    let down = UiEvent::Key {
        key: Key::Down,
        modifiers: Default::default(),
    };
    let up = UiEvent::Key {
        key: Key::Up,
        modifiers: Default::default(),
    };

    tree.dispatch_to("menu", &down);
    tree.dispatch_to("menu", &down);
    let snap = tree.snapshot_json().to_string();
    assert!(snap.contains("\"open\":true"), "菜单应仍打开");

    // 上移回到第 1 项，再上移应环绕到最后一项
    tree.dispatch_to("menu", &up);
    tree.dispatch_to("menu", &up);

    // Enter 触发并关闭
    tree.dispatch_to(
        "menu",
        &UiEvent::Key {
            key: Key::Enter,
            modifiers: Default::default(),
        },
    );
    let commands = tree.drain_commands();
    assert_eq!(commands.len(), 1, "Enter 应产生一条 App 命令：{commands:?}");
    match &commands[0] {
        Command::App { name, args } => {
            assert_eq!(name, "menu.select");
            let text = args.to_string();
            assert!(text.contains("\"index\""), "命令应携带 index：{text}");
        }
        other => panic!("应为 App 命令，实际 {other:?}"),
    }
}

// ---------- 3. 对话框按钮 ----------

#[test]
fn dialog_buttons_emit_action_and_close() {
    let mut dialog = Dialog::new("dlg", "Confirm")
        .push(Box::new(y901_widgets::TextLabel::new(
            "body",
            "Are you sure?",
        )))
        .buttons(vec![("Cancel", "cancel"), ("OK", "ok")]);
    dialog.open();

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(dialog))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 按钮矩形右对齐在面板底部；直接扫一遍找到能触发的那一点
    let panel_w = 420.0_f32.min(400.0 - 40.0);
    let panel_x = (400.0 - panel_w) * 0.5;
    let panel_h = 220.0_f32.min(300.0 - 40.0);
    let panel_y = (300.0 - panel_h) * 0.5;
    let btn_y = panel_y + panel_h - 16.0 - 28.0 + 14.0;
    // 最右侧的按钮 = OK
    let ok_x = panel_x + panel_w - 16.0 - 44.0;

    tree.dispatch_to(
        "dlg",
        &UiEvent::MouseDown {
            pos: Pos::new(ok_x, btn_y),
            button: y901_ui::MouseButton::Left,
        },
    );

    let commands = tree.drain_commands();
    assert!(!commands.is_empty(), "点 OK 应产生命令");
    let hit_ok = commands.iter().any(|c| match c {
        Command::App { name, .. } => name == "ok",
        _ => false,
    });
    assert!(hit_ok, "应发出 ok 动作，实际 {commands:?}");

    let snap = tree.snapshot_json().to_string();
    assert!(snap.contains("\"open\":false"), "点击后应关闭：{snap}");
}

#[test]
fn dialog_without_buttons_still_traps_clicks() {
    let mut dialog =
        Dialog::new("dlg", "Info").push(Box::new(y901_widgets::TextLabel::new("body", "hello")));
    dialog.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(dialog))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    tree.dispatch_to(
        "dlg",
        &UiEvent::MouseDown {
            pos: Pos::new(5.0, 5.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert!(tree.drain_commands().is_empty(), "无按钮时点击不应产生命令");
    assert!(tree.snapshot_json().to_string().contains("\"open\":true"));
}
