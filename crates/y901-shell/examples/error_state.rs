//! ErrorState 示例：只有标题 / 标题+详情+重试。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::error_state::ErrorState;

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, cmd: Command) -> bool {
        if let Command::App { name, args } = cmd {
            eprintln!("[example] command {} args={:?}", name, args);
        }
        true
    }
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
    let a = ErrorState::new("a", "加载失败").flex_item(FlexItem::height(140.0));
    let b = ErrorState::new("b", "同步失败")
        .description("连接超时，请检查网络后重试")
        .action("重试", y901_ui::Command::app("error_state:retry", y901_ui::json::Json::Null))
        .flex_item(FlexItem::height(220.0));
    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(16.0)
        .push(Box::new(a))
        .push(Box::new(b));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options {
            theme: Some(example_theme()), width: 520, height: 480, ..Default::default() })
}