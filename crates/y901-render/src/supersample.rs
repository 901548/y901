//! **仅测试用**：超采样光栅化，用作"接近理想"的无偏真值。
//!
//! 为什么要单独实现一份：要判断"重光栅化 vs 盒式滤波谁更接近理想"，
//! 真值必须由**与两条候选都不同**的数学路径产生。现有光栅器是扫描线 + 面积累积，
//! 这里用 4×4 点采样 + 绕数，两者独立；如果复用同一份绕数代码，
//! 测的只是"共享代码是否自洽"，而不是"谁更接近理想"。
//!
//! 这个模块**不进生产路径**（`#[cfg(test)]`），也不追求性能。

use crate::font_engine::{glyph_geometry, Line};

/// 每个像素每边的子采样点数（4 → 16 个采样点）。
pub const SAMPLES: u32 = 4;

/// 超采样得到的字形位图。
pub struct Supersampled {
    /// 宽度（像素）。
    pub width: u32,
    /// 高度（像素）。
    pub height: u32,
    /// 覆盖率 `0..=255`，行优先。
    pub coverage: Vec<u8>,
}

/// 判断点 `(px, py)` 是否在轮廓内（**非零绕数**，与 TrueType 外圈顺时针/内圈逆时针一致）。
///
/// 单位：像素（调用方负责从 26.6 定点换算）。边界规则：`y0 <= py < y1` 半开，
/// 保证共享顶点的两条边不会重复计数。
fn winding_at(lines: &[(f32, f32, f32, f32)], px: f32, py: f32) -> i32 {
    let mut winding = 0;
    for &(x0, y0, x1, y1) in lines {
        if y0 == y1 {
            continue; // 水平边不参与绕数
        }
        let upward = y1 > y0;
        let (lo, hi) = if upward { (y0, y1) } else { (y1, y0) };
        if py < lo || py >= hi {
            continue;
        }
        // 边在 py 处的 x
        let t = (py - y0) / (y1 - y0);
        let x_at = x0 + (x1 - x0) * t;
        if x_at > px {
            winding += if upward { 1 } else { -1 };
        }
    }
    winding
}

/// 用超采样渲染一个字形（与 `rasterize_glyph_phased` 相同的包围盒口径，便于逐像素比较）。
pub fn supersample_glyph(font_data: &[u8], ch: char, size_px: f32) -> Option<Supersampled> {
    let geom = glyph_geometry(font_data, ch, size_px)?;
    supersample_lines(&geom.lines)
}

/// 对已展平的轮廓做超采样（像素坐标由 26.6 定点换算）。
fn supersample_lines(raw: &[Line]) -> Option<Supersampled> {
    let lines: Vec<(f32, f32, f32, f32)> = raw
        .iter()
        .map(|l| {
            (
                l.x0 as f32 / 64.0,
                l.y0 as f32 / 64.0,
                l.x1 as f32 / 64.0,
                l.y1 as f32 / 64.0,
            )
        })
        .collect();
    if lines.is_empty() {
        return None;
    }

    // 与 `rasterize_glyph_phased` 相同的包围盒算法（1px 留边）
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for &(x0, y0, x1, y1) in &lines {
        for (x, y) in [(x0, y0), (x1, y1)] {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    let bearing_x = min_x.floor();
    let bearing_y = min_y.floor();
    let width = ((max_x.ceil() - bearing_x).max(1.0) as u32) + 1;
    let height = ((max_y.ceil() - bearing_y).max(1.0) as u32) + 1;

    // 平移到位图坐标
    let shifted: Vec<(f32, f32, f32, f32)> = lines
        .iter()
        .map(|&(x0, y0, x1, y1)| (x0 - bearing_x, y0 - bearing_y, x1 - bearing_x, y1 - bearing_y))
        .collect();

    let n = SAMPLES;
    let total = (n * n) as f32;
    let mut coverage = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let mut inside = 0u32;
            for sy in 0..n {
                for sx in 0..n {
                    let px = x as f32 + (sx as f32 + 0.5) / n as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / n as f32;
                    if winding_at(&shifted, px, py) != 0 {
                        inside += 1;
                    }
                }
            }
            coverage[(y * width + x) as usize] = ((inside as f32 / total) * 255.0).round() as u8;
        }
    }

    Some(Supersampled {
        width,
        height,
        coverage,
    })
}

/// 一个 8×8 像素的轴对齐矩形轮廓（26.6 定点），用于验证超采样实现本身。
fn rect_lines(x0: f32, y0: f32, w: f32, h: f32) -> Vec<Line> {
    let p = |v: f32| (v * 64.0).round() as i32;
    vec![
        Line {
            x0: p(x0),
            y0: p(y0),
            x1: p(x0 + w),
            y1: p(y0),
        },
        Line {
            x0: p(x0 + w),
            y0: p(y0),
            x1: p(x0 + w),
            y1: p(y0 + h),
        },
        Line {
            x0: p(x0 + w),
            y0: p(y0 + h),
            x1: p(x0),
            y1: p(y0 + h),
        },
        Line {
            x0: p(x0),
            y0: p(y0 + h),
            x1: p(x0),
            y1: p(y0),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mip::MipLevel;
    use crate::MyFontEngine;

    /// 变异对照：8×8 纯矩形，解析覆盖率已知。
    ///
    /// 若这条不过，后面的所有 MAD 对比都作废 —— 因为真值本身是错的。
    #[test]
    fn supersample_matches_analytic_rectangle() {
        // 矩形正好覆盖像素 0..8 × 0..8 的整块区域
        let lines = rect_lines(0.0, 0.0, 8.0, 8.0);
        let s = supersample_lines(&lines).expect("矩形");
        assert_eq!((s.width, s.height), (9, 9), "1px 留边后应为 9×9");

        // 内部 8×8 全 255（误差 ≤ 1/16 = 16/255）
        let mut worst_in = 0i32;
        for y in 0..8 {
            for x in 0..8 {
                worst_in = worst_in.max((255 - s.coverage[(y * s.width + x) as usize] as i32).abs());
            }
        }
        assert!(
            worst_in <= 16,
            "矩形内部覆盖率偏离解析值 {worst_in}/255 > 16/255（超采样实现有 bug）"
        );

        // 留边那一列/行应为 0
        for y in 0..9 {
            let v = s.coverage[(y * s.width + 8) as usize];
            assert_eq!(v, 0, "第 8 列应是留边，实测 {v}");
        }
        for x in 0..9 {
            let v = s.coverage[(8 * s.width + x) as usize];
            assert_eq!(v, 0, "第 8 行应是留边，实测 {v}");
        }
    }

    fn engine(px: f32, atlas: u32) -> MyFontEngine {
        let (cw, ch) = crate::cell_metrics_for(px);
        MyFontEngine::with_atlas_size(
            crate::load_font(&crate::default_test_font_path()).expect("font"),
            px,
            y901_ui::Size::new(cw, ch),
            atlas,
        )
        .expect("engine")
    }

    fn extract(engine: &mut MyFontEngine, ch: char) -> Option<MipLevel> {
        let (w, h, ax, ay) = {
            let g = engine.glyph(ch)?;
            (g.width, g.height, g.atlas_x as usize, g.atlas_y as usize)
        };
        let atlas_size = engine.atlas_size() as usize;
        let atlas = engine.atlas();
        let mut cov = vec![0u8; (w * h) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let si = ((ay + y) * atlas_size + (ax + x)) * 4;
                cov[y * w as usize + x] = atlas.get(si).copied().unwrap_or(0);
            }
        }
        Some(MipLevel::new(w, h, cov))
    }


    /// **对齐后**的 MAD：在 ±2 像素窗口里找最小差异。
    ///
    /// 为什么需要：字形位图的包围盒原点由各自的字号决定，直接按左上角对齐会把
    /// "整体平移了半像素"算成"形状不同"，从而**系统性冤枉**其中一条路径。
    /// 取窗口内最小差异等价于允许一次整体平移对齐，比左上角对齐公平得多。
    fn mad_aligned(a: &MipLevel, b: &Supersampled) -> Option<f32> {
        let mut best: Option<f32> = None;
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let mut sum = 0u64;
                let mut n = 0u64;
                for y in 0..b.height {
                    let ay = y as i32 + dy;
                    if ay < 0 || ay >= a.height as i32 {
                        continue;
                    }
                    for x in 0..b.width {
                        let ax = x as i32 + dx;
                        if ax < 0 || ax >= a.width as i32 {
                            continue;
                        }
                        let ia = (ay as u32 * a.width + ax as u32) as usize;
                        let ib = (y * b.width + x) as usize;
                        sum += (a.coverage[ia] as i16 - b.coverage[ib] as i16).unsigned_abs() as u64;
                        n += 1;
                    }
                }
                if n < 16 {
                    continue;
                }
                let mad = sum as f32 / (255.0 * n as f32);
                best = Some(match best {
                    Some(v) => v.min(mad),
                    None => mad,
                });
            }
        }
        best
    }

    fn mad_cropped(a: &MipLevel, b: &Supersampled) -> Option<f32> {
        let w = a.width.min(b.width);
        let h = a.height.min(b.height);
        if w == 0 || h == 0 {
            return None;
        }
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in 0..h {
            for x in 0..w {
                let ia = (y * a.width + x) as usize;
                let ib = (y * b.width + x) as usize;
                sum += (a.coverage[ia] as i16 - b.coverage[ib] as i16).unsigned_abs() as u64;
                n += 1;
            }
        }
        Some(sum as f32 / (255.0 * n as f32))
    }

    #[test]
    #[ignore = "无偏质量测量：--ignored --nocapture 手动跑"]
    fn which_method_is_closer_to_supersampled_truth() {
        let font = crate::load_font(&crate::default_test_font_path()).expect("font");
        let mut base = engine(15.0, 2048);
        let mut half = engine(7.5, 1024);

        let classes: [(&str, Vec<char>); 3] = [
            ("大写字母", ('A'..='Z').collect()),
            ("小写字母", ('a'..='z').collect()),
            ("数字", ('0'..='9').collect()),
        ];

        eprintln!("[ss] | 字形类别 | 样本 | 重光栅化 MAD | 盒式滤波 MAD | 谁更近 | 差距 |");
        let t0 = std::time::Instant::now();
        let (mut all_re, mut all_box, mut all_n) = (0.0f64, 0.0f64, 0usize);
        let (mut aligned_re, mut aligned_box, mut aligned_n) = (0.0f64, 0.0f64, 0usize);

        for (name, chars) in &classes {
            let (mut s_re, mut s_box, mut n) = (0.0f64, 0.0f64, 0usize);
            let (a_re, a_box, an) = (0.0f64, 0.0f64, 0usize);
            for &ch in chars {
                let truth = match supersample_glyph(&font, ch, 7.5) {
                    Some(t) => t,
                    None => continue,
                };
                let l0 = match extract(&mut base, ch) {
                    Some(l) => l,
                    None => continue,
                };
                let l1_re = match extract(&mut half, ch) {
                    Some(l) => l,
                    None => continue,
                };
                let l1_box = l0.downsample_box();
                let (re, bx) = match (mad_cropped(&l1_re, &truth), mad_cropped(&l1_box, &truth)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                };
                // 对齐后的对比（允许±2px 整体平移）
                let (re_a, bx_a) = match (mad_aligned(&l1_re, &truth), mad_aligned(&l1_box, &truth)) {
                    (Some(a), Some(b)) => (a, b),
                    _ => continue,
                };
                aligned_re += re_a as f64;
                aligned_box += bx_a as f64;
                aligned_n += 1;
                s_re += re as f64;
                s_box += bx as f64;
                n += 1;
            }
            if n == 0 {
                continue;
            }
            let mean_re = s_re / n as f64;
            let mean_box = s_box / n as f64;
            let winner = if mean_re < mean_box { "重光栅化" } else { "盒式滤波" };
            let gap = if mean_re.min(mean_box) > 0.0 {
                100.0 * (mean_box - mean_re).abs() / mean_box.min(mean_re)
            } else {
                0.0
            };
            eprintln!(
                "[ss] | {} | {} | {:.4} | {:.4} | {} | {:.1}% |",
                name, n, mean_re, mean_box, winner, gap
            );
            if an > 0 {
                eprintln!(
                    "[ss] |   对齐后 | {} | {:.4} | {:.4} | {} | |",
                    an,
                    a_re / an as f64,
                    a_box / an as f64,
                    if a_re < a_box { "重光栅化" } else { "盒式滤波" }
                );
            }
            aligned_re += a_re;
            aligned_box += a_box;
            aligned_n += an;
            all_re += s_re;
            all_box += s_box;
            all_n += n;
        }
        let elapsed = t0.elapsed();
        if all_n > 0 {
            let mean_re = all_re / all_n as f64;
            let mean_box = all_box / all_n as f64;
            eprintln!(
                "[ss] 合计 {} 个字形（{:?}）：重光栅化 {:.4}，盒式滤波 {:.4}",
                all_n, elapsed, mean_re, mean_box
            );
            assert!(mean_re > 0.0 && mean_box > 0.0);
            if aligned_n > 0 {
                eprintln!(
                    "[ss] 对齐后合计 {} 个：重光栅化 {:.4}，盒式滤波 {:.4}",
                    aligned_n,
                    aligned_re / aligned_n as f64,
                    aligned_box / aligned_n as f64
                );
            }
        }
    }
}
