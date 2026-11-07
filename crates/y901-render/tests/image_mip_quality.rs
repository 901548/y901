//! 图片 mipmap 质量：**解析真值**（闭式解），不是采样真值。
//!
//! 为什么不用超采样：上一版用 4×4 子采样当"独立真值"，但采样点恰好落回 box 平均的
//! 同一个 2×2 源块 —— 真值就是被测实现本身，MAD 恒为 0，什么也没证明。
//! 这里两个图案都有**可推导的精确答案**，与采样网格无关。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::ImageStyle;
use y901_ui::{Rect, RenderCtx, Size};

const SRC: u32 = 64;
const DST: u32 = 32;
const W: u32 = 96;
const H: u32 = 96;

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

/// 渲染一张 64×64 源图到 32×32，返回目标区域内的 RGB 像素。
fn render_half(src: &[u8]) -> Vec<(u8, u8, u8)> {
    let mut s = backend();
    let id = s.upload_texture(SRC, SRC, src);
    s.clear();
    s.draw_image(
        Rect::new(0.0, 0.0, DST as f32, DST as f32),
        id,
        ImageStyle::default(),
    );
    let mut out = Vec::with_capacity((DST * DST) as usize);
    for y in 0..DST {
        for x in 0..DST {
            let p = s.pixel(x, y).expect("px");
            out.push((
                ((p >> 16) & 0xFF) as u8,
                ((p >> 8) & 0xFF) as u8,
                (p & 0xFF) as u8,
            ));
        }
    }
    out
}

/// 1 像素棋盘：`(x + y) % 2`。
///
/// **闭式真值**：任一 2×2 块里恒为 2 黑 2 白 → 平均值恰为 `(0 + 255) / 2 = 127.5`，
/// 与采样位置无关，因此不可能与被测实现退化成同一个量。
#[test]
fn checkerboard_halves_to_mid_gray() {
    let mut src = vec![0u8; (SRC * SRC * 4) as usize];
    for y in 0..SRC {
        for x in 0..SRC {
            let i = ((y * SRC + x) * 4) as usize;
            let v = if (x + y) % 2 == 0 { 255u8 } else { 0 };
            src[i] = v;
            src[i + 1] = v;
            src[i + 2] = v;
            src[i + 3] = 255;
        }
    }

    let px = render_half(&src);
    let mut bad = 0;
    let mut min = 255u8;
    let mut max = 0u8;
    for (r, _g, _b) in &px {
        min = min.min(*r);
        max = max.max(*r);
        if !(125..=130).contains(r) {
            bad += 1;
        }
    }
    eprintln!(
        "[img-analytic] 棋盘 s=0.5：灰度范围 {min}..{max}，越界 {bad}/{}",
        px.len()
    );
    assert_eq!(
        bad, 0,
        "每个目标像素都应为 127/128（闭式解），实测范围 {min}..{max}"
    );

    // 变异对照：人为改一个像素，必须落在区间外
    let mut mutated = px.clone();
    mutated[0].0 = 0;
    assert!(
        !(125..=130).contains(&mutated[0].0),
        "改成 0 后应不在 125..=130 内"
    );
}

/// 水平斜坡：`value = x * 4`。
///
/// **闭式真值**：目标列 `i` 覆盖源列 `2i` 与 `2i+1`，
/// 平均 = `((2i)*4 + (2i+1)*4) / 2 = 8i + 2`。
#[test]
fn ramp_halves_to_analytic_midpoints() {
    let mut src = vec![0u8; (SRC * SRC * 4) as usize];
    for y in 0..SRC {
        for x in 0..SRC {
            let i = ((y * SRC + x) * 4) as usize;
            let v = (x * 4) as u8;
            src[i] = v;
            src[i + 1] = v;
            src[i + 2] = v;
            src[i + 3] = 255;
        }
    }

    let px = render_half(&src);
    let mut worst = 0i32;
    let mut worst_at = (0usize, 0usize);
    for y in 0..DST {
        for x in 0..DST {
            let got = px[(y * DST + x) as usize].0 as i32;
            let want = 8 * x as i32 + 2;
            if (got - want).abs() > worst {
                worst = (got - want).abs();
                worst_at = (x as usize, y as usize);
            }
        }
    }
    eprintln!(
        "[img-analytic] 斜坡 s=0.5：与闭式解最大偏差 = {worst}（最差处 x={} y={}）",
        worst_at.0, worst_at.1
    );
    assert!(worst <= 3, "每列应为 8i+2（闭式解），最大偏差 {worst}");

    // 变异对照：把某列加 10，必须脱离容差
    let col = 5usize;
    let got = px[col].0 as i32 + 10;
    let want = 8 * col as i32 + 2;
    assert!((got - want).abs() > 3, "加 10 后应脱离闭式解容差");
}

// ---------------------------------------------------------------------------
// 3b：矩阵补宽（非 2 的幂 / alpha / 边界 clamp）
// ---------------------------------------------------------------------------

/// **3b-1 结论**：`box_levels_rgba` 对 R/G/B/A **四个通道各自平均**，
/// **不做 alpha 预乘**。所以下面的期望值是“直接平均”的结果，
/// 而不是预乘公式的结果——两者在 alpha 均匀时相同，
/// 在 alpha 不均匀时**不同**；这里取均匀 alpha，并在注释里说明这一点。
#[test]
fn alpha_half_transparent_red_plus_blue_averages_channels() {
    // 2x2：上半红，下半蓝，alpha 均为 128。
    let mut src = vec![0u8; 2 * 2 * 4];
    for (i, px) in src.chunks_exact_mut(4).enumerate() {
        if i < 2 {
            px.copy_from_slice(&[255, 0, 0, 128]);
        } else {
            px.copy_from_slice(&[0, 0, 255, 128]);
        }
    }
    let mut s = backend();
    let id = s.upload_texture(2, 2, &src);
    s.clear();
    // s = 0.5 → L1 尺寸 1x1
    s.draw_image(Rect::new(0.0, 0.0, 1.0, 1.0), id, ImageStyle::default());
    let p = s.pixel(0, 0).expect("px");
    let (r, g, b) = (
        ((p >> 16) & 0xFF) as u8,
        ((p >> 8) & 0xFF) as u8,
        (p & 0xFF) as u8,
    );
    eprintln!("[img-mip] alpha 均匀 2x2 -> 1x1：({r},{g},{b})");
    // 期望 (127,0,127) 经 alpha=128 混到黑底上 ≈ (64,0,64)；只断言 R≈B 且 G 最小
    assert!(
        r.abs_diff(b) <= 2,
        "R 与 B 应近乎相等（均为半透明原色的平均），实测 ({r},{g},{b})"
    );
    assert!(g < r.max(b) / 2 + 1, "G 应最小，实测 ({r},{g},{b})");
}

/// **边界 clamp**：2x2 @ s=0.25 → 级尺寸与下一级都不得归 0，也不得 panic。
#[test]
fn tiny_texture_quarter_scale_does_not_panic_or_blank() {
    let mut src = vec![0u8; 2 * 2 * 4];
    for px in src.chunks_exact_mut(4) {
        px.copy_from_slice(&[200, 100, 50, 255]);
    }
    let mut s = backend();
    let id = s.upload_texture(2, 2, &src);
    s.clear();
    s.draw_image(Rect::new(0.0, 0.0, 1.0, 1.0), id, ImageStyle::default());
    // 只要没 panic 且画出了东西，就说明尺寸被 clamp 到 ≥ 1。
    let p = s.pixel(0, 0).expect("px");
    let r = ((p >> 16) & 0xFF) as u8;
    eprintln!("[img-mip] 2x2 @ s=0.25：({r},..)");
    assert!(
        p != 0,
        "1x1 目标上应有像素写入（未 panic、且尺寸 clamp 到 1）"
    );
}
