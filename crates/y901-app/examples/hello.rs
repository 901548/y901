//! 最小示例：一个按钮 + 一行文字，点按钮改文字。
//!
//! 运行：
//! ```text
//! cargo run --example hello -p y901-app
//! ```
//!
//! 展示的是**用户会怎么写**：建树 → 声明交互（`Button::on_click` → 应用命令）→ 交给运行时。
//! 没有手写坐标，也没有绕开框架 API。

use y901_agent::Policy;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Color, Command, Edges, Flex, FlexItem, Json, UiEvent, UiTree};
use y901_widgets::{Button, TextLabel};

/// 极简 Host：驱动 Tick，并处理按钮产生的**应用命令**。
struct DemoHost {
    clicks: u32,
}

impl Host for DemoHost {
    fn idle(&mut self, tree: &mut UiTree) {
        // Tick 是组件心跳（动画、轮询）。没有组件需要时立即返回，不占 CPU。
        for _ in 0..4 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }
    }

    fn command(&mut self, tree: &mut UiTree, cmd: Command) -> bool {
        // 组件互不认识：按钮只发一条 `Command::App`，由应用层决定做什么。
        let Command::App { name, .. } = &cmd else {
            return false;
        };
        if name != "demo.click" {
            return false;
        }
        self.clicks += 1;
        if let Some(label) = tree.node_as_mut::<TextLabel>("label") {
            label.set_text(format!("你点了 {} 次", self.clicks));
        }
        tree.invalidate_all();
        true
    }

    fn policy(&self) -> Policy {
        Policy::default()
    }
}

/// 建树：一列（标题 + 按钮）。
fn build() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(24.0))
        .gap(12.0)
        .push(Box::new(
            TextLabel::new("label", "还没点过")
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(28.0)),
        ))
        .push(Box::new(
            Button::new("btn", "点我")
                .on_click("demo.click", Json::obj())
                .flex_item(FlexItem::height(36.0)),
        ));
    UiTree::new(Box::new(root))
}

fn main() -> Result<(), String> {
    let options = Options {
        title: "Y901 · hello".to_string(),
        width: 520,
        height: 220,
        ..Default::default()
    };
    create_runtime().run(Box::new(DemoHost { clicks: 0 }), Box::new(build), options)
}
