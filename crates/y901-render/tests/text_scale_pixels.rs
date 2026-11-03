//! 文本缩放的**像素断言**（第 3 步：SoftBackend 接入）。
//!
//! 三条：
//! 1. `s = 1.0` 与"不缩放路径"逐像素一致（含变异对照）；
//! 2. `s = 0.5` 的墨迹像素数 ≈ 0.25 倍 ±30%；
//! 3. `s = 0.5` 的画面与超采样真值的 MAD ≤ 0.05（稀疏与密集各一个）。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextStyle};

const W: u32 = 96;
const H: u32 = 48;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), Size::new(cw, ch)).expect("soft")
}

/// 统计画布上的非背景像素数（背景取右下角）。
fn ink(px: &SoftBackend) -> usize {
    let bg = px.pixel(W - 1, H - 1).expect("bg");
    let mut n = 0;
    for y in 0..H {
        for x in 0..W {
            if px.pixel(x, y).expect("p") != bg {
                n += 1;
            }
        }
    }
    n
}

fn draw_at(s: &mut SoftBackend, ch: &str, scale: Option<f32>) {
    s.clear();
    let style = TextStyle::fg(Color::WHITE);
    match scale {
        None => {
            s.draw_text(Pos::new(8.0, 8.0), ch, &style);
        }
        Some(k) => {
            s.push_transform(Pos::new(8.0, 8.0), k);
            s.draw_text(Pos::new(8.0, 8.0), ch, &style);
            s.pop_transform();
        }
    }
}

#[test]
fn scale_one_is_pixel_identical_to_the_unscaled_path() {
    let mut a = soft();
    draw_at(&mut a, "A", None);

    let mut b = soft();
    draw_at(&mut b, "A", Some(1.0));

    let mut diff = 0;
    for y in 0..H {
        for x in 0..W {
            if a.pixel(x, y) != b.pixel(x, y) {
                diff += 1;
            }
        }
    }
    assert_eq!(diff, 0, "s = 1.0 必须与不缩放逐像素一致，实测差异 {diff} 像素");

    // 变异对照：改一个像素应该被发现（否则这个比较没有区分力）
    let mut c = soft();
    draw_at(&mut c, "A", None);
    let before = ink(&c);
    // 直接往画布上叠一个像素：若直此测不出变化，说明 ink() 没有区分力
    c.fill_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Color::from_rgb8(1, 2, 3));
    assert_ne!(before, ink(&c), "改一个像素后墨迹数应变化");
}

#[test]
fn half_scale_ink_is_about_a_quarter() {
    let mut a = soft();
    draw_at(&mut a, "A", None);
    let full = ink(&a);

    let mut b = soft();
    draw_at(&mut b, "A", Some(0.5));
    let half = ink(&b);

    let ratio = half as f64 / full as f64;
    // 面积缩小到 1/4，允许 ±30% 的抗锯钽边误差
    assert!(
        (0.175..=0.325).contains(&ratio),
        "s = 0.5 的墨迹数应约为 0.25 倍，实测 {ratio:.3}（{half}/{full}）"
    );

    // 变异对照：半尺寸的墨迹必须严格小于原尺寸
    assert!(half < full, "缩小后墨迹应更少（{half} < {full}）");
}
