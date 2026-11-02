//! # y901-widgets
//!
//! 官方**通用**组件库。约定：
//!
//! 1. 每个组件只实现 `y901_ui::Widget`，不知道后端是 D3D11 还是别的；
//! 2. 组件之间**互不认识**——不引用对方的 id，也不调用对方的方法；
//! 3. 需要联动时，组件只发 `Command`，由应用层解释语义；
//! 4. **不依赖任何具体子系统**（PTY、网络、文件系统…）。
//!    需要那些的组件住在 `y901-extras`。
//!
//! 目前提供：
//! - [`Button`]：点击只发命令
//! - [`TextLabel`]：只显示文字
//! - [`TextInput`]：单行输入（含 IME / 选区 / 剪贴板抽象）
//! - [`ScrollView`]：滚动容器（局部重绘友好）
//! - [`ListView`]：通用虚拟列表（行内容由回调提供）
//! - [`StyledText`]：富文本（段落 + 代码块）
//! - [`Flex`]/[`Stack`]：布局（在 `y901_ui` 里）
//! - [`Menu`]/[`Dialog`]：浮层内容（挂载交 `UiTree::popups`）
//!
//! [`Flex`]: y901_ui::Flex
//! [`Stack`]: y901_ui::Stack

pub mod button;
pub mod label;
pub mod checkbox;
pub mod image;
pub mod list;
pub mod list_view;
pub mod overlay;
pub mod rich_text;
pub mod scroll;
pub mod split;
pub mod style;
pub mod progress;
pub mod tabs;
pub mod text_input;

pub use button::Button;
pub use label::{parse_color, TextLabel};
pub use list::VerticalList;
pub use list_view::{ListView, RowPainter};
pub use overlay::{Dialog, Menu};
pub use rich_text::{Block, InlineStyle, RichText, Span, StyledText};
pub use scroll::{Scrollbar, ScrollView};
pub use split::SplitView;
pub use style::WidgetStyle;
pub use text_input::{Clipboard, MemoryClipboard, NullClipboard, TextInput};
