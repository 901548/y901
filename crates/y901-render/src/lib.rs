//! # y901-render
//!
//! **\u5e73\u53f0\u65e0\u5173\u7684\u6e32\u67d3\u5c42**\uff1a\u7eaf CPU \u5149\u6805\u5316\u5668 + \u5b57\u5f62\u5f15\u64ce\u5951\u7ea6 + \u81ea\u7814 TTF \u5b9e\u73b0\u3002
//!
//! ## \u4e3a\u4ec0\u4e48\u8981\u62bd\u8fd9\u4e00\u5c42
//!
//! \u4e09\u4e2a\u5e73\u53f0\u7684\u5dee\u5f02\u53ea\u5728"\u7a97\u53e3 + \u4e8b\u4ef6 + \u50cf\u7d20\u4e0a\u5c4f"\u3002\u6e32\u67d3\u672c\u8eab\uff08\u77e9\u5f62/\u5706\u89d2/\u6e10\u53d8/\u5b57\u5f62\uff09
//! \u662f\u4e00\u6a21\u4e00\u6837\u7684\u2014\u2014\u62bd\u51fa\u6765\u4e4b\u540e\uff0cX11 \u4e0e\u5c06\u6765\u7684 macOS \u53ea\u9700\u8981\u5199"`present(&[u8])`\u5c31\u884c\u3002
//!
//! ## \u4f9d\u8d56\u7ea6\u675f\uff08\u786c\u89c4\u5219\uff09
//!
//! - **\u4e0d\u5f97\u4f9d\u8d56 `windows-sys`** \u2014\u2014 \u5426\u5219 X11/macOS \u7aef\u6839\u672c\u7f16\u4e0d\u8fc7\uff1b
//! - \u53ea\u4f9d\u8d56 `y901-ui`\uff08`RenderCtx` \u5951\u7ea6\uff09\u4e0e `ttf-parser`\uff08\u7eaf Rust\uff09\u3002
//!
//! ## \u5c42\u6b21
//!
//! ```text
//! y901-ui      \u2190 RenderCtx / TextMetrics / Theme / Color
//!    \u2191
//! y901-render  \u2190 SoftBackend\uff08\u5149\u6805\u5316\uff09 + FontEngine trait + MyFontEngine
//!    \u2191                    \u2191
//! y901-backend-win32      y901-backend-x11
//! \uff08D3D11 + GDI \u4e0a\u5c4f\uff09    \uff08X11 \u4e0a\u5c4f\uff09
//! ```

#![deny(missing_docs)]
pub mod mip;

pub mod atlas;
pub mod font;
pub mod font_engine;
/// 圆角矩形 SDF（与 D3D 着色器逐字对应）。
pub mod sdf;
pub mod soft;
#[cfg(test)]
mod supersample;

pub use font::{cell_metrics_for, DirtyRect, FontEngine, GlyphInfo};
pub use font_engine::{
    debug_ascii, default_test_font_path, glyph_geometry, load_font, polygon_area, rasterize_glyph,
    rasterize_glyph_phased, resolve_font_path, GlyphBitmap, GlyphGeometry, Line, MyFontEngine,
};
pub use soft::SoftBackend;
