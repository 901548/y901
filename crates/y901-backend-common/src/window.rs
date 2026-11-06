//! 窗口抽象：事件循环需要的**最小**能力集。
//!
//! 设计原则：只收两个后端**同名且语义相同**的能力。语义不同的一律不进 trait，
//! 留给后端自己实现（见各方法说明里的"语义"一条）。

use y901_runtime::{FontConfig, RawEvent};
use y901_ui::theme::Theme;
use y901_ui::UiTree;

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
    fn render(&mut self, tree: &mut UiTree) -> Result<(), String>;

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
    fn after_frame(&mut self, _tree: &mut UiTree, _host: &mut dyn y901_runtime::Host) {}

    /// **Present 之后**的钩子（快照**回读**、显示后的统计等）。
    ///
    /// 为何不合并到 `after_frame`：快照必须在**本帧已经提交**之后才能回读像素，
    /// 放到渲染前去读只会得到**上一帧**的内容。两个钩子不是冗余：
    /// `after_frame` 可以改 `tree`，`after_present` 不会改——它只读取已提交的结果。
    fn after_present(&mut self, _tree: &mut UiTree, _host: &mut dyn y901_runtime::Host) {}
}
