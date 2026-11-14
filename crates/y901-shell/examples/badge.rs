//! Badge 示例：五种 kind + 纯圆点。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Align, Command, Edges, Flex, FlexItem, Role, State, Theme, UiEvent, UiTree};
use y901_widgets::badge::{Badge, BadgeKind};

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 { if !tree.dispatch(&UiEvent::Tick).is_handled() { break; } }
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool { false }
}

/// 示例主题：`Y901_THEME=light` 时用 LIGHT，否则 DARK。
fn example_theme() -> Theme {
    match std::env::var("Y901_THEME").ok().as_deref() {
        Some("light") => y901_ui::theme::LIGHT,
        _ => Theme::DARK,
    }
}

fn build() -> UiTree {
    let theme = example_theme();
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let root = Flex::row("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(12.0)
        .align(Align::Center)
        .push(Box::new(Badge::new("d", "默认").kind(BadgeKind::Default)))
        .push(Box::new(Badge::new("a", "Beta").kind(BadgeKind::Accent)))
        .push(Box::new(Badge::new("s", "成功").kind(BadgeKind::Success)))
        .push(Box::new(Badge::new("w", "警告").kind(BadgeKind::Warning)))
        .push(Box::new(Badge::new("e", "失败").kind(BadgeKind::Error)))
        .push(Box::new(Badge::dot("dot-e").kind(BadgeKind::Error).flex_item(FlexItem::fixed(8.0, 8.0))))
        .push(Box::new(y901_widgets::TextLabel::new("t", "← 未读圆点").size(12.0)));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options {
            theme: Some(example_theme()), width: 560, height: 120, ..Default::default() })
}