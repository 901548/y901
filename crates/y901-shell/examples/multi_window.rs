//! 多窗口示例：两个独立窗口，各自一个 `Shell`。
//!
//! 验证的事：`PlatformRuntime::run_many` 能在同一进程里跑多个窗口，
//! 且**关一个窗口不会杀掉另一个**（退出条件已改为计数归零）。
//!
//! 运行：
//! ```bash
//! cargo run -p y901-app --example multi_window
//! cargo run -p y901-app --example multi_window -- --force-soft
//! ```

use y901_runtime::{Host, Options, WindowFactory};
use y901_shell::create_runtime;
use y901_ui::{Color, Command, Edges, Flex, FlexItem, UiTree};
use y901_widgets::TextLabel;

/// 最小 Host：每帧推进 Tick。
struct Shell;

impl Host for Shell {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..8 {
        }
        for _ in tree.poll_subscriptions(std::time::Instant::now()) {}
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false
    }
}

/// 每个窗口各自的界面树（标题里带窗口序号，便于目视区分）。
fn build_window(i: usize) -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(24.0))
        .push(Box::new(
            TextLabel::new("title", format!("窗口 {i}"))
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(28.0)),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.set_focus(Some("title".to_string()));
    tree
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--force-soft") {
        std::env::set_var("Y901_FORCE_SOFT", "1");
    }

    let options_list = vec![
        Options {
            title: "Y901 窗口 1".to_string(),
            width: 640,
            height: 480,
            ..Default::default()
        },
        Options {
            title: "Y901 窗口 2".to_string(),
            width: 640,
            height: 480,
            ..Default::default()
        },
    ];

    // 工厂：`Host` 与 `UiTree` 都不是 `Send`，必须在各自线程内创建。
    let make: WindowFactory = Box::new(|i| (Box::new(Shell) as Box<dyn Host>, build_window(i)));

    create_runtime().run_many(options_list, make)
}
