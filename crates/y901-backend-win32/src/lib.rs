//! Win32 平台后端：D3D11 渲染 + DirectWrite 字体 + IME。
//!
//! 这是 `y901_ui::RenderCtx` / `TextMetrics` 的唯一实现，也是 `PlatformRuntime`
//! 的 Win32 实现。上层（y901-app）不直接依赖 `windows-sys` 的类型。

pub mod backend;
pub mod ime;

pub use backend::D3dBackend;
