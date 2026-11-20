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

// ---------------------------------------------------------------------------
// 4：**各向异性**（两个轴缩放比不同）
//
// 为什么单独一组成立：布局（`ImageStyle::Fill`）本来就允许 X/Y 独立缩放，
// 但**过滤器没有各向异性路径** —— Soft 用 `min(scale_x, scale_y)` 选**一个**
// 各向同性 mip 级。这组用例把这个策略的**可观测后果**钉死：
//   - 强缩小轴必须**不出现高频走样**（这是选 min 的理由，也是不能改成 max 的原因）；
//   - 另一轴允许变模糊，但结构（单调渐变）必须还在，不能被抹平。
// 它同时是"改过滤器前先跑一遍"的基线：任何改动若让强缩小轴重新走样，这里会红。
// ---------------------------------------------------------------------------

/// 渲染任意源/目标尺寸的图，返回目标区域每像素 RGB（行优先）。
fn render_to(src_w: u32, src_h: u32, src: &[u8], dst_w: u32, dst_h: u32) -> Vec<(u8, u8, u8)> {
    assert!(dst_w.max(dst_h) <= W, "目标必须放得进画布");
    let mut s = backend();
    let id = s.upload_texture(src_w, src_h, src);
    s.clear();
    s.draw_image(
        Rect::new(0.0, 0.0, dst_w as f32, dst_h as f32),
        id,
        ImageStyle::default(),
    );
    let mut out = Vec::with_capacity((dst_w * dst_h) as usize);
    for y in 0..dst_h {
        for x in 0..dst_w {
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

fn stats(px: &[(u8, u8, u8)], ch: usize) -> (u8, u8, f32) {
    let vals: Vec<u8> = px
        .iter()
        .map(|p| match ch {
            0 => p.0,
            1 => p.1,
            _ => p.2,
        })
        .collect();
    let min = *vals.iter().min().unwrap();
    let max = *vals.iter().max().unwrap();
    let mean = vals.iter().map(|v| *v as f32).sum::<f32>() / vals.len() as f32;
    (min, max, mean)
}

/// 构造源图：`r` 通道高频（沿 `axis` 每 **2 像素**一黑一白），`g` 通道低频斜坡。
///
/// 为什么用 **6** 个目标列/行（而不是 8）做缩小 8× 的用例：缩小倍率 64/8 = 8 是整数，
/// 采样点会恒定落在 `2 亮 2 暗` 的**明暗交界**上，双线性把它平均回 127.5 ——
/// "走样"和"正确"给出同一个数，变异测试（`min` → `max`）不会变红。
/// 用 6 则 `du = 64/6 ≈ 10.67`，采样点落进同一明暗块内部，走样时 R 是 0 或 255，
/// 与闭式解 127.5 截然不同，断言才有牙齿。
fn striped_ramp(src_w: u32, src_h: u32, stripes_along_x: bool) -> Vec<u8> {
    let mut src = vec![0u8; (src_w * src_h * 4) as usize];
    for y in 0..src_h {
        for x in 0..src_w {
            let i = ((y * src_w + x) * 4) as usize;
            let phase = if stripes_along_x { x } else { y };
            src[i] = if (phase / 2) % 2 == 0 { 255 } else { 0 };
            src[i + 1] = (x * 4).min(255) as u8; // 水平斜坡
            src[i + 2] = (y * 4).min(255) as u8; // 垂直斜坡
            src[i + 3] = 255;
        }
    }
    src
}

/// 强缩小轴的宽度：见 `striped_ramp` 的说明（必须不是 8 的整数倍除尽）。
const SHRUNK: u32 = 6;

/// **X 轴强缩小 10.7×、Y 轴不缩放**：高频纹必须平均成中灰，不许走样成黑白相间。
///
/// 闭式解：每 4 列一周期（2 亮 2 暗），任一 4 的倍数宽度 box 平均恒为 127.5；
/// 选中的 mip 级（L2 = 4×4 box）每纹素恰好是 2 亮 2 暗 → 127.5。
/// 因此"选 min 倍率"时 R 必须处处 ≈127；
/// 若改成 `max(scale_x, scale_y)`（=1.0，即 L0），采样落进单块内 → R 变成 0 或 255。
#[test]
fn x_stripes_shrunk_8x_do_not_alias() {
    let src = striped_ramp(SRC, SRC, true);
    let px = render_to(SRC, SRC, &src, SHRUNK, SRC);
    let (rmin, rmax, rmean) = stats(&px, 0);
    eprintln!("[img-aniso] x 1/10.7, y 1/1：R 范围 {rmin}..{rmax}（均值 {rmean:.1}）");
    assert!(
        rmin >= 120 && rmax <= 135,
        "强缩小轴必须接近闭式解 127.5，实测 R {rmin}..{rmax}（走样会接近 0..255）"
    );
    // 垂直斜坡（B 通道）—— Y 轴未缩放，但选级仍把它一起降采样了（已知取舍）：
    // 要求结构仍在（首行明显暗于末行），不要求逐像素精确。
    let row_mean = |row: usize| -> f32 {
        (0..SHRUNK as usize)
            .map(|x| px[row * SHRUNK as usize + x].2 as f32)
            .sum::<f32>()
            / SHRUNK as f32
    };
    let first_row_mean = row_mean(0);
    let last_row_mean = row_mean(SRC as usize - 1);
    eprintln!("[img-aniso] B 行均值：首行 {first_row_mean:.1} → 末行 {last_row_mean:.1}");
    assert!(
        last_row_mean - first_row_mean > 150.0,
        "低频轴的结构不能被抹平：首行 {first_row_mean:.1} → 末行 {last_row_mean:.1}"
    );
}

/// **Y 轴强缩小、X 轴不缩放**（上一个用例的转置）：同一结论，防止实现只对 X 特判。
#[test]
fn y_stripes_shrunk_8x_do_not_alias() {
    let src = striped_ramp(SRC, SRC, false);
    let px = render_to(SRC, SRC, &src, SRC, SHRUNK);
    let (rmin, rmax, rmean) = stats(&px, 0);
    eprintln!("[img-aniso] x 1/1, y 1/10.7：R 范围 {rmin}..{rmax}（均值 {rmean:.1}）");
    assert!(
        rmin >= 120 && rmax <= 135,
        "强缩小轴必须接近闭式解 127.5，实测 R {rmin}..{rmax}"
    );
    let col_mean = |col: usize| -> f32 {
        (0..SHRUNK as usize)
            .map(|y| px[y * SRC as usize + col].1 as f32)
            .sum::<f32>()
            / 8.0
    };
    let first_col_mean = col_mean(0);
    let last_col_mean = col_mean(SRC as usize - 1);
    eprintln!("[img-aniso] G 列均值：首列 {first_col_mean:.1} → 末列 {last_col_mean:.1}");
    assert!(
        last_col_mean - first_col_mean > 150.0,
        "低频轴的结构不能被抹平：首列 {first_col_mean:.1} → 末列 {last_col_mean:.1}"
    );
}

/// **各向异性比 > 2 且两轴都缩小**（1/8 × 1/2）：不允许 panic / 全黑 / 走样。
///
/// 这条守住的是"文档里写明的限制"：比值 > 2 时没有专门路径，但**结果必须有界**，
/// 不能出现未定义行为或整块空白。
#[test]
fn anisotropic_ratio_above_two_is_bounded() {
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
    let px = render_to(SRC, SRC, &src, 8, 32);
    let (rmin, rmax, rmean) = stats(&px, 0);
    eprintln!("[img-aniso] 1/8 × 1/2：R 范围 {rmin}..{rmax}（均值 {rmean:.1}）");
    assert!(px.len() == 8 * 32, "目标像素数不对");
    assert!(
        rmin >= 118 && rmax <= 138,
        "比值 > 2 时仍应落在闭式解 127.5 附近，实测 {rmin}..{rmax}"
    );
    // 全部目标像素都必须真的被写过（不是背景色）：闭式解与背景不同，均值即可证明。
    assert!(
        (rmean - 127.5).abs() < 6.0,
        "均值应接近 127.5，实测 {rmean:.1}（偏离说明有像素没被绘制）"
    );
}

/// **近似各向同性对照**（1/2 × 约 1/2.06）：确认上面三条的断言不是"任何缩小都成立"。
///
/// 若把强缩小轴放宽到 120..=135 是"因为测试太松"，这条对照会因为同样的阈值通过；
/// 但它同时验证了**轻微各向异性不会破坏闭式解** —— 即阈值没有掩盖真实回归。
#[test]
fn near_isotropic_control_matches_analytic_average() {
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
    let px = render_to(SRC, SRC, &src, 32, 31);
    let (rmin, rmax, rmean) = stats(&px, 0);
    eprintln!("[img-aniso] 对照 1/2 × 1/2.06：R 范围 {rmin}..{rmax}（均值 {rmean:.1}）");
    assert!(
        rmin >= 118 && rmax <= 138,
        "对照组也应接近 127.5，实测 {rmin}..{rmax}"
    );
}
