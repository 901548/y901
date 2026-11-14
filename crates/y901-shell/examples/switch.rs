//! Switch 示例：关 / 开 / 关（带标签）。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Align, Command, Edges, Flex, FlexItem, Role, State, Theme, UiEvent, UiTree};
use y901_widgets::switch::Switch;
use y901_widgets::TextLabel;

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 { if !tree.dispatch(&UiEvent::Tick).is_handled() { break; } }
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool { false }
}

fn row(id: &'static str, label: &'static str, on: bool) -> Flex {
    Flex::row(format!("{id}-row"))
        .gap(16.0)
        .align(Align::Center)
        .push(Box::new(TextLabel::new(format!("{id}-t"), label).size(14.0)))
        .push(Box::new(TextLabel::new(format!("{id}-sp"), "").flex_item(FlexItem {
            width: y901_ui::widget::Dimension::Flex(1.0),
            ..FlexItem::NO_LIMITS
        })))
        .push(Box::new(Switch::new(id, on).flex_item(FlexItem::fixed(44.0, 24.0))))
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(16.0)
        .push(Box::new(row("a", "自动保存", false)))
        .push(Box::new(row("b", "显示行号", true)))
        .push(Box::new(row("c", "同步到云端", false)));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options { width: 420, height: 240, ..Default::default() })
}