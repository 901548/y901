//! EmptyState 示例：两种变体（只有标题 / 标题+说明+按钮）。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::empty_state::EmptyState;

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

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let a = EmptyState::new("a", "暂无数据").flex_item(FlexItem::height(140.0));
    let b = EmptyState::new("b", "还没有文件")
        .description("点击上方按钮上传第一个文件")
        .action(
            "上传",
            y901_ui::Command::app("empty_state:activate", y901_ui::json::Json::Null),
        )
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
    create_runtime().run(
        Box::new(H),
        Box::new(build),
        Options {
            width: 520,
            height: 480,
            ..Default::default()
        },
    )
}
