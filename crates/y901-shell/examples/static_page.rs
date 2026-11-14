//! 极简静态页面 v4：观察**基础渲染质量 + 主题观感**。
//!
//! 只用 `Flex` + `TextLabel` + `TextInput` + `Button` + `Table`（不用 Grid / Image / ListView）。
//! 所有颜色来自 `Theme::DARK`；"弱化文字"在本页用 **中性灰**（从主题文字色派生），
//! 因为 `muted_text_color()` 的 (0.55,0.58,0.65) 在深底上会偏蓝，看起来像强调色而非说明文字。
//!
//! 运行：`cargo run --example static_page`

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{
    widget::Dimension, Align, Color, Command, Edges, Flex, FlexItem, Justify, Role, State, Theme,
    UiEvent, UiTree,
};
use y901_widgets::style::WidgetStyle;
use y901_widgets::table::{Table, TableColumn};
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

/// **中性灰**：把主题文字色按比例压暗（不引入硬编码 RGB）。
///
/// 为什么不用 `theme.muted_text_color()`：它是 (0.55,0.58,0.65)，B 通道比 R 高 0.10，
/// 在深色底上肉眼看起来是"淡蓝"而不是"灰"——说明文字不该有色彩倾向。
fn neutral_muted(theme: &Theme) -> Color {
    let fg = theme.style(Role::TEXT, State::Normal).fg;
    // 用**亮度**求灰：三通道取同一个值 → B-R = 0（彻底不偏蓝）。
    let l = (0.299 * fg.r + 0.587 * fg.g + 0.114 * fg.b) * 0.64;
    Color::rgba(l, l, l, 1.0)
}

/// 一行"标签（右对齐、固定 80）+ 输入框"。
fn form_row(
    id: &'static str,
    label: &'static str,
    value: &'static str,
    input_item: FlexItem,
    theme: &Theme,
    text_fg: Color,
) -> Flex {
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
    let card_bg = theme.style(Role::PANEL, State::Hovered).bg;
    let text_fg = theme.style(Role::TEXT, State::Normal).fg;
    let muted = neutral_muted(&theme);

    let divider = Flex::row("divider")
        .background(theme.divider_color())
        .flex_item(FlexItem::height(1.0));

    // ---- 设置卡片 ----
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
            text_fg,
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
            text_fg,
        )))
        .push(Box::new(form_row(
            "port",
            "端口",
            "8080",
            FlexItem::fixed(100.0, 32.0),
            &theme,
            text_fg,
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

    // ---- 连接历史卡片（用 Table 填满下半屏）----
    let history_table = Table::new("history")
        .column(TableColumn::flex("时间"))
        .column(TableColumn::fixed("状态", 80.0).align(Align::Center))
        .column(TableColumn::fixed("耗时", 80.0).align(Align::End))
        .row(vec![
            "2026-11-14 10:32".to_string(),
            "成功".to_string(),
            "2.3s".to_string(),
        ])
        .row(vec![
            "2026-11-14 09:15".to_string(),
            "失败".to_string(),
            "超时".to_string(),
        ])
        .row(vec![
            "2026-11-13 18:40".to_string(),
            "成功".to_string(),
            "1.8s".to_string(),
        ])
        .selected(Some(0))
        .flex_item(FlexItem::height(32.0 + 3.0 * 28.0));

    let history = Flex::column("history-card")
        .background(card_bg)
        .radius(8.0)
        .border(theme.divider_color(), 1.0)
        .padding(Edges::all(16.0))
        .gap(12.0)
        .push(Box::new(
            TextLabel::new("history-title", "连接历史")
                .size(13.0)
                .color(muted),
        ))
        .push(Box::new(history_table));

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
        .push(Box::new(history))
        // spacer：把状态文字推到底部
        .push(Box::new(Flex::row("spacer").flex_item(FlexItem {
            height: Dimension::Flex(1.0),
            ..FlexItem::NO_LIMITS
        })))
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