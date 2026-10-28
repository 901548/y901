//! 审计：有 `&mut self` 运行时方法的组件必须能 downcast（`as_any_mut`）。
//!
//! 全部走真实路径 `tree.find_mut(id) → as_any_mut → downcast_mut::<T>()`。

use y901_ui::{
    Constraints, Flex, FlexItem, Rect, Size, TextStyle, TextMetrics, UiTree,
};
use y901_widgets::{ListView, Menu, TerminalWidget, TextLabel};

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

fn tree_with(child: Box<dyn y901_ui::Widget>) -> UiTree {
    let root = Flex::column("root").push(child);
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(600.0, 400.0), &Mono);
    tree
}

#[test]
fn text_label_runtime_set_text_via_downcast() {
    let mut tree = tree_with(Box::new(
        TextLabel::new("lbl", "old").flex_item(FlexItem::height(20.0)),
    ));

    let changed = {
        let mut done = false;
        if let Some(label) = tree
            .find_mut("lbl")
            .and_then(|w| w.as_any_mut())
            .and_then(|a| a.downcast_mut::<TextLabel>())
        {
            label.set_text("new text");
            done = true;
        }
        done
    };
    assert!(changed, "TextLabel 必须支持 as_any_mut");

    // 读回：语义 label 反映新文本（组件自己不会自动标脏，这里直接问语义）
    let sem = tree.find("lbl").expect("lbl").semantics();
    assert_eq!(sem.label, "new text");
}

#[test]
fn list_view_runtime_set_items_via_downcast() {
    let mut tree = tree_with(Box::new(
        ListView::new("list", 20.0).flex_item(FlexItem::fill()),
    ));


    if let Some(lv) = tree
        .find_mut("list")
        .and_then(|w| w.as_any_mut())
        .and_then(|a| a.downcast_mut::<ListView>())
    {
        assert_eq!(lv.row_count(), 0, "初始没有行");
        lv.set_items(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(lv.row_count(), 3, "set_items 之后行数必须变化");
    } else {
        panic!("ListView 必须支持 as_any_mut");
    }
}

#[test]
fn menu_open_close_via_downcast() {
    let mut tree = tree_with(Box::new(
        Menu::new("menu").items(vec!["Open".into(), "Quit".into()]),
    ));

    let menu = tree
        .find_mut("menu")
        .and_then(|w| w.as_any_mut())
        .and_then(|a| a.downcast_mut::<Menu>())
        .expect("Menu 必须支持 as_any_mut");

    assert!(!menu.is_open(), "默认关闭");
    menu.open();
    assert!(menu.is_open(), "open() 之后必须打开");
    menu.close();
    assert!(!menu.is_open(), "close() 之后必须关闭");
}

#[test]
fn terminal_downcast_reaches_pty_driver() {
    let mut tree = tree_with(Box::new(
        TerminalWidget::new("term", 80, 24).flex_item(FlexItem::fill()),
    ));

    let term = tree
        .find_mut("term")
        .and_then(|w| w.as_any_mut())
        .and_then(|a| a.downcast_mut::<TerminalWidget>())
        .expect("TerminalWidget 必须支持 as_any_mut");

    // 不真的 attach_shell（那会 spawn 进程，CI 环境不适合）
    assert!(!term.has_shell(), "未 attach 时不应有 shell");
    // 只读驱动接口可达即可（write 在没有 PTY 时是 no-op，不 panic）
    term.write(b"echo hi\r");
    // 注意：cols 由 arrange 按矩形宽度重算（600px / 8px ≈ 75），不是构造时的 80
    assert!(term.grid().cols > 0 && term.grid().rows > 0, "网格必须已布局");
}

#[test]
fn downcast_to_wrong_type_returns_none() {
    let mut tree = tree_with(Box::new(
        TextLabel::new("lbl", "x").flex_item(FlexItem::height(20.0)),
    ));
    let any = tree
        .find_mut("lbl")
        .and_then(|w| w.as_any_mut())
        .expect("TextLabel 支持 as_any_mut");
    assert!(
        any.downcast_mut::<ListView>().is_none(),
        "downcast 到错误类型必须返回 None（不是 panic）"
    );
}

#[test]
fn widgets_layout_uses_real_rects() {
    // 兜底：确保上面这些组件在布局里真的拿到矩形（否则 downcast 测试可能是空跑）
    let mut tree = tree_with(Box::new(
        TextLabel::new("lbl", "x").flex_item(FlexItem::height(20.0)),
    ));
    tree.layout(Size::new(600.0, 400.0), &Mono);
    let r: Rect = tree.rect_of("lbl").expect("lbl 有矩形");
    assert!(r.h > 0.0 && !r.intersect(r).is_empty());
    let _ = Constraints::loose(Size::new(1.0, 1.0));
}
