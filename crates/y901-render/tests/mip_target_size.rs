//! **mip 级绘制的尺寸必须 = `L0 × scale`**，而不是"该级位图的原始尺寸"。
//!
//! 背景（审查 P1-4）：`SoftBackend` 曾经在 `level > 0` 时直接把 slot 按 1:1 贴上去
//! （L1=1/2、L2=1/4 尺寸），但 `advance` 与 `ascent` 用的是**连续**的 `scale`。
//! 于是 `s ∈ (0.25, 0.5)` 时字形按 1/4 画、字距按 0.4× 推进 —— 字宽与推进脱钩、
//! 基线也对不齐。生产里弹层只用 0.8~1.0（level 0）所以没显形，但契约是自相矛盾的。
//!
//! 这条用例只用 CPU 后端（无 GPU），因此进默认套件：
//! 同一字形在 `s = 1.0 / 0.5 / 0.4` 下的墨迹高度比，必须各自接近 `s`。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, RenderCtx, Size, TextStyle};

/// 用的字号偏大（64px）：只有把"±1px 取整误差"压到比值噪声以下，
/// 才能用紧公差区分"按 slot 尺寸画"（0.25×）与"按目标尺寸画"（0.30×）——
/// 两者只差 0.05，宽松容差会同时放过两种实现（第一版就吃过这个亏）。
const PX: f32 = 64.0;
const W: u32 = 420;
const H: u32 = 220;

fn canvas() -> SoftBackend {
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

/// 墨迹包围盒高度（背景取右下角像素）。
fn ink_height(s: &SoftBackend) -> u32 {
    let bg = s.pixel(W - 2, H - 2).expect("bg");
    let mut y0 = u32::MAX;
    let mut y1 = 0u32;
    for y in 0..H {
        for x in 0..W {
            if s.pixel(x, y).expect("px") != bg {
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    assert!(y0 != u32::MAX, "没有画出任何墨迹");
    y1 - y0 + 1
}

/// 在给定变换缩放下渲染一个字形，返回墨迹高度。
fn height_at(scale: f32) -> u32 {
    let mut s = canvas();
    let style = TextStyle::fg(Color::rgba(0.95, 0.96, 1.0, 1.0));
    s.clear();
    if (scale - 1.0).abs() < f32::EPSILON {
        s.draw_text(Pos::new(20.0, 20.0), "H", &style);
    } else {
        s.push_transform(Pos::new(20.0, 20.0), scale);
        s.draw_text(Pos::new(20.0, 20.0), "H", &style);
        s.pop_transform();
    }
    ink_height(&s)
}

#[test]
fn mip_level_draws_at_target_size_not_slot_size() {
    let full = height_at(1.0);
    assert!(full >= 40, "64px 字形的墨迹高度只有 {full}，用例前提不成立");

    // 0.5 → L1；0.30 → L2（`level_for = round(log2(1/s))`）。
    // **0.30 是鉴别力所在**：按 slot 画会得到 ≈0.25，按目标画才是 0.30。
    for (scale, tol) in [(0.5_f32, 0.04_f32), (0.30, 0.035)] {
        let got = height_at(scale) as f32 / full as f32;
        assert!(
            (got - scale).abs() <= tol,
            "s={scale}：墨迹高度比 {got:.3}，期望 ≈{scale}（容差 {tol}）\
             —— 明显偏小说明又按 mip slot 的原始尺寸绘制了（P1-4 回归）"
        );
    }
}
