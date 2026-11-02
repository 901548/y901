//! 状态 + 动画示例：计数器，数字变化时进度条平滑跟随。
//!
//! 运行：
//! ```text
//! cargo run --example counter -p y901-app
//! ```
//!
//! 展示三件事：
//! 1. `Flex` 布局 + `ProgressBar` / `Button` 组件组合；
//! 2. **应用状态**在 `Host` 里，不在组件里（组件只声明"我被点了"）；
//! 3. 数字与进度条同步更新。

use y901_agent::Policy;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Color, Command, Edges, Flex, FlexItem, Json, UiEvent, UiTree};
use y901_widgets::{Button, ProgressBar, TextLabel};

/// 计数的应用状态。
struct CounterApp {
    value: i32,
}

impl CounterApp {
    fn max() -> i32 {
        100
    }

    fn apply(&self, tree: &mut UiTree) {
        let ratio = (self.value.max(0) as f32 / Self::max() as f32).clamp(0.0, 1.0);
        if let Some(bar) = tree.node_as_mut::<ProgressBar>("bar") {
            bar.set_value(ratio);
        }
        if let Some(label) = tree.node_as_mut::<TextLabel>("value") {
            label.set_text(format!("{} / {}", self.value.max(0), Self::max()));
        }
        tree.invalidate_all();
    }
}

impl Host for CounterApp {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }
    }

    fn command(&mut self, tree: &mut UiTree, cmd: Command) -> bool {
        let Command::App { name, .. } = &cmd else {
            return false;
        };
        match name.as_str() {
            "counter.inc" => self.value = (self.value + 10).min(Self::max()),
            "counter.dec" => self.value = (self.value - 10).max(0),
            "counter.reset" => self.value = 0,
            _ => return false,
        }
        self.apply(tree);
        true
    }

    fn policy(&self) -> Policy {
        Policy::default()
    }
}

fn build() -> UiTree {
    let buttons = Flex::row("buttons")
        .gap(8.0)
        .flex_item(FlexItem::height(36.0))
        .push(Box::new(
            Button::new("dec", "−10")
                .on_click("counter.dec", Json::obj())
                .flex_item(FlexItem::width(90.0)),
        ))
        .push(Box::new(
            Button::new("inc", "+10")
                .on_click("counter.inc", Json::obj())
                .flex_item(FlexItem::width(90.0)),
        ))
        .push(Box::new(
            Button::new("reset", "重置")
                .on_click("counter.reset", Json::obj())
                .flex_item(FlexItem::width(90.0)),
        ));

    let root = Flex::column("root")
        .background(Color::from_rgb8(14, 14, 20))
        .padding(Edges::all(24.0))
        .gap(14.0)
        .push(Box::new(
            TextLabel::new("value", "0 / 100")
                .color(Color::rgba(0.9, 0.94, 1.0, 1.0))
                .flex_item(FlexItem::height(32.0)),
        ))
        .push(Box::new(
            ProgressBar::new("bar", 0.0).flex_item(FlexItem::height(20.0)),
        ))
        .push(Box::new(buttons));

    // 打开窗口后立刻显示正确初值
    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    tree
}

fn main() -> Result<(), String> {
    let options = Options {
        title: "Y901 · counter".to_string(),
        width: 520,
        height: 260,
        ..Default::default()
    };
    create_runtime().run(Box::new(CounterApp { value: 0 }), Box::new(build), options)
}
