//! Y901 扩展组件库：**不进入核心框架**的、依赖具体子系统的组件。
//!
//! 与 `y901-widgets` 的边界：
//!
//! | | `y901-widgets` | `y901-extras` |
//! |---|---|---|
//! | 定位 | 通用基础组件（按钮/文本/输入/滚动/布局） | 面向具体子系统的复杂组件 |
//! | 依赖 | 只依赖 `y901-ui`（+ 纯数据 `y901-core`） | 允许依赖 PTY 等外部系统 |
//! | 例子 | `Button` `TextLabel` `TextInput` `ScrollView` `Flex` `Stack` | `TerminalWidget`（Grid + VT 解析 + PTY） |
//!
//! 为什么把终端拆出去：终端的"网格排版 + VT 转义序列 + PTY 进程"是**策略**，
//! 不是 GUI 框架的**机制**。留在 `y901-widgets` 里会有两个坏结果：
//!
//! 1. 基础组件库被迫依赖 `portable-pty` / `crossbeam-channel`，任何只想用按钮的
//!    应用也得把 PTY 拉进依赖树；
//! 2. 框架的"通用性"叙事被一个具体业务打破——下一个要用串口/日志流的组件
//!    也会想往 `y901-widgets` 里塞。

use y901_ui::Role;

/// 终端组件的语义角色。
///
/// 它属于**扩展层**，不在 `y901-ui` / `y901-widgets` 里——框架只认识
/// panel/button/text 这些通用角色，"terminal" 是子系统自己定义的。
pub const ROLE_TERMINAL: Role = Role("terminal");

pub mod palette;
pub mod terminal;
pub mod vt;

pub use terminal::TerminalWidget;
