//! 变换栈的像素级验收：缩放后的矩形位置、尺寸与嵌套复合。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, Rect, RenderCtx, Size};

const W: u32 = 200;
const H: u32 = 200;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

/// 统计非背景像素的包围盒（`None` = 一个像素都没画）。
fn ink_bbox(soft: &SoftBackend, bg: u32) -> Option<(u32, u32, u32, u32)> {
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..H {
        for x in 0..W {
            let p = soft.pixel(x, y).expect("像素");
            if p == bg {
                continue;
            }
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    bbox
}

#[test]
fn scaled_rect_lands_at_half_size_around_origin() {
    let mut s = soft();
    s.clear();
    let bg = s.pixel(0, 0).expect("bg");

    // 以 (100,100) 为中心，缩放 0.5，画 (100,100,100,100) 的矩形
    s.push_transform(Pos::new(100.0, 100.0), 0.5);
    s.fill_rect(Rect::new(100.0, 100.0, 100.0, 100.0), Color::WHITE);
    s.pop_transform();

    let (x0, y0, x1, y1) = ink_bbox(&s, bg).expect("应有像素");
    eprintln!("[transform] 缩放 0.5 后的墨迹包围盒 = ({x0},{y0})..({x1},{y1})");

    // 期望：左上角 (100,100) 不动（它就是锚点），右下角 (200,200) → (150,150)
    assert!((x0 as i32 - 100).abs() <= 1, "左边应在 x=100，实际 {x0}");
    assert!((y0 as i32 - 100).abs() <= 1, "上边应在 y=100，实际 {y0}");
    assert!(
        (x1 as i32 - 150).abs() <= 1,
        "右边应在 x=150（半宽），实际 {x1}"
    );
    assert!(
        (y1 as i32 - 150).abs() <= 1,
        "下边应在 y=150（半高），实际 {y1}"
    );
}

#[test]
fn nested_transforms_multiply() {
    let mut s = soft();
    s.clear();
    let bg = s.pixel(0, 0).expect("bg");

    s.push_transform(Pos::new(0.0, 0.0), 0.5);
    s.push_transform(Pos::new(0.0, 0.0), 0.5);
    s.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE);
    s.pop_transform();
    s.pop_transform();

    let (x0, y0, x1, y1) = ink_bbox(&s, bg).expect("应有像素");
    eprintln!("[transform] 嵌套 0.5×0.5 的包围盒 = ({x0},{y0})..({x1},{y1})");
    // 100 * 0.5 * 0.5 = 25
    assert!(
        (x1 as i32 - 24).abs() <= 2,
        "嵌套后右边界应约 25，实际 {x1}"
    );
    assert!(
        (y1 as i32 - 24).abs() <= 2,
        "嵌套后下边界应约 25，实际 {y1}"
    );
    assert_eq!((x0, y0), (0, 0), "锚点在原点时左上角不动");
}

#[test]
fn pop_transform_restores_absolute_coordinates() {
    let mut s = soft();
    s.clear();
    let bg = s.pixel(0, 0).expect("bg");

    // 先缩放，再画一个"外面"的矩形：pop 之后的绘制必须回到绝对坐标
    s.push_transform(Pos::new(0.0, 0.0), 0.5);
    s.pop_transform();
    s.fill_rect(Rect::new(50.0, 50.0, 20.0, 20.0), Color::WHITE);

    let (x0, y0, x1, y1) = ink_bbox(&s, bg).expect("应有像素");
    assert_eq!((x0, y0), (50, 50), "pop 后应回到绝对坐标");
    assert_eq!((x1, y1), (69, 69), "尺寸不应被缩放");
}

#[test]
fn degenerate_scale_is_ignored_not_crashing() {
    let mut s = soft();
    s.clear();
    let bg = s.pixel(0, 0).expect("bg");
    // scale = 0 / 负数 / NaN 都应被忽略（否则矩形退化成 0 或反向 → 光栅化越界）
    for bad in [0.0f32, -1.0, f32::NAN] {
        s.push_transform(Pos::new(10.0, 10.0), bad);
        s.fill_rect(Rect::new(10.0, 10.0, 40.0, 40.0), Color::WHITE);
        s.pop_transform();
    }
    let (x0, y0, x1, y1) = ink_bbox(&s, bg).expect("应有像素");
    assert_eq!(
        (x0, y0, x1, y1),
        (10, 10, 49, 49),
        "非法缩放应被忽略，按原坐标画"
    );
}
