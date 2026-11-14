//! 帧步：与桌面共用的那一段（签名已在 `docs/WASM.md` §8.1 定下）。

use y901_runtime::window::WindowBackend;
use y901_runtime::{Host, RawEvent};
use y901_ui::UiTree;

/// 本帧观测结果，供调用方做节拍 / 统计决策。
pub struct FrameStats {
    /// 本帧是否有事件或布局工作。
    pub had_work: bool,
    /// 本帧是否重新布局。
    pub need_layout: bool,
    /// 本帧是否真正上屏。
    pub presented: bool,
}

/// 消化一批"已到手"的事件并推进一帧；**不含阻塞等待、不含计时**。
///
/// 内部覆盖：事件消化 → 命令/idle → `tick_animations` → 重排 → 主题 →
/// `after_frame` → `render` → `after_present`。Tick 节拍与 profile 计时留在调用方。
pub fn frame_step<W: WindowBackend>(
    window: &mut W,
    tree: &mut UiTree,
    host: &mut dyn Host,
    events: &[RawEvent],
) -> Result<FrameStats, String> {
    let _ = (window, tree, host, events);
    todo!("S1：从 `y901_runtime::runtime::run_loop` 抽出通用段")
}