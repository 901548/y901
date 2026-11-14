//! ErrorState 示例：只有标题 / 标题+详情+重试。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Role, State, Theme, UiEvent, UiTree};
use y901_widgets::error_state::ErrorState;

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
    let a = ErrorState::new("a", "加载失败").flex_item(FlexItem::height(140.0));
    let b = ErrorState::new("b", "同步失败")
        .description("连接超时，请检查网络后重试")
        .action("重试", || {})
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
    create_runtime().run(Box::new(H), Box::new(build), Options { width: 520, height: 480, ..Default::default() })
}