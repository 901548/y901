//! 极简静态页面 v2：观察**基础渲染质量 + 主题观感**。
//!
//! 只用 `Flex` + `TextLabel` + `TextInput` + `Button`（不用 Grid / Image / ListView）。
//! 所有颜色来自 `Theme::DARK`（含 `muted_text_color()` / `divider_color()`），
//! 所有样式来自 `WidgetStyle` 的语义构造器（`input_field` / `primary_button` / ...）。
//!
//! 运行：`cargo run --example static_page`

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{
    widget::Dimension, Align, Command, Edges, Flex, FlexItem, Justify, Role, State, Theme, UiEvent,
    UiTree,
};
use y901_widgets::style::WidgetStyle;
use y901_widgets::{Button, TextInput, TextLabel};

struct Shell;

impl Host for Shell {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }
    }
    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false
    }
}

/// 一行"标签（右对齐、固定 80）+ 输入框"。
fn form_row(
    id: &'static str,
    label: &'static str,
    value: &'static str,
    input_item: FlexItem,
    theme: &Theme,
) -> Flex {
    let text_fg = theme.style(Role::TEXT, State::Normal).fg;
    let label_cell = Flex::row(format!("{id}-lbl"))
        .justify(Justify::End)
        .flex_item(FlexItem::width(80.0))
        .push(Box::new(
            TextLabel::new(format!("{id}-lbl-t"), label)
                .size(14.0)
                .color(text_fg),
        ));

    Flex::row(format!("{id}-row"))
        .gap(16.0)
        .align(Align::Center)
        .push(Box::new(label_cell))
        .push(Box::new(
            TextInput::new(format!("{id}-in"))
                .with_text(value)
                .widget_style(WidgetStyle::input_field(theme))
                .flex_item(input_item),
        ))
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let panel_bg = theme.style(Role::PANEL, State::Normal).bg;
    // 卡片：比 panel 亮一档（同一个槽位的 Hovered 态），不新增槽位。
    let card_bg = theme.style(Role::PANEL, State::Hovered).bg;
    let text_fg = theme.style(Role::TEXT, State::Normal).fg;
    let muted = theme.muted_text_color();

    let divider = Flex::row("divider")
        .background(theme.divider_color())
        .flex_item(FlexItem::height(1.0));

    let card = Flex::column("card")
        .background(card_bg)
        .radius(8.0)
        .border(theme.divider_color(), 1.0)
        .padding(Edges::all(20.0))
        .gap(12.0)
        .push(Box::new(form_row(
            "user",
            "用户名",
            "admin",
            FlexItem {
                width: Dimension::Flex(1.0),
                height: Dimension::Px(32.0),
                ..FlexItem::NO_LIMITS
            },
            &theme,
        )))
        .push(Box::new(form_row(
            "pass",
            "密码",
            "••••••••",
            FlexItem {
                width: Dimension::Flex(1.0),
                height: Dimension::Px(32.0),
                ..FlexItem::NO_LIMITS
            },
            &theme,
        )))
        .push(Box::new(form_row(
            "port",
            "端口",
            "8080",
            FlexItem::fixed(100.0, 32.0),
            &theme,
        )))
        .push(Box::new(
            Flex::row("actions")
                .gap(8.0)
                .justify(Justify::End)
                .push(Box::new(
                    Button::new("cancel", "取消")
                        .style(WidgetStyle::secondary_button(&theme))
                        .flex_item(FlexItem::height(32.0)),
                ))
                .push(Box::new(
                    Button::new("save", "保存")
                        .style(WidgetStyle::primary_button(&theme))
                        .flex_item(FlexItem::height(32.0)),
                )),
        ));

    let root = Flex::column("root")
        .background(panel_bg)
        .padding(Edges::all(24.0))
        .gap(16.0)
        .push(Box::new(
            TextLabel::new("title", "设置")
                .size(24.0)
                .bold()
                .color(text_fg),
        ))
        .push(Box::new(
            TextLabel::new("hint", "修改后点击保存生效")
                .size(13.0)
                .color(muted),
        ))
        .push(Box::new(divider))
        .push(Box::new(card))
        // spacer：把状态文字推到底部
        .push(Box::new(
            Flex::row("spacer").flex_item(FlexItem {
                height: Dimension::Flex(1.0),
                ..FlexItem::NO_LIMITS
            }),
        ))
        .push(Box::new(
            TextLabel::new("status", "上次保存：2026-11-14 10:30")
                .size(12.0)
                .color(muted),
        ));

    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(Shell),
        Box::new(build),
        Options {
            title: "Y901 · static page".to_string(),
            width: 800,
            height: 600,
            ..Default::default()
        },
    )
}