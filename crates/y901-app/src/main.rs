//! Y901 启动器。
//!
//! 只做四件事：解析参数、构建最小 UI 树、装配 Host（MCP/RPC）、
//! 调用平台运行时。**不含任何业务组件构造**。

mod host;

use y901_agent::{McpServer, Policy};
use y901_backend_win32::runtime::Win32Runtime;
use y901_runtime::{Options, PlatformRuntime};
use y901_ui::{Color, Edges, Flex, FlexItem, Size, UiTree};
use y901_widgets::TextLabel;

use host::{HeadlessMetrics, Shell};

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

/// `--mcp --headless`：不开窗口，纯协议（供 MCP 客户端与 CI 使用）。
fn run_headless(policy: Policy) -> Result<(), String> {
    let mut tree = build_shell();
    tree.layout(Size::new(1200.0, 800.0), &HeadlessMetrics);
    let mut server = McpServer::new(policy);
    let jobs = y901_mcp::bridge::spawn_io(
        std::io::BufReader::new(std::io::stdin()),
        std::io::stdout(),
    );
    y901_mcp::bridge::run_blocking(&mut server, &mut tree, &jobs);
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_mcp = args.iter().any(|a| a == "--mcp");
    let headless = args.iter().any(|a| a == "--headless");
    let want_rpc = args.iter().any(|a| a == "--rpc");

    let token = std::env::var("Y901_TOKEN").unwrap_or_default();
    let policy = Policy {
        token: token.clone(),
        redact: true,
    };

    if want_mcp && headless {
        return run_headless(policy);
    }

    let mut shell = Shell::new(policy.clone());

    if want_mcp {
        let jobs = y901_mcp::bridge::spawn_io(
            std::io::BufReader::new(std::io::stdin()),
            std::io::stdout(),
        );
        shell.mcp = Some((McpServer::new(policy.clone()), jobs));
        eprintln!("[y901] MCP bridge attached (stdio)");
    }

    if want_rpc {
        if token.is_empty() {
            eprintln!("[y901] --rpc 需要 Y901_TOKEN，已拒绝启动 TCP 通道");
        } else {
            let (tx, rx) = std::sync::mpsc::channel();
            match y901_agent::rpc::spawn_with_token(y901_agent::rpc::DEFAULT_ADDR, tx, token) {
                Ok((addr, hub)) => {
                    shell.rpc_rx = Some(rx);
                    shell.hub = hub;
                    eprintln!("[y901] RPC listening on {addr} (token required)");
                }
                Err(err) => eprintln!("[y901] RPC 启动失败: {err}"),
            }
        }
    }

    Win32Runtime::run(Box::new(shell), Box::new(build_shell), Options::default())
}
