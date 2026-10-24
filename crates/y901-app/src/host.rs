//! Host 装配：MCP / RPC 服务与 `Host` trait 的实现。
//!
//! 启动器本身只负责"参数 → 建树 → 跑"，服务宿主单独放这里，
//! 这样 `main.rs` 保持在启动器该有的体积。

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use y901_agent::{McpServer, Pending, Policy, RpcRequest};
use y901_runtime::Host;
use y901_ui::{Command, Json, Size, TextStyle, TextMetrics, UiEvent, UiTree};

/// MCP / RPC 的宿主。
pub struct Shell {
    pub policy: Policy,
    pub mcp: Option<(McpServer, Receiver<y901_mcp::bridge::Job>)>,
    pub rpc_rx: Option<Receiver<Pending>>,
    pub hub: y901_agent::Broadcaster,
}

impl Shell {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            mcp: None,
            rpc_rx: None,
            hub: y901_agent::Broadcaster::new(),
        }
    }
}

impl Host for Shell {
    fn idle(&mut self, tree: &mut UiTree) {
        // RPC：逐条处理，UI 线程不阻塞
        if let Some(rx) = &self.rpc_rx {
            while let Ok(pending) = rx.try_recv() {
                handle_rpc(tree, &self.policy, pending);
            }
        }

        // MCP：每帧最多 8 条
        if let Some((server, jobs)) = self.mcp.as_mut() {
            y901_mcp::bridge::pump(server, tree, jobs, 8);
        }

        // Tick：驱动组件心跳（PTY、光标闪烁）
        for _ in 0..8 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }

        // 语义订阅：差分 + 节流由框架负责
        for event in tree.poll_subscriptions(Instant::now()) {
            self.hub.send(&event.to_string());
        }
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false // 启动器没有业务命令
    }

    fn policy(&self) -> Policy {
        self.policy.clone()
    }
}

/// RPC 请求 → 统一 dispatcher。
fn handle_rpc(tree: &mut UiTree, policy: &Policy, pending: Pending) {
    let result = match &pending.request {
        // 与 MCP 共用同一个网关入口（同样的过滤、脱敏、类型校验）
        RpcRequest::GetTree => y901_agent::dispatch_request(tree, policy, "get_tree", &Json::obj())
            .map_err(|e| e.code().to_string()),
        RpcRequest::Query { q } => {
            let params = Json::obj().set("q", q.clone());
            y901_agent::dispatch_request(tree, policy, "query", &params)
                .map_err(|e| e.code().to_string())
        }
        RpcRequest::Ping => Ok(Json::obj().set("pong", true)),
        RpcRequest::Dispatch { node, event } => {
            let params = Json::obj()
                .set("id", node.clone())
                .set("event", event.to_json());
            y901_agent::dispatch_request(tree, policy, "dispatch", &params)
                .map_err(|e| e.code().to_string())
        }
        RpcRequest::Invoke { node, action, args } => {
            let params = Json::obj()
                .set("id", node.clone())
                .set("action", action.clone())
                .set("args", args.clone());
            y901_agent::dispatch_request(tree, policy, "invoke", &params)
                .map_err(|e| e.code().to_string())
        }
        RpcRequest::Focus { node } => {
            if tree.set_focus(Some(node.clone())) {
                Ok(Json::obj().set("focused", node.clone()))
            } else {
                Err(format!("unknown node: {node}"))
            }
        }
        RpcRequest::Subscribe { node, interval_ms } => {
            if tree.subscribe(node, Duration::from_millis(*interval_ms)) {
                Ok(Json::obj().set("subscribed", node.clone()))
            } else {
                Err(format!("unknown node: {node}"))
            }
        }
        RpcRequest::Unsubscribe { node } => {
            tree.unsubscribe(node);
            Ok(Json::obj().set("unsubscribed", node.clone()))
        }
        RpcRequest::Unknown { method } => Err(format!("unknown method: {method}")),
    };
    y901_agent::rpc::reply_to(pending, result);
}

/// 无窗口环境的等宽度量（headless 也要能算出真实 rect）。
pub struct HeadlessMetrics;

impl TextMetrics for HeadlessMetrics {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        let mut w = 0.0;
        for ch in text.chars() {
            w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
        }
        Size::new(w, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}
