//! 纹理采样与 ImageFit 的像素级验收（纯 CPU 后端）。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::{ImageFit, ImageStyle, TextureId};
use y901_ui::{Color, Rect, RenderCtx, Size};

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

/// 取像素的 RGB（BGRA 存储）。
fn rgb(s: &SoftBackend, x: u32, y: u32) -> (u8, u8, u8) {
    let p = s.pixel(x, y).expect("像素");
    (((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8)
}

/// 造一张 w×h 的纯色纹理。
fn solid(s: &mut SoftBackend, w: u32, h: u32, c: (u8, u8, u8)) -> TextureId {
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        bytes.extend_from_slice(&[c.0, c.1, c.2, 255]);
    }
    s.upload_texture(w, h, &bytes)
}

#[test]
fn upload_rejects_bad_sizes() {
    let mut s = soft();
    assert_eq!(
        s.upload_texture(0, 4, &[]),
        TextureId::INVALID,
        "宽度 0 应返回无效句柄"
    );
    assert_eq!(
        s.upload_texture(4, 4, &[0; 8]),
        TextureId::INVALID,
        "字节数不足应返回无效句柄"
    );
    let id = solid(&mut s, 4, 4, (10, 20, 30));
    assert!(id.is_valid(), "正常上传应得到有效句柄");
}

#[test]
fn solid_texture_fills_the_rect_with_exact_colour() {
    let mut s = soft();
    s.clear();
    let id = solid(&mut s, 2, 2, (200, 40, 60));
    s.draw_image(
        Rect::new(8.0, 8.0, 32.0, 32.0),
        id,
        ImageStyle::default(),
    );
    // 中心与四角都应接近原色（双线性对纯色是恒等映射）
    for (x, y) in [(10, 10), (39, 10), (10, 39), (39, 39), (24, 24)] {
        let (r, g, b) = rgb(&s, x, y);
        assert!(
            (r as i32 - 200).abs() <= 2 && (g as i32 - 40).abs() <= 2 && (b as i32 - 60).abs() <= 2,
            "({x},{y}) 应为 (200,40,60)，实际 ({r},{g},{b})"
        );
    }
    // 矩形外不该被画
    let (r, _, _) = rgb(&s, 2, 2);
    assert!(r < 200, "矩形外不应有纹理像素，实际 r={r}");
}

#[test]
fn two_by_two_quadrants_sample_close_to_the_source_colours() {
    let mut s = soft();
    s.clear();
    // 红 / 绿 / 蓝 / 白
    let bytes: Vec<u8> = vec![
        255, 0, 0, 255, // (0,0) 红
        0, 255, 0, 255, // (1,0) 绿
        0, 0, 255, 255, // (0,1) 蓝
        255, 255, 255, 255, // (1,1) 白
    ];
    let id = s.upload_texture(2, 2, &bytes);
    s.draw_image(Rect::new(0.0, 0.0, 20.0, 20.0), id, ImageStyle::default());

    // 四象限中心：双线性会把颜色平滑过渡，所以只要求"偏该象限的主色"
    let (r, g, b) = rgb(&s, 5, 5);
    assert!(r > g && r > b, "左上应偏红，实际 ({r},{g},{b})");
    let (r, g, b) = rgb(&s, 15, 5);
    assert!(g > r && g > b, "右上应偏绿，实际 ({r},{g},{b})");
    let (r, g, b) = rgb(&s, 5, 15);
    assert!(b > r && b > g, "左下应偏蓝，实际 ({r},{g},{b})");
    let (r, g, b) = rgb(&s, 15, 15);
    assert!(
        r > 150 && g > 150 && b > 150,
        "右下应偏白，实际 ({r},{g},{b})"
    );
}

#[test]
fn cover_crops_sides_and_contain_letterboxes() {
    let mut s = soft();
    // 100x50 的纯色纹理：用 Cover 铺 50x50 会裁掉左右；Contain 会留上下空白
    let id = solid(&mut s, 100, 50, (10, 200, 120));

    // Cover：目标矩形被完全填满
    s.clear();
    s.draw_image(
        Rect::new(0.0, 0.0, 50.0, 50.0),
        id,
        ImageStyle {
            fit: ImageFit::Cover,
            ..Default::default()
        },
    );
    let (r, g, b) = rgb(&s, 25, 25);
    assert!(
        (r as i32 - 10).abs() <= 3 && (g as i32 - 200).abs() <= 3 && (b as i32 - 120).abs() <= 3,
        "Cover 应填满目标，中心实际 ({r},{g},{b})"
    );
    let (r, _, _) = rgb(&s, 25, 1);
    assert!(r < 60, "Cover 下上下不应留白（y=1 应是纹理色），实际 r={r}");

    // Contain：100x50 → 50x25，垂直居中，上下各留 12~13px 空白
    s.clear();
    s.draw_image(
        Rect::new(0.0, 0.0, 50.0, 50.0),
        id,
        ImageStyle {
            fit: ImageFit::Contain,
            ..Default::default()
        },
    );
    let (r, _, _) = rgb(&s, 25, 2);
    assert!(r < 60, "Contain 下顶部应留白，实际 r={r}");
    let (r, g, _) = rgb(&s, 25, 25);
    assert!(
        (r as i32 - 10).abs() <= 3 && (g as i32 - 200).abs() <= 3,
        "Contain 中部应是纹理色，实际 ({r},{g})"
    );
}

#[test]
fn tint_multiplies_the_sampled_colour() {
    let mut s = soft();
    s.clear();
    // 白色纹理 + 红色 tint = 纯红
    let id = solid(&mut s, 8, 8, (255, 255, 255));
    s.draw_image(
        Rect::new(4.0, 4.0, 20.0, 20.0),
        id,
        ImageStyle {
            tint: Some(Color::rgba(1.0, 0.0, 0.0, 1.0)),
            ..Default::default()
        },
    );
    let (r, g, b) = rgb(&s, 12, 12);
    assert!(
        r > 230 && g < 20 && b < 20,
        "白色 × 红 tint 应得到纯红，实际 ({r},{g},{b})"
    );
}

#[test]
fn invalid_texture_draws_nothing() {
    let mut s = soft();
    s.clear();
    let before = rgb(&s, 10, 10);
    s.draw_image(Rect::new(0.0, 0.0, 20.0, 20.0), TextureId::INVALID, ImageStyle::default());
    assert_eq!(rgb(&s, 10, 10), before, "无效句柄不应改动任何像素");
}
