//! Alert 示例：四种 kind 各一条。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Role, State, Theme, UiTree};
use y901_widgets::alert::{Alert, AlertKind};

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
    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(
            Alert::new("i", AlertKind::Info, "有新版本可用")
                .on_dismiss(y901_ui::Command::app(
                    "alert:dismiss",
                    y901_ui::json::Json::Null,
                ))
                .flex_item(FlexItem::height(48.0)),
        ))
        .push(Box::new(
            Alert::new("s", AlertKind::Success, "保存成功").flex_item(FlexItem::height(48.0)),
        ))
        .push(Box::new(
            Alert::new("w", AlertKind::Warning, "磁盘空间不足")
                .description("剩余 120 MB，建议清理缓存")
                .on_dismiss(y901_ui::Command::app(
                    "alert:dismiss",
                    y901_ui::json::Json::Null,
                ))
                .flex_item(FlexItem::height(64.0)),
        ))
        // STYLE-OVERRIDE：验证 WidgetStyle::bg 确实覆盖默认面板底色
        .push(Box::new(
            Alert::new("styled", AlertKind::Info, "style 覆盖底色")
                .style(
                    y901_widgets::WidgetStyle::default()
                        .bg(y901_ui::Color::rgba(0.10, 0.45, 0.85, 1.0)),
                )
                .flex_item(FlexItem::height(48.0)),
        ))
        .push(Box::new(
            Alert::new("e", AlertKind::Error, "连接已断开")
                .description("请检查网络后重试")
                .on_dismiss(y901_ui::Command::app(
                    "alert:dismiss",
                    y901_ui::json::Json::Null,
                ))
                .flex_item(FlexItem::height(64.0)),
        ));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(H),
        Box::new(build),
        Options {
            theme: Some(example_theme()),
            width: 520,
            height: 340,
            ..Default::default()
        },
    )
}
