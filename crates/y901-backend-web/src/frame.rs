//! 帧步：直接委托给 `y901_runtime::frame_step`（与桌面共用同一段逻辑）。

pub use y901_runtime::runtime::{frame_step, FrameStats};
