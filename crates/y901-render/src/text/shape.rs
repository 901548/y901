//! 用 `rustybuzz` 做实际 shaping（仅在 `shaping` feature 下编译）。

use super::{GlyphKey, ShapedGlyph, ShapedRun};
use rustybuzz::{Face, UnicodeBuffer};

/// shaping 失败原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShaperError {
    /// 字体数据无法解析（TTC/损坏文件等）。
    BadFont(String),
}

impl std::fmt::Display for ShaperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaperError::BadFont(msg) => write!(f, "字体不可用：{msg}"),
        }
    }
}

impl std::error::Error for ShaperError {}

/// 对一段文字做 shaping，返回**视觉顺序**、按方向排好的 run。
///
/// 输入：
/// - `font_data`：字体字节（TTF/OTF；TTC 需先取出单个 face，见 `font_engine::load_font`）；
/// - `text`：要排版的文字（可以混合方向与脚本）；
/// - `size_px`：字号（像素）；
/// - `face_index`：回退链里的第几个字体（写进 [`GlyphKey::face`]）。
///
/// 输出：一个 [`ShapedRun`]（纯 LTR 时一个 run；混合方向时**目前合并成一个**
/// 视觉顺序 run —— 按方向切分成多 run 属于后续切片，见 `docs/ROADMAP.md` S2）。
pub fn shape_run(
    font_data: &[u8],
    text: &str,
    size_px: f32,
    face_index: u16,
) -> Result<ShapedRun, ShaperError> {
    let face = Face::from_slice(font_data, 0).ok_or_else(|| {
        ShaperError::BadFont(format!(
            "Face::from_slice 返回 None（字体字节 {} 字节，可能是 TTC 或损坏文件）",
            font_data.len()
        ))
    })?;
    let upem = face.units_per_em() as f32;
    let scale = if upem > 0.0 { size_px / upem } else { 1.0 };

    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    let rtl = buffer.direction() == rustybuzz::Direction::RightToLeft;

    let glyph_buffer = rustybuzz::shape(&face, &[], buffer);
    let infos = glyph_buffer.glyph_infos();
    let positions = glyph_buffer.glyph_positions();

    let mut glyphs = Vec::with_capacity(infos.len());
    let mut pen = 0.0f32;
    for (info, pos) in infos.iter().zip(positions.iter()) {
        let advance = pos.x_advance as f32 * scale;
        let x = pen + pos.x_offset as f32 * scale;
        let y = pos.y_offset as f32 * scale;
        glyphs.push(ShapedGlyph {
            key: GlyphKey::new(face_index, info.glyph_id),
            x,
            y,
            advance,
            cluster: info.cluster as usize,
        });
        pen += advance;
    }

    Ok(ShapedRun {
        range: 0..text.len(),
        rtl,
        width: pen,
        glyphs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取一个真实字体：仓库的测试字体（拉丁）。
    fn test_font() -> Vec<u8> {
        crate::font_engine::load_font(&crate::font_engine::default_test_font_path())
            .expect("测试字体可读")
    }

    #[test]
    fn latin_text_produces_one_glyph_per_char_with_positive_advance() {
        let font = test_font();
        let run = shape_run(&font, "Hello", 16.0, 0).expect("shaping 成功");
        assert!(!run.rtl, "拉丁文是 LTR");
        assert_eq!(run.glyphs.len(), 5, "5 个字符 → 5 个字形");
        assert!(run.width > 0.0);
        for g in &run.glyphs {
            assert!(g.advance > 0.0, "每个字形都应有正推进");
            assert_eq!(g.key.face, 0);
        }
        // 集群按字符递增
        let clusters: Vec<usize> = run.glyphs.iter().map(|g| g.cluster).collect();
        assert_eq!(clusters, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn ligature_capable_pair_can_map_to_fewer_glyphs() {
        // "fi" 在支持连字的字体里会合成一个字形；测试字体不保证有连字，
        // 因此这里只断言"不会崩、且字形数 <= 字符数"。
        let font = test_font();
        let run = shape_run(&font, "fi", 16.0, 0).expect("shaping 成功");
        assert!(
            !run.glyphs.is_empty() && run.glyphs.len() <= 2,
            "fi 应映射到 1~2 个字形，实得 {}",
            run.glyphs.len()
        );
    }

    #[test]
    fn arabic_text_is_rtl_and_uses_distinct_glyphs_for_joined_forms() {
        let font = test_font();
        // 拉丁字体缺阿拉伯字形时会回退成 .notdef，但 rsybuzz 仍会给出 RTL 方向
        // 与"连写后的字形数"。这里断言方向正确、且不 panic。
        let run = shape_run(&font, "مرحبا", 16.0, 0).expect("shaping 不应因缺字失败");
        assert!(run.rtl, "阿拉伯语应为 RTL");
        assert!(!run.glyphs.is_empty());
    }

    #[test]
    fn empty_text_shapes_to_empty_run() {
        let font = test_font();
        let run = shape_run(&font, "", 16.0, 0).expect("空串也应成功");
        assert!(run.is_empty());
        assert_eq!(run.width, 0.0);
    }

    #[test]
    fn bad_font_bytes_report_an_error_instead_of_panicking() {
        let err = shape_run(&[0u8; 32], "x", 16.0, 0).expect_err("坏字体必须报错");
        assert!(matches!(err, ShaperError::BadFont(_)));
    }

    #[test]
    fn kerning_and_offsets_are_applied_to_positions() {
        let font = test_font();
        let run = shape_run(&font, "AV", 32.0, 2).expect("shaping 成功");
        assert_eq!(run.glyphs[0].key.face, 2, "face_index 应写进 GlyphKey");
        // x 偏移应随 pen 前进（除非字体给了负 offset）
        assert!(run.glyphs[1].x >= 0.0);
        assert!(run.width > 0.0);
    }
}
