//! Skeleton 示例：列表骨架 / 卡片骨架 / 头像+文字。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{widget::Dimension, Align, Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::skeleton::Skeleton;

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool { false }
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let card = theme.style(Role::PANEL, State::Hovered).bg;

    let list = Skeleton::new("list")
        .rect(Dimension::Flex(1.0), 180.0, 8.0)
        .flex_item(FlexItem::fixed(220.0, 180.0));

    let rows = Skeleton::new("rows")
        .text(Dimension::Flex(1.0), 14.0)
        .text(Dimension::Flex(1.0), 14.0)
        .text(Dimension::Px(160.0), 14.0)
        .gap(10.0)
        .flex_item(FlexItem::fixed(240.0, 62.0));

    let avatar_text = Flex::row("avatar-row")
        .gap(12.0)
        .align(Align::Center)
        .push(Box::new(Skeleton::new("avatar").circle(40.0).flex_item(FlexItem::fixed(40.0, 40.0))))
        .push(Box::new(
            Skeleton::new("lines")
                .text(Dimension::Px(180.0), 12.0)
                .text(Dimension::Px(120.0), 12.0)
                .gap(8.0)
                .flex_item(FlexItem::fixed(200.0, 32.0)),
        ));

    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(24.0)
        .push(Box::new(
            Flex::row("top").gap(24.0)
                .push(Box::new(Flex::column("c1").background(card).padding(Edges::all(16.0)).push(Box::new(list))))
                .push(Box::new(Flex::column("c2").background(card).padding(Edges::all(16.0)).push(Box::new(rows)))),
        ))
        .push(Box::new(avatar_text));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options { width: 620, height: 320, ..Default::default() })
}