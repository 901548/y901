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
    /// 客户端 X（CSS 像素，相对 canvas）。
    pub x: f64,
    /// 客户端 Y（CSS 像素，相对 canvas）。
    pub y: f64,
    /// 按键编号（0 左 / 1 中 / 2 右）。
    pub button: i16,
}

/// 滚轮输入。
pub struct WheelInput {
    /// 滚动增量（正 = 向下）。
    pub delta_y: f64,
}
