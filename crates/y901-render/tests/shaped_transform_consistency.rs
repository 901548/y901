//! **shaped 绘制与 cell 绘制必须在变换下一致**（feature `shaping`）。
//!
//! 曾经的 bug：`draw_text_shaped` 入口先 `map_point(origin)`，两条回退路径
//! （缺字体字节 / shape 失败）却把**已经映射过**的原点传给 `draw_text_sized`，
//! 而后者入口又映射一次 —— 变换被应用两次（`p → o + (p−o)·s²`）。
//! 恒等变换下完全看不出来，只有缩放动画（弹层 0.8）里的复杂文本会整体偏移。
//!
//! 测法：同一段文本、同一个非恒等变换，分别走 shaped 路径与 cell 路径
//! （`needs_shaping == false` 的 ASCII 走 cell），比较像素包围盒的左边界。
//! 若变换被重复应用，shaped 文本会明显偏左/偏右。

#![cfg(feature = "shaping")]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextStyle};

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        16.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(400, 200, Box::new(engine), cell).expect("soft")
}

/// 首列有墨迹的像素 x（用于比较"文本从哪里开始画"）。
fn first_ink_x(b: &SoftBackend) -> Option<u32> {
    let bg = b.pixel(0, 0)?;
    (0..b.width()).find(|x| (0..b.height()).any(|y| b.pixel(*x, y) != Some(bg)))
}

#[test]
fn shaped_text_under_transform_lands_like_cell_text() {
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let origin = Pos::new(40.0, 60.0);

    // 参照：ASCII（`needs_shaping == false` → cell 路径，这条路径一直是正确的）
    let mut cell = backend();
    cell.clear();
    cell.push_transform(Pos::new(0.0, 0.0), 0.8);
    cell.draw_text(origin, "abc", &style);
    cell.pop_transform();
    let cell_x = first_ink_x(&cell).expect("cell 路径应有墨迹");

    // 被测：复杂脚本（走 shaped 路径；希伯来字母本身不是 ASCII，宽度不同，
    // 但**起点**必须与 cell 路径一致——两者都在同一个 origin、同一个变换下）
    let mut shaped = backend();
    shaped.clear();
    shaped.push_transform(Pos::new(0.0, 0.0), 0.8);
    shaped.draw_text(origin, "שלום", &style);
    shaped.pop_transform();
    let shaped_x = first_ink_x(&shaped).expect("shaped 路径应有墨迹");

    // 重复映射会让起点变成 (p−o)·s² + o = 40·0.64 = 25.6（偏左 14px）；
    // 正确结果是 40·0.8 = 32。允许 3px 的字形左侧 bearing 差异。
    let expected = (origin.x * 0.8).round() as u32;
    assert!(
        shaped_x.abs_diff(expected) <= 3,
        "shaped 文本起点应在 x≈{expected}（变换只应用一次），实际 {shaped_x}；\
         cell 路径参考 {cell_x}"
    );
}

/// 恒等变换下 shaped 与 cell 的起点也必须一致（回归护栏）。
#[test]
fn shaped_text_without_transform_starts_at_origin() {
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    let origin = Pos::new(40.0, 60.0);
    let mut s = backend();
    s.clear();
    s.draw_text(origin, "שלום", &style);
    let x = first_ink_x(&s).expect("应有墨迹");
    assert!(
        x.abs_diff(40) <= 3,
        "恒等变换下 shaped 文本应从 origin.x 附近开始，实际 {x}"
    );
}

/// 圆角填充的**半径必须跟随变换缩放**（与描边/渐变圆角一致）。
///
/// 旧实现只把矩形乘了缩放、半径没乘：缩放动画里填充的圆角比描边更大，
/// 角上会漏出一条彩色细边。
#[test]
fn round_rect_radius_scales_with_transform() {
    let r = Rect::new(20.0, 20.0, 200.0, 120.0);
    let color = Color::rgba(0.9, 0.2, 0.2, 1.0);

    // 参照：恒等变换下半径 40 的填充
    let mut a = backend();
    a.clear();
    a.fill_round_rect(r, color, 40.0);
    let plain = a.frame_hash();

    // 用 0.5 缩放绘制同样半径：半径若正确缩放（→20），结果应与
    // "恒等变换、半径 20、矩形减半"的绘制一致。
    let mut b = backend();
    b.clear();
    b.push_transform(Pos::new(0.0, 0.0), 0.5);
    b.fill_round_rect(r, color, 40.0);
    b.pop_transform();

    let mut c = backend();
    c.clear();
    c.fill_round_rect(Rect::new(10.0, 10.0, 100.0, 60.0), color, 20.0);
    let half = c.frame_hash();

    assert_ne!(plain, half, "两种半径应画出不同像素（否则用例无意义）");
    assert_eq!(
        b.frame_hash(),
        half,
        "缩放 0.5 时半径 40 必须等价于恒等变换下半径 20（半径要跟着变换缩放）"
    );
}
