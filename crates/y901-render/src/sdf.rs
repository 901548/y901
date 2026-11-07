//! 圆角矩形 SDF 覆盖率计算。
//!
//! 与 `crates/y901-backend-win32/src/backend/d3d11.rs` 的 `ps_main` 中 SDF 分支
//! **逐字对应**。改这里必须同步改那里（反之亦然），否则 D3D 与 Soft 的描边
//! 会出现肉眼可见的宽度偏差。
//!
//! 公式来源：标准圆角矩形有向距离场 + 1px 线性过渡覆盖率。
//! 无 `ddx`/`ddy`、无多采样，`softness` 固定 `1.0`——正因为如此，CPU 才能低成本复现。

/// 圆角矩形有向距离场。
///
/// - `p`：相对矩形**中心**的像素坐标（像素中心采样，即 `x + 0.5`）
/// - `half`：矩形半尺寸
/// - `r`：圆角半径（内部会 clamp 到 `min(half)`，与着色器一致）
///
/// 返回：`< 0` 在内部，`> 0` 在外部，单位 = 像素。
#[inline]
pub fn rounded_rect_distance(p: (f32, f32), half: (f32, f32), r: f32) -> f32 {
    // 对应 HLSL: float r = clamp(input.params.x, 0.0, min(half_size.x, half_size.y));
    let r = r.clamp(0.0, half.0.min(half.1));
    // 对应: float2 q = abs(p) - half_size + r;
    let qx = p.0.abs() - half.0 + r;
    let qy = p.1.abs() - half.1 + r;
    // 对应: float d = min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    qx.max(qy).min(0.0) + (ox * ox + oy * oy).sqrt() - r
}

/// 描边覆盖率：`|d| <= width/2` 的环，1px 线性过渡。
///
/// 对应 HLSL：`saturate(0.5 - (abs(d) - w * 0.5) / soft)`。
/// `softness` 固定 `1.0`，保留参数只为与着色器参数位（`params.y`）对齐。
#[inline]
pub fn stroke_coverage(d: f32, width: f32, softness: f32) -> f32 {
    let soft = softness.max(0.001);
    (0.5 - (d.abs() - width * 0.5) / soft).clamp(0.0, 1.0)
}

/// 实心填充覆盖率：`saturate(0.5 - d / soft)`（着色器的非描边分支）。
#[inline]
pub fn fill_coverage(d: f32, softness: f32) -> f32 {
    let soft = softness.max(0.001);
    (0.5 - d / soft).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HALF: (f32, f32) = (50.0, 25.0);
    const R: f32 = 8.0;

    #[test]
    fn center_is_inside() {
        let d = rounded_rect_distance((0.0, 0.0), HALF, R);
        assert!(d < 0.0, "中心必须在内部，d={d}");
    }

    #[test]
    fn far_corner_is_outside() {
        // 右上方，远离圆角
        let d = rounded_rect_distance((HALF.0 + 10.0, HALF.1 + 10.0), HALF, R);
        assert!(d > 0.0, "角外必须在外部，d={d}");
    }

    #[test]
    fn straight_edge_is_near_zero() {
        // 上边中点（不涉及圆角）
        let d = rounded_rect_distance((0.0, -HALF.1), HALF, R);
        assert!(d.abs() < 0.01, "直边上 d 应 ≈ 0，实测 {d}");
    }

    #[test]
    fn corner_arc_point_is_near_zero() {
        // 右上方圆心位置：(half.0 - r, -(half.1 - r))，再沿 45° 外推 r
        let (cx, cy) = (HALF.0 - R, -(HALF.1 - R));
        let k = R / std::f32::consts::SQRT_2;
        let d = rounded_rect_distance((cx + k, cy - k), HALF, R);
        assert!(d.abs() < 0.05, "圆弧上 d 应 ≈ 0，实测 {d}");
    }

    #[test]
    fn stroke_coverage_hits_half_at_the_band_edge() {
        // 环的边缘是 |d| = w/2；w=2 时 d=1.0 → cov = 0.5
        let cov = stroke_coverage(1.0, 2.0, 1.0);
        assert!((cov - 0.5).abs() < 1e-6, "|d|=w/2 时 cov 应为 0.5，实测 {cov}");
    }

    #[test]
    fn stroke_coverage_is_opaque_at_the_band_center() {
        // d = 0 是线宽的中心线 → 完全不透明
        let cov = stroke_coverage(0.0, 2.0, 1.0);
        assert!((cov - 1.0).abs() < 1e-6, "线中心 cov 应为 1，实测 {cov}");
    }

    #[test]
    fn stroke_coverage_is_full_inside_band() {
        // width=2 → 环半宽 1；d=0.5 在环内 → cov = 1
        let cov = stroke_coverage(0.5, 2.0, 1.0);
        assert!((cov - 1.0).abs() < 1e-6, "d=0.5,width=2 时 cov 应为 1，实测 {cov}");
    }

    #[test]
    fn stroke_coverage_is_zero_outside_band() {
        // d=1.5 > width/2 + 0.5 → cov = 0
        let cov = stroke_coverage(1.5, 2.0, 1.0);
        assert_eq!(cov, 0.0, "d=1.5,width=2 时应完全在环外");
    }

    #[test]
    fn fill_coverage_matches_shader_branch() {
        assert!((fill_coverage(0.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(fill_coverage(0.6, 1.0), 0.0);
        assert_eq!(fill_coverage(-0.6, 1.0), 1.0);
    }
}
