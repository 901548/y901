//! **墨迹深度与行高是不同度量**：真实字体引擎的 `ascent + descent` 可能大于 `line_height`。
//!
//! 这些用例只检查 `TextMetrics::ink_bounds()` 的字体度量，不断言 `TextLabel` 的高度策略。
//! `TextLabel` 仍按 `line_height` 计算高度；脏区墨迹余量由 `UiTree` 单独处理。
//!
//! 用真实字体引擎（无 GPU）钉住三件事：
//! 1. 本测试字体的 `ink_bounds` 下深度大于 `line_height`；
//! 2. `ink_bounds` 深度随字号线性缩放；
//! 3. 报告应用字体度量，后者不要求墨迹深度一定大于行高。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Size, TextMetrics, TextStyle};

const PX: f32 = 16.0;
const W: u32 = 200;
const H: u32 = 80;

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(PX);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        PX,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

#[test]
fn ink_depth_exceeds_line_height() {
    let b = backend();
    let style = TextStyle::default();
    let (above, below) = b.ink_bounds(&style);
    let line_h = b.line_height(&style);
    assert!(above >= 0.0, "原点以上不应为负：{above}");
    assert!(
        below > line_h,
        "墨迹深度 {below:.2} 应大于行高 {line_h:.2}（否则本用例失去意义；\
         说明字体度量改动过，需要重新核对 P1-5 的结论）"
    );
    eprintln!(
        "[ink] line_h={line_h:.2} ink_depth={below:.2} overflow={:.2}",
        below - line_h
    );
}

#[test]
fn report_app_font_metrics() {
    // 打印应用实际字体的度量，便于判断脏区外扩量该取多少。
    let path = y901_render::resolve_font_path("NSimSun");
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    if let Ok(engine) = MyFontEngine::new(y901_render::load_font(&path).expect("font"), 16.0, cell)
    {
        let b = SoftBackend::new_headless(64, 64, Box::new(engine), cell).expect("soft");
        let style = TextStyle::default();
        eprintln!(
            "[ink] app font({path}): line_h={:.2} ink={:.2} gap={:.2}",
            b.line_height(&style),
            b.ink_bounds(&style).1,
            b.ink_bounds(&style).1 - b.line_height(&style)
        );
        // 应用字体的名义度量**可能刚好等于行高**（SimSun 实测 gap=0），
        // 所以真正的外扩不能只靠它 —— `UiTree` 另外兜了一个最小安全边距。
        assert!(b.line_height(&style) > 0.0);
    }
}

#[test]
fn ink_depth_scales_with_font_size() {
    let b = backend();
    let small = b.ink_bounds(&TextStyle::default().size(8.0)).1;
    let big = b.ink_bounds(&TextStyle::default().size(32.0)).1;
    assert!(
        (big / small - 4.0).abs() < 0.2,
        "墨迹深度应随字号线性缩放：8px → {small:.2}，32px → {big:.2}"
    );
}
