//! ComboBox 示例：3 / 8 / 15 项的三种下拉框。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Align, Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::combo_box::ComboBox;
use y901_widgets::TextLabel;

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

fn row(id: &'static str, label: &'static str, items: Vec<String>, sel: Option<usize>) -> Flex {
    Flex::row(format!("{id}-row"))
        .gap(12.0)
        .align(Align::Center)
        .push(Box::new(TextLabel::new(format!("{id}-t"), label).size(13.0)))
        .push(Box::new(
            ComboBox::new(id, items)
                .selected(sel)
                .command(y901_ui::Command::app("combo_box:select", y901_ui::json::Json::Null))
                .flex_item(FlexItem { height: y901_ui::widget::Dimension::Px(32.0), ..FlexItem::NO_LIMITS })
                .flex_item(FlexItem::fixed(260.0, 32.0)),
        ))
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let few: Vec<String> = vec!["低".into(), "中".into(), "高".into()];
    let eight: Vec<String> = (1..=8).map(|i| format!("选项 {i}")).collect();
    let many: Vec<String> = (1..=15).map(|i| format!("项目 {i}")).collect();
    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(28.0)
        .push(Box::new(row("few", "优先级", few, Some(1))))
        .push(Box::new(row("eight", "并行度", eight, Some(3))))
        .push(Box::new(row("many", "目标", many, None)));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options { width: 520, height: 320, ..Default::default() })
}