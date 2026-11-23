//! **混排双向文本必须画在行内、且不重叠**（feature `shaping`）。
//!
//! 旧实现用"负 pen + 事后整体平移"模拟 RTL 对齐，只有一个文字段时凑巧正确；
//! 一旦 LTR 段落里夹了 RTL 段（`"hello שלום"`），`place_run` 会拿到
//! `rtl_base = pen` 并把该段翻到 `pen - width` —— **画到英文前面**、与英文重叠。
//!
//! 现在先全部 shape（拿到每段宽度），再按视觉顺序（RTL 段落整体反向）逐段摆放，
//! RTL 段传"该段右边界"。断言两条：
//! 1. 没有任何像素画在行首左侧（旧实现会溢出到 `origin.x` 左边）；
//! 2. 所有像素都在行宽范围内（不越界）。

#![cfg(feature = "shaping")]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, RenderCtx, Size, TextStyle};

const W: u32 = 400;
const H: u32 = 60;

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        16.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

/// 返回最左/最右被画到的列（没有像素时返回 `None`）。
fn painted_span(s: &SoftBackend) -> Option<(u32, u32)> {
    let p = s.pixels();
    let mut min_x = u32::MAX;
    let mut max_x = 0u32;
    let mut any = false;
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            // 背景是深色；任何明显更亮的像素都算"画上了"
            if p[i] > 32 || p[i + 1] > 32 || p[i + 2] > 32 {
                any = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
            }
        }
    }
    any.then_some((min_x, max_x))
}

fn draw(text: &str, origin_x: f32) -> SoftBackend {
    let mut s = backend();
    s.clear();
    let style = TextStyle::fg(Color::WHITE).size(16.0);
    s.draw_text(Pos::new(origin_x, 8.0), text, &style);
    s
}

/// **关键回归**：混排时 RTL 段必须排在 LTR 段**之后**（不重叠）。
///
/// 判据用"宽度可加"：分别量出拉丁段与希伯来段各自的墨迹宽度，
/// 混排那一行的最右像素应接近"拉丁段末端 + 希伯来段宽度"。
/// 旧实现把 RTL 段翻到 `pen - width`（英文前面），最右像素只到拉丁段末端 ——
/// 差一个整段宽度，这条断言抓得住。
#[test]
fn mixed_direction_runs_do_not_overlap() {
    let latin = draw("hello ", 40.0);
    let (_, latin_max) = painted_span(&latin).expect("拉丁段应有像素");
    let hebrew = draw("שלום", 40.0);
    let (heb_min, heb_max) = painted_span(&hebrew).expect("希伯来段应有像素");
    let heb_w = (heb_max - heb_min) as f32;

    let mixed = draw("hello שלום", 40.0);
    let (min_x, max_x) = painted_span(&mixed).expect("混排应有像素");

    assert!(min_x >= 40, "混排不应画到行首左侧（min_x={min_x}）");
    assert!(
        (max_x as f32) >= (latin_max as f32) + heb_w * 0.7,
        "RTL 段必须排在拉丁段之后：混排最右像素 {max_x}，\
         拉丁段末端 {latin_max}，希伯来段宽度约 {heb_w} —— \
         旧实现会把它翻到英文前面重叠，最右只到 {latin_max}"
    );
}

#[test]
fn rtl_paragraph_stays_inside_the_line() {
    let s = draw("שלום hello", 40.0);
    let (min_x, _) = painted_span(&s).expect("应该有像素");
    assert!(min_x >= 40, "RTL 段落画到了行首左侧（min_x={min_x} < 40）");
}

#[test]
fn shaped_text_does_not_paint_outside_the_viewport_width() {
    let s = draw("hello שלום 123", 10.0);
    let (_, max_x) = painted_span(&s).expect("应该有像素");
    assert!(max_x < W, "不应越右边界（max_x={max_x}）");
}

/// 纯拉丁文本：整行像素应落在"起点 → 起点 + 一段合理宽度"内（回归基准）。
#[test]
fn latin_text_is_unaffected() {
    let s = draw("hello world", 20.0);
    let (min_x, max_x) = painted_span(&s).expect("应该有像素");
    // 允许 1~2px 的左侧 bearing（'h' 的左边缘不在原点正上方）
    assert!(
        (20..=22).contains(&min_x),
        "拉丁文本应从起点附近开始画（min_x={min_x}）"
    );
    assert!(max_x < 20 + 160, "拉丁文本宽度应合理（max_x={max_x}）");
}
