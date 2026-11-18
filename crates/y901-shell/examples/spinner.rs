//! Spinner 示例：三个不同直径的旋转指示器。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Align, Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::spinner::Spinner;
use y901_widgets::TextLabel;

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool {
        false
    }
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let root = Flex::row("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(24.0)
        .align(Align::Center)
        .push(Box::new(
            Spinner::new("s1")
                .size(16.0)
                .flex_item(FlexItem::fixed(16.0, 16.0)),
        ))
        .push(Box::new(
            Spinner::new("s2")
                .size(32.0)
                .flex_item(FlexItem::fixed(32.0, 32.0)),
        ))
        .push(Box::new(
            Spinner::new("s3")
                .size(48.0)
                .flex_item(FlexItem::fixed(48.0, 48.0)),
        ))
        .push(Box::new(TextLabel::new("t", "正在加载…").size(14.0)));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(H),
        Box::new(build),
        Options {
            width: 420,
            height: 160,
            ..Default::default()
        },
    )
}
