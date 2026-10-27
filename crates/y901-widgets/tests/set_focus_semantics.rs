//! `UiTree::set_focus` 的语义条件。
//!
//! 返回 `true` 当且仅当：节点存在 + 在 Modal 作用域内 + 路径全 open +
//! 目标 focusable 且未禁用。与 `rebuild_indices` / `render_pass` / `cursor_at` /
//! `rebuild_focus_order` 的 `is_open` 跳过保持一致。

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiTree, Widget,
};
use y901_widgets::{Button, Dialog, TextInput};

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

/// 一个永不可聚焦的探针。
struct Plain {
    id: &'static str,
    rect: Rect,
}

impl Widget for Plain {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn tree_with_dialog(open: bool) -> UiTree {
    let mut dlg = Dialog::new("dlg", "Rename");
    dlg.push_child(Box::new(
        TextInput::new("i1").flex_item(FlexItem::height(24.0)),
    ));
    dlg.set_open(open);

    let root = Flex::column("root")
        .push(Box::new(dlg))
        .push(Box::new(
            Button::new("btn", "OK").flex_item(FlexItem::height(32.0)),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree
}

// ---------- 新语义 ----------

#[test]
fn set_focus_rejects_closed_dialog_input() {
    let mut tree = tree_with_dialog(false);
    assert_eq!(tree.focus(), None, "前置：无焦点");

    let ok = tree.set_focus(Some("i1".to_string()));
    assert!(!ok, "关闭的 Dialog 内部节点不可聚焦");
    assert_eq!(tree.focus(), None, "拒绝后焦点不能改变");
}

#[test]
fn set_focus_accepts_open_dialog_input() {
    let mut tree = tree_with_dialog(true);
    let ok = tree.set_focus(Some("i1".to_string()));
    assert!(ok, "打开的 Dialog 内部节点应可聚焦");
    assert_eq!(tree.focus(), Some("i1"));
}

#[test]
fn set_focus_rejects_non_focusable() {
    let root = Flex::column("root").push(Box::new(Plain {
        id: "plain",
        rect: Rect::ZERO,
    }));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let ok = tree.set_focus(Some("plain".to_string()));
    assert!(!ok, "focusable()==false 的节点不可聚焦");
    assert_eq!(tree.focus(), None);
}

#[test]
fn set_focus_rejects_disabled() {
    let root = Flex::column("root").push(Box::new(
        TextInput::new("i1").disabled(true).flex_item(FlexItem::height(24.0)),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let ok = tree.set_focus(Some("i1".to_string()));
    assert!(!ok, "禁用节点不可聚焦");
    assert_eq!(tree.focus(), None);
}

// ---------- 回归 ----------

#[test]
fn initial_focus_before_layout_works() {
    // main.rs 在**首次 layout 之前**调 set_focus；此时 focus_order 为空，
    // 但路径上所有节点 is_open == true → 必须成功。
    let root = Flex::column("root").push(Box::new(
        TextInput::new("title").flex_item(FlexItem::height(24.0)),
    ));
    let mut tree = UiTree::new(Box::new(root));

    let ok = tree.set_focus(Some("title".to_string()));
    assert!(ok, "首次 layout 之前的初始聚焦必须成功");
    assert_eq!(tree.focus(), Some("title"));

    // 之后 layout 不应把焦点弄丢
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert_eq!(tree.focus(), Some("title"));
}

#[test]
fn modal_focus_after_open_works() {
    // 打开 Modal 后框架会强制把焦点移进 Modal 子树（enforce_modal_focus），
    // 那条路径同样走 set_focus —— 必须成功。
    let mut tree = tree_with_dialog(false);
    let _ = tree.set_focus(Some("btn".to_string()));
    assert_eq!(tree.focus(), Some("btn"));

    // 打开对话框：Action{"open"} 不存在，直接改字段 + 重排（模拟 Host 的做法）
    {
        let dlg = tree
            .find_mut("dlg")
            .expect("dlg")
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Dialog>())
            .expect("Dialog downcast");
        dlg.set_open(true);
    }
    tree.mark_layout_dirty("dlg");
    tree.invalidate_all();
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 框架已把焦点限制进 Modal 子树
    let focus = tree.focus().map(|s| s.to_string());
    assert!(
        focus.as_deref() == Some("i1") || focus.as_deref() == Some("dlg"),
        "打开 Modal 后焦点必须进入其子树，实际 {focus:?}"
    );
    // 再显式设一次，必须成功
    let ok = tree.set_focus(Some("i1".to_string()));
    assert!(ok, "Modal 内部节点必须可聚焦");
    assert_eq!(tree.focus(), Some("i1"));
}
