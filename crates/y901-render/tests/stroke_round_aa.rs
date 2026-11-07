//! Soft 的圆角描边：**带抗锯齿**（SDF 覆盖率），与 D3D shader 同公式。
//!
//! 这里断言的不是"画得像"，而是三条可证伪的性质：
//! 1. 直边中点是**满覆盖**（cov=1）
//! 2. 环内（矩形中心）与环外**完全不被写**
//! 3. 圆弧附近**存在部分覆盖像素**（0 < alpha < 255）——这一条是"有 AA"的定义

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Rect, RenderCtx, Size};

const W: u32 = 200;
const H: u32 = 140;

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

/// 取像素的 alpha 通道（`pack` 把 alpha 放最高字节）。
fn alpha(s: &SoftBackend, x: u32, y: u32) -> u32 {
    (s.pixel(x, y).expect("px") >> 24) & 0xff
}

#[test]
fn rounded_stroke_is_antialiased_and_bounded() {
    let mut s = backend();
    s.clear();
    // 背景已知：clear 后应当是完全一致的颜色，便于判断"未被写"
    let bg = s.pixel(0, 0).expect("bg");

    let rect = Rect::new(20.0, 20.0, 120.0, 80.0);
    let color = Color::rgba(1.0, 0.2, 0.2, 1.0);
    // radius=8, width=2：直边中点必然满覆盖，圆弧附近必然有部分覆盖
    s.stroke_round_rect(rect, color, 2.0, 8.0);

    // 1) 直边中点：上边中段（远离圆角）
    let mid_x = (rect.x + rect.w * 0.5) as u32;
    let edge_y = rect.y as u32;
    let a_edge = alpha(&s, mid_x, edge_y);
    assert_eq!(a_edge, 255, "直边中点应满覆盖，实测 alpha={a_edge}");

    // 2) 环内（矩形中心）不被写
    let cx = (rect.x + rect.w * 0.5) as u32;
    let cy = (rect.y + rect.h * 0.5) as u32;
    assert_eq!(
        s.pixel(cx, cy).expect("px"),
        bg,
        "环内（中心）必须保持背景，描边不应填充内部"
    );

    // 3) 环外（矩形外 15px）不被写
    assert_eq!(
        s.pixel((rect.x - 15.0) as u32, cy).expect("px"),
        bg,
        "环外必须保持背景"
    );

    // 4) 圆弧附近必须存在部分覆盖像素 → 证明有 AA
    //
    // 注意：**不能用 alpha 判断**。背景是不透明的，`blend` 之后
    // 目标 alpha 恒为 255；覆盖率只体现在 **RGB 的混合比例**上。
    let bg_rgb = bg & 0x00ff_ffff;
    let full_rgb = s.pixel(mid_x, edge_y).expect("px") & 0x00ff_ffff;
    assert_ne!(bg_rgb, full_rgb, "满覆盖像素应与背景不同");
    let mut partial = 0;
    for y in 0..H {
        for x in 0..W {
            let rgb = s.pixel(x, y).expect("px") & 0x00ff_ffff;
            if rgb != bg_rgb && rgb != full_rgb {
                partial += 1;
            }
        }
    }
    eprintln!("[stroke] 部分覆盖像素数 = {partial}");
    assert!(
        partial > 0,
        "圆弧上必须有部分覆盖像素（RGB 不等于背景也不等于满覆盖）；为 0 说明没有抗锯齿"
    );
}
