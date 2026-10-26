//! 命令：组件与外界之间唯一的"意图"通道。
//!
//! 框架**只搬运，不解释**：
//! - `Command::Node` 交给树自己路由到目标节点（变成一条 `UiEvent::Action`）；
//! - `Command::App` 原样交给应用层，框架不知道 `name` 是什么意思。
//!
//! 这样组件 A 不需要认识组件 B，框架也不需要认识任何一方。

use crate::json::Json;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// 请某个节点执行一个动作。框架负责找到节点并投递 `UiEvent::Action`。
    Node {
        target: String,
        action: String,
        args: Json,
    },
    /// 请应用层处理。框架原样上抛，不理解语义。
    App { name: String, args: Json },
}

impl Command {
    /// 构造"发给某个节点"的命令：`target` 是节点 id，`action` 由组件自己解释。
    pub fn node(target: impl Into<String>, action: impl Into<String>, args: Json) -> Self {
        Self::Node {
            target: target.into(),
            action: action.into(),
            args,
        }
    }

    /// 构造应用级命令：框架只路由到 `Host::command`，不认识 `name` 的语义。
    pub fn app(name: impl Into<String>, args: Json) -> Self {
        Self::App {
            name: name.into(),
            args,
        }
    }

    /// 是否应用级命令（`target` 为空）。
    pub fn is_app(&self) -> bool {
        matches!(self, Command::App { .. })
    }
}
