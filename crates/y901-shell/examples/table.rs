//! Table 示例：**排序 + 虚拟滚动 + 键盘导航**。
//!
//! 演示三件事：
//! 1. 点击表头排序（升 → 降 → 取消，三态循环）；
//! 2. 2000 行数据下只渲染可见行（滚动时表头固定）；
//! 3. ↑↓ / Home / End / PageUp / PageDown 键盘选行，滚轮滚动。
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

fn row(name: &str, size: u64, mtime: &str) -> Vec<String> {
    vec![name.to_string(), size.to_string(), mtime.to_string()]
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let card = theme.style(Role::PANEL, State::Hovered).bg;

    // 合成 2000 行：足够触发虚拟滚动（200 高的可视区只画 ~7 行）。
    let mut table = Table::new("files")
        .column(TableColumn::flex("名称"))
        .column(TableColumn::fixed("大小", 96.0).align(y901_ui::Align::End))
        .column(TableColumn::fixed("修改时间", 150.0).not_sortable())
        .selected(Some(1))
        .command(Command::app("table:select", y901_ui::json::Json::Null))
        .sort_command(Command::app("table:sort", y901_ui::json::Json::Null))
        .flex_item(FlexItem::fill());
    let mut rows = Vec::with_capacity(2000);
    for i in 0..2000u64 {
        let name = format!("file-{i:04}.dat");
        let size = (i * 7919 % 100_000) + 12;
        let mtime = format!("2026-11-{:02} 09:{:02}", (i % 28) + 1, i % 60);
        rows.push(row(&name, size, &mtime));
    }
    table.set_rows(rows);

    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(
            y901_widgets::TextLabel::new("title", "文件（2000 行 · 点表头排序）").size(18.0),
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
            height: 420,
            ..Default::default()
        },
    )
}
