//! **文本 shaping 与双向文本**（S2 的协议/算法层，暂不接入控件）。
//!
//! ## 这个模块解决什么
//!
//! 现有文本管线是"**逐字符查 cmap + 等宽 cell 推进**"（见 `font_engine`）。
//! 它足以排版 CJK 与拉丁文，但对**复杂文本**无能为力：
//!
//! - 阿拉伯语/希伯来语：要从右往左排，且字符按位置**连写**成不同字形；
//! - 印度语系：一个"视觉音节"由多个字符重组（`ि` 会跑到辅音左边）；
//! - 连字/kerning：`fi` 可能是一个字形，字距也不等于 advance 之和；
//! - 混合方向：一段文字里同时有 LTR 与 RTL，需要按 Unicode Bidi 算法重排。
//!
//! 这些都**不是**"多查一次 cmap"能解决的，必须有 shaping 引擎。
//! 本模块用 [`rustybuzz`]（HarfBuzz 的纯 Rust 移植，MIT，复用我们已钉的
//! `ttf-parser 0.25`）做字形映射，用 [`unicode_bidi`] 做方向重排。
//!
//! ## 为什么默认不启用
//!
//! `rustybuzz` 在 `shaping` feature 之后。默认构建仍走 cell 路径：
//!
//! - 像素基线、启动时间、二进制体积都不变（既有门禁继续有效）；
//! - 需要复杂文本的应用显式 `features = ["shaping"]`；
//! - 这样"加 shaping"不会一次性改变所有应用的排版结果（避免整站重排 + 基线全红）。
//!
//! ## 与渲染层的边界（下一步）
//!
//! 本模块只产出 [`ShapedRun`]（字形 id + 位置 + advance），**不画像素**。
//! 要真正上屏还需要两件事，属于 `docs/ROADMAP.md` S2 的后续切片：
//!
//! 1. 字形图集的键从 `char` 换成 [`GlyphKey`]（同一字符在不同上下文 =
//!    不同字形，`char` 键会命中错图集槽位）；
//! 2. `Widget::measure` 增加"给定可用宽度"的重载，因为 shaping 之后
//!    行宽不再等于 `字符数 × cell 宽`。

mod bidi;
mod cluster;
pub mod layout;
#[cfg(feature = "shaping")]
mod shape;

pub use bidi::{paragraph_level, reorder_visual};
pub use cluster::{cluster_boundaries, visual_to_logical};

/// 组合记号判定（布局层用它识别必须 shaping 的文字）。
pub use cluster::is_combining_mark as is_combining_mark_char;
pub use layout::{needs_shaping, place_run, LaidGlyph};
#[cfg(feature = "shaping")]
pub use shape::{shape_run, ShaperError};

/// 字形缓存键：**同一字符在不同上下文会映射到不同字形**，所以键必须带 face 与 glyph id。
///
/// 为什么不用 `char`：`fi` 连字、阿拉伯语的词中形、印度语系的重组音节都是
/// "多个字符 → 一个/多个不同字形"的关系，用 `char` 作键会让不同上下文共用同一张位图。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphKey {
    /// 字体面序号（回退链里的第几个字体）。
    pub face: u16,
    /// 该字体内部的字形 id。
    pub glyph: u32,
}

impl GlyphKey {
    /// 构造键。
    pub const fn new(face: u16, glyph: u32) -> Self {
        Self { face, glyph }
    }

    /// **cell 路径的兼容键**：把字符本身当作 glyph id（face 固定 0）。
    ///
    /// 现有引擎按字符取位图，这个构造让老路径可以继续用同一套图集类型，
    /// 而新路径用真实 glyph id —— 两者不会互相覆盖（face=0 与 face>0 不冲突，
    /// 且真正接入时会统一成 glyph id）。
    pub const fn from_char(ch: char) -> Self {
        Self {
            face: 0,
            glyph: ch as u32,
        }
    }
}

/// 一个已完成 shaping 的字形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapedGlyph {
    /// 图集/字体查找键。
    pub key: GlyphKey,
    /// 相对行首的 x 偏移（像素，已含 kerning/连字造成的位移）。
    pub x: f32,
    /// y 偏移（像素；印度语系的 mark 会被抬高/降低）。
    pub y: f32,
    /// 本字形的推进量（像素）。
    pub advance: f32,
    /// 该字形对应的**源字符**区间（字节偏移，用于光标/选区映射）。
    pub cluster: usize,
}

/// 一段同方向、同字体、同字号的字形序列。
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedRun {
    /// 这段文字的**逻辑**字节区间（在原始字符串里的起止）。
    pub range: std::ops::Range<usize>,
    /// 方向：`true` = 从右往左。
    pub rtl: bool,
    /// 字形序列：RTL 时按**视觉顺序**（从左到右）存储，便于直接绘制。
    pub glyphs: Vec<ShapedGlyph>,
    /// 整段宽度（像素）。
    pub width: f32,
}

impl ShapedRun {
    /// 空 run。
    pub fn empty() -> Self {
        Self {
            range: 0..0,
            rtl: false,
            glyphs: Vec::new(),
            width: 0.0,
        }
    }

    /// 按视觉顺序返回字形个数。
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// 是否没有字形。
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_key_distinguishes_face_and_glyph() {
        let a = GlyphKey::new(0, 42);
        let b = GlyphKey::new(1, 42);
        let c = GlyphKey::new(0, 43);
        assert_ne!(a, b, "不同字体的同号字形不能共用一个槽位");
        assert_ne!(a, c);
        assert_eq!(a, GlyphKey::new(0, 42));
    }

    #[test]
    fn char_key_uses_face_zero_so_it_never_collides_with_real_glyph_ids() {
        let key = GlyphKey::from_char('中');
        assert_eq!(key.face, 0);
        assert_eq!(key.glyph, '中' as u32);
        // 真实字形 id 走 face>0 或不同 glyph，不会与字符码点混淆。
        assert_ne!(GlyphKey::from_char('a'), GlyphKey::new(0, 0));
    }

    #[test]
    fn empty_run_is_empty() {
        let run = ShapedRun::empty();
        assert!(run.is_empty());
        assert_eq!(run.len(), 0);
        assert_eq!(run.width, 0.0);
    }
}
