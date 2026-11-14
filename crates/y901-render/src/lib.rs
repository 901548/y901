//! # y901-render
//!
//! **平台无关的渲染层**：纯 CPU 光栅化器 + 字形引擎契约 + 自研 TTF 实现。
//!
//! ## 为什么要抽这一层
//!
//! 三个平台的差异只在"窗口 + 事件 + 像素上屏"。渲染本身（矩形/圆角/渐变/字形）
//! 是一模一样的——抽出来之后，X11 与将来的 macOS 只需要写"`present(&[u8])`就行。
//!
//! ## 依赖约束（硬规则）
//!
//! - **不得依赖 `windows-sys`** —— 否则 X11/macOS 端根本编不过；
//! - 只依赖 `y901-ui`（`RenderCtx` 契约）与 `ttf-parser`（纯 Rust）。
//!
//! ## 层次
//!
//! ```text
//! y901-ui      ← RenderCtx / TextMetrics / Theme / Color
//!    ↑
//! y901-render  ← SoftBackend（光栅化） + FontEngine trait + MyFontEngine
//!    ↑                    ↑
//! y901-backend-win32      y901-backend-x11
//! （D3D11 + GDI 上屏）    （X11 上屏）
//! ```

#![deny(missing_docs)]
/// 图像缩放（mipmap 生成与采样）。
pub mod mip;

/// 图集存储与 LRU 驱逐。
pub mod atlas;
/// 字体管理（加载、指标、字形缓存）。
pub mod font;
/// 自研 TrueType 光栅器（不依赖 DirectWrite）。
pub mod font_engine;
/// 圆角矩形 SDF（与 D3D 着色器逐字对应）。
pub mod sdf;

/// 缺字替身映射（当前只有 U+2212 → '-')。
pub mod font_fallback;

/// 两个渲染后端共享的渲染状态（剪往栈 / 变换栈 / 主题 / 动画）。
pub mod render_state;
/// CPU 光栅后端（SoftBackend）。
pub mod soft;
#[cfg(test)]
mod supersample;

pub use font::{cell_metrics_for, DirtyRect, FontEngine, GlyphInfo};
pub use font_engine::{
    debug_ascii, default_test_font_path, glyph_geometry, load_font, polygon_area, rasterize_glyph,
    rasterize_glyph_phased, resolve_font_path, GlyphBitmap, GlyphGeometry, Line, MyFontEngine,
};
pub use soft::SoftBackend;
