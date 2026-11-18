//! **输入法（IME）抽象层**。
//!
//! 目标：让 X11 后端能够接收中文输入，而不把具体输入法框架写死进事件循环。
//!
//! 设计要点：
//! 1. **框架契约不变**：IME 产生的事件最终变成 `RawEvent::Ime { text, composition }`，
//!    这两者在 `y901-runtime` / `Widgets`（`TextInput.preedit`）里早已支持（Web 端就是这么接的），
//!    因此不需要改 `Widget` / `RenderCtx` / `EventCtx` / `runtime`。
//! 2. **可降级**：IBus 未运行、DBus 不可达时返回 [`NoIme`]，按键一律放行，
//!    **行为与"没有 IME"完全一致**（不崩溃、不吞键、不改变英文输入）。
//! 3. **线程模型**：D-Bus 是异步的，但主循环是同步阻塞的（`wait_events`）。
//!    因此 IME 客户端内部用**独立线程 + `mpsc`** 收集信号，主循环只做非阻塞的 `try_recv`
//!    —— 与 `clipboard.rs` 已有的线程模式一致。

use std::sync::mpsc::Receiver;

/// IBus 客户端实现（见 `ibus.rs`）。
pub mod ibus;

/// 一次来自输入法的文本事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// 预编辑串：**只显示不提交**（对应 `RawEvent::Ime { composition: true }`）。
    Preedit(String),
    /// 已提交文本：**插入到输入框**（对应 `RawEvent::Ime { composition: false }`）。
    Commit(String),
}

/// 一个按键交给输入法处理后的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// 输入法不关心这个键：照旧发 `RawEvent::KeyDown`。
    Forward,
    /// 输入法吃掉了这个键（例如组字中的字母、空格选词）：**不再发 `KeyDown`**。
    Consumed,
}

/// 输入法客户端。
///
/// 所有方法都必须**非阻塞**（除 `focus_*` 这类一次性控制调用外），
/// 因为它们都在事件循环里被调用。
pub trait ImeClient: Send {
    /// 是否真的接上了输入法（`NoIme` 恒为 false）。
    fn available(&self) -> bool;

    /// 把按键交给输入法；返回 [`KeyOutcome::Consumed`] 时调用方不得再发 `KeyDown`。
    fn filter_key(&mut self, keyval: u32, keycode: u32, state: u32) -> KeyOutcome;

    /// 告知输入法候选窗应该出现在哪里（输入框光标位置，屏幕坐标）。
    fn set_cursor_location(&mut self, x: i32, y: i32, w: i32, h: i32);

    /// 输入框获得焦点（IME 需要它来跟踪组字状态）。
    fn focus_in(&mut self);

    /// 输入框失去焦点（同时应提交/丢弃未完成的组字）。
    fn focus_out(&mut self);

    /// 取走本轮已到达的事件（**非阻塞**，主循环每帧调用一次）。
    fn take_events(&mut self) -> Vec<ImeEvent>;
}

/// **降级实现**：没有输入法（IBus 未运行 / 非 Linux / D-Bus 不可用）。
///
/// 语义保证：`filter_key` 永远 `Forward`、`take_events` 永远空 ——
/// 因此"没有 IME"与"IME 关闭"在事件流上**完全不可区分**。
#[derive(Debug, Default)]
pub struct NoIme;

impl ImeClient for NoIme {
    fn available(&self) -> bool {
        false
    }

    fn filter_key(&mut self, _keyval: u32, _keycode: u32, _state: u32) -> KeyOutcome {
        KeyOutcome::Forward
    }

    fn set_cursor_location(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {}

    fn focus_in(&mut self) {}

    fn focus_out(&mut self) {}

    fn take_events(&mut self) -> Vec<ImeEvent> {
        Vec::new()
    }
}

/// 事件接收端的通用小工具：把 `Receiver` 里能拿到的全部事件取出来。
pub(crate) fn drain(rx: &Receiver<ImeEvent>) -> Vec<ImeEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        // 连续相同预编辑串没有意义：去重可以省一次重绘。
        if let (Some(ImeEvent::Preedit(prev)), ImeEvent::Preedit(next)) = (out.last(), &ev) {
            if prev == next {
                continue;
            }
        }
        out.push(ev);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// `NoIme` 必须**不吞键、不产生事件、不抢焦点行为**。
    #[test]
    fn no_ime_forwards_every_key() {
        let mut ime = NoIme;
        assert!(!ime.available());
        for kv in [0x61u32, 0x0d, 0x20, 0x1008ff] {
            assert_eq!(ime.filter_key(kv, 0, 0), KeyOutcome::Forward);
        }
        assert!(ime.take_events().is_empty());
    }

    /// 连续相同预编辑串只保留一次（避免无意义重绘）。
    #[test]
    fn drain_dedups_consecutive_identical_preedit() {
        let (tx, rx) = channel();
        tx.send(ImeEvent::Preedit("ni".into())).unwrap();
        tx.send(ImeEvent::Preedit("ni".into())).unwrap();
        tx.send(ImeEvent::Preedit("nih".into())).unwrap();
        tx.send(ImeEvent::Commit("\u{4f60}\u{597d}".into()))
            .unwrap();
        let evs = drain(&rx);
        assert_eq!(
            evs,
            vec![
                ImeEvent::Preedit("ni".into()),
                ImeEvent::Preedit("nih".into()),
                ImeEvent::Commit("\u{4f60}\u{597d}".into()),
            ]
        );
    }
}
