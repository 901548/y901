//! 平台后端之间的**共同部分**。
//!
//! 这里只放两个后端（Win32 / X11）已验证同构的东西：
//!
//! - [`window::WindowBackend`]：一次事件循环需要的窗口能力
//! - [`runtime::run_loop`]：两边逐段同构的事件循环骨架
//!
//! **不放**平台特有的东西：窗口创建、事件来源初始化、IME、光标形状、
//! D3D 与 CPU 光栅器的差异、Present 细节。它们留在各自后端，
//! 通过 trait 方法或钩子注入，不在本 crate 里写 `cfg` 分支。

#![deny(missing_docs)]

pub mod runtime;
pub mod window;
