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
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

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
    pub fn new(value: T) -> Self {
        Self {
            value: Rc::new(RefCell::new(value)),
            listeners: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub fn get(&self) -> T {
        self.value.borrow().clone()
    }

    /// 写入新值；**只有真的变了**才通知（避免无意义重绘）。
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

    /// 无条件写入（值不实现 PartialEq 时用）。
    pub fn replace(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.notify();
    }

    /// 就地修改并通知。
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        f(&mut self.value.borrow_mut());
        self.notify();
    }

    /// 组件在 `on_mount` 里调用：登记自己，值变化时把 id 推进失效队列。
    pub fn subscribe(&self, owner: impl Into<String>, queue: Invalidation) {
        let owner = owner.into();
        let mut list = self.listeners.borrow_mut();
        if let Some(existing) = list.iter_mut().find(|l| l.owner == owner) {
            existing.queue = queue;
            return;
        }
        list.push(Listener { owner, queue });
    }

    /// 组件在 `on_unmount` 里调用。
    pub fn unsubscribe(&self, owner: &str) {
        self.listeners.borrow_mut().retain(|l| l.owner != owner);
    }

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
