//! # Y901
//!
//! **纯 Rust 桌面 UI 框架的单一入口**。依赖这一个 crate 就够：
//!
//! ```toml
//! [dependencies]
//! y901 = "0.3.0"
//! ```
//!
//! ```no_run
//! use y901::prelude::*;
//!
//! // 业务代码不需要任何 `#[cfg(target_os)]`：平台分叉在 y901 内部完成。
//! let runtime = y901::create_runtime();
//! let _ = runtime; // 真正启动需要窗口环境（见 README 的 hello 示例）
//! ```
//!
//! ## 结构
//!
//! 子 crate 都是**实现细节**，这里统一 re-export；需要更细的依赖控制时才直接用它们：
//!
//! | 模块 | 子 crate | 职责 |
//! |---|---|---|
//! | [`core`] | `y901-core` | 终端网格、重排、列宽等纯数据原语 |
//! | [`ui`] | `y901-ui` | 控件树、布局、事件、主题、渲染契约 |
//! | [`render`] | `y901-render` | 字体引擎、软光栅器、图集、mip |
//! | [`widgets`] | `y901-widgets` | 通用基础控件（按钮/文本/输入/滚动/布局容器） |
//! | [`extras`] | `y901-extras` | 面向具体子系统的复杂控件 |
//! | [`runtime`] | `y901-runtime` | 平台无关事件循环与 `Host`/`PlatformRuntime` 契约 |
//! | [`shell`] | `y901-shell` | `create_runtime()`、剪贴板、密钥、快照 |
//! | [`backend`] | Windows / X11 / Web | 当前目标的平台后端（`cfg` 选择） |
//!
//! ## 加新模块的规矩
//!
//! 新能力**先**进对应的子 crate，**再**在这里 re-export 一次 —— 这样"一个入口"不会
//! 随时间退化成"入口只覆盖一半"。`prelude` 只放**启动应用必用**的类型，
//! 不要把每个控件都塞进去（用户 `use y901::widgets::*` 即可）。
//!
//! ## 为什么不是"一个大 crate"
//!
//! - **编译期**：改一个控件不必重编渲染器与平台后端；
//! - **依赖裁剪**：只想用布局原语的人不必拉 D3D11 / X11 协议栈；
//! - **平台条件依赖**：`backend` 只在对应目标平台参与编译（wasm 用 `cdylib` 导出，
//!   桌面完全不编译浏览器绑定）。
//!
//! 代价是安装时依赖树里会看到若干 `y901-*` —— 但它们都是同一个 workspace 的
//! **同版本** crate，不引入任何第三方 C/C++ 依赖。

#![deny(missing_docs)]

/// 终端网格、重排与列宽等**纯数据**原语。
pub use y901_core as core;
/// 面向具体子系统的扩展控件。
pub use y901_extras as extras;
/// 字体引擎、软光栅器、图集与 mip。
pub use y901_render as render;
/// 平台无关的事件循环骨架与 `Host` / `PlatformRuntime` 契约。
pub use y901_runtime as runtime;
/// 应用壳：`create_runtime()`、剪贴板、密钥、快照。
pub use y901_shell as shell;
/// 控件树、布局、事件、主题与渲染契约。
pub use y901_ui as ui;
/// 通用基础控件。
pub use y901_widgets as widgets;

/// 当前目标平台的**具体后端实现**（浏览器 / wasm32）。
#[cfg(target_arch = "wasm32")]
pub use y901_backend_web as backend;
/// 当前目标平台的**具体后端实现**。
///
/// 业务代码通常**不需要**它（用 [`create_runtime`] 即可）；需要触碰后端细节
/// （例如查询 X11 的 DPI 缩放）时才用。
#[cfg(target_os = "windows")]
pub use y901_backend_win32 as backend;
/// 当前目标平台的**具体后端实现**（X11）。
#[cfg(target_os = "linux")]
pub use y901_backend_x11 as backend;

/// 创建**当前平台**的运行时。
///
/// 这是启动一个 Y901 应用的唯一入口：返回的 `Box<dyn PlatformRuntime>` 只需要
/// `.run(host, build, options)`，业务代码不出现任何平台 `cfg`。
///
/// 未实现的平台不会 panic，而是返回一个"运行即报错"的实现在 `run` 时给出明确提示。
pub use y901_shell::create_runtime;

/// 启动应用与画界面**必用**的类型。
///
/// 只放"每个应用都会用到"的东西；控件请 `use y901::widgets::*`，
/// 特殊能力请直接用对应模块。**不要**把它扩成"全量 re-export"——
/// 那会让 glob 导入变得不可预测。
/// 启动应用与画界面**必用**的类型。
///
/// 只放"每个应用都会用到"的东西；控件请 `use y901::widgets::*`，
/// 特殊能力请直接用对应模块。**不要**把它扩成"全量 re-export"——
/// 那会让 glob 导入变得不可预测。
///
/// 注意这里**逐个列**控件，而不是 `pub use crate::widgets::prelude::*`：
/// 已发布的 `y901-widgets 0.2.5` 里没有 `prelude` 模块（那是本版新加的），
/// 而 `cargo publish` 的校验构建是**对着 registry 里已发布的依赖**编的 ——
/// 引用它会让 `y901` 自己发不出去（实测报 `unresolved import crates::widgets::prelude`）。
/// 逐项列只用各版本都有的根 re-export，更稳。
pub mod prelude {
    pub use crate::runtime::{Host, Options, PlatformRuntime};
    pub use crate::ui::{Color, Constraints, Pos, Rect, Size, TextStyle, UiEvent, UiTree, Widget};
    pub use crate::widgets::{
        Badge, Button, Checkbox, ComboBox, Image, ProgressBar, RadioGroup, ScrollView, Slider,
        Spinner, Switch, Tabs, TextInput, TextLabel, Tooltip, VerticalList,
    };
}

#[cfg(test)]
mod tests {
    /// **入口契约**：`create_runtime()` 必须可直接调用（构造 runtime 不需要窗口），
    /// 且返回的 trait object 能在未 `run` 的情况下安全析构。
    #[test]
    fn single_entry_creates_a_runtime() {
        let rt = crate::create_runtime();
        drop(rt); // 析构不得 panic（真正的窗口在 run 里才创建）
    }

    /// **prelude 可用性**：`use y901::prelude::*` 之后，常用类型与控件名不冲突、
    /// 且确实是同一个类型（这里抽查 `Size` 与 `UiTree`）。
    #[test]
    fn prelude_reexports_the_real_types() {
        use crate::prelude::*;
        let s = Size::new(3.0, 4.0);
        assert_eq!((s.w, s.h), (3.0, 4.0));
        // 只验证类型身份：`UiTree` 来自 y901-ui（`y901::ui::UiTree`）。
        fn takes_ui_tree(_t: &UiTree) {}
        let _ = takes_ui_tree;
    }

    /// **模块别名必须指向同一个 crate**：`y901::ui` 与 `y901_ui` 是同一套类型。
    #[test]
    fn module_aliases_point_at_the_same_crates() {
        let a: crate::ui::Size = crate::ui::Size::new(1.0, 2.0);
        let b: y901_ui::Size = a;
        assert_eq!(b.w, 1.0);
    }
}
