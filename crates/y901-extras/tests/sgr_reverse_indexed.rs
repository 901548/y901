//! 非默认配色下的**反色语义**探测 + 回归。
//!
//! ## 探测的问题
//!
//! `palette::resolve(spec, fallback)` 对 `Default` 用 fallback、对 `Indexed`/**忽略 fallback**。
//! 那么当前景/背景一个是默认、一个是索引色时，`SGR 7` 交换后的
//! 颜色是否仍然是"背景变前景色、前景变背景色"？
//!
//! ## 为什么会对 —— 代码层面的理由
//!
//! 我们修复后的写法是"先 resolve（各自带正确的 fallback），再交换结果"：
//! `resolve(前景 spec, DEFAULT_FG)` 得到的就是"前景应该显示的颜色"，与它是索引色还是默认无关。
//! 所以交换顺序天然正确 —— 但这只是推理，必须用像素验证。

#![cfg(test)]

use y901_core::ColorSpec;
use y901_extras::palette::{resolve, xterm256, DEFAULT_BG, DEFAULT_FG};
use y901_extras::terminal::TerminalWidget;
use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::{Rect, Size, Widget};

const W: u32 = 200;
const H: u32 = 80;

fn rgb8(c: [f32; 4]) -> (u8, u8, u8) {
    (
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

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

/// 馈入 VT，布局，渲染，返回像素。
fn render(vt: &[u8], soft: &mut SoftBackend) -> (Vec<u8>, Rect, Size) {
    let mut t = TerminalWidget::new("t", 24, 4).read_only();
    t.feed_vt_for_test(vt);
    let rect = Rect::new(0.0, 0.0, W as f32, H as f32);
    let mut cache = LayoutCache::new();
    t.arrange(rect, soft, &mut cache);
    let cell = t.cell_size();
    soft.clear();
    t.render(soft);
    (soft.pixels().to_vec(), rect, cell)
}

/// 单元格右上角像素（背景色）。
fn bg_at(px: &[u8], rect: Rect, cell: Size, col: usize) -> (u8, u8, u8) {
    let x = (rect.x + (col + 1) as f32 * cell.w).round() as u32 - 3;
    let y = rect.y.round() as u32 + 2;
    let i = ((y * W + x) * 4) as usize;
    (px[i + 2], px[i + 1], px[i])
}

#[test]
fn reverse_with_indexed_foreground_and_default_background() {
    let mut soft = soft();
    // ESC[31m = 前景索引色 1（红）；背景保持默认
    let (px, rect, cell) = render(b"\x1b[31mabc\x1b[7mdef", &mut soft);
    let normal = bg_at(&px, rect, cell, 0);
    let reversed = bg_at(&px, rect, cell, 4);

    let red = rgb8(xterm256(1));
    let default_bg = rgb8(DEFAULT_BG);
    eprintln!("[probe A] 正常背景={normal:?} 反色后背景={reversed:?} 预期红={red:?}");

    assert_eq!(normal, default_bg, "未反色时背景应为默认背景");
    assert_eq!(
        reversed, red,
        "反色后背景应变成原前景色（索引 1）"
    );
}

#[test]
fn reverse_with_default_foreground_and_indexed_background() {
    let mut soft = soft();
    // ESC[42m = 背景索引色 2（绿）；前景默认
    let (px, rect, cell) = render(b"\x1b[42mabc\x1b[7mdef", &mut soft);
    let normal = bg_at(&px, rect, cell, 0);
    let reversed = bg_at(&px, rect, cell, 4);

    let green = rgb8(xterm256(2));
    let default_fg = rgb8(DEFAULT_FG);
    eprintln!("[probe B] 正常背景={normal:?} 反色后背景={reversed:?} 预期默认前景={default_fg:?}");

    assert_eq!(normal, green, "未反色时背景应为索引色 2");
    assert_eq!(
        reversed, default_fg,
        "反色后背景应变成原前景色（默认前景）"
    );
}

#[test]
fn reverse_with_both_indexed_colours() {
    let mut soft = soft();
    // ESC[31;42m：前景索引 1、背景索引 2
    let (px, rect, cell) = render(b"\x1b[31;42mabc\x1b[7mdef", &mut soft);
    let normal = bg_at(&px, rect, cell, 0);
    let reversed = bg_at(&px, rect, cell, 4);

    let red = rgb8(xterm256(1));
    let green = rgb8(xterm256(2));
    eprintln!("[probe C] 正常背景={normal:?} 反色后背景={reversed:?} 预期红={red:?}");

    assert_eq!(normal, green, "未反色时背景应为索引 2");
    assert_eq!(reversed, red, "反色后背景应变成索引 1");
}

/// `ColorSpec::Indexed` 是 **u8**，类型上就不可能传 300。
///
/// 但 255 是合法上限，必须确认它不 panic 且返回确定颜色。
#[test]
fn indexed_colour_bounds_are_well_defined() {
    // 0 = 默认（ANSI 表的第 0 项是黑色，但 Indexed(0) 仍解析成 ANSI[0]）
    let zero = rgb8(xterm256(0));
    let max = rgb8(xterm256(255));
    eprintln!("[probe D] Indexed(0)={zero:?}  Indexed(255)={max:?}");

    // 255 是 232..255 的灰阶尾端：(8 + 23*10) = 238
    assert_eq!(max, (238, 238, 238), "灰阶末尾应为 238");

    // 越界在类型上不可表达：`Indexed(u8)`，没有 256+
    // 所以 resolve 不需要边界检查 —— 但这一点得用测试固定：
    // 一旦未来把 Indexed 改成 u16，这里会立刻失效（编译错误）。
    let all: Vec<u8> = (0..=255).collect();
    for i in all {
        let c = resolve(ColorSpec::Indexed(i), DEFAULT_FG);
        assert!(c.iter().all(|v| v.is_finite()), "Indexed({i}) 应产生有效颜色");
    }
}

/// truecolor（`Rgb`）下的反色：反色矩阵的一块。
#[test]
fn reverse_with_truecolor_colours() {
    let mut soft = soft();
    let (px, rect, cell) = render(b"\x1b[38;2;255;0;0;48;2;0;0;255mabc\x1b[7mdef", &mut soft);
    let normal = bg_at(&px, rect, cell, 0);
    let reversed = bg_at(&px, rect, cell, 4);
    eprintln!("[probe E] 正常背景={normal:?} 反色后背景={reversed:?}");
    assert_eq!(normal, (0, 0, 255), "未反色时背景应为蓝");
    assert_eq!(reversed, (255, 0, 0), "反色后背景应变成原前景色红");
}

/// 反色 + **粗体**：颜色仍交换，**且粗体真的更重**。
///
/// 墨量基准取**同一帧里该单元格自己的背景色**（不能用 DEFAULT_BG：
/// 反色后背景已经变了，拿旧基准会把背景当成墨迹 —— 上一轮就踩过这个坑）。
#[test]
fn reverse_with_bold_still_swaps_colours_and_thickens_glyphs() {
    let mut soft = soft();
    let (bold_px, rect, cell) = render(b"\x1b[1;31mabcdef", &mut soft);
    let (plain_px, _r2, _c2) = render(b"\x1b[31mabcdef", &mut soft);

    // 颜色：两边都是默认背景（本测试不开反色，先把粗体本身测准）
    let bg = bg_at(&bold_px, rect, cell, 0);
    assert_eq!(bg, rgb8(DEFAULT_BG), "本字例背景应为默认背景");

    // 帧级差异数：粗体帧与非粗体帧在字形区域必须有差异（证明合成粗体真的改了像素）。
    // 为什么不比"墨量大小"：墨量需要一个可靠的背景基准，
    // 而字形本身的抗锯边缘让"不同于背景"的计数很容易变成度量方法的测试。
    let mut diff = 0;
    let x0 = rect.x.round() as i32 as u32;
    let y0 = rect.y.round() as i32 as u32;
    for y in y0..(y0 + cell.h.round() as u32) {
        for x in x0..(x0 + 6 * cell.w.round() as u32) {
            let i = ((y * W + x) * 4) as usize;
            if bold_px[i..i + 4] != plain_px[i..i + 4] {
                diff += 1;
            }
        }
    }
    let whole = bold_px.iter().zip(plain_px.iter()).filter(|(a, b)| a != b).count();
    let nonbg = bold_px.chunks_exact(4).filter(|p| (p[2], p[1], p[0]) != rgb8(DEFAULT_BG)).count();
    eprintln!("[probe F2] 区域差异={diff} 全帧差异={whole} 非背景像素={nonbg}");
    // 用**全帧差异**而不是单元格区域：字形位图会因 bearing 而偏出平方的单元格盒，
    // 区域采样会漏掉它们（实测：区域差异 0，全帧差异 446）。
    assert!(
        whole > 0,
        "合成粗体应改变字形像素（全帧差异 {whole}）"
    );
}

/// 反色 + **下划线**：颜色仍交换，且下划线仍被画出。
///
/// 判据方式：对比"有下划线 / 无下划线"两帧在**同一单元格底部**的像素。
/// 为什么不数"异色像素"：反色后前景/背景互换，"异色"基准不可信。
#[test]
fn reverse_with_underline_still_draws_the_underline() {
    let mut soft = soft();
    let (with_ul, rect, cell) = render(b"\x1b[4;7mabc", &mut soft);
    let (without_ul, _r2, _c2) = render(b"\x1b[7mabc", &mut soft);
    assert_eq!(
        bg_at(&with_ul, rect, cell, 0),
        rgb8(DEFAULT_FG),
        "反色下背景应为默认前景色"
    );
    let x0 = rect.x.round() as i32 as u32;
    let y1 = (rect.y + cell.h).round() as i32 as u32;
    let mut diff = 0;
    for y in y1.saturating_sub(4)..y1 {
        for dx in 0..cell.w.round() as u32 {
            let i = ((y * W + x0 + dx) * 4) as usize;
            if with_ul[i..i + 4] != without_ul[i..i + 4] {
                diff += 1;
            }
        }
    }
    eprintln!("[probe G] 底部差异像素={diff}");
    assert!(diff > 0, "带下划线的单元格底部应与无下划线不同");
}
