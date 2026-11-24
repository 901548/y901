//! **`TextLabel` 的折行测量必须等于 `TextMetrics::measure_with`**。
//!
//! 这两个数字是同一个语义的两种入口：
//! * `TextLabel::measure`（控件自己报告"我要多高"）；
//! * `TextMetrics::measure_with`（应用/容器在没有控件实例时问"这段文字占多大"）。
//!
//! 它们必须**逐位一致**，否则同一段文字会出现"控件按 3 行留高度、容器按 2 行排版"
//! 的错位。实现上两者都走 `y901_ui::wrap`，这条用例把"同源"钉住。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Constraints, Size, TextMetrics, TextStyle, Widget};
use y901_widgets::TextLabel;

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        16.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(400, 300, Box::new(engine), cell).expect("soft")
}

#[test]
fn label_wrap_measure_equals_measure_with() {
    let metrics = backend();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let cases = [
        "hello",
        "aaa bbb ccc ddd eee fff",
        "paragraph one\nparagraph two\nthree",
        "a\n",
        "abcdefghijklmnopqrstuvwxyz",
    ];
    for avail in [40.0f32, 80.0, 160.0, f32::INFINITY] {
        for text in cases {
            let mut label = TextLabel::new("l", text).wrap(true);
            let c = Constraints {
                min: Size::ZERO,
                max: Size::new(avail, f32::INFINITY),
                ..Default::default()
            };
            let mut cache = y901_ui::layout::LayoutCache::new();
            let by_label = label.measure(&metrics, c, &mut cache);
            let by_api = metrics.measure_with(text, &style, avail);
            assert_eq!(
                by_label, by_api,
                "文本 {text:?}（可用宽度 {avail}）：控件测量 {by_label:?} 与 \
                 measure_with {by_api:?} 不一致 —— 折行规则已经分叉"
            );
        }
    }
}
