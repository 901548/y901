//! 极轻量响应式：`Signal<T>` + 失效队列。
//!
//! 设计取舍：**Signal 不认识 UiTree**。
//! - Signal 只持有值 + 订阅者名单；
//! - 值变化时把"订阅者 id"推进一个共享的失效队列（`Invalidation`）；
//! - UiTree 在每帧开始时排空队列，把这些节点标脏并重算布局。
//!
//! 这样数据层（业务）和视图层（框架）互不依赖，业务代码只写 `signal.set(x)`。

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

/// 失效队列：订阅者 id 的去重集合。
#[derive(Clone, Default)]
pub struct Invalidation {
    inner: Rc<RefCell<BTreeSet<String>>>,
}

impl Invalidation {
    /// 新建空队列（通常由 `UiTree` 持有，组件通过 `MountCtx` 拿到 clone）。
    ///
    /// 返回：与其它 clone 共享同一底层集合的句柄。
    pub fn new() -> Self {
        Self::default()
    }

    /// 把一个 id 推进队列（**去重**：同一帧内重复推只保留一份）。
    ///
    /// 输入：owner id。返回：无。副作用：加入集合；不触发任何重绘。
    /// 失败：无。注意它只登记"谁失效了"，真正标脏在 `drain` 之后由框架做。
    pub fn push(&self, id: impl Into<String>) {
        self.inner.borrow_mut().insert(id.into());
    }

    /// 取走全部待处理 id（框架每帧调用一次）。
    pub fn drain(&self) -> Vec<String> {
        let mut set = self.inner.borrow_mut();
        let out: Vec<String> = set.iter().cloned().collect();
        set.clear();
        out
    }

    /// 队列是否为空（没有待处理的失效）。
    ///
    /// 返回：`true` = 自上次 `drain` 以来没有任何变化。无副作用。
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// 队列里待处理的 id 数量（诊断用）。
    ///
    /// 返回：去重后的个数。无副作用。
    /// 注意：这是"本帧有多少个节点需要重新检查"，不是"发生了几次 set"。
    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }
}

/// 一个可观察的值。
pub struct Signal<T> {
    value: Rc<RefCell<T>>,
    listeners: Rc<RefCell<Vec<Listener>>>,
}

struct Listener {
    owner: String,
    queue: Invalidation,
}

impl<T: Clone> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            listeners: self.listeners.clone(),
        }
    }
}

impl<T: Clone + 'static> Signal<T> {
    /// 新建一个信号，初始值 `value`，无订阅者。
    ///
    /// 返回：可 `clone()` 的句柄（clone 共享同一份值与订阅表，不是深拷贝）。
    /// 副作用：无。失败：无。
    pub fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
            listeners: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// 读当前值的**克隆**。
    ///
    /// 返回：值的副本（`T: Clone`，因此每次读都复制——大对象请用 `update` 就地改）。
    /// 副作用：无。失败：无。
    /// 注意：在 `RefCell` 已被可变借用期间调用会 panic（同线程重入）。
    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    /// 写入新值；**只有真的变了**才通知（避免无意义重绘）。
    ///
    /// 输入：新值；比较用 `PartialEq`。
    /// 返回：`true` = 值变了并已通知订阅者；`false` = 值相同，什么都没发生。
    /// 副作用：值变化时把每个订阅者的 id 推进各自失效队列（不直接重绘）。
    pub fn set(&self, value: T) -> bool
    where
        T: PartialEq,
    {
        {
            let mut slot = self.value.borrow_mut();
            if *slot == value {
                return false;
            }
            *slot = value;
        }
        self.notify();
        true
    }

    /// 无条件写入并通知（值不实现 `PartialEq` 时用）。
    ///
    /// 输入：新值。返回：无。
    /// 副作用：**即使值相同也会通知**——订阅者会收到一次无意义的失效。
    /// 失败：无。适用场景：类型没有 `PartialEq`，或"相同值也需要刷新"的显式语义。
    pub fn replace(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.notify();
    }

    /// 就地修改并通知（避免为大对象做一次克隆）。
    ///
    /// 输入：`f`（拿到 `&mut T`）。返回：无。
    /// 副作用：无条件通知订阅者（哪怕 `f` 什么都没改）。
    /// 失败：`f` 里若再调 `get()`/`set()` 会触发 `RefCell` 双重借用 panic。
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.value.borrow_mut());
        self.notify();
    }

    /// 登记订阅者：值变化时把 `owner` 推进 `queue`。
    ///
    /// 输入：`owner`（通常是节点 id）、`queue`（`MountCtx::invalidation`）。
    /// 返回：无。副作用：加入订阅表；**同一 owner 重复订阅会覆盖队列而非重复登记**。
    /// 时机：只在 `on_mount` 里调（框架此时才提供 `MountCtx`）。
    pub fn subscribe(&self, owner: impl Into<String>, queue: Invalidation) {
        let owner = owner.into();
        let mut list = self.listeners.borrow_mut();
        if let Some(existing) = list.iter_mut().find(|l| l.owner == owner) {
            existing.queue = queue;
            return;
        }
        list.push(Listener { owner, queue });
    }

    /// 取消订阅（按 owner 精确匹配）。
    ///
    /// 输入：`owner`。返回：无。
    /// 副作用：从订阅表移除；owner 不存在时是 no-op（幂等）。
    /// 时机：`on_unmount` 里调，否则节点销毁后仍会被推进失效队列（泄漏 + 无意义失效）。
    pub fn unsubscribe(&self, owner: &str) {
        self.listeners.borrow_mut().retain(|l| l.owner != owner);
    }

    /// 当前订阅者数量（诊断用）。
    ///
    /// 返回：订阅表长度。无副作用。
    /// 注意：它**不**区分 owner 是否还活着——节点被 `remove_node` 删除时
    /// 若没走 `on_unmount`，这里的计数会偏高。
    pub fn subscriber_count(&self) -> usize {
        self.listeners.borrow().len()
    }

    fn notify(&self) {
        for l in self.listeners.borrow().iter() {
            l.queue.push(l.owner.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_twice_with_same_value_does_not_notify() {
        let q = Invalidation::new();
        let sig = Signal::new(1u32);
        sig.subscribe("a", q.clone());

        assert!(sig.set(2));
        assert_eq!(q.len(), 1);

        q.drain();
        assert!(!sig.set(2), "同值不应触发");
        assert!(q.is_empty());
    }

    #[test]
    fn unsubscribe_stops_notifications() {
        let q = Invalidation::new();
        let sig = Signal::new("x".to_string());
        sig.subscribe("a", q.clone());
        sig.unsubscribe("a");

        sig.set("y".to_string());
        assert!(q.is_empty());
        assert_eq!(sig.subscriber_count(), 0);
    }

    #[test]
    fn multiple_subscribers_all_get_notified() {
        let q = Invalidation::new();
        let sig = Signal::new(0u32);
        sig.subscribe("a", q.clone());
        sig.subscribe("b", q.clone());

        sig.set(1);
        let mut ids = q.drain();
        ids.sort();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn drain_clears_queue() {
        let q = Invalidation::new();
        q.push("a");
        q.push("a");
        assert_eq!(q.len(), 1, "重复 id 去重");
        assert_eq!(q.drain().len(), 1);
        assert!(q.is_empty());
    }
}
