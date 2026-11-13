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
    assert!(
        w13 < w16,
        "13px 必须比 16px 窄：w13={w13}, w16={w16}"
    );
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
