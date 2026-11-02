//! Agent 安全网关。
//!
//! 职责边界：
//! - **认识语义**（id / action / revision），不认识坐标——坐标由网关从 `rect` 反算；
//! - **不放行**：Token、能力作用域、版本锁、敏感字段脱敏，四道闸都在这一层；
//! - **不渲染**：只通过 `UiTree::dispatch_to` 间接驱动组件。
//!
//! `y901-ui` 从此不再有网络代码。

#![warn(missing_docs)]
pub mod api;
pub mod mcp;
pub mod rpc;

pub use api::{dispatch_request, handle_call, ApiCall, ApiError, Policy};
pub use mcp::{run_stdio, McpServer};
pub use rpc::{spawn, Broadcaster, Pending, RpcEvent, RpcRequest};
