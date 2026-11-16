//! `fill_rect` 的**不透明快路径**必须与逐像素混合逐位等价。
//!
//! 优化点：`(px >> 24) == 255` 时用行切片 `fill(px)` 代替 `blend()` 循环。
//! 三条断言钉死语义：
//! 1. 不透明色**完全覆盖**旧内容（与 `blend` 的 `sa == 255 → src` 一致）；
//! 2. 半透明色**仍走混合**（不能被快路径吞掉）；
//! 3. 越界矩形被裁剪，既不 panic 也不写到缓冲外。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Rect, RenderCtx, Size};

const W: u32 = 32;
const H: u32 = 32;

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), Size::new(cw, ch)).expect("soft")
}

fn rgb(s: &SoftBackend, x: u32, y: u32) -> (u32, u32, u32) {
    let p = s.pixel(x, y).expect("px");
    ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff)
}

/// 不透明填充直接覆盖：先铺白，再填不透明红，像素必须**精确**等于红。
#[test]
fn opaque_fill_overwrites_exactly() {
    let mut s = backend();
    s.clear();
    s.fill_rect(
        Rect::new(0.0, 0.0, W as f32, H as f32),
        Color::rgba(1.0, 1.0, 1.0, 1.0),
    );
    s.fill_rect(
        Rect::new(8.0, 8.0, 16.0, 16.0),
        Color::rgba(1.0, 0.0, 0.0, 1.0),
    );

    // 区域内：纯红（不透明的 blend 结果是 src，逐位相同）
    for (x, y) in [(8, 8), (12, 16), (23, 23)] {
        assert_eq!(rgb(&s, x, y), (255, 0, 0), "inside at ({x},{y})");
    }
    // 区域外：仍是白
    for (x, y) in [(7, 7), (24, 24), (0, 0)] {
        assert_eq!(rgb(&s, x, y), (255, 255, 255), "outside at ({x},{y})");
    }
}

/// 半透明填充不能被快路径接走：白底 + 50% 红 = 红通道 255、绿蓝 128 左右。
#[test]
fn translucent_fill_still_blends() {
    let mut s = backend();
    s.clear();
    s.fill_rect(
        Rect::new(0.0, 0.0, W as f32, H as f32),
        Color::rgba(1.0, 1.0, 1.0, 1.0),
    );
    s.fill_rect(
        Rect::new(0.0, 0.0, 16.0, 16.0),
        Color::rgba(1.0, 0.0, 0.0, 0.5),
    );

    let (r, g, b) = rgb(&s, 8, 8);
    assert_eq!(r, 255, "red stays saturated");
    assert!((g as i32 - 128).abs() <= 1, "green = {g}");
    assert!((b as i32 - 128).abs() <= 1, "blue = {b}");
}

/// 越界矩形：裁剪后只画窗口内那部分，且不越界写。
#[test]
fn out_of_bounds_fill_is_clipped() {
    let mut s = backend();
    s.clear();
    s.fill_rect(
        Rect::new(-8.0, -8.0, W as f32 + 8.0, H as f32 + 8.0),
        Color::rgba(0.0, 0.0, 1.0, 1.0),
    );

    assert_eq!(rgb(&s, 0, 0), (0, 0, 255));
    assert_eq!(rgb(&s, W - 1, H - 1), (0, 0, 255));
}

/// 完全在窗口外的矩形：什么都不做，缓冲保持清屏色。
#[test]
fn fully_outside_fill_leaves_buffer_untouched() {
    let mut s = backend();
    s.clear();
    let before = s.pixel(0, 0).expect("px");
    s.fill_rect(
        Rect::new(W as f32 + 8.0, H as f32 + 8.0, 8.0, 8.0),
        Color::rgba(1.0, 0.0, 0.0, 1.0),
    );
    assert_eq!(s.pixel(0, 0).expect("px"), before);
}
