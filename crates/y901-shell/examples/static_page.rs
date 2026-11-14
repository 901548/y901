//! 极简静态页面：只为观察**基础渲染质量**（文字清晰度、对齐、间距、颜色、圆角）。
//!
//! 刻意只用 `Flex` + `TextLabel` + `TextInput` + `Button`：不用 Grid / Image / ListView。
//! 运行：`cargo run --example static_page`
//!
//! 规格：800x600、padding 24、gap 16、字号 24/14/13/12、控件高 32、尺寸取 4/8/12/16/24。

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{
    widget::Dimension, Align, Color, Command, Edges, Flex, FlexItem, Justify, Role, State, Theme,
    UiEvent, UiTree,
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

/// 调色板：**全部来自 `Theme::DARK`**，不硬编码色值。
struct Palette {
    bg: Color,
    fg: Color,
    muted: Color,
    border: Color,
    accent: Color,
}

fn palette() -> Palette {
    let panel = Theme::DARK.style(Role::PANEL, State::Normal);
    let button = Theme::DARK.style(Role::BUTTON, State::Normal);
    Palette {
        bg: panel.bg,
        fg: panel.fg,
        muted: panel.border,
        border: panel.border,
        accent: button.bg,
    }
}

/// 一行"标签（右对齐、固定宽）+ 输入框"。
///
/// 注：`TextLabel` 没有 align 参数，所以右对齐是用"宽度固定的行 + `justify(End)`"实现的；
/// 这是本次唯一一处 API 不顺手的地方（已在报告里记录）。
fn form_row(
    id: &'static str,
    label: &'static str,
    value: &'static str,
    input: FlexItem,
    p: &Palette,
) -> Flex {
    let label_cell = Flex::row(format!("{id}-lbl"))
        .justify(Justify::End)
        .flex_item(FlexItem::width(80.0))
        .push(Box::new(TextLabel::new(format!("{id}-lbl-t"), label).size(14.0).color(p.fg)));

    Flex::row(format!("{id}-row"))
        .gap(16.0)
        .align(Align::Center)
        .push(Box::new(label_cell))
        .push(Box::new(
            TextInput::new(format!("{id}-in"))
                .with_text(value)
                .flex_item(input),
        ))
}

fn build() -> UiTree {
    let p = palette();

    // 1px 分割线：用"1px 高的 Flex + 背景色"表达（框架暂无 Divider 组件）。
    let divider = Flex::row("divider")
        .background(p.border)
        .flex_item(FlexItem::height(1.0));

    let primary = WidgetStyle::default().bg(p.accent).radius(6.0);
    let secondary = WidgetStyle::default()
        .bg(p.muted)
        .radius(6.0)
        .border(p.border, 1.0);

    let root = Flex::column("root")
        .background(p.bg)
        .padding(Edges::all(24.0))
        .gap(16.0)
        // 1. 标题
        .push(Box::new(
            TextLabel::new("title", "设置")
                .size(24.0)
                .bold()
                .color(p.fg),
        ))
        // 2. 说明
        .push(Box::new(
            TextLabel::new("hint", "修改后点击保存生效")
                .size(13.0)
                .color(p.muted),
        ))
        // 3. 分割线
        .push(Box::new(divider))
        // 4. 表单区
        .push(Box::new(
            Flex::column("form")
                .gap(12.0)
                .push(Box::new(form_row(
                    "user",
                    "用户名",
                    "admin",
                    FlexItem { width: Dimension::Flex(1.0), height: Dimension::Px(32.0), ..FlexItem::NO_LIMITS },
                    &p,
                )))
                .push(Box::new(form_row(
                    "pass",
                    "密码",
                    "••••••••",
                    FlexItem { width: Dimension::Flex(1.0), height: Dimension::Px(32.0), ..FlexItem::NO_LIMITS },
                    &p,
                )))
                .push(Box::new(form_row(
                    "port",
                    "端口",
                    "8080",
                    FlexItem::fixed(100.0, 32.0),
                    &p,
                ))),
        ))
        // 5. 按钮行（右对齐）
        .push(Box::new(
            Flex::row("actions")
                .gap(8.0)
                .justify(Justify::End)
                .push(Box::new(
                    Button::new("cancel", "取消")
                        .style(secondary)
                        .flex_item(FlexItem::height(32.0)),
                ))
                .push(Box::new(
                    Button::new("save", "保存")
                        .style(primary)
                        .flex_item(FlexItem::height(32.0)),
                )),
        ))
        // 6. 状态文字
        .push(Box::new(
            TextLabel::new("status", "上次保存：2026-11-14 10:30")
                .size(12.0)
                .color(p.muted),
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