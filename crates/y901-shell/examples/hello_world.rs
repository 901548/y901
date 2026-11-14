//! Y901 最小示例：建树 + 平台运行时 + 跑事件循环。
//!
//! 运行：
//! ```text
//! cargo run --example hello_world                 # 默认（D3D11 + 系统字体）
//! cargo run --example hello_world -- --force-soft # 纯 CPU 光栅器
//! cargo run --example hello_world -- --font=self  # 自研 TTF 引擎
//! ```
//!
//! 这个文件原本是 `crates/y901-app`（一个 86 行的壳 crate）。
//! 它不实现任何组件、不定义布局规则，本质就是「怎么把框架跑起来」的示范，
//! 所以降级为 example —— 框架需要一个能读的入口样例，而不是一个多余的 crate。

use std::time::Instant;

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Color, Command, Edges, Flex, FlexItem, UiTree};
use y901_widgets::TextLabel;

/// 最小 `Host`：每帧推进 Tick，并消耗定时订阅。
struct Shell;

impl Host for Shell {
    fn idle(&mut self, tree: &mut UiTree) {
        // Tick 有上限：防止某个控件每帧都要求再一帧而死循环。
        for _ in 0..8 {
        }
        // 定时订阅即使没有消费者也要排空，否则会积压。
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false // 示例应用不响应命令
    }
}

/// 最小启动树：一张标签。
fn build_shell() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(24.0))
        .push(Box::new(
            TextLabel::new("title", "Y901")
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(28.0)),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.set_focus(Some("title".to_string()));
    tree
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--font=self") {
        std::env::set_var("Y901_FONT", "self");
        eprintln!("[y901] --font=self：使用自写 TTF 引擎");
    }
    if args.iter().any(|a| a == "--force-soft") {
        std::env::set_var("Y901_FORCE_SOFT", "1");
        eprintln!("[y901] --force-soft：使用 CPU 光栅器");
    }

    create_runtime().run(Box::new(Shell), Box::new(build_shell), Options::default())
}