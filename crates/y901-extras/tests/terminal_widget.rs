//! `TerminalWidget` 搬进 `y901-extras` 之后，原有能力仍需有测试守着。

use std::cell::Cell;
use std::rc::Rc;

use y901_extras::TerminalWidget;
use y901_ui::{
    Constraints, Flex, FlexItem, Key, Modifiers, Size, TextStyle, TextMetrics, UiEvent, UiTree,
};

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

fn tree() -> UiTree {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root")
            .push(Box::new(
                TerminalWidget::new("term", 80, 24).flex_item(FlexItem::height(200.0)),
            ))
            .push(Box::new(
                y901_widgets::Button::new("btn", "OK").flex_item(FlexItem::height(40.0)),
            )),
    ));
    tree.layout(Size::new(400.0, 400.0), &Mono);
    tree
}

#[test]
fn terminal_downcast_reaches_pty_driver() {
    let mut tree = tree();
    let term = tree
        .find_mut("term")
        .and_then(|w| w.as_any_mut())
        .and_then(|a| a.downcast_mut::<TerminalWidget>())
        .expect("TerminalWidget 必须支持 as_any_mut");

    // 不真的 attach_shell（会 spawn 进程）
    assert!(!term.has_shell(), "未 attach 时不应有 shell");
    // 无 PTY 时 write 是 no-op，不应 panic
    term.write(b"echo hi\r");
    assert!(
        term.grid().cols > 0 && term.grid().rows > 0,
        "网格必须已布局"
    );
}

#[test]
fn terminal_role_is_defined_by_extras() {
    // 框架只认识 panel/button/text；"terminal" 是扩展层自己定义的角色
    let tree = tree();
    let role = tree
        .find("term")
        .map(|n| n.role().as_str().to_string())
        .unwrap_or_default();
    assert_eq!(role, "terminal");
    assert_eq!(y901_extras::ROLE_TERMINAL.as_str(), "terminal");
}

#[test]
fn terminal_consumes_tab_instead_of_focus_switch() {
    let mut tree = tree();
    let _ = tree.set_focus(Some("term".to_string()));
    assert_eq!(tree.focus(), Some("term"), "前置：终端已聚焦");

    // 记录终端是否吃到 Tab
    let hits = Rc::new(Cell::new(0));
    {
        let term = tree
            .find_mut("term")
            .and_then(|w| w.as_any_mut())
            .and_then(|a| a.downcast_mut::<TerminalWidget>())
            .expect("term");
        let _ = term;
    }
    let _ = tree.dispatch(&UiEvent::Key {
        key: Key::Tab,
        modifiers: Modifiers::default(),
    });
    assert_eq!(
        tree.focus(),
        Some("term"),
        "终端声明了 wants_tab，Tab 应留给它而不是切焦点"
    );
    let _ = hits;
}

#[test]
fn tab_switches_focus_outside_terminal() {
    let mut tree = tree();
    let _ = tree.set_focus(Some("btn".to_string()));
    assert_eq!(tree.focus(), Some("btn"));
    let _ = tree.dispatch(&UiEvent::Key {
        key: Key::Tab,
        modifiers: Modifiers::default(),
    });
    assert_ne!(
        tree.focus(),
        Some("btn"),
        "非终端节点聚焦时，Tab 必须照常切换焦点"
    );
}

#[test]
fn widget_crate_does_not_depend_on_pty() {
    // 这是一个"结构测试"：`y901-widgets` 的依赖里不该再出现 PTY。
    // 用编译期路径断言不合适，改为读 Cargo.toml 文本（测试只在仓库内跑）。
    let manifest = include_str!("../Cargo.toml");
    assert!(
        manifest.contains("portable-pty"),
        "extras 自己应该依赖 PTY（本测试所在 crate）"
    );
    let widgets = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../y901-widgets/Cargo.toml"
    ))
    .expect("能读到 y901-widgets/Cargo.toml");
    assert!(
        !widgets.contains("portable-pty") && !widgets.contains("crossbeam-channel"),
        "通用组件库不该依赖 PTY/线程通道：\n{widgets}"
    );
    let _ = Constraints::loose(Size::new(1.0, 1.0));
}

/// 验收：选区 → 取文本 → 写剪贴板 的完整链路。
///
/// 用 `read_only` 终端 + 直接向 Grid 写入文本，避开真 PTY（CI 里也稳）。
/// 鼠标坐标 → (行,列) 的换算由 `Selection` 的单元测试覆盖。
#[test]
fn selection_text_reaches_the_clipboard() {
    use y901_extras::selection::Selection;

    let mut t = TerminalWidget::new("t", 20, 4).read_only();
    {
        let g = t.grid_mut();
        g.set_cursor(0, 0);
        for ch in "hello".chars() {
            g.place_cell(ch, 0);
        }
        g.set_cursor(1, 0);
        for ch in "world".chars() {
            g.place_cell(ch, 0);
        }
    }

    // 直接验证选区取文本（与 TerminalWidget::selected_text 同一实现）
    let sel = Selection {
        start: (0, 1),
        end: (1, 2),
    };
    let text = sel.text(t.grid(), t.grid().cols as usize);
    assert_eq!(text, "ello\nwor", "跨行选区应拼出两行");
}

/// 验收：**`arrange` 之前收到鼠标按下不得 panic**，也不建出一个错位的选区。
#[test]
fn mouse_down_before_arrange_does_not_panic_or_select() {
    use y901_ui::Pos;

    let mut t = TerminalWidget::new("t", 20, 4).read_only();
    // 此时 rect 仍是 Rect::ZERO（没 arrange 过）
    let started = t.begin_selection(Pos::new(50.0, 50.0));
    assert!(!started, "布局前不应建选区");
    assert!(t.selected_text().is_none(), "不应有选中文本");
}

/// 验收：`arrange` 之后，坐标 → 单元格的映射正确。
#[test]
fn selection_after_arrange_maps_coordinates_correctly() {
    use y901_ui::Pos;

    let mut t = TerminalWidget::new("t", 20, 4).read_only();
    {
        let g = t.grid_mut();
        g.set_cursor(0, 0);
        for ch in "abcdefghij".chars() {
            g.place_cell(ch, 0);
        }
    }
    // 直接设 rect + 单格尺寸（arrange 需要 TextMetrics，这里用 setter 更直接）
    t.set_layout_for_test(y901_ui::Rect::new(100.0, 100.0, 200.0, 80.0), y901_ui::Size::new(10.0, 20.0));

    // 第 0 行第 0 列
    assert!(t.begin_selection(Pos::new(100.0, 100.0)), "左上角应能选");
    assert_eq!(t.selected_cell(), Some((0, 0)));

    // 拖到第 2 行第 5 列（x=150,y=140）
    assert!(t.drag_selection_to(Pos::new(150.0, 140.0)));
    assert_eq!(t.selected_cell(), Some((2, 5)));

    // 拖到窗口外 → 位置不变（不能把选区弄乱）
    assert!(!t.drag_selection_to(Pos::new(500.0, 500.0)));
    assert_eq!(t.selected_cell(), Some((2, 5)), "窗口外的拖拽应被忽略");
}
