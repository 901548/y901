//! Windows UIA 桥（`windows-uia` feature）。
//!
//! 用 `accesskit_windows::SubclassingAdapter`：它给指定 HWND 挂一个子类化窗口过程，
//! 拦截 `WM_GETOBJECT` 并回答 UIA 的查询 —— 我们只需提供两件事：
//!
//! 1. **初始树**（[`ActivationHandler::request_initial_tree`]）：UIA 客户端第一次
//!    连接时调用，必须返回完整树；
//! 2. **动作处理**（[`ActionHandler::do_action`]）：用户用"讲述人"/UIA 客户端点了
//!    按钮时，把 `ActionRequest` 入队，交给应用线程下一帧处理。
//!
//! ## 必须遵守的时序
//!
//! `SubclassingAdapter::new` **必须在窗口第一次显示之前**调用（库内部会 panic，
//! 因为 UIA 的挂载时机与窗口可见性绑定）。所以调用方应在
//! `CreateWindowEx` 之后、`ShowWindow` 之前调用 [`WindowsUia::new`]。
//!
//! ## 线程模型
//!
//! `ActionHandler::do_action` **可能在工作线程被调用**（UIA 客户端在自己的线程
//! 上发起请求）。因此这里用 `Mutex<VecDeque>` 把动作排队，应用线程在
//! [`WindowsUia::poll_action`] 里取 —— 绝不能在 `do_action` 里直接碰 `UiTree`。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use accesskit::{ActionRequest, ActivationHandler, TreeUpdate};
use accesskit_windows::{SubclassingAdapter, HWND};

/// 待处理动作队列（`do_action` 写入，应用线程读出）。
///
/// 用 `Arc<Mutex<..>>` 而不是 `RefCell`：`ActionHandler` 要求 `Send`，
/// 且可能跨线程调用。
type ActionQueue = Arc<Mutex<VecDeque<ActionRequest>>>;

/// 初始树提供者：由后端在挂载时注入（返回 `None` = 暂时没有树）。
type TreeProvider = Arc<Mutex<Option<TreeUpdate>>>;

struct Activation {
    initial: TreeProvider,
}

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // 取走一次即可：后续更新由 `push()` 主动推。
        self.initial.lock().ok().and_then(|mut slot| slot.take())
    }
}

struct Actions {
    queue: ActionQueue,
}

impl accesskit::ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        // **不在这里处理**：这个回调可能在工作线程上，碰 `UiTree` 会踩数据竞争。
        if let Ok(mut queue) = self.queue.lock() {
            // 防止异常客户端刷爆内存：队列超过 256 条就丢弃最旧的。
            if queue.len() >= 256 {
                queue.pop_front();
            }
            queue.push_back(request);
        }
    }
}

/// Windows UIA 桥。**持有期间窗口才有无障碍树。**
pub struct WindowsUia {
    adapter: SubclassingAdapter,
    queue: ActionQueue,
    tree_slot: TreeProvider,
}

impl WindowsUia {
    /// 给 `hwnd` 挂上 UIA 桥。
    ///
    /// # Panics
    ///
    /// 窗口**已经可见**时 `accesskit_windows` 会 panic（UIA 的挂载时机约束）。
    /// 调用方必须在 `ShowWindow` 之前调用本函数。
    pub fn new(hwnd: HWND) -> Self {
        let queue: ActionQueue = Arc::new(Mutex::new(VecDeque::new()));
        let tree_slot: TreeProvider = Arc::new(Mutex::new(None));
        let adapter = SubclassingAdapter::new(
            hwnd,
            Activation {
                initial: Arc::clone(&tree_slot),
            },
            Actions {
                queue: Arc::clone(&queue),
            },
        );
        Self {
            adapter,
            queue,
            tree_slot,
        }
    }

    /// 推一棵树出去。
    ///
    /// 第一次调用会被缓存下来，等 UIA 客户端真正连接时由
    /// `request_initial_tree` 取走；之后的调用直接走 `update_if_active`
    /// （没有客户端连接时什么都不做，**零开销**）。
    pub fn push(&mut self, update: TreeUpdate) {
        // 先试着走"已有客户端"的快速路径。
        // `update_if_active` 要求 factory 返回一个**新的** update（FnOnce 只能消费一次），
        // 所以这里克隆一份：TreeUpdate 是纯数据，克隆成本远低于重建整棵树。
        let cloned = update.clone();
        let had_client = self.adapter.update_if_active(move || cloned).is_some();
        if !had_client {
            // 还没有客户端连接：把最新树存起来供 `request_initial_tree` 用。
            // 只保留最后一份（旧树对客户端没有价值，而且会占内存）。
            if let Ok(mut slot) = self.tree_slot.lock() {
                *slot = Some(update);
            }
        }
    }

    /// 取出一个待处理的系统动作。
    pub fn poll_action(&mut self) -> Option<ActionRequest> {
        self.queue.lock().ok().and_then(|mut q| q.pop_front())
    }

    /// 卸载桥（窗口销毁前调用；`drop` 也会自动卸载）。
    pub fn shutdown(self) {
        drop(self);
    }
}

#[cfg(test)]
mod tests {
    // 这里只验证**不依赖 HWND 的部分**：动作队列与初始树槽位的语义。
    // 真正的 UIA 交互需要真窗口 + 讲述人，属手工验收（见 docs/ROADMAP.md S3）。
    use super::*;
    use accesskit::{Action, ActionHandler as _, NodeId, TreeId, TreeInfo};

    fn empty_update() -> TreeUpdate {
        TreeUpdate {
            nodes: vec![(NodeId(0), accesskit::Node::new(accesskit::Role::Window))],
            tree: Some(TreeInfo {
                root: NodeId(0),
                toolkit_name: Some("y901".into()),
                toolkit_version: None,
            }),
            tree_id: TreeId::ROOT,
            focus: NodeId(0),
        }
    }

    #[test]
    fn action_queue_drops_the_oldest_entry_when_full() {
        let queue: ActionQueue = Arc::new(Mutex::new(VecDeque::new()));
        let mut handler = Actions {
            queue: Arc::clone(&queue),
        };
        for i in 0..300 {
            handler.do_action(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: NodeId(i),
                data: None,
            });
        }
        let len = queue.lock().unwrap().len();
        assert_eq!(len, 256, "队列上限 256，防止异常客户端刷爆内存");
        // 最旧的应当被丢弃：队首不是 NodeId(0)。
        let first = queue.lock().unwrap().front().map(|r| r.target_node);
        assert_eq!(first, Some(NodeId(44)), "丢弃最旧的 44 条（300-256）");
    }

    #[test]
    fn initial_tree_is_taken_exactly_once() {
        let slot: TreeProvider = Arc::new(Mutex::new(Some(empty_update())));
        let mut activation = Activation {
            initial: Arc::clone(&slot),
        };
        assert!(activation.request_initial_tree().is_some(), "第一次应有树");
        assert!(
            activation.request_initial_tree().is_none(),
            "第二次必须返回 None（树已被取走，由 push 主动更新）"
        );
    }

    #[test]
    fn initial_tree_returns_none_when_nothing_was_pushed() {
        let slot: TreeProvider = Arc::new(Mutex::new(None));
        let mut activation = Activation { initial: slot };
        assert!(activation.request_initial_tree().is_none());
    }
}
