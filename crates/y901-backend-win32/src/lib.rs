//! Win32 平台后端：D3D11 渲染 + DirectWrite 字体 + IME。
//!
//! 这是 `y901_ui::RenderCtx` / `TextMetrics` 的唯一实现，也是 `PlatformRuntime`
//! 的 Win32 实现。上层（y901-app）不直接依赖 `windows-sys` 的类型。

#![deny(missing_docs)]
// **整个 crate 是 Windows 专用的** —— 在非 Windows 上变成空 crate。
//
// 为何需要这一层：源码里有 4 处裸 `#[link(name = "d3d11"/"d3dcompiler"/"dwrite"/"imm32")]`，
// `#[link]` 是**链接器指令**，不管符号有没有被引用都会要求对应库 ——
// 于是 `cargo test --workspace` 在 Linux 上在**链接阶段**报 `-ld3d11` 找不到。
//
// 为何选 crate 级而不是逐个 extern 块加 cfg：
// crate 级一刀切，**不会漏掉第 5 个 `#[link]`**（与 sync 脚本那次“枚举→生成”同一思路）。
// 跨 crate 只有一处引用（`y901-shell` 的 `create_runtime`），且已在 `#[cfg(target_os = "windows")]` 分支内。
#![cfg(windows)]
pub mod backend;
pub mod ime;
pub mod runtime;
pub mod window;

pub use backend::font::cell_metrics_for;
// 字形引擎的平台无关部分现在在 `y901-render`（不依赖 windows-sys）。
pub use backend::D3dBackend;
pub use runtime::Win32Runtime;
pub use window::Window;
pub use y901_render::font_engine;
pub use y901_render::soft;
