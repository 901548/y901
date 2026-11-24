//! **渐变填充的像素金标**（垂直 / 水平 / 半透明 / 斜向）。
//!
//! 背景：`fill_gradient_rect_dir` 原先逐像素算 lerp + 三次 clamp —— 1280×800 整屏
//! 要 4.2 ms，是同尺寸 `fill_rect` 的 45 倍。主题背景几乎都是**轴对齐**渐变
//! （垂直占绝大多数），于是加了"垂直每行一色 / 水平每列一色"的快路。
//!
//! 快路最大的风险是**改变像素**。这四个哈希是在**旧实现**上录的（stash A/B 取的），
//! 现在钉成金标：任何一处快路算错（颜色、透明度、裁剪边界、行/列顺序）都会立刻红。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Rect, RenderCtx, Size};

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(1280, 800, Box::new(engine), cell).expect("soft")
}

#[test]
fn vertical_gradient_pixels_match_reference() {
    let mut b = soft();
    b.clear();
    b.fill_gradient_rect_dir(
        Rect::new(0.0, 0.0, 1280.0, 800.0),
        Color::rgba(0.05, 0.05, 0.07, 1.0),
        Color::rgba(0.2, 0.3, 0.5, 1.0),
        (0.0, 1.0),
    );
    assert_eq!(
        b.frame_hash(),
        0xCCC3_481D_455D_DB25,
        "垂直渐变（整屏）像素变了：快路算错或裁剪边界变了"
    );
}

#[test]
fn horizontal_gradient_pixels_match_reference() {
    let mut b = soft();
    b.clear();
    b.fill_gradient_rect_dir(
        Rect::new(10.0, 10.0, 300.0, 200.0),
        Color::rgba(1.0, 0.0, 0.0, 1.0),
        Color::rgba(0.0, 0.0, 1.0, 1.0),
        (1.0, 0.0),
    );
    assert_eq!(
        b.frame_hash(),
        0xB897_BCC9_5DA4_8B25,
        "水平渐变像素变了：列缓存（`grad_lut`）或混合路径算错"
    );
}

#[test]
fn translucent_vertical_gradient_pixels_match_reference() {
    let mut b = soft();
    b.clear();
    b.fill_gradient_rect_dir(
        Rect::new(10.0, 10.0, 300.0, 200.0),
        Color::rgba(0.2, 0.6, 0.9, 0.5),
        Color::rgba(0.9, 0.1, 0.3, 0.8),
        (0.0, 1.0),
    );
    assert_eq!(
        b.frame_hash(),
        0xCDCD_BB07_6106_2045,
        "半透明垂直渐变像素变了：快路里的 blend 分支算错"
    );
}

#[test]
fn diagonal_gradient_pixels_match_reference() {
    let mut b = soft();
    b.clear();
    b.fill_gradient_rect_dir(
        Rect::new(10.0, 10.0, 300.0, 200.0),
        Color::rgba(0.2, 0.6, 0.9, 1.0),
        Color::rgba(0.9, 0.1, 0.3, 1.0),
        (0.6, 0.8),
    );
    assert_eq!(
        b.frame_hash(),
        0x0759_6B32_C901_AC72,
        "斜向渐变像素变了：通用路径被改坏"
    );
}
