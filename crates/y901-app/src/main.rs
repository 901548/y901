//! Y901 桌面应用入口。
//!
//! 只负责：构建界面树 + 创建平台运行时 + 把它跑起来。
//! 不包含任何 agent / RPC / MCP 通道。

mod host;

use y901_shell::create_runtime;
use y901_runtime::Options;
use y901_ui::{Color, Edges, Flex, FlexItem, UiTree};
use y901_widgets::TextLabel;

use host::Shell;

/// 最小启动树：一张标签。示例应用由使用者自行编写。
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
    // `--force-soft`：强制 CPU 光栅器，不创建 D3D11 设备。
    // `--font=self`：使用自写 TTF 引擎，而非系统字体。
    if args.iter().any(|a| a == "--font=self") {
        std::env::set_var("Y901_FONT", "self");
        eprintln!("[y901] --font=self：使用自写 TTF 引擎");
    }
    if args.iter().any(|a| a == "--force-soft") {
        std::env::set_var("Y901_FORCE_SOFT", "1");
        eprintln!("[y901] --force-soft：使用 CPU 光栅器");
    }

    let shell = Shell::new();
    create_runtime().run(Box::new(shell), Box::new(build_shell), Options::default())
}
