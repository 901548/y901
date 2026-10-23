//! # y901-widgets
//!
//! 官方组件库。约定：
//!
//! 1. 每个组件只实现 `y901_ui::Widget`，不知道后端是 D3D11 还是别的；
//! 2. 组件之间**互不认识**——不引用对方的 id，也不调用对方的方法；
//! 3. 需要联动时，组件只发 `Command`，由应用层解释语义。
//!
//! 目前提供：
//! - [`TerminalWidget`]：Grid + VT 解析 + PTY
//! - [`Button`]：点击只发命令
//! - [`TextLabel`]：只显示文字
//! - [`VerticalList`]：纵向容器样本

use y901_ui::Role;

pub mod button;
pub mod label;
pub mod list;
pub mod palette;
pub mod list_view;
pub mod overlay;
pub mod scroll;
pub mod split;
pub mod terminal;
pub mod text_input;
pub mod vt;

pub use button::Button;
pub use label::TextLabel;
pub use list::VerticalList;
pub use list_view::ListView;
pub use overlay::{Dialog, Menu};
pub use scroll::{Scrollbar, ScrollView};
pub use split::SplitView;
pub use terminal::TerminalWidget;
pub use text_input::{Clipboard, MemoryClipboard, NullClipboard, TextInput};

/// 终端组件自己的语义角色。
///
/// 框架只认识 panel/button/text 这些通用角色；"terminal" 是**组件库**定义的。
/// 这正是框架无需为业务改代码的证明。
pub const ROLE_TERMINAL: Role = Role("terminal");

/// 常用 ANSI 颜色（给别的组件复用）。
pub use palette::{resolve, xterm256, DEFAULT_BG, DEFAULT_FG};
