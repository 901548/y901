//! **per-text 字号在度量层生效**的对照测试（无需 GPU）。
//!
//! 背景：表头（13）与数据行（16）的字号差异，**不能**用"像素带高"比较 ——
//! 中文表头与 ASCII 文件名的字形高度本就不同。正确做法是比较**字宽**：
//! 同一串字符串在 `size(13)` 与 `size(16)` 下的 `measure_text` 宽度之比应 ≈ 13/16。

use y901_render::cell_metrics_for;
use y901_render::font::FontEngine;
use y901_render::soft::SoftBackend;
use y901_ui::geometry::Size;
use y901_ui::{TextMetrics, TextStyle};

/// 最小引擎：`cell_span` 走真实契约，字形不出图。
struct MockEngine;

impl FontEngine for MockEngine {
    fn glyph(&mut self, _ch: char) -> Option<&y901_render::font::GlyphInfo> {
        None
    }
    fn cell_span(&self, ch: char) -> u32 {
        y901_core::Grid::width_of_char(ch) as u32
    }
    fn atlas(&self) -> &[u8] {
        &[]
    }
    fn atlas_size(&self) -> u32 {
        0
    }
    fn take_dirty(&mut self) -> (bool, Vec<y901_render::font::DirtyRect>) {
        (false, Vec::new())
    }
    fn begin_frame(&mut self) {}
    fn ascent(&self) -> f32 {
        12.0
    }
    fn cell_width(&self) -> f32 {
        cell_metrics_for(16.0).0
    }
    fn cell_height(&self) -> f32 {
        cell_metrics_for(16.0).1
    }
}

fn backend() -> SoftBackend {
    let (cw, ch) = cell_metrics_for(16.0);
    SoftBackend::new(200, 100, Box::new(MockEngine), Size::new(cw, ch), None).expect("soft")
}

#[test]
fn size_13_is_narrower_than_size_16() {
    let b = backend();
    let text = "名称";
    let w13 = b.measure_text(text, &TextStyle::default().size(13.0)).w;
    let w16 = b.measure_text(text, &TextStyle::default().size(16.0)).w;
    assert!(w13 < w16, "13px 必须比 16px 窄：w13={w13}, w16={w16}");
    // cell 是整数字形格宽，四舍五入会带来偏差，所以用 10% 容差而不是精确比值。
    let ratio = w13 / w16;
    let expected = 13.0 / 16.0;
    assert!(
        (ratio - expected).abs() < 0.10,
        "宽度比应接近 13/16={expected:.3}，实测 {ratio:.3}"
    );
}

#[test]
fn default_size_equals_size_16_ignoring_dpi() {
    // 场景 DPI 固定 1.0（Y901_TEST_DPI_SCALE 未设时本机为 1.25，故这里只比较两者关系）。
    let b = backend();
    let text = "M3";
    let w_default = b.measure_text(text, &TextStyle::default()).w;
    let w16 = b.measure_text(text, &TextStyle::default().size(16.0)).w;
    let scale = b.scale();
    // None 路径按引擎 cell（16*scale）；Some(16) 路径同样是 16*scale → 两者必须相等。
    assert!(
        (w_default - w16).abs() < 0.01,
        "默认字号应与 size(16) 等宽（scale={scale}）：default={w_default}, size16={w16}"
    );
}

// ---- 下面两条是**像素级**的：字号必须真的改变字形，而不只是改变推进宽度 ----

mod pixels {
    use super::*;
    use y901_render::MyFontEngine;
    use y901_ui::{Color, Pos, RenderCtx};

    fn real_backend() -> SoftBackend {
        let (cw, ch) = cell_metrics_for(16.0);
        let engine = MyFontEngine::new(
            y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
            16.0,
            Size::new(cw, ch),
        )
        .expect("engine");
        SoftBackend::new_headless(240, 120, Box::new(engine), Size::new(cw, ch)).expect("soft")
    }

    /// 画一段文字，返回墨迹包围盒 `(宽, 高)`。
    fn ink_size(b: &mut SoftBackend, text: &str, size: f32) -> (u32, u32) {
        b.clear();
        let style = TextStyle::fg(Color::WHITE).size(size);
        RenderCtx::draw_text(b, Pos::new(8.0, 8.0), text, &style);
        let bg = b.pixel(239, 119).expect("bg");
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for y in 0..120 {
            for x in 0..240 {
                if b.pixel(x, y).expect("px") != bg {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        assert!(x0 != u32::MAX, "{text:?}@{size} 应有墨迹");
        (x1 - x0 + 1, y1 - y0 + 1)
    }

    /// **回归**：`FontEngine::glyph_at_size` 的默认实现会忽略字号、返回基准字号位图。
    /// 一旦 `MyFontEngine` 忘了转发这个 trait 方法，这里立刻失败：
    /// 12px 的标签会画 16px 字形（挤在一起），24px 的标题也画 16px 字形（大小不一）。
    #[test]
    fn bigger_size_draws_a_taller_glyph() {
        let mut b = real_backend();
        let (_, h12) = ink_size(&mut b, "HW", 12.0);
        let (_, h16) = ink_size(&mut b, "HW", 16.0);
        let (_, h24) = ink_size(&mut b, "HW", 24.0);
        eprintln!("[per-size] 墨迹高：12px={h12}, 16px={h16}, 24px={h24}");
        assert!(
            h12 < h16 && h16 < h24,
            "字形高度必须随字号增长：12={h12}, 16={h16}, 24={h24}（trait 未转发？）"
        );
        // 16 → 24 是 1.5 倍，允许 ±25%（整数字形格 + 抗锯边）
        let ratio = h24 as f32 / h16 as f32;
        assert!(
            (1.1..=1.7).contains(&ratio),
            "24/16 的墨迹高度比应在 1.5 附近，实测 {ratio:.2}"
        );
    }

    /// 12px 的文字**不能**画成 16px 宽，否则行内文字互相重叠（"挤在一起"）。
    #[test]
    fn smaller_size_draws_a_narrower_glyph() {
        let mut b = real_backend();
        let (w12, _) = ink_size(&mut b, "HW", 12.0);
        let (w16, _) = ink_size(&mut b, "HW", 16.0);
        eprintln!("[per-size] 墨迹宽：12px={w12}, 16px={w16}");
        assert!(
            w12 < w16,
            "12px 的墨迹宽度应小于 16px：12={w12}, 16={w16}"
        );
    }
}

