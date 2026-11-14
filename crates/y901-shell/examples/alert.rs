//! Alert 示例：四种 kind 各一条。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Role, State, Theme, UiEvent, UiTree};
use y901_widgets::alert::{Alert, AlertKind};

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
    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(Alert::new("i", AlertKind::Info, "有新版本可用").dismissible(|| {}).flex_item(FlexItem::height(48.0))))
        .push(Box::new(Alert::new("s", AlertKind::Success, "保存成功").flex_item(FlexItem::height(48.0))))
        .push(Box::new(Alert::new("w", AlertKind::Warning, "磁盘空间不足").description("剩余 120 MB，建议清理缓存").dismissible(|| {}).flex_item(FlexItem::height(64.0))))
        .push(Box::new(Alert::new("e", AlertKind::Error, "连接已断开").description("请检查网络后重试").dismissible(|| {}).flex_item(FlexItem::height(64.0))));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(Box::new(H), Box::new(build), Options { width: 520, height: 340, ..Default::default() })
}