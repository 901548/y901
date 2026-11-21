//! 最小示例：**只依赖 `y901` 这一个 crate**。
//!
//! ```text
//! cargo run --example hello
//! ```
//!
//! 和 `crates/y901-shell/examples/hello.rs` 是同一个程序，区别只在 **import 路径**：
//! 这里全部走 `y901::`（单一入口），用来证明"用户只装一个 crate 也能写出完整应用"。
//! 如果你的项目需要更细的依赖控制（比如不装平台后端），才直接依赖子 crate。

use y901::prelude::*;
use y901::ui::{Command, Edges, Flex, FlexItem, Json};

/// 极简 Host：处理按钮发来的**应用命令**。
struct DemoHost {
    clicks: u32,
}

impl Host for DemoHost {
    fn idle(&mut self, _tree: &mut UiTree) {}

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
}

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
    // 平台分叉在这里面完成：业务代码没有任何 `#[cfg(target_os)]`。
    y901::create_runtime().run(Box::new(DemoHost { clicks: 0 }), Box::new(build), options)
}
