//! 扫描线光栅化：非零环绕 + 面积累积覆盖率。
//!
//! **算法**：水平方向用精确覆盖（区间与像素列的重叠长度），
//! 垂直方向用 4 子扫描线累积。两边都是"面积累积"，不是采样：
//! 每条子扫描线上的区间对每个像素列的贡献是**精确重叠长度**，
//! 只有垂直方向被量化成 1/4 像素。
//!
//! 为什么不直接做 4x4 超采样：水平方向保持解析度对斜线很重要（`/`、`A`的斜边），
//! 而 4×4 会把水平也量化。当前方案在同等成本下质量更好。

use super::flatten::Line;

/// 覆盖度位图（0..=255，行主序）。
pub struct Coverage(pub Vec<u8>);

/// 垂直子扫描线数：4 → 每个像素行的覆盖度精度 1/4。
const SS: u32 = 8;

/// 光栅化：输入 26.6 定点线段，输出 `width*height` 覆盖度。
///
/// 返回：`None` = 尺寸为 0。
/// 副作用：无。
///
/// 为什么要半开区间 `[ymin, ymax)`：扫描线恰好穿过两条边的共用顶点时，
/// 若两边都算，环绕数会被加两次 → 出现 1px 竖线或洞。
pub fn rasterize(lines: &[Line], width: u32, height: u32) -> Option<Coverage> {
    if width == 0 || height == 0 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    // 行累积：每个像素累加"被覆盖的子扫描线数"的分数部分
    let mut acc = vec![0f32; w * h];
    // 每条子扫描线的交点缓存（复用，避免每行分配）
    let mut xs: Vec<(f32, i32)> = Vec::with_capacity(lines.len());

    // 以 26.6 取样：每个像素行分成 SS 条，取每条的中心
    let sub_step = 64 / SS as i32; // 16（1/4 像素 = 16/64）
    for row in 0..h {
        let base = (row as i32) * 64 + sub_step / 2;
        for s in 0..SS as i32 {
            let y = base + s * sub_step; // 26.6

            // ---- 收集交点 ----
            xs.clear();
            for l in lines {
                let (ymin, ymax) = if l.y0 < l.y1 {
                    (l.y0, l.y1)
                } else {
                    (l.y1, l.y0)
                };
                // 半开区间：ymin <= y < ymax
                if y < ymin || y >= ymax {
                    continue;
                }
                let dy = (l.y1 - l.y0) as f32;
                if dy == 0.0 {
                    continue;
                }
                let t = (y - l.y0) as f32 / dy;
                let x = l.x0 as f32 + (l.x1 - l.x0) as f32 * t;
                // 方向：y 增大 = +1，减小 = -1
                xs.push((x, if l.y1 > l.y0 { 1 } else { -1 }));
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            // ---- 非零环绕：扫出内部区间 ----
            let mut winding = 0i32;
            let mut span_start = 0f32;
            for (x, dir) in xs.iter() {
                let prev = winding;
                winding += dir;
                if prev == 0 && winding != 0 {
                    span_start = *x;
                } else if prev != 0 && winding == 0 {
                    // 区间 [span_start, x] 属于内部
                    add_span(&mut acc, row, w, span_start / 64.0, *x / 64.0);
                }
            }
        }
    }

    // 归一化：SS 条子扫描线 → 0..=255
    let mut out = vec![0u8; w * h];
    let inv = 255.0 / SS as f32;
    for (i, a) in acc.iter().enumerate() {
        let v = (a * inv).round().clamp(0.0, 255.0);
        out[i] = v as u8;
    }
    Some(Coverage(out))
}

/// 把一条内部区间 `[x0, x1)`（像素坐标）按**精确重叠长度**加到当前行。
///
/// 这就是"水平解析覆盖"：区间与每个像素列的交集长度直接就是该列在这条子扫描线上的覆盖分数。
#[inline]
fn add_span(acc: &mut [f32], row: usize, w: usize, x0: f32, x1: f32) {
    if x1 <= x0 {
        return;
    }
    let row_base = row * w;
    let start = x0.floor().max(0.0) as usize;
    let end = (x1.ceil() as isize).max(0) as usize;
    let end = end.min(w);
    if start >= end {
        return;
    }
    for col in start..end {
        let left = col as f32;
        let right = left + 1.0;
        let overlap = x1.min(right) - x0.max(left);
        if overlap > 0.0 {
            acc[row_base + col] += overlap.min(1.0);
        }
    }
}
