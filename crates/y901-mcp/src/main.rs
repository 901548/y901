//! `y901-mcp`：把 Y901 的语义树暴露给 MCP 客户端（Claude Desktop / Cursor）。
//!
//! 运行方式：
//! ```text
//! y901-mcp            # 无窗口，自带一棵演示树，走 stdio
//! ```
//! 环境变量：
//! - `Y901_TOKEN`：非空时校验 Token（与网关 Policy 共用）。

use y901_agent::{McpServer, Policy};
use y901_ui::{Flex, FlexItem, Signal, Size, TextMetrics, TextStyle, UiTree};
use y901_widgets::{Button, TextInput};

/// 无窗口环境的等宽度量（仅用于把树排出一套合理的几何）。
struct Metrics;

impl TextMetrics for Metrics {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> y901_ui::Size {
        let mut w = 0.0;
        for ch in text.chars() {
            w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
        }
        Size::new(w, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 演示树：一个输入框 + 一个按钮 + 一个状态标签（由 Signal 驱动）。
fn demo_tree() -> UiTree {
    let status = Signal::new("idle".to_string());

    let root = Flex::column("root")
        .push(Box::new(
            TextInput::new("search")
                .with_text("")
                .flex_item(FlexItem::height(32.0)),
        ))
        .push(Box::new(
            Button::new("go", "Go").flex_item(FlexItem::height(32.0)),
        ))
        .push(Box::new(
            y901_widgets::TextLabel::new("status", "idle").bind(status),
        ));

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(800.0, 600.0), &Metrics);
    tree
}

fn main() {
    let token = std::env::var("Y901_TOKEN").unwrap_or_default();
    let policy = Policy {
        token,
        redact: true,
    };

    // 后端进程：日志一律写 stderr，绝不污染 stdout（stdout 是协议通道）
    eprintln!("[y901-mcp] server started (protocol 2024-11-05)");

    // 注意：StdinLock 不是 Send，所以把 stdin 本体交给线程，在那边再 lock
    let jobs =
        y901_mcp::bridge::spawn_io(std::io::BufReader::new(std::io::stdin()), std::io::stdout());

    let mut tree = demo_tree();
    let mut server = McpServer::new(policy);
    y901_mcp::bridge::run_blocking(&mut server, &mut tree, &jobs);
}
