//! **格数语义**的两后端对照测试（只需 soft 侧可实例化；D3D 侧的等价函数由
//! `y901-backend-win32` 的 `span_tests` 覆盖）。
//!
//! 契约（`y901_core::Grid::width_of_char`）：0（组合符/零宽）→ 0 格，窄 → 1，宽 → 2；
//! 控制字符由 `unwrap_or(1)` 兜底为 1。**后端不得做二次加工**（曾有的 `.max(1)` 已删）。

use y901_render::cell_metrics_for;
use y901_render::font::FontEngine;
use y901_render::soft::SoftBackend;
use y901_ui::geometry::Size;
use y901_ui::{TextMetrics, TextStyle};

/// 最小 `FontEngine`：`cell_span` 走真实契约（`y901_core::Grid::width_of_char`），
/// 字形一律返回 `None`（本测试只关心**度量**，不关心出图）。
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

/// 造一个 soft 后端：字号 16 ⇒ cell = `cell_metrics_for(16.0)`。
fn backend() -> SoftBackend {
    let (cw, ch) = cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    SoftBackend::new(200, 100, Box::new(MockEngine), cell, None).expect("soft backend")
}

#[test]
fn combining_mark_adds_no_width() {
    let b = backend();
    let plain = b.measure_text("a", &TextStyle::default()).w;
    let combined = b.measure_text("a\u{301}", &TextStyle::default()).w;
    assert_eq!(
        combined, plain,
        "组合字符 U+0301 必须占 0 格：plain={plain}, combined={combined}"
    );
}

#[test]
fn zero_width_space_has_zero_width() {
    let b = backend();
    let w = b.measure_text("\u{200b}", &TextStyle::default()).w;
    assert_eq!(w, 0.0, "零宽空格 U+200B 宽度必须为 0（实际 {w}）");
}

#[test]
fn wide_char_is_two_cells() {
    let b = backend();
    let (cw, _) = cell_metrics_for(16.0);
    let w = b.measure_text("\u{4e2d}", &TextStyle::default()).w;
    assert!(
        (w - 2.0 * cw).abs() < 0.01,
        "中 应占 2 格 = {}，实际 {w}",
        2.0 * cw
    );
}

#[test]
fn control_char_is_one_cell() {
    let b = backend();
    let (cw, _) = cell_metrics_for(16.0);
    let w = b.measure_text("\u{1}", &TextStyle::default()).w;
    assert!(
        (w - cw).abs() < 0.01,
        "控制字符 U+0001 应由 unwrap_or(1) 兜底为 1 格 = {cw}，实际 {w}"
    );
}
