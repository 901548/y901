//! Soft 后端的 `opacity` 语义：逐图元乘法，alpha = `color.a * coverage * opacity`。
//!
//! 嵌套是**乘积**（`RenderState::current_opacity`），不是覆盖。
//! 这里断言的三条：opacity 0.5 半透明化、嵌套 0.5×0.5=0.25、opacity 1.0 无副作用。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Rect, RenderCtx, Size};

const W: u32 = 64;
const H: u32 = 64;

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

/// 取像素的红通道。
///
/// **为何不看 alpha**：底色是不透明的，`blend` 之后目标 alpha 恒为 255，
/// 不透明度只体现在 **RGB 的混合比例** 上。
fn red(s: &SoftBackend, x: u32, y: u32) -> u32 {
    (s.pixel(x, y).expect("px") >> 16) & 0xff
}

/// 先铺不透明黑底，再按给定不透明度画红色方块，返回中心像素的**红通道**。
///
/// 黑底是必需的：若底色为白，`red*0.5 + white*0.5` 仍是 255，看不出变化。
fn red_square_alpha(opacities: &[f32]) -> u32 {
    let mut s = backend();
    s.clear();
    s.fill_rect(
        Rect::new(0.0, 0.0, W as f32, H as f32),
        Color::rgba(0.0, 0.0, 0.0, 1.0),
    );
    for &o in opacities {
        s.push_opacity(o);
    }
    s.fill_rect(
        Rect::new(16.0, 16.0, 32.0, 32.0),
        Color::rgba(1.0, 0.0, 0.0, 1.0),
    );
    let a = red(&s, 32, 32);
    for _ in opacities {
        s.pop_opacity();
    }
    a
}

#[test]
fn opacity_halves_alpha() {
    let a = red_square_alpha(&[0.5]);
    assert!(
        (a as i32 - 128).abs() <= 1,
        "opacity 0.5 时 alpha 应 ≈128，实测 {a}"
    );
}

#[test]
fn nested_opacity_multiplies() {
    let a = red_square_alpha(&[0.5, 0.5]);
    assert!(
        (a as i32 - 64).abs() <= 1,
        "嵌套 0.5×0.5 应得 alpha ≈64，实测 {a}（若是 128 说明用了覆盖而非乘积）"
    );
}

#[test]
fn opacity_one_is_noop() {
    let plain = red_square_alpha(&[]);
    let with_one = red_square_alpha(&[1.0]);
    assert_eq!(plain, 255, "不设 opacity 时应完全不透明，实测 {plain}");
    assert_eq!(with_one, plain, "opacity 1.0 必须与不设置完全一致");
}
