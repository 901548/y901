//! # y901-ui
//!
//! Y901 的框架层：**不认识 D3D11，不认识 PTY，不认识 Win32，也不认识任何业务**。
//!
//! 三层结构：
//!
//! ```text
//! y901-ui      框架：Widget trait / UiTree / Flex 布局 / 事件路由 / RPC
//! y901-widgets 组件：只实现 Widget，互不认识
//! y901-app     应用：Win32 窗口 + D3D11 后端（实现 RenderCtx）+ 启动
//! ```
//!
//! 平台能力通过两个 trait 注入：
//! - [`widget::TextMetrics`]：布局期要的字体度量
//! - [`widget::RenderCtx`]：渲染期的画布
//!
//! 框架对二者的实现一无所知，所以同一棵树既能画到 D3D11，
//! 也能画到测试用的记录器里。
//!
//! ## 最小例子
//!
//! ```no_run
//! use y901_ui::{Axis, Flex, UiTree};
//!
//! let root = Flex::column("root")
//!     .push(Box::new(Flex::row("toolbar").flex_item(y901_ui::FlexItem::fixed(0.0, 32.0))))
//!     .push(Box::new(Flex::row("content").flex_item(y901_ui::FlexItem::fill())));
//!
//! let mut tree = UiTree::new(Box::new(root));
//! // tree.layout(viewport, &metrics);
//! // tree.render(&mut backend);
//! ```

pub mod color;
pub mod command;
pub mod event;
pub mod geometry;
pub mod invalidate;
pub mod json;
pub mod layout;
pub mod query;
pub mod signal;
pub mod state;
pub mod text;
pub mod theme;
pub mod tree;
pub mod widget;

pub use color::{Color, TextStyle};
pub use command::Command;
pub use event::{EventPhase, EventResult, Key, Modifiers, MouseButton, PointerCapture, UiEvent};
pub use geometry::{Edges,Pos,Rect,Size, place_popup};
pub use invalidate::{InvalidationHandle, PendingInvalidation};
pub use json::Json;
pub use layout::{flex_arrange, flex_measure, layout_tree, Flex, Stack};
pub use query::{run as query_nodes, Query};
pub use signal::{Invalidation, Signal};
pub use text::TextBuffer;
pub use theme::{State, Style, Theme};
pub use tree::{hash_semantics, DirtyRegion, NodeInfo, UiTree};
pub use widget::MountCtx;
pub use widget::{
    ActionSpec, Axis, Axis2, Constraints, Cursor, Dimension, EventCtx, FlexItem, InputKind,
    RenderCtx, Role, Semantics, TextMetrics, Widget, AA_SAMPLES, GRADIENT_STEPS,
};
