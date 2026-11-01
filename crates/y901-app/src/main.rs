//! Y901 启动器。
//!
//! 只做四件事：解析参数、构建最小 UI 树、装配 Host（MCP/RPC）、
//! 调用平台运行时。**不含任何业务组件构造**。

mod host;

use y901_agent::{McpServer, Policy};
use y901_shell::create_runtime;
use y901_runtime::{Host, Options};
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

    let mut server = McpServer::new(policy.clone());
    let mut host = Shell::new(policy);
    let jobs =
        y901_mcp::bridge::spawn_io(std::io::BufReader::new(std::io::stdin()), std::io::stdout());

    // 与 GUI 路径共用同一个 Host：命令必须经 host.command，不允许第二条执行路径
    loop {
        match jobs.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(job) => {
                let response = server.handle_line(&mut tree, &job.line);
                let _ = job.reply.send(response);
                for cmd in tree.drain_commands() {
                    let _ = host.command(&mut tree, cmd);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => host.idle(&mut tree),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_mcp = args.iter().any(|a| a == "--mcp");
    let headless = args.iter().any(|a| a == "--headless");
    let want_rpc = args.iter().any(|a| a == "--rpc");
    // `--force-soft`：纯 CPU 渲染（不创建 D3D11 设备）。
    // 底层用环境变量传递（`Window::create` 读 `Y901_FORCE_SOFT`），
    // 因为 `Options` 是公开结构体，加字段会破坏所有现有构造点。
    // `--font=self`：用自研 TTF 引擎（底层读 `Y901_FONT`）。
    // 默认 legacy（DirectWrite）；自研引擎初始化失败会自动回退。
    if args.iter().any(|a| a == "--font=self") {
        std::env::set_var("Y901_FONT", "self");
        eprintln!("[y901] --font=self：使用自研 TTF 字形引擎");
    }
    if args.iter().any(|a| a == "--force-soft") {
        std::env::set_var("Y901_FORCE_SOFT", "1");
        eprintln!("[y901] --force-soft：使用纯 CPU 软渲染后端");
    }

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

    create_runtime().run(Box::new(shell), Box::new(build_shell), Options::default())
}
