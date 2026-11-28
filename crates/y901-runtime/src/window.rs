//! 窗口抽象：事件循环需要的**最小**能力集。
//!
//! 设计原则：只收两个后端**同名且语义相同**的能力。语义不同的一律不进 trait，
//! 留给后端自己实现（见各方法说明里的"语义"一条）。

use crate::{FontConfig, RawEvent};
use y901_ui::theme::Theme;
use y901_ui::UiTree;

/// 一次 `render` 调用的结果。
///
/// 用于让 `run_loop` 知道本帧是否真的提交了像素，
/// 而**不需要自己再查 `tree.dirty()`**——脏区判定只有一处：后端 `render` 内部。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderOutcome {
    /// 没有脏区，后端跳过了本次渲染（保留上一帧）
    Skipped,
    /// 后端渲染并提交了本帧（full 还是 partial 由后端自己定）
    Presented,
}

/// 一次事件循环迭代需要窗口提供的能力。
///
/// 实现者负责把平台事件循环的差异藏在自己的实现里：
///
/// - Win32 是"非阻塞 pump + 空闲时 wait_message"
/// - X11 是"带超时的 poll"
///
/// 调用方只看到 [`WindowBackend::wait_events`]。
pub trait WindowBackend {
    /// 取一批事件；`wait_ms` 是空闲时的最长等待。
    ///
    /// 返回 `None` 表示窗口要求退出（关闭按钮 / 连接断开）。
    /// 返回 `Some(vec![])` 表示超时且没有事件——这不是退出。
    ///
    /// **语义**：本方法可以阻塞至多 `wait_ms`。
    fn wait_events(&mut self, wait_ms: u32) -> Option<Vec<RawEvent>>;

    /// 当前逻辑视口尺寸（客户区，单位：逻辑像素）。
    fn viewport(&self) -> (u32, u32);

    /// 重新布局：把新尺寸告知后端并让整棵树按新视口重排。
    ///
    /// **语义**：两个后端的实现不同（Win32 由窗口驱动、X11 先 resize 光栅器再 layout），
    /// 但对调用方而言都是"尺寸变了，请重排并全量重绘"。
    fn relayout(&mut self, tree: &mut UiTree);

    /// 绘制一帧（含提交到屏幕）。
    ///
    /// **语义**：Win32 内部走 D3D 脏区提交 + Present；X11 走 CPU 光栅器 + PutImage。
    /// 调用方不关心差异，只保证"返回后这一帧已经可见"。
    ///
    /// **脏区判定就在这里**：
    /// - `tree.dirty()` 为空 → 返回 [`RenderOutcome::Skipped`]，不提交。
    /// - 非空 → 返回 [`RenderOutcome::Presented`]。**用 full 还是 partial 由后端自己决定**，
    ///   trait 不做统一要求（Win32 D3D 走 partial，Win32 soft 与 X11 走 full）。
    ///
    /// **调用方不得自己查 `tree.dirty()`**：两层判定会互相掩盖，
    /// 一旦两边不一致就会出现"有脏区但不画"的难查 bug。
    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String>;

    /// 运行时切换 borderless fullscreen。
    ///
    /// 默认实现为空操作（供 mock / 旧后端编译通过）。
    ///
    /// 后端实现时注意：
    /// - **Win32**：需保存/恢复窗口样式与位置
    ///   （`SetWindowLongPtrW` + `SetWindowPos(.., SWP_FRAMECHANGED)` + `MoveWindow`）
    /// - **X11**：发 EWMH `_NET_WM_STATE` ClientMessage（`data[0]` = 1 ADD / 0 REMOVE），
    ///   边框与几何由 WM 负责恢复
    fn set_fullscreen(&mut self, _on: bool) -> Result<(), String> {
        Ok(())
    }

    /// 切换主题。失败不影响后续帧，调用方只记录日志。
    fn set_theme(&mut self, theme: Theme) -> Result<(), String>;

    /// 切换字体（可选能力）。
    ///
    /// X11 后端在启动时固定字体，返回 `Unsupported` 即可；
    /// 这是**能力可缺省**，不是语义不同。
    fn set_font(&mut self, font: &FontConfig) -> Result<(), String> {
        let _ = font;
        Err("set_font: 该后端不支持运行时切换字体".to_string())
    }

    /// **渲染之前**的后端收尾：IME 定位、字体请求、重绘请求、光标形状、快照**请求**。
    ///
    /// 这里**可以修改 `tree`**（例如 `invalidate_all()`），因为它跑在渲染之前——
    /// 修改会在本帧生效。默认空实现；平台特有的东西放这里，**不进 run_loop**。
    fn after_frame(&mut self, _tree: &mut UiTree, _host: &mut dyn crate::Host) {}

    /// **Present 之后**的钩子（快照**回读**、显示后的统计等）。
    ///
    /// 为何不合并到 `after_frame`：快照必须在**本帧已经提交**之后才能回读像素，
    /// 放到渲染前去读只会得到**上一帧**的内容。两个钩子不是冗余：
    /// `after_frame` 可以改 `tree`，`after_present` 不会改——它只读取已提交的结果。
    fn after_present(&mut self, _tree: &mut UiTree, _host: &mut dyn crate::Host) {}

    /// **空闲时也刷一次无障碍树**（默认空实现）。
    ///
    /// 为什么必须有：无障碍树是"推"给 UIA/AT-SPI 的，而我们的推送跟在**渲染**后面 ——
    /// 应用空闲不重绘时就不推送。于是**读屏器在应用空闲之后才启动时，会读到一棵空树**
    /// （实测：启动 `a11y_demo`，等它静下来，再查询 UIA，`DESCENDANTS=0`；
    /// 动一下窗口强制重绘，树立刻回来）。用户看到的现象是"读屏器说这个窗口什么都没有"。
    ///
    /// `run_loop` 在空闲帧里按固定间隔调用它（见 `A11Y_REFRESH`），后端实现只需
    /// "把当前树再推一次；没有客户端连接时这次推送是零开销"。
    fn refresh_a11y(&mut self, _tree: &mut UiTree) {}

    /// 本后端的分阶段统计**种类**；`None` = 该后端未接入统计（默认）。
    ///
    /// 为什么需要它：`render_stats::RenderStats` 构造时必须知道后端种类，
    /// 而种类决定**能力表**（哪些阶段可测、哪些不可测），能力表又是
    /// `validate()` 拒绝伪造数据的依据。runtime 无法从窗口对象猜出种类
    ///（`--force-soft` 的 Win32 与 D3D11 是同一类型），所以由后端自报。
    ///
    /// 默认 `None`：测试替身与未接入的后端无需改动。
    fn render_stats_backend(&self) -> Option<crate::render_stats::BackendKind> {
        None
    }

    /// 取走**上一帧**的分阶段统计；`None` = 没有可用统计（默认）。
    ///
    /// 语义是"取走"：调用方每帧最多取一次，第二次调用必然得到 `None`，
    /// 因此不存在重复消费；每帧由后端重新构造，因此不存在跨帧污染。
    ///
    /// 默认实现返回 `None` —— 这是**可选能力**，已有后端与测试替身无需修改即可编译。
    fn take_render_stats(&mut self) -> Option<crate::render_stats::RenderStats> {
        None
    }
}
