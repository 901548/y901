//! 线程安全的标脏句柄（[`InvalidationHandle`]）。
//!
//! **它只通知"有变化"，不是线程安全的状态容器。**
//!
//! 后台线程可以调用 [`InvalidationHandle::invalidate`] / [`InvalidationHandle::invalidate_all`]
//! 让 UI 线程在下一帧重绘（或重排），但**不能借此直接修改组件字段**：
//! 句柄里没有 `UiTree`，也没有任何组件引用。跨线程的数据必须由应用自己用
//! `Arc<Mutex<…>>` 管理，并在 UI 线程 drain 之后由应用把数据写进组件。
//!
//! 典型用法（应用层示意，引用了应用自己的符号，不参与编译）：
//!
//! ```text
//! // 建树后立刻取句柄，交给后台任务
//! let handle = tree.invalidation_handle();
//! std::thread::spawn(move || {
//!     let rows = fetch_sftp_listing();          // 后台干活
//!     *shared_rows.lock().unwrap() = rows;      // 自己的锁保护数据
//!     handle.invalidate("file_list");           // 只通知"该重绘了"
//! });
//! ```
//!
//! 与组件的分工：**组件的重绘请求走 [`crate::EventResult`]，
//! 应用层的异步任务重绘请求走 `InvalidationHandle`。**

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// 后台线程提交的一条标脏请求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingInvalidation {
    /// 只标脏某个节点。
    One(String),
    /// 整窗标脏（优先级最高：只要出现一次，其他 `One` 都会被它覆盖）。
    All,
}

/// 线程安全的标脏句柄（`Send + Sync`）。
///
/// 内部只有 `Arc<Mutex<Vec<..>>>` 与 `Arc<AtomicBool>`，
/// **不持有 `UiTree` / `Rc` / `RefCell`**，因此可以在任意线程持有与调用。
#[derive(Clone, Default)]
pub struct InvalidationHandle {
    /// 待处理请求（UI 线程 drain 时取走）。
    pending: Arc<Mutex<Vec<PendingInvalidation>>>,
    /// "有活"标志：后台线程置位，UI 线程 drain 时清零。
    ///
    /// 目前主循环是**非阻塞忙轮询**（`PeekMessageW`），每帧 `drain` 一次即可，
    /// 因此这个 flag 只作为"是否有请求"的快速判断，不承担唤醒职责。
    woken: Arc<AtomicBool>,
}

impl InvalidationHandle {
    /// 新建一个空句柄（通常由 `UiTree` 持有，通过 `UiTree::invalidation_handle()` 取出）。
    ///
    /// 返回：可 `Clone` 的句柄，clone 之间共享同一份队列。
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求标脏某个节点（**可在任意线程调用**）。
    ///
    /// 输入：节点 id（不存在时框架侧会静默忽略，与 `UiTree::invalidate` 一致）。
    /// 副作用：把请求入队并置 `woken`；UI 线程下一帧 drain 后生效。
    /// 失败：无（Mutex 中毒时复用内部数据，不 panic）。
    pub fn invalidate(&self, id: impl Into<String>) {
        self.push(PendingInvalidation::One(id.into()));
    }

    /// 请求整窗标脏（**可在任意线程调用**）。
    ///
    /// 副作用：入队 `All` 并置 `woken`。与 `invalidate(id)` 同帧出现时，
    /// 最终状态是"整窗"（合并取最强），不会漏掉任何一块。
    pub fn invalidate_all(&self) {
        self.push(PendingInvalidation::All);
    }

    /// 取走全部待处理请求（**只允许 UI 线程调用**）。
    ///
    /// 返回：按入队顺序的请求列表；同时把 `woken` 清零。
    /// 说明：这是框架内部接口（`UiTree::drain_external_invalidations` 调用），
    /// 不保证与 `invalidate` 的先后顺序以外的任何语义。
    #[doc(hidden)]
    pub fn drain(&self) -> Vec<PendingInvalidation> {
        self.woken.store(false, Ordering::Relaxed);
        let mut queue = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *queue)
    }

    /// 当前是否有未处理的请求（诊断用；**不要**用它替代 `drain`——两者之间有竞态）。
    ///
    /// 返回：`true` = 至少有一条请求尚未 drain。
    #[doc(hidden)]
    pub fn has_pending(&self) -> bool {
        self.woken.load(Ordering::Relaxed)
    }

    /// 入队一条请求并置位 `woken`。
    fn push(&self, item: PendingInvalidation) {
        {
            let mut queue = self
                .pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            queue.push(item);
        }
        self.woken.store(true, Ordering::Relaxed);
    }
}

/// 编译期断言：句柄必须能跨线程移动与共享。
///
/// 这不是装饰：`InvalidationHandle` 的**唯一目的**就是给后台线程用，
/// 若它意外变回非 `Send`/`Sync`（例如有人加了 `Rc` 字段），这里会直接编译失败。
const _: () = {
    fn assert_send_sync<T: Send + Sync>() {}
    let _ = assert_send_sync::<InvalidationHandle>;
};
