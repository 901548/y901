//! SoftBackend 的字形渲染回归测试。
//!
//! 起因：一次误判——我以为 SoftBackend 不画字，实际是**默认颜色透明**加上**测试字体没有中文字形**。
//! 这两条现在都用测试固定下来，避免下一个人重走这段弯路。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, RenderCtx, Size, TextStyle};

fn engine() -> (MyFontEngine, Size) {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let e = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    (e, cell)
}

fn ink(s: &SoftBackend) -> usize {
    let bg = s.pixel(0, 59).expect("bg");
    let mut n = 0;
    for y in 0..60 {
        for x in 0..200 {
            if s.pixel(x, y).expect("px") != bg {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn latin_glyph_really_renders() {
    let (e, cell) = engine();
    let mut s = SoftBackend::new_headless(200, 60, Box::new(e), cell).expect("soft");
    s.clear();
    s.draw_text(Pos::new(10.0, 10.0), "A", &TextStyle::fg(Color::WHITE));
    let n = ink(&s);
    eprintln!("[glyph] 'A' 墨迹像素 = {n}");
    assert!(n > 20, "拉丁字母应有明显墨迹，实测 {n}");
}

#[test]
fn default_text_style_is_visible() {
    // 默认样式若前景透明，文字就"画了但看不见"——这是真实发生过的坑。
    let (e, cell) = engine();
    let mut s = SoftBackend::new_headless(200, 60, Box::new(e), cell).expect("soft");
    s.clear();
    s.draw_text(Pos::new(10.0, 10.0), "A", &TextStyle::default());
    let n = ink(&s);
    eprintln!("[glyph] 默认样式 'A' 墨迹像素 = {n}");
    assert!(n > 20, "TextStyle::default() 应可见，实测 {n}");
}

#[test]
fn latin_test_font_has_no_cjk_glyphs() {
    // 测试字体是 segoeui.ttf（拉丁）：中文字形取不到。
    // 这是**字体限制**，不是渲染器缺陷：中文界面应选中文字体（如 msyh/simsun）。
    let (mut e, _cell) = engine();
    assert!(
        e.glyph('父').is_none(),
        "如果这里变成 Some，说明测试字体已换成支持中文的，请更新本测试"
    );
    assert!(e.glyph('A').is_some(), "拉丁字母应存在");
}
