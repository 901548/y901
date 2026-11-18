//! Popover 示例（临时）：点击锚点区弹出 Flex 容器内容。
use std::time::Instant;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Edges, Flex, FlexItem, Rect, Role, State, Theme, UiTree, Widget};
use y901_widgets::popover::Popover;
use y901_widgets::{Button, TextLabel};

struct H;
impl Host for H {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }
    fn command(&mut self, _t: &mut UiTree, _c: Command) -> bool {
        false
    }
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let card = theme.style(Role::PANEL, State::Hovered).bg;

    let pop = Popover::new("pop", Rect::new(24.0, 96.0, 220.0, 44.0), move || {
        Box::new(
            Flex::column("pop-content")
                .background(card)
                .radius(8.0)
                .padding(Edges::all(12.0))
                .gap(8.0)
                .push(Box::new(TextLabel::new("p-title", "导出设置").size(14.0)))
                .push(Box::new(
                    TextLabel::new("p-desc", "包含图片与元数据").size(12.0),
                ))
                .push(Box::new(
                    Button::new("p-ok", "导出").flex_item(FlexItem::height(28.0)),
                )),
        ) as Box<dyn Widget>
    });

    let root = Flex::column("root")
        .background(bg)
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(
            Button::new("trigger", "导出…").flex_item(FlexItem::height(32.0)),
        ))
        .push(Box::new(
            TextLabel::new("hint", "点击下方区域展开").size(12.0),
        ))
        .push(Box::new(pop));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(H),
        Box::new(build),
        Options {
            width: 560,
            height: 420,
            ..Default::default()
        },
    )
}
