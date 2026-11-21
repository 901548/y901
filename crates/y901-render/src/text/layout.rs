//! 复杂文本的**行内排版**：把一段文字变成"要画哪些字形、画在哪、推进多少"。
//!
//! ## 什么时候走这里
//!
//! 只在这段文字**真的需要 shaping**时：[`needs_shaping`] 判定含阿拉伯/希伯来/印度语系
//! 或组合记号。纯 ASCII、纯 CJK 继续走原来的逐字符 cell 路径 ——
//! 这样"加 shaping"不会一次性改动所有应用的排版结果（避免整站重排和基线全红）。
//!
//! ## 与 cell 契约的关系
//!
//! shaping 之后行宽不再等于"字符数 × 格宽"，但**行高与 baseline 仍由 `cell_metrics_for`
//! 决定**：格契约管的是行盒子，不是字形位置。
//!
//! ## 这里不做折行
//!
//! 折行是布局层的事（需要可用宽度）。本模块只排一行，调用方自己决定在哪断。

use unicode_script::{Script, UnicodeScript};

use super::{ShapedGlyph, ShapedRun};

/// 一个已定位、可直接绘制的字形。
#[derive(Clone, Copy, Debug)]
pub struct LaidGlyph {
    /// 字体面序号（回退链用，当前恒 0）。
    pub face: u16,
    /// 字形 id。
    pub glyph: u32,
    /// 相对行首原点的 x 偏移（像素，已含 shaping 的 kerning/连字位移）。
    pub x: f32,
    /// 相对基线的 y 偏移（像素，正 = 向下）。
    pub y: f32,
    /// 该字形对应的**源字符**字节偏移（光标/选区映射用）。
    pub cluster: usize,
}

/// 这一行是否需要 shaping。
///
/// 判定规则：出现**复杂脚本**（阿拉伯、希伯来、印度语系、泰文等）或**组合记号**即为真。
/// 拉丁/CJK 返回 `false`，继续走 cell 路径（同等宽契约一致，也不会扰动既有像素基线）。
pub fn needs_shaping(text: &str) -> bool {
    text.chars().any(|ch| {
        if crate::text::is_combining_mark_char(ch) {
            return true;
        }
        matches!(
            ch.script(),
            Script::Arabic
                | Script::Hebrew
                | Script::Syriac
                | Script::Thaana
                | Script::Nko
                | Script::Devanagari
                | Script::Bengali
                | Script::Gurmukhi
                | Script::Gujarati
                | Script::Oriya
                | Script::Tamil
                | Script::Telugu
                | Script::Kannada
                | Script::Malayalam
                | Script::Sinhala
                | Script::Thai
                | Script::Lao
                | Script::Tibetan
                | Script::Myanmar
                | Script::Khmer
        )
    })
}

/// 把一段 `ShapedRun` 铺到行上，返回带绝对偏移的字形列表与总宽度。
///
/// `rtl_base` = 段落是 RTL 时的**行右边界** x（RTL 从右往左排）；
/// LTR 传 `0.0` 即可（偏移相对行首）。
///
/// 为什么把布局（而非绘制）单独抽出来：它不碰像素，能在没有图集/没有窗口的情况下单测。
pub fn place_run(run: &ShapedRun, rtl_base: f32) -> (Vec<LaidGlyph>, f32) {
    let mut out = Vec::with_capacity(run.glyphs.len());
    let width = run.width;
    // RTL：整行从右边界往左排，因此每个字形的位置要按"行宽 - 自身偏移"翻过来。
    let flip = if run.rtl { rtl_base - width } else { rtl_base };
    for ShapedGlyph {
        key,
        x,
        y,
        advance: _,
        cluster,
    } in &run.glyphs
    {
        out.push(LaidGlyph {
            face: key.face,
            glyph: key.glyph,
            x: flip + x,
            y: *y,
            cluster: *cluster,
        });
    }
    (out, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{GlyphKey, ShapedGlyph};

    fn run(glyphs: Vec<(u32, f32, f32, usize)>, rtl: bool, width: f32) -> ShapedRun {
        ShapedRun {
            range: 0..4,
            rtl,
            glyphs: glyphs
                .into_iter()
                .map(|(g, x, y, cluster)| ShapedGlyph {
                    key: GlyphKey::new(0, g),
                    x,
                    y,
                    advance: 8.0,
                    cluster,
                })
                .collect(),
            width,
        }
    }

    #[test]
    fn ascii_does_not_need_shaping() {
        assert!(!needs_shaping("hello world"));
        assert!(!needs_shaping("中文字符")); // CJK 走 cell 契约
        assert!(!needs_shaping(""));
    }

    #[test]
    fn complex_scripts_need_shaping() {
        assert!(needs_shaping("مرحبا"), "阿拉伯语");
        assert!(needs_shaping("שלום"), "希伯来语");
        assert!(needs_shaping("कि"), "天城文");
        assert!(
            needs_shaping("abc مرحبا"),
            "混排里只要有一段复杂脚本就整体走 shaping"
        );
    }

    #[test]
    fn combining_mark_alone_needs_shaping() {
        assert!(
            needs_shaping("e\u{0301}"),
            "组合重音必须 shaping（否则会占一格错位）"
        );
    }

    #[test]
    fn ltr_placement_keeps_shaped_offsets() {
        let r = run(vec![(1, 0.0, 0.0, 0), (2, 8.0, 0.0, 1)], false, 16.0);
        let (laid, width) = place_run(&r, 0.0);
        assert_eq!(width, 16.0);
        assert_eq!(laid.len(), 2);
        assert_eq!((laid[0].x, laid[1].x), (0.0, 8.0));
        assert_eq!(laid[0].glyph, 1);
    }

    #[test]
    fn rtl_placement_starts_from_the_right_edge() {
        // 行右边界 100、行宽 16：整行应落在 x ∈ [84, 100)。
        let r = run(vec![(1, 0.0, 0.0, 0), (2, 8.0, 0.0, 1)], true, 16.0);
        let (laid, _) = place_run(&r, 100.0);
        assert_eq!(laid[0].x, 84.0);
        assert_eq!(laid[1].x, 92.0);
    }

    #[test]
    fn y_offset_and_clusters_are_preserved() {
        let r = run(vec![(7, 0.0, -2.0, 3)], false, 8.0);
        let (laid, _) = place_run(&r, 0.0);
        assert_eq!(laid[0].y, -2.0);
        assert_eq!(laid[0].cluster, 3);
    }

    #[test]
    fn empty_run_lays_out_nothing() {
        let (laid, width) = place_run(&ShapedRun::empty(), 50.0);
        assert!(laid.is_empty());
        assert_eq!(width, 0.0);
    }
}
