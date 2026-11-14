//! Popover 示例：点击按钮弹出一个带内容的浮层。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Rect, Role, State, Theme, UiEvent, UiTree};
use y901_widgets::popover::Popover;
use y901_widgets::{Button, TextLabel};

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 { if !tree.dispatch(&UiEvent::Tick).is_handled() { break; } }
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool { false }
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let content: Box<dyn y901_ui::Widget> = Box::new(
        Flex::column("pop-content")
            .gap(8.0)
            .push(Box::new(TextLabel::new("p-title", "导出设置").size(14.0)))
            .push(Box::new(TextLabel::new("p-desc", "包含图片与元数据").size(12.0)))
            .push(Box::new(Button::new("p-ok", "导出").flex_item(FlexItem::height(28.0)))),
    );
    let pop = Popover::new("pop", Rect::new(24.0, 64.0, 120.0, 32.0), content)
        .flex_item(FlexItem::fixed(300.0, 160.0));

    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(Button::new("trigger", "导出…").flex_item(FlexItem::height(32.0))))
        .push(Box::new(pop));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options { width: 520, height: 360, ..Default::default() })
}