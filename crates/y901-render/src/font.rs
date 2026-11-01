//! 字形引擎的**平台无关**部分：trait 与数据结构。
//!
//! 具体实现分居各处：
//! - `MyFontEngine`（自研 TTF）在本 crate 的 [`crate::font_engine`]；
//! - `TextEngine`（DirectWrite）在 `y901-backend-win32`（它依赖 windows-sys）。

pub struct DirtyRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct GlyphInfo {
    pub width: u32,
    pub height: u32,
    /// 相对基线原点（pen position）的墨迹偏移。
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance: f32,
    /// 图集内像素坐标。
    pub atlas_x: u32,
    pub atlas_y: u32,
    /// 最近一次使用帧（LRU）。
    pub last_used: u64,
}

impl GlyphInfo {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// 字形引擎的统一接口。
///
/// 为什么要这个 trait：两个后端（D3D11 / 纯 CPU）都只需这 9 个能力。
/// 把它们抽出来之后，`TextEngine`（DirectWrite）与 [`MyFontEngine`]（自研）
/// 可以**互相替换**，而 `y901-widgets` / `y901-ui` 一行不改。
///
/// 【硬约束】图集像素格式与脏区协议必须一致：
/// RGBA、`rgb` = 覆盖度、`a` = 255；`take_dirty` 的 `reset` 是消耗式。
pub trait FontEngine {
    /// 取字形（可能会现场光栅化并写图集）。
    fn glyph(&mut self, ch: char) -> Option<&GlyphInfo>;
    /// 字符占用的格数。
    fn cell_span(&self, ch: char) -> u32;
    /// 图集像素（RGBA）。
    fn atlas(&self) -> &[u8];
    /// 图集边长。
    fn atlas_size(&self) -> u32;
    /// 取走脏区（消耗式协议）。
    fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>);
    /// 开始一帧。
    fn begin_frame(&mut self);
    /// 基线高度。
    fn ascent(&self) -> f32;
    /// 格宽度。
    fn cell_width(&self) -> f32;
    /// 格高度。
    fn cell_height(&self) -> f32;
}


pub fn cell_metrics_for(font_size: f32) -> (f32, f32) {
    let size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        1.0
    };
    ((size * 0.5).round().max(1.0), size.round().max(1.0))
}
