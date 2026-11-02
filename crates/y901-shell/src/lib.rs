//! # y901-shell
//!
//! **全平台的唯一分叉点**。
//!
//! 为什么单独一个 crate：业务层（Helm）只需 `y901_shell::create_runtime()`，
//! 不得出现任何 `#[cfg(target_os)]`。把分叉集中在**这一个文件**里：
//!
//! - 新增一个平台 → 只改这里的一个 `#[cfg]` 分支 + 新建一个 backend crate；
//! - 业务代码永远只认 `Box<dyn PlatformRuntime>`。
//!
//! ## 当前状态
//!
//! | 平台 | 后端 | 状态 |
//! |---|---|---|
//! | Windows | `y901-backend-win32`（D3D11 + 纯 CPU 软渲染 + Win32 窗口/事件/IME） | ✅ 完整 |
//! | Linux / macOS | 尚未实现（见 `docs/CROSS_PLATFORM_PLAN.md`） | ⏳ S2/S3 排期中 |
//!
//! 未实现的平台不会 panic，而是返回 `Err("请看文档")——这样 CI 能在 Linux 上
//! `cargo build` 通过（做类型检查），只是真正跑的时候会得到一个清楚的错误。

pub mod clipboard;
pub mod widget_clipboard;
pub mod secret;
pub mod snapshot;
pub use widget_clipboard::WidgetClipboard;

use y901_runtime::PlatformRuntime;
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
use y901_runtime::{Host, Options};
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
use y901_ui::UiTree;

/// 创建当前平台的运行时。
///
/// 返回：一个 `Box<dyn PlatformRuntime>`——业务只需调 `.run(...)`。
/// 副作用：无（真正的窗口在 `run` 里创建）。
pub fn create_runtime() -> Box<dyn PlatformRuntime> {
    #[cfg(target_os = "windows")]
    {
        Box::new(y901_backend_win32::runtime::Win32Runtime)
    }

    #[cfg(target_os = "linux")]
    {
        Box::new(y901_backend_x11::runtime::X11Runtime)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Box::new(UnsupportedRuntime)
    }
}

/// 未实现平台的运行时：不 panic，只报错。
///
/// 为什么不 `unimplemented!()`：那会让"Linux 上能不能编译"变成"能编译但一跑就崩"，
/// 而 `cargo check --target x86_64-unknown-linux-gnu` 是我们唯一的跨平台验证手段（见文档 §5）。
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
struct UnsupportedRuntime;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl PlatformRuntime for UnsupportedRuntime {
    fn run(
        &mut self,
        _host: Box<dyn Host>,
        _build: Box<dyn FnOnce() -> UiTree>,
        _options: Options,
    ) -> Result<(), String> {
        Err(format!(
            "当前平台（{} {}）尚未实现后端；\
             已完成 Windows，Linux/X11 与 macOS 见 docs/CROSS_PLATFORM_PLAN.md 的 S2/S4",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    }
}
