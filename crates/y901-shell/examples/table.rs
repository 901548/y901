//! Table 示例：5 行 3 列，选中第 2 行。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::table::{Table, TableColumn};

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

fn row(name: &str, size: &str, mtime: &str) -> Vec<String> {
    vec![name.to_string(), size.to_string(), mtime.to_string()]
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let card = theme.style(Role::PANEL, State::Hovered).bg;

    let table = Table::new("files")
        .column(TableColumn::flex("名称"))
        .column(TableColumn::fixed("大小", 96.0).align(y901_ui::Align::End))
        .column(TableColumn::fixed("修改时间", 150.0))
        .row(row("screen.png", "1.2 MB", "2026-11-14 09:12"))
        .row(row("notes.md", "4 KB", "2026-11-13 22:40"))
        .row(row("setup.exe", "18 MB", "2026-11-10 08:05"))
        .row(row("diagram.svg", "96 KB", "2026-11-09 17:31"))
        .row(row("data.csv", "2.4 MB", "2026-11-02 11:58"))
        .selected(Some(1))
        .command(y901_ui::Command::app(
            "table:select",
            y901_ui::json::Json::Null,
        ))
        .flex_item(FlexItem::height(32.0 + 5.0 * 28.0));

    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(16.0)
        .push(Box::new(
            y901_widgets::TextLabel::new("title", "文件").size(20.0),
        ))
        .push(Box::new(
            Flex::column("card")
                .background(card)
                .padding(Edges::all(8.0))
                .push(Box::new(table)),
        ));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(H),
        Box::new(build),
        Options {
            width: 640,
            height: 320,
            ..Default::default()
        },
    )
}
