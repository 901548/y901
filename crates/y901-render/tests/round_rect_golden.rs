//! **圆角矩形填充的像素金标**（不透明 / 半透明 / 大圆角 / 小平移分数坐标）。
//!
//! 背景：`fill_round_rect` 每像素算一次 SDF（`rounded_rect_distance` 里有 `sqrt`），
//! 400×300 要 0.68 ms。加了"确定内部"快路：中间带里覆盖率恒为 1 的像素一次性
//! `pack` + 逐像素 `blend`，只有边界带才走 SDF —— 实测 0.09 ms。
//!
//! 这四个哈希录自**旧实现**（`git stash` A/B），新旧逐位相同；钉成金标后，
//! 快路一旦把边界带/圆角/半透明混合算错就会立刻红。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Rect, RenderCtx, Size};

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(400, 300, Box::new(engine), cell).expect("soft")
}

fn hash_of(rect: Rect, color: Color, radius: f32) -> u64 {
    let mut b = soft();
    b.clear();
    b.fill_round_rect(rect, color, radius);
    b.frame_hash()
}

#[test]
fn opaque_rounded_rect_pixels_match_reference() {
    assert_eq!(
        hash_of(
            Rect::new(20.0, 20.0, 300.0, 200.0),
            Color::rgba(0.2, 0.8, 0.2, 1.0),
            16.0
        ),
        0x426D_2A71_0248_073D,
        "不透明圆角矩形像素变了：内部快路或圆角边界算错"
    );
}

#[test]
fn translucent_rounded_rect_pixels_match_reference() {
    assert_eq!(
        hash_of(
            Rect::new(10.5, 12.25, 200.0, 150.0),
            Color::rgba(0.9, 0.4, 0.1, 0.5),
            8.0
        ),
        0xF1BC_49B1_FDB3_9C05,
        "半透明圆角矩形像素变了：快路的 blend 分支算错（分数坐标也必须一致）"
    );
}

#[test]
fn large_radius_rounded_rect_pixels_match_reference() {
    assert_eq!(
        hash_of(
            Rect::new(5.0, 5.0, 380.0, 280.0),
            Color::rgba(0.1, 0.2, 0.9, 1.0),
            40.0
        ),
        0xB23E_21FE_83F6_5845,
        "大圆角（r=40）像素变了：中间带判定或圆弧边界算错"
    );
}

#[test]
fn small_radius_rounded_rect_pixels_match_reference() {
    assert_eq!(
        hash_of(
            Rect::new(30.0, 30.0, 60.0, 40.0),
            Color::rgba(0.5, 0.5, 0.5, 1.0),
            3.0
        ),
        0x6CB2_111F_1EC3_B6FD,
        "小圆角（r=3）像素变了：快路区间算错"
    );
}
