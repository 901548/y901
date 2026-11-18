//! DOM 事件 → 结构化输入。转成 `RawEvent` 的工作在 `lib.rs` 的 S1 实现里。

/// 键盘输入（`KeyboardEvent.code` + `key`）。
pub struct KeyInput {
    /// 物理键位（如 `"KeyA"` / `"ArrowDown"`）。
    pub code: String,
    /// 字符键值（如 `"a"`）。
    pub key: String,
}

/// 鼠标输入（canvas 相对坐标）。
pub struct MouseInput {
    pub x: f64,
    pub y: f64,
    pub button: i16,
}

/// 滚轮输入。
pub struct WheelInput {
    pub delta_y: f64,
}
