//! Win32 平台后端：D3D11 渲染 + DirectWrite 字体 + IME。
//!
//! 这是 `y901_ui::RenderCtx` / `TextMetrics` 的唯一实现，也是 `PlatformRuntime`
//! 的 Win32 实现。上层（y901-app）不直接依赖 `windows-sys` 的类型。

pub mod backend;
pub mod ime;
pub mod runtime;
pub mod window;

pub use backend::font::cell_metrics_for;
// 字形引擎的平台无关部分现在在 `y901-render`（不依赖 windows-sys）。
pub use y901_render::font_engine;
pub use y901_render::soft;
pub use backend::D3dBackend;
pub use runtime::Win32Runtime;
pub use window::Window;
