//! `Host` 实现：把运行时的 Tick 转给界面树。
//!
//! 这里只做 GUI 需要的事：推进动画/定时事件。
//! 不包含任何 agent / RPC / MCP 通道。

use std::time::Instant;

use y901_runtime::Host;
use y901_ui::{Command, UiEvent, UiTree};

/// 最小 `Host` 实现：每帧推进 Tick，并消耗定时订阅。
pub struct Shell;

impl Shell {
    /// 创建一个不带任何外部通道的 Host。
    pub fn new() -> Self {
        Self
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for Shell {
    fn idle(&mut self, tree: &mut UiTree) {
        // Tick 有上限：防止某个控件每帧都要求再一帧而死循环。
        for _ in 0..8 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }
        // 定时订阅事件在虽然没有消费者的情况下也要排空，否则会积压。
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false // 演示应用不响应命令
    }
}
