//! **复杂脚本的"量"必须等于"画"**（feature `shaping`）。
//!
//! `draw_text` 对复杂脚本走 shaped 路径，返回的推进是 shaping 给出的 advance 之和；
//! 而 `measure_text` 过去一直按"字符数 × 格宽"算。两者不一致的直接后果：
//! 标签宽度与文字实际宽度不符（裁字 / 留白），折行按错误宽度断句。
//!
//! 这条用例把两侧钉在一起：`measure_text` 的宽度必须等于 `draw_text` 返回的推进，
//! 并且**必须不同于**按格数算出来的宽度（否则说明 shaping 测量没生效）。

#![cfg(feature = "shaping")]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, RenderCtx, Size, TextMetrics, TextStyle};

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        16.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(400, 120, Box::new(engine), cell).expect("soft")
}

#[test]
fn shaped_measure_equals_shaped_advance() {
    let mut s = backend();
    s.clear();
    let style = TextStyle::fg(Color::WHITE).size(16.0);

    for text in ["שלום", "مرحبا", "hello שלום", "שלום hello"] {
        let measured = s.measure_text(text, &style).w;
        let drawn = s.draw_text(Pos::new(8.0, 8.0), text, &style);

        assert!(
            (measured - drawn).abs() < 0.5,
            "{text}: measure_text={measured} 与 draw_text 推进={drawn} 不一致\
             （布局会按错的宽度裁字/留白）"
        );

        // 反向证据：按"格数 × 格宽"算出来的宽度应当与它不同，否则本用例证明不了
        // "shaping 测量真的生效"（只证明了两个函数都退化成格数）。
        let by_cells = by_cells(text, 16.0);
        assert!(
            (measured - by_cells).abs() > 0.5,
            "{text}: shaping 宽度 {measured} 与格数宽度 {by_cells} 相同 —— \
             说明 measure_text 仍走格路径，本用例失去意义"
        );
    }
}

/// 纯 ASCII / CJK 不受影响：仍按格数算（与像素基线一致）。
#[test]
fn plain_text_measure_is_unchanged() {
    let s = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    for text in ["hello", "abc 123", "中文"] {
        let measured = s.measure_text(text, &style).w;
        let expected = by_cells(text, 16.0);
        assert!(
            (measured - expected).abs() < 0.01,
            "{text}: 纯 ASCII/CJK 仍应按格数度量（{measured} vs {expected}）"
        );
    }
}
/// 按"格数 × 格宽"算宽度（`SoftBackend` 的格路径口径）：格数来自框架唯一来源
/// `y901_core::Grid::width_of_char`，格宽来自 `cell_metrics_for`。
fn by_cells(text: &str, px: f32) -> f32 {
    let cell_w = y901_render::cell_metrics_for(px).0;
    let cells: f32 = text
        .chars()
        .map(|c| y901_core::Grid::width_of_char(c) as f32)
        .sum();
    cells * cell_w
}
