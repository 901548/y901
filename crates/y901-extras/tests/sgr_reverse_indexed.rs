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
