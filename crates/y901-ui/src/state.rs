//! 组件局部状态：“改了就自动标脏”的 `State<T>`。
//!
//! 为什么还需要它：`EventCtx::request_redraw()` 能解决“事件里知道自己脏了”，
//! 但解决不了“**非事件**途径改数据”（后台循环、定时器、
//! 回调）——那些地方拿不到 `EventCtx`。
//!
//! `State<T>` 把“改值”和“标脏属主”绑在一起：
//!
//! ```text
//! let mut count = tree.use_state("counter", 0u32);   // 绑定到 id="counter" 的组件
//! count.set(count.get() + 1);                        // 自动把 "counter" 标脏
//! ```
//!
//! 实现上它就是 `Signal<T>` + 一个 owner id，没有新的响应式引擎。

//! 注意：它不在 crate 根重导出（根上的 `State` 是主题状态
//! `Normal/Hovered/Pressed`）。用 `use y901_ui::state::State;`。
//!
//! 两个 `State` 重名不是命名惰性：主题的是“控件外观态”，这个是“组件数据态”，
//! 概念上就应该分开。

use crate::signal::{Invalidation, Signal};

/// 一个绑定到某个控件 id 的可变状态。
///
/// 读取不标脏；写入（值真正变化时）把所有订阅者推进失效队列，
/// 下一帧由 `UiTree` 统一把它们标脏。
pub struct State<T> {
    signal: Signal<T>,
    owner: String,
}

impl<T: Clone + PartialEq + 'static> State<T> {
    /// 新建一个状态，**并立即订阅**（`Signal::set` 会通过 `queue` 标脏该 owner）。
    ///
    /// 旧文档写的是"不会自动订阅，要 `use_state` 才会" —— 与代码不符：构造函数本身
    /// 就订阅了。按旧文档推断的调用方会以为"没订阅所以不用管"，而真实语义是
    /// "已经订阅、随 `State` 一起销毁"：`State` 持有的 `Signal` 是它**私有**的，
    /// `State` 析构时信号与订阅表一起消失，不会残留 listener。
    ///
    /// 需要"多个控件共享同一份数据"时，请把 `Signal` 克隆出去分别订阅
    ///（`Signal::subscribe` 按 owner 去重，重复订阅只更新队列）。
    pub fn new(owner: impl Into<String>, initial: T, queue: Invalidation) -> Self {
        let signal = Signal::new(initial);
        let owner = owner.into();
        signal.subscribe(owner.clone(), queue);
        Self { signal, owner }
    }

    /// 归属的控件 id。
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// 读取当前值（拷贝）。
    pub fn get(&self) -> T {
        self.signal.get()
    }

    /// 写入新值。
    ///
    /// 返回：`true` = 值变了且已通知（下一帧自动标脏）；
    /// `false` = 值相同，什么都没发生。
    pub fn set(&self, value: T) -> bool {
        self.signal.set(value)
    }

    /// 就地修改（避免为大对象做一次克隆）。
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        self.signal.update(f);
    }

    /// 变更时让另一个控件也标脏（例：父容器需要重排）。
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
