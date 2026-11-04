//! 图片 mipmap 的质量断言（#18 之后的图片 LOD 步骤 2b）。
//!
//! 真值**独立构造**：对源图做 4×4 超采样后平均，再映射到目标尺寸；
//! 不经过 `box_levels_rgba`（那是被测实现），否则就是"拿自己当自己的真值"。
//!
//! 跑法：
//! ```text
//! cargo test -p y901-render --release --test image_mip_quality -- --nocapture
//! ```

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::ImageStyle;
use y901_ui::{Rect, RenderCtx, Size};

const W: u32 = 160;
const H: u32 = 120;
/// 源图：4 像素周期的高频棋盘 + 一条渐变，专门用来暴露"选级错误"和"降采样质量差"。
const SRC: u32 = 128;

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

fn source_rgba() -> Vec<u8> {
    let mut v = vec![0u8; (SRC * SRC * 4) as usize];
    for y in 0..SRC {
        for x in 0..SRC {
            let i = ((y * SRC + x) * 4) as usize;
            // 8 像素棋盘 + 水平渐变：棋盘测"是否漏掉高频"，渐变测"整体亮度是否偏"
            let checker = if ((x / 8) + (y / 8)) % 2 == 0 { 255 } else { 0 };
            let grad = (x * 255 / SRC) as u8;
            v[i] = ((checker as u16 + grad as u16) / 2) as u8;
            v[i + 1] = ((checker as u16 + 255 - grad as u16) / 2) as u8;
            v[i + 2] = checker;
            v[i + 3] = 255;
        }
    }
    v
}

/// **独立真值**：4×4 超采样平均，得到 `dw × dh` 的目标图。
fn supersampled_truth(src: &[u8], dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw * dh * 4) as usize];
    for ty in 0..dh {
        for tx in 0..dw {
            let mut acc = [0u32; 4];
            for sy in 0..4u32 {
                for sx in 0..4u32 {
                    // 目标像素 (tx,ty) 覆盖源图的 4×4 子块
                    let gx = (tx * 4 + sx) * SRC / (dw * 4);
                    let gy = (ty * 4 + sy) * SRC / (dh * 4);
                    let si = ((gy.min(SRC - 1) * SRC + gx.min(SRC - 1)) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += src[si + c] as u32;
                    }
                }
            }
            let di = ((ty * dw + tx) * 4) as usize;
            for c in 0..4 {
                out[di + c] = (acc[c] / 16) as u8;
            }
        }
    }
    out
}

fn render_scaled(scale: f32) -> (Vec<u8>, u32, u32) {
    let mut s = backend();
    let src = source_rgba();
    let id = s.upload_texture(SRC, SRC, &src);
    s.clear();
    let rect = Rect::new(8.0, 8.0, (SRC as f32 * scale).round(), (SRC as f32 * scale).round());
    s.draw_image(rect, id, ImageStyle::default());
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let p = s.pixel(x, y).expect("px");
            out.push((p >> 16) as u8);
            out.push((p >> 8) as u8);
            out.push(p as u8);
            out.push(255);
        }
    }
    (out, W, H)
}

/// 目标矩形内的 MAD：看**画出来的那块**，不看空白背景。
fn mad_in_rect(a: &[u8], b: &[u8], w: u32, x0: u32, y0: u32, rw: u32, rh: u32) -> f64 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in y0..(y0 + rh).min(H) {
        for x in x0..(x0 + rw).min(W) {
            let i = ((y * w + x) * 4) as usize;
            // b 是真值图，它自己的宽度是 rw（不是屏幕宽度）：
            // 用屏幕坐标去索引会越界（这是我第一版的 bug，测试直接报了 index out of bounds）。
            let j = (((y - y0) * rw + (x - x0)) * 4) as usize;
            for c in 0..3 {
                sum += (a[i + c] as i32 - b[j + c] as i32).unsigned_abs() as u64;
            }
            n += 3;
        }
    }
    sum as f64 / (255.0 * n.max(1) as f64)
}

#[test]
#[ignore = "**真值不独立**：我的 4×4 采样点落回 2×2 源像素，等价于 box，MAD 恒为 0 —— 需重新构造"]
fn half_scale_matches_supersampled_truth() {
    let (px, w, _h) = render_scaled(0.5);
    let src = source_rgba();
    let dw = SRC / 2;
    let truth = supersampled_truth(&src, dw, dw);

    // 目标矩形在 (8,8)，尺寸 dw×dw
    let mad = mad_in_rect(&px, &truth, w, 8, 8, dw, dw);
    eprintln!("[img-mip] s=0.5 与超采样真值 MAD = {mad:.4}");

    // 变异对照：人为改一个像素，MAD 必须变大（证明测量有区分力）
    let mut mutated = px.clone();
    let mi = ((8 * w + 8) * 4) as usize;
    mutated[mi] = mutated[mi].wrapping_add(97);
    let mad_mut = mad_in_rect(&mutated, &truth, w, 8, 8, dw, dw);
    eprintln!("[img-mip] 变异对照 MAD = {mad_mut:.4}（应 > {mad:.4}）");
    assert!(mad_mut > mad, "改一个像素后 MAD 应变大");

    assert!(mad <= 0.05, "与超采样真值的 MAD = {mad:.4} > 0.05");
}
