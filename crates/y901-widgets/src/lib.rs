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

#![deny(missing_docs)]
pub mod alert;
pub mod badge;
pub mod button;
pub mod checkbox;
pub mod combo_box;
pub mod context_menu;
pub mod empty_state;
pub mod error_state;
pub mod image;
pub mod label;
pub mod list;
pub mod list_view;
pub mod overlay;
pub mod popover;
pub mod progress;
pub mod radio;
pub mod rich_text;
pub mod scroll;
pub mod skeleton;
pub mod slider;
pub mod spinner;
pub mod split;
mod state_layout;
pub mod style;
pub mod switch;
pub mod table;
pub mod tabs;
pub mod text_input;
pub mod tooltip;
pub mod tree_view;

pub use badge::{Badge, BadgeKind};
pub use button::Button;
pub use checkbox::Checkbox;
pub use combo_box::ComboBox;
pub use context_menu::ContextMenu;
pub use image::Image;
pub use label::{parse_color, TextLabel};
pub use list::VerticalList;
pub use list_view::{ListView, RowPainter};
pub use overlay::{Dialog, Menu};
pub use popover::Popover;
pub use progress::ProgressBar;
pub use radio::RadioGroup;
pub use rich_text::{Block, InlineStyle, RichText, Span, StyledText};
pub use scroll::{ScrollView, Scrollbar};
pub use skeleton::{Skeleton, SkeletonShape};
pub use slider::Slider;
pub use split::SplitView;
pub use style::WidgetStyle;
pub use tabs::Tabs;
pub use text_input::{Clipboard, MemoryClipboard, NullClipboard, TextInput};
pub use tooltip::{Tooltip, TooltipHost};
pub use tree_view::{TreeItem, TreeView};

/// 把一条命令的 `args` 替换成新的（`Command` 本身没有 `with_args`，只能重建）。
///
/// 为什么在组件层而不是 `y901-ui`：这只是"带上本次事件的参数"的便利函数，
/// 属于组件约定而非框架能力；框架侧保持 `Command` 的极简定义。
pub(crate) fn command_with_args(
    cmd: &y901_ui::Command,
    args: y901_ui::json::Json,
) -> y901_ui::Command {
    use y901_ui::Command;
    match cmd {
        Command::App { name, .. } => Command::app(name.clone(), args),
        Command::Node { target, action, .. } => Command::node(target.clone(), action.clone(), args),
    }
}
pub use alert::{Alert, AlertKind};
pub use empty_state::EmptyState;
pub use error_state::ErrorState;
pub use spinner::Spinner;
pub use switch::Switch;
pub use table::{Table, TableColumn};
/// 常用控件的 glob 导入入口：`use y901_widgets::prelude::*;`。
///
/// 为什么单独一个 prelude 而不是让用户 `use y901_widgets::*`：
/// 后者会把 `Block` / `Span` / `RowPainter` 这类**只在特定场景用**的类型
/// 也塞进作用域，容易与业务自己的同名类型撞车。这里只放"画界面几乎必用"的那批。
pub mod prelude {
    pub use crate::{
        Badge, Button, Checkbox, ComboBox, Image, ProgressBar, RadioGroup, ScrollView, Slider,
        Spinner, Switch, Tabs, TextInput, TextLabel, Tooltip, VerticalList,
    };
}
