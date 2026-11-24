//! **按宽度量文本**（`TextMetrics::measure_with`）：折行规则必须与渲染端一致。
//!
//! 这张 API 的意义是"先量后排"：控件要知道自己需要多高，就必须用**与渲染同一套**
//! 折行规则量。若两处各写一套，就会出现"量出 2 行、画成 3 行"（文字被裁或留白）。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Size, TextMetrics, TextStyle};

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        16.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(400, 200, Box::new(engine), cell).expect("soft")
}

#[test]
fn text_that_fits_is_measured_as_one_line() {
    let s = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let single = s.measure_text("hello", &style);
    let wrapped = s.measure_with("hello", &style, 1000.0);
    assert_eq!(wrapped.w, single.w, "放得下时宽度应与单行测量一致");
    assert_eq!(wrapped.h, s.line_height(&style), "放得下时高度应为一行");
}

#[test]
fn long_text_wraps_and_height_grows() {
    let s = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let text = "aaa bbb ccc ddd eee fff ggg hhh";
    let avail = single_word_width(&s, &style) * 2.5;
    let size = s.measure_with(text, &style, avail);
    let line_h = s.line_height(&style);
    assert!(
        size.h >= 2.0 * line_h - 0.01,
        "宽度只够两个词时至少折成 2 行，实际 {size:?}"
    );
    assert!(
        size.w <= avail + 0.01,
        "最宽行不应超过可用宽度（{:.2} > {avail:.2}）",
        size.w
    );
    assert_eq!(
        size.h % line_h,
        0.0,
        "高度必须是行高的整数倍（否则最后一行会与下一行重叠）"
    );
}

#[test]
fn explicit_newlines_force_lines_even_when_they_would_fit() {
    let s = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let size = s.measure_with("a\nb\nc", &style, 10_000.0);
    assert_eq!(
        size.h,
        s.line_height(&style) * 3.0,
        "显式换行必须成 3 行，不受可用宽度影响"
    );
}

#[test]
fn a_word_wider_than_the_line_is_broken_like_the_renderer_does() {
    // 规则与 `y901_ui::wrap`（= 渲染端折行）完全一致：没有可断的空白时**按字符切**。
    // 这条用例存在的意义是钉住"量"与"画"同源 —— 若哪天 measure_with 自己另写一套
    // 折行，"单个超宽词"就是最先分叉的地方（一边 1 行、一边 4 行）。
    let s = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let word = "abcdefghijklmnop";
    let w = s.measure_text(word, &style).w;
    let size = s.measure_with(word, &style, w / 2.0);
    let line_h = s.line_height(&style);
    assert!(
        size.h >= 2.0 * line_h - 0.01,
        "超宽词必须按字符折成多行（与渲染端一致），实际 {size:?}"
    );
    assert!(
        size.w <= w / 2.0 + 0.01,
        "折行后每行都不应超过可用宽度（{} > {}）",
        size.w,
        w / 2.0
    );
    assert_eq!(size.h % line_h, 0.0, "高度必须是行高的整数倍");
}

#[test]
fn unbounded_width_falls_back_to_single_line() {
    let s = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let text = "aaa bbb ccc";
    let single = s.measure_text(text, &style);
    for avail in [f32::INFINITY, 0.0, -10.0] {
        let size = s.measure_with(text, &style, avail);
        assert_eq!(size, single, "无界/非法宽度应退回单行测量（avail={avail}）");
    }
}

/// 一个单 word 的宽度（用于构造"只够放下 N 个词"的可用宽度）。
fn single_word_width(s: &SoftBackend, style: &TextStyle) -> f32 {
    s.measure_text("aaa", style).w
}
