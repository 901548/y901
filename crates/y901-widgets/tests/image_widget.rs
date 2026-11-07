//! Image 控件：PNG 解码、惰性上传、像素回读。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::{ImageFit, ImageStyle};
use y901_ui::layout::LayoutCache;
use y901_ui::{Color, Rect, Size, Widget};
use y901_widgets::image::Image;

const W: u32 = 64;
const H: u32 = 64;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

fn rgb(s: &SoftBackend, x: u32, y: u32) -> (u8, u8, u8) {
    let p = s.pixel(x, y).expect("像素");
    (
        ((p >> 16) & 0xFF) as u8,
        ((p >> 8) & 0xFF) as u8,
        (p & 0xFF) as u8,
    )
}

/// 用 png crate 现场编码一张 `w*h` 的纯色 PNG（避免把二进制测试资源塞进仓库）。
fn encode_png(w: u32, h: u32, c: (u8, u8, u8)) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().expect("png header");
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            data.extend_from_slice(&[c.0, c.1, c.2, 255]);
        }
        writer.write_image_data(&data).expect("png data");
    }
    out
}

fn arrange(img: &mut Image, rect: Rect) {
    let soft = soft();
    let mut cache = LayoutCache::new();
    img.arrange(rect, &soft, &mut cache);
}

#[test]
fn png_roundtrip_keeps_colour_and_size() {
    let png = encode_png(4, 3, (12, 34, 56));
    let img = Image::from_png("pic", &png).expect("应能解码");
    assert_eq!(img.image_size(), (4, 3), "尺寸应来自 PNG 头");
}

#[test]
fn decoded_png_renders_with_the_original_colour() {
    let png = encode_png(2, 2, (200, 60, 20));
    let mut img = Image::from_png("pic", &png).expect("应能解码");
    arrange(&mut img, Rect::new(8.0, 8.0, 32.0, 32.0));

    let mut soft = soft();
    soft.clear();
    img.render(&mut soft);

    let (r, g, b) = rgb(&soft, 24, 24);
    assert!(
        (r as i32 - 200).abs() <= 3 && (g as i32 - 60).abs() <= 3 && (b as i32 - 20).abs() <= 3,
        "中心应为 (200,60,20)，实际 ({r},{g},{b})"
    );
}

#[test]
fn tint_produces_the_tinted_colour() {
    let png = encode_png(2, 2, (255, 255, 255));
    let mut img = Image::from_png("icon", &png)
        .expect("解码")
        .tint(Color::rgba(0.0, 0.0, 1.0, 1.0));
    arrange(&mut img, Rect::new(4.0, 4.0, 24.0, 24.0));

    let mut soft = soft();
    soft.clear();
    img.render(&mut soft);
    let (r, g, b) = rgb(&soft, 16, 16);
    assert!(
        b > 230 && r < 20 && g < 20,
        "白色 × 蓝 tint 应得纯蓝，实际 ({r},{g},{b})"
    );
}

#[test]
fn contain_letterboxes_a_wide_image() {
    // 8x4 的图放进 32x32 的控件：Contain 下应垂直居中，上下留白
    let png = encode_png(8, 4, (0, 200, 0));
    let mut img = Image::from_png("wide", &png)
        .expect("解码")
        .with_style(ImageStyle {
            fit: ImageFit::Contain,
            ..Default::default()
        });
    arrange(&mut img, Rect::new(0.0, 0.0, 32.0, 32.0));

    let mut soft = soft();
    soft.clear();
    img.render(&mut soft);

    let (r, _, _) = rgb(&soft, 16, 2);
    assert!(r < 60, "Contain 下顶部应留白，实际 r={r}");
    let (_, g, _) = rgb(&soft, 16, 16);
    assert!(g > 150, "中部应是图片色，实际 g={g}");
}

#[test]
fn bad_png_bytes_return_an_error_not_a_panic() {
    let err = Image::from_png("x", b"not a png");
    assert!(err.is_err(), "非法数据应返回 Err");
}

#[test]
fn short_rgba_buffer_is_padded_instead_of_panicking() {
    // 给 2x2 的尺寸但只给 4 字节（应该是 16）：应补零，不 panic
    let img = Image::from_rgba("x", 2, 2, &[1, 2, 3, 4]);
    assert_eq!(img.image_size(), (2, 2));
}
