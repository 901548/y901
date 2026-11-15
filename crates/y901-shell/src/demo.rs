//! 演示页面：**桌面示例与 wasm 共用同一份 UI**。
//!
//! 从 `examples/static_page.rs` 原样提取，目的是让
//! ① 示例不再自己维护一份页面；
//! ② `docs/WASM.md` §5 的"帧哈希跨后端对拍"有**同一个 UI 源**。

use y901_ui::widget::Dimension;
use y901_ui::theme::Theme;
use y901_ui::{Align, Color, Edges, Flex, FlexItem, Justify, Role, State, UiTree};
use y901_widgets::style::WidgetStyle;
use y901_widgets::table::{Table, TableColumn};
use y901_widgets::{Button, TextInput, TextLabel};
/// 构建演示页面（桌面示例与 wasm 页面共用同一份 UI）。
pub fn build_demo_page() -> UiTree {
    let theme = Theme::DARK;
    let panel_bg = theme.style(Role::PANEL, State::Normal).bg;
    let card_bg = theme.style(Role::PANEL, State::Hovered).bg;
    let text_fg = theme.style(Role::TEXT, State::Normal).fg;
    let muted = theme.muted_text_color();

    let divider = Flex::row("divider")
        .background(theme.divider_color())
        .flex_item(FlexItem::height(1.0));

    // ---- 设置卡片 ----
    let card = Flex::column("card")
        .background(card_bg)
        .radius(8.0)
        .border(theme.divider_color(), 1.0)
        // **padding 分档（有意设计，不是遗漏）**：
        //   16 = 密集列表（历史/行表）
        //   20 = 常规卡片
        //   24 = 页面边距
        // 不同层级用不同节奏；不要为了"统一"而抹平层级差异。
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

/// 表单一行（标签 + 控件）。
pub(crate) fn form_row(
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
