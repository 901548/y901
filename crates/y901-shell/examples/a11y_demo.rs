//! **无障碍验收演示页**（配合 [`docs/A11Y_ACCEPTANCE.md`](../../../docs/A11Y_ACCEPTANCE.md)）。
//!
//! 目的：给"真机读屏器验收"一个**固定靶子**。页面上每个控件都有明确的
//! label / 状态 / 值域，验收时只需照着清单核对读屏器念出来的内容，而不是
//! 自己临时搭界面。
//!
//! 运行：
//! ```text
//! cargo run -p y901-shell --features a11y --example a11y_demo
//! ```
//! （`--features a11y` 会把无障碍桥转发到平台后端：Windows UIA / Linux AT-SPI。
//! 不加也能跑，但读屏器看不到东西 —— 这正是要验收的部分。）
//!
//! ## 期望（读屏器应当念出）
//!
//! | 控件 | 期望播报 |
//! |---|---|
//! | 标题 | "Y901 无障碍验收" |
//! | 按钮 | 按钮 + 名称"保存（Ctrl+S）" |
//! | 复选框 | 复选框 + 名称 + 勾选/未勾选（点一下要变） |
//! | 开关 | 开关 + 开/关（点一下要变） |
//! | 滑杆 | 滑杆 + 名称 + 当前值 + 最小/最大 |
//! | 数值输入 | 数值输入 + 名称 + 当前值 + 范围 |
//! | 文本框 | 文本框 + 名称 + 当前文本 |
//! | 列表 | 列表 + 项数，逐项念出名称 |
//! | 告警 | 告警 + 标题 + 描述（assertive：应主动播报） |
//! | 状态行 | 点击按钮后变为"已保存 N 次"（polite 实时区域） |
//!
//! 注意：本页**不追求好看**，只追求每个控件都能被单独定位与核对。

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Align, Command, Edges, Flex, FlexItem, Json, Role, State, Theme, UiTree};
use y901_widgets::{
    Alert, AlertKind, Button, Checkbox, NumberInput, Slider, Switch, TextInput, TextLabel,
    VerticalList,
};

/// 演示用的 Host：只处理"保存"按钮，把次数写回状态行。
struct A11yHost {
    saves: u32,
}

impl Host for A11yHost {
    fn idle(&mut self, _tree: &mut UiTree) {}

    fn command(&mut self, tree: &mut UiTree, cmd: Command) -> bool {
        let Command::App { name, .. } = &cmd else {
            return false;
        };
        match name.as_str() {
            "a11y.save" => {
                self.saves += 1;
                if let Some(status) = tree.node_as_mut::<TextLabel>("status") {
                    status.set_text(format!("已保存 {} 次", self.saves));
                }
                tree.invalidate_all();
                true
            }
            _ => false,
        }
    }
}

/// 一行"标签 + 控件"，保证读屏器能按标签定位控件。
fn field(
    id: &'static str,
    label: &'static str,
    control: Box<dyn y901_ui::Widget>,
    fg: y901_ui::Color,
) -> Flex {
    Flex::row(format!("{id}-row"))
        .gap(12.0)
        .align(Align::Center)
        .flex_item(FlexItem::height(36.0))
        .push(Box::new(
            Flex::row(format!("{id}-lbl"))
                .flex_item(FlexItem::width(140.0))
                .push(Box::new(
                    TextLabel::new(format!("{id}-lbl-t"), label).color(fg),
                )),
        ))
        .push(Box::new(
            Flex::row(format!("{id}-slot"))
                .flex_item(FlexItem::width(240.0))
                .push(control),
        ))
}

fn build() -> UiTree {
    let theme = Theme::DARK;
    let bg = theme.style(Role::PANEL, State::Normal).bg;
    let card = theme.style(Role::PANEL, State::Hovered).bg;
    let fg = theme.style(Role::TEXT, State::Normal).fg;

    // 演示列表：用 VerticalList 承载若干"行"，让读屏器能念出列表与项数。
    let mut list = VerticalList::new("a11y-list")
        .label("待办事项")
        .flex_item(FlexItem::height(96.0));
    for i in 1..=3 {
        list = list.push(Box::new(
            TextLabel::new(format!("item-{i}"), format!("列表项 {i}")).color(fg),
        ));
    }

    let body = Flex::column("body")
        .gap(10.0)
        .padding(Edges::all(16.0))
        .background(card)
        .radius(8.0)
        .push(Box::new(
            TextLabel::new("title", "Y901 无障碍验收")
                .size(20.0)
                .color(fg)
                .flex_item(FlexItem::height(32.0)),
        ))
        .push(Box::new(
            TextLabel::new(
                "hint",
                "本页用于读屏器手工验收；每个控件都有明确标签与状态。",
            )
            .color(theme.muted_text_color())
            .flex_item(FlexItem::height(24.0)),
        ))
        .push(Box::new(field(
            "save",
            "按钮",
            Box::new(
                Button::new("save-btn", "保存（Ctrl+S）")
                    .on_click("a11y.save", Json::obj())
                    .flex_item(FlexItem::height(32.0)),
            ),
            fg,
        )))
        .push(Box::new(field(
            "check",
            "复选框",
            Box::new(Checkbox::new("check-box", "启用同步", false)),
            fg,
        )))
        .push(Box::new(field(
            "switch",
            "开关",
            Box::new(Switch::new("switch-ctl", true).label("启用同步")),
            fg,
        )))
        .push(Box::new(field(
            "slider",
            "滑杆",
            Box::new(Slider::new("slider-ctl", 0.5).label("音量")),
            fg,
        )))
        .push(Box::new(field(
            "number",
            "数值输入",
            Box::new(
                NumberInput::new("number-ctl", 42.0)
                    .label("重试次数")
                    .range(0.0, 100.0)
                    .step(1.0)
                    .flex_item(FlexItem::height(32.0)),
            ),
            fg,
        )))
        .push(Box::new(field(
            "text",
            "文本框",
            Box::new(
                TextInput::new("text-ctl")
                    .label("备注")
                    .with_text("可编辑文本")
                    .flex_item(FlexItem::height(32.0)),
            ),
            fg,
        )))
        .push(Box::new(field(
            "list",
            "列表",
            Box::new(list.flex_item(FlexItem::height(96.0))),
            fg,
        )))
        .push(Box::new(
            Alert::new("alert", AlertKind::Warning, "这是一个告警")
                .description("读屏器应主动播报（aria-live=assertive）")
                .flex_item(FlexItem::height(64.0)),
        ))
        .push(Box::new(
            TextLabel::new("status", "尚未保存")
                .color(theme.muted_text_color())
                .flex_item(FlexItem::height(24.0)),
        ));

    UiTree::new(Box::new(
        Flex::column("root")
            .background(bg)
            .padding(Edges::all(20.0))
            .push(Box::new(body)),
    ))
}

fn main() -> Result<(), String> {
    #[cfg(all(feature = "a11y", target_os = "windows"))]
    eprintln!("[a11y_demo] Windows UIA 桥已启用：请用「讲述人」(Win+Ctrl+Enter) 验收");
    #[cfg(all(feature = "a11y", target_os = "linux"))]
    eprintln!("[a11y_demo] Linux AT-SPI 桥已启用：请用 Orca 验收");
    #[cfg(not(feature = "a11y"))]
    eprintln!(
        "[a11y_demo] 未启用 a11y feature：读屏器看不到任何内容。\
         请用 `cargo run -p y901-shell --features a11y --example a11y_demo`"
    );
    create_runtime().run(
        Box::new(A11yHost { saves: 0 }),
        Box::new(build),
        Options {
            title: "Y901 无障碍验收".to_string(),
            width: 560,
            height: 640,
            ..Default::default()
        },
    )
}
