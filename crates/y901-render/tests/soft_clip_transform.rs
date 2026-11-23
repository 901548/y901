//! 软光栅器的三个坐标系/合成不变量（回归用，纯 CPU、跨平台）。
//!
//! 这三条都是"画出来的像素与语义不符"，肉眼在忙碌界面里几乎看不出来，
//! 但都能用 32×32 的离屏缓冲精确断言：
//!
//! 1. **负坐标矩形必须被裁剪，而不是平移**：`x=-8, w=20` 只该画 `0..12`；
//!    旧实现把左上角钳到 0 却保留原宽，于是被裁掉的部分补到了右边。
//! 2. **图片必须服从透明栈**：`push_opacity(0.2)` 下图片要淡出，
//!    与同色 `fill_rect` 一致（D3D 后端对每个实例都乘 opacity）。
//! 3. **渐变圆角矩形必须过变换栈**：缩放 0.5 时它要和 `fill_round_rect` 覆盖同一区域，
//!    否则弹层打开动画里渐变面板会保持原尺寸、与子节点错位。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::image::ImageStyle;
use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextStyle};

const W: u32 = 32;
const H: u32 = 32;

fn backend() -> SoftBackend {
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

/// 取 (x, y) 的 RGB。
fn rgb(s: &SoftBackend, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * W + x) * 4) as usize;
    let p = s.pixels();
    (p[i + 2], p[i + 1], p[i])
}

fn clear(s: &mut SoftBackend) {
    s.clear();
}

#[test]
fn negative_rect_is_clipped_not_shifted() {
    let mut s = backend();
    clear(&mut s);
    let bg = rgb(&s, 0, 5);

    // x 从 -8 开始、宽 20：可见部分只有 0..12
    s.fill_rect(Rect::new(-8.0, 0.0, 20.0, 10.0), Color::WHITE);

    assert_eq!(rgb(&s, 0, 5), (255, 255, 255), "可见部分必须画上");
    assert_eq!(rgb(&s, 11, 5), (255, 255, 255), "x=11 仍在可见范围内");
    assert_eq!(
        rgb(&s, 13, 5),
        bg,
        "x=13 不该被画：旧实现把 -8 钳到 0 却保留宽度，会在这里画出多余像素"
    );
    assert_eq!(rgb(&s, 19, 5), bg, "右侧 8 像素都不该被画");
}

#[test]
fn fully_offscreen_rect_paints_nothing() {
    let mut s = backend();
    clear(&mut s);
    let bg = rgb(&s, 5, 5);
    s.fill_rect(Rect::new(-30.0, 0.0, 10.0, 10.0), Color::WHITE);
    assert_eq!(rgb(&s, 0, 5), bg, "完全在屏幕外的矩形不应画出任何像素");
}

#[test]
fn draw_image_respects_the_opacity_stack() {
    let mut s = backend();
    clear(&mut s);
    // 4×4 纯白图片
    let white = [255u8, 255, 255, 255].repeat(16);
    let id = s.upload_texture(4, 4, &white);

    // 对照组：全不透明的纯色填充
    s.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), Color::WHITE);
    let opaque_fill = rgb(&s, 4, 4);

    clear(&mut s);
    s.push_opacity(0.2);
    s.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), Color::WHITE);
    s.pop_opacity();
    let translucent_fill = rgb(&s, 4, 4);

    clear(&mut s);
    s.push_opacity(0.2);
    s.draw_image(Rect::new(0.0, 0.0, 8.0, 8.0), id, ImageStyle::default());
    s.pop_opacity();
    let translucent_image = rgb(&s, 4, 4);

    assert_ne!(
        translucent_fill, opaque_fill,
        "前置：透明栈确实起作用（fill_rect 变淡）"
    );
    assert_eq!(
        translucent_image, translucent_fill,
        "同一透明度下图片应与纯色填充一致；旧实现图片被写死为不透明"
    );
}

#[test]
fn gradient_round_rect_follows_the_transform_stack() {
    let mut s = backend();
    clear(&mut s);

    // 缩放 0.5 + 原点 (8,8)：rect(8,8,16,16) 覆盖 8..16，而不是 8..24
    s.push_transform(Pos::new(8.0, 8.0), 0.5);
    s.fill_gradient_round_rect(
        Rect::new(8.0, 8.0, 16.0, 16.0),
        Color::WHITE,
        Color::WHITE,
        (0.0, 1.0),
        4.0,
    );
    s.pop_transform();

    assert_eq!(rgb(&s, 10, 10), (255, 255, 255), "缩放后矩形内部应被画上");
    assert_ne!(
        rgb(&s, 20, 20),
        (255, 255, 255),
        "x=20 在缩放后的矩形之外（8..16）；若被画上说明没走变换栈"
    );

    // 对照：**不缩放**时同一矩形要覆盖到 8..24（证明上面挡住的是缩放，不是矩形本身）
    let mut s2 = backend();
    clear(&mut s2);
    s2.fill_gradient_round_rect(
        Rect::new(8.0, 8.0, 16.0, 16.0),
        Color::WHITE,
        Color::WHITE,
        (0.0, 1.0),
        4.0,
    );
    assert_eq!(
        rgb(&s2, 20, 20),
        (255, 255, 255),
        "对照：不缩放时 (20,20) 应在矩形内部"
    );
}

/// 字体度量占位：`SoftBackend` 自己实现了 `TextMetrics`，这里只是让编译器闭嘴。
#[allow(dead_code)]
fn _assert_style(_s: &TextStyle) {}

/// **文本背景不得二次映射**：`origin` 已在设备空间，逐字推进的背景块必须
/// 与字形落在同一格，而不是"第二格起偏移一半、宽度也缩一半"。
///
/// 用**两个空格**做样本：空格不画字形，于是这些像素只可能来自背景块，
/// 不会被字形颜色混淆（用 "aa" 时字形本身盖住了采样点，测试会失去分辨力）。
#[test]
fn text_background_is_not_mapped_twice() {
    let mut s = backend();
    clear(&mut s);
    let bg = rgb(&s, 31, 31);

    // 缩放 0.5 + 原点 (8,8)：cell=8px（size 16）→ 缩放后每格宽 4
    // 第一格背景 = 8..12，第二格 = 12..16（都在设备空间）
    let style = TextStyle::default().with_bg(Color::WHITE).size(16.0);
    s.push_transform(Pos::new(8.0, 8.0), 0.5);
    s.draw_text(Pos::new(8.0, 8.0), "  ", &style);
    s.pop_transform();

    assert_eq!(rgb(&s, 9, 9), (255, 255, 255), "第一格背景应在 8..12");
    assert_eq!(
        rgb(&s, 14, 9),
        (255, 255, 255),
        "第二格背景应在 12..16；二次映射会把它挪到 10..12，这里就空了"
    );
    assert_eq!(
        rgb(&s, 17, 9),
        bg,
        "x=17 在两格背景之外；若被画上说明背景块宽度被放大"
    );
}
/// **回归**：`push_transform` / `push_opacity` 泄漏不能跨帧累积。
///
/// 旧实现每帧只清裁剪栈。任何一个控件漏了 `pop_transform`（或 render 中途 panic），
/// 之后**每一帧**都会带着残留缩放绘制 —— 表现为"界面整体变小"，且没有任何报错。
#[test]
fn leaked_transform_and_opacity_do_not_survive_a_frame_boundary() {
    let mut s = backend();
    clear(&mut s);
    // 故意泄漏：压栈不弹
    s.push_transform(Pos::new(0.0, 0.0), 0.5);
    s.push_opacity(0.2);

    // 下一帧开始（clear 相当于 frame 边界）
    clear(&mut s);
    s.fill_rect(Rect::new(0.0, 0.0, 8.0, 8.0), Color::WHITE);

    assert_eq!(
        rgb(&s, 4, 4),
        (255, 255, 255),
        "泄漏的透明度不应影响下一帧（旧实现会变暗）"
    );
    assert_eq!(
        rgb(&s, 6, 6),
        (255, 255, 255),
        "泄漏的缩放不应影响下一帧（旧实现只会画 4x4）"
    );
}
