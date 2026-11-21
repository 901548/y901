//! Linux AT-SPI 桥（`unix-atspi` feature）。
//!
//! `accesskit_unix` 在内部起一条 **D-Bus** 连接，把 AccessKit 树暴露成 AT-SPI 对象，
//! 供 Orca 等读屏器读取。它不绑 HWND（X11 下没有这种东西），所以 API 与 Windows 版
//! 完全不同：
//!
//! - `Adapter::new` 起线程并注册 D-Bus 名；
//! - 树用 `update_if_active(factory)` 推（没有客户端连上时 factory 不会被调用）；
//! - 动作在工作线程回调（`ActionHandler`），因此同样只能入队。
//!
//! ## 已知成本（诚实记录）
//!
//! - 这是本项目引入的**第二套 D-Bus 栈**（IBus 客户端是自研 raw D-Bus）。
//!   两者都在 feature 后，不启用时 `Cargo.lock` 不受影响、二进制不增加字节；
//! - `accesskit_unix` 会自建线程与异步运行时（`futures-lite`），
//!   与 `zbus` 在同一进程共存；实测以"桥析构即退出"为准。
//!
//! ## 线程模型
//!
//! 与 Windows 版本一致：`do_action` 可能在工作线程，只往 `Mutex<VecDeque>` 里放；
//! 应用线程用 [`UnixAtspi::poll_action`] 取出。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use accesskit::{ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use accesskit_unix::Adapter;

type ActionQueue = Arc<Mutex<VecDeque<ActionRequest>>>;
type TreeSlot = Arc<Mutex<Option<TreeUpdate>>>;

struct Activation {
    initial: TreeSlot,
}

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        self.initial.lock().ok().and_then(|mut s| s.take())
    }
}

struct Deactivation;

impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {
        // 无本地缓存要清：树在每次 push 时重建，槽位由 `request_initial_tree` 取走。
    }
}

struct Actions {
    queue: ActionQueue,
}

impl accesskit::ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if let Ok(mut q) = self.queue.lock() {
            if q.len() >= 256 {
                q.pop_front();
            }
            q.push_back(request);
        }
    }
}

/// Linux AT-SPI 桥。
pub struct UnixAtspi {
    adapter: Adapter,
    queue: ActionQueue,
    tree_slot: TreeSlot,
}

impl UnixAtspi {
    /// 建立桥（立即返回；D-Bus 连接在内部线程里建立）。
    ///
    /// 与 Windows 不同，这里**不要求**调用时机在主循环之前：AT-SPI 不绑窗口。
    /// 但推荐在窗口创建后立刻建，避免用户先连读屏器时拿不到树。
    pub fn new() -> Self {
        let queue: ActionQueue = Arc::new(Mutex::new(VecDeque::new()));
        let tree_slot: TreeSlot = Arc::new(Mutex::new(None));
        let adapter = Adapter::new(
            Activation {
                initial: Arc::clone(&tree_slot),
            },
            Actions {
                queue: Arc::clone(&queue),
            },
            Deactivation,
        );
        Self {
            adapter,
            queue,
            tree_slot,
        }
    }

    /// 推一棵树出去（无客户端连接时缓存最后一份，供初次激活使用）。
    pub fn push(&mut self, update: TreeUpdate) {
        let cloned = update.clone();
        self.adapter.update_if_active(move || cloned);
        // 无论有没有客户端都更新槽位：新连上的客户端要拿到**最新**树，
        // 而不是它连上之前某一帧的旧树。
        if let Ok(mut slot) = self.tree_slot.lock() {
            *slot = Some(update);
        }
    }

    /// 设置根窗口的包围盒（AT-SPI 用它换算坐标）。
    pub fn set_root_window_bounds(&mut self, outer: accesskit::Rect, inner: accesskit::Rect) {
        self.adapter.set_root_window_bounds(outer, inner);
    }

    /// 通知窗口焦点变化（读屏器据此决定要不要播报）。
    pub fn set_window_focused(&mut self, focused: bool) {
        self.adapter.update_window_focus_state(focused);
    }

    /// 取出一个待处理动作。
    pub fn poll_action(&mut self) -> Option<ActionRequest> {
        self.queue.lock().ok().and_then(|mut q| q.pop_front())
    }
}

impl Default for UnixAtspi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
    fn action_queue_is_bounded() {
        let queue: ActionQueue = Arc::new(Mutex::new(VecDeque::new()));
        let mut h = Actions {
            queue: Arc::clone(&queue),
        };
        for i in 0..300 {
            h.do_action(ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: NodeId(i),
                data: None,
            });
        }
        let q = queue.lock().unwrap();
        assert_eq!(q.len(), 256);
        assert_eq!(q.front().map(|r| r.target_node), Some(NodeId(44)));
    }

    #[test]
    fn initial_tree_is_single_use() {
        let slot: TreeSlot = Arc::new(Mutex::new(Some(empty_update())));
        let mut a = Activation {
            initial: Arc::clone(&slot),
        };
        assert!(a.request_initial_tree().is_some());
        assert!(a.request_initial_tree().is_none());
    }

    #[test]
    fn deactivation_is_a_noop_that_does_not_panic() {
        Deactivation.deactivate_accessibility();
    }
}
