//! \u7ec4\u4ef6\u5c40\u90e8\u72b6\u6001\uff1a\u201c\u6539\u4e86\u5c31\u81ea\u52a8\u6807\u810f\u201d\u7684 `State<T>`\u3002
//!
//! \u4e3a\u4ec0\u4e48\u8fd8\u9700\u8981\u5b83\uff1a`EventCtx::request_redraw()` \u80fd\u89e3\u51b3\u201c\u4e8b\u4ef6\u91cc\u77e5\u9053\u81ea\u5df1\u810f\u4e86\u201d\uff0c
//! \u4f46\u89e3\u51b3\u4e0d\u4e86\u201c**\u975e\u4e8b\u4ef6**\u9014\u5f84\u6539\u6570\u636e\u201d\uff08\u540e\u53f0\u5faa\u73af\u3001\u5b9a\u65f6\u5668\u3001
//! \u56de\u8c03\uff09\u2014\u2014\u90a3\u4e9b\u5730\u65b9\u62ff\u4e0d\u5230 `EventCtx`\u3002
//!
//! `State<T>` \u628a\u201c\u6539\u503c\u201d\u548c\u201c\u6807\u810f\u5c5e\u4e3b\u201d\u7ed1\u5728\u4e00\u8d77\uff1a
//!
//! ```ignore
//! let mut count = tree.use_state("counter", 0u32);   // \u7ed1\u5b9a\u5230 id="counter" \u7684\u7ec4\u4ef6
//! count.set(count.get() + 1);                        // \u81ea\u52a8\u628a "counter" \u6807\u810f
//! ```
//!
//! \u5b9e\u73b0\u4e0a\u5b83\u5c31\u662f `Signal<T>` + \u4e00\u4e2a owner id\uff0c\u6ca1\u6709\u65b0\u7684\u54cd\u5e94\u5f0f\u5f15\u64ce\u3002

//! 注意：它不在 crate 根重导出（根上的 `State` 是主题状态
//! `Normal/Hovered/Pressed`）。用 `use y901_ui::state::State;`。
//!
//! 两个 `State` 重名不是命名惰性：主题的是“控件外观态”，这个是“组件数据态”，
//! 概念上就应该分开。

use crate::signal::{Invalidation, Signal};

/// \u4e00\u4e2a\u7ed1\u5b9a\u5230\u67d0\u4e2a\u63a7\u4ef6 id \u7684\u53ef\u53d8\u72b6\u6001\u3002
///
/// \u8bfb\u53d6\u4e0d\u6807\u810f\uff1b\u5199\u5165\uff08\u503c\u771f\u6b63\u53d8\u5316\u65f6\uff09\u628a\u6240\u6709\u8ba2\u9605\u8005\u63a8\u8fdb\u5931\u6548\u961f\u5217\uff0c
/// \u4e0b\u4e00\u5e27\u7531 `UiTree` \u7edf\u4e00\u628a\u5b83\u4eec\u6807\u810f\u3002
pub struct State<T> {
    signal: Signal<T>,
    owner: String,
}

impl<T: Clone + PartialEq + 'static> State<T> {
    /// \u65b0\u5efa\u4e00\u4e2a\u72b6\u6001\uff08\u4e0d\u4f1a\u81ea\u52a8\u8ba2\u9605\uff1b\u7528 [`crate::UiTree::use_state`] \u624d\u4f1a\uff09\u3002
    pub fn new(owner: impl Into<String>, initial: T, queue: Invalidation) -> Self {
        let signal = Signal::new(initial);
        let owner = owner.into();
        signal.subscribe(owner.clone(), queue);
        Self { signal, owner }
    }

    /// \u5f52\u5c5e\u7684\u63a7\u4ef6 id\u3002
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// \u8bfb\u53d6\u5f53\u524d\u503c\uff08\u62f7\u8d1d\uff09\u3002
    pub fn get(&self) -> T {
        self.signal.get()
    }

    /// \u5199\u5165\u65b0\u503c\u3002
    ///
    /// \u8fd4\u56de\uff1a`true` = \u503c\u53d8\u4e86\u4e14\u5df2\u901a\u77e5\uff08\u4e0b\u4e00\u5e27\u81ea\u52a8\u6807\u810f\uff09\uff1b
    /// `false` = \u503c\u76f8\u540c\uff0c\u4ec0\u4e48\u90fd\u6ca1\u53d1\u751f\u3002
    pub fn set(&self, value: T) -> bool {
        self.signal.set(value)
    }

    /// \u5c31\u5730\u4fee\u6539\uff08\u907f\u514d\u4e3a\u5927\u5bf9\u8c61\u505a\u4e00\u6b21\u514b\u9686\uff09\u3002
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.signal.update(f);
    }

    /// \u53d8\u66f4\u65f6\u8ba9\u53e6\u4e00\u4e2a\u63a7\u4ef6\u4e5f\u6807\u810f\uff08\u4f8b\uff1a\u7236\u5bb9\u5668\u9700\u8981\u91cd\u6392\uff09\u3002
    pub fn also_invalidate(&self, other: impl Into<String>, queue: Invalidation) {
        self.signal.subscribe(other, queue);
    }
}

impl<T: Clone + PartialEq + 'static> std::fmt::Debug for State<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("owner", &self.owner)
            .field("value", &self.get())
            .finish()
    }
}
