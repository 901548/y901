//! `SGR 7`（反色）的**独立**像素回读断言。
//!
//! ## 为什么要独立一条
//!
//! 之前 `SGR 7` 一直是**失效的**（先交换 spec 再解析 → 两边解析结果相同）。
//! 选区反色的测试只能间接证明"同一条 swap 路径”没问题，不能证明 `ESC[7m` 自己对。
//! 所以这里**直接断言具体 RGB**（不是"变了没有"）——
//! 否则"变成红色"这种错误也能通过。

#![cfg(test)]

use y901_extras::palette::{resolve, DEFAULT_BG, DEFAULT_FG};
use y901_extras::terminal::TerminalWidget;
use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::{Rect, Size, Widget};

const W: u32 = 200;
const H: u32 = 80;

/// 把 `[f32;4]` 颜色转成 `SoftBackend` 存储的 `0xAARRGGBB`（只取 RGB）。
fn rgb_of(c: [f32; 4]) -> (u8, u8, u8) {
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

/// 建一个已馈入 VT 字节并完成布局的终端。
fn terminal(vt: &[u8], soft: &mut SoftBackend) -> (TerminalWidget, Rect, Size) {
    let mut t = TerminalWidget::new("t", 24, 4).read_only();
    t.feed_vt_for_test(vt);
    let rect = Rect::new(0.0, 0.0, W as f32, H as f32);
    let mut cache = LayoutCache::new();
    t.arrange(rect, soft, &mut cache);
    let cell = t.cell_size();
    (t, rect, cell)
}

/// 取某单元格**右上角**的像素（避开字形与下伸部）。
fn corner_px(px: &[u8], rect: Rect, cell: Size, row: usize, col: usize) -> (u8, u8, u8) {
    let x = (rect.x + (col + 1) as f32 * cell.w).round() as u32 - 3;
    let y = (rect.y + row as f32 * cell.h).round() as u32 + 2;
    let i = ((y * W + x) * 4) as usize;
    (px[i + 2], px[i + 1], px[i]) // BGRA → RGB
}

#[test]
fn sgr7_swaps_default_colours_with_exact_rgb() {
    let mut soft = soft();
    // "abc" 默认配色；然后 ESC[7m 进入反色，"def" 应反色
    let (mut t, rect, cell) = terminal(b"abc\x1b[7mdef", &mut soft);
    soft.clear();
    t.render(&mut soft);
    let px = soft.pixels().to_vec();

    let normal = rgb_of(resolve(y901_core::ColorSpec::Default, DEFAULT_BG)); // 默认背景
    let inverted = rgb_of(resolve(y901_core::ColorSpec::Default, DEFAULT_FG)); // 默认前景

    // "abc" 保持默认背景
    for col in 0..3 {
        assert_eq!(
            corner_px(&px, rect, cell, 0, col),
            normal,
            "字符 {col} 应保持默认背景 {normal:?}"
        );
    }
    // "def" 背景应变成默认前景色
    for col in 3..6 {
        assert_eq!(
            corner_px(&px, rect, cell, 0, col),
            inverted,
            "字符 {col} 应反色为默认前景 {inverted:?}"
        );
    }
    assert_ne!(normal, inverted, "默认前景与背景不能相同");
    eprintln!("[sgr7] 正常={normal:?} 反色={inverted:?}");
}

#[test]
fn sgr27_restores_default_colours() {
    let mut soft = soft();
    // ESC[7m "def" ESC[27m "ghi"
    let (mut t, rect, cell) = terminal(b"\x1b[7mdef\x1b[27mghi", &mut soft);
    soft.clear();
    t.render(&mut soft);
    let px = soft.pixels().to_vec();

    let normal = rgb_of(resolve(y901_core::ColorSpec::Default, DEFAULT_BG));
    let inverted = rgb_of(resolve(y901_core::ColorSpec::Default, DEFAULT_FG));

    for col in 0..3 {
        assert_eq!(
            corner_px(&px, rect, cell, 0, col),
            inverted,
            "ESC[7m 之后的字符 {col} 应反色"
        );
    }
    for col in 3..6 {
        assert_eq!(
            corner_px(&px, rect, cell, 0, col),
            normal,
            "ESC[27m 之后的字符 {col} 应恢复正常"
        );
    }
    eprintln!("[sgr27] 恢复正确");
}

#[test]
fn selection_on_top_of_sgr7_cancels_out() {
    let mut soft = soft();
    let (mut t, rect, cell) = terminal(b"\x1b[7mabc", &mut soft);

    // 选中 abc（屏幕坐标→单元格走真实路径）
    assert!(t.begin_selection(y901_ui::Pos::new(
        rect.x + 0.5 * cell.w,
        rect.y + 0.5 * cell.h
    )));
    assert!(t.drag_selection_to(y901_ui::Pos::new(
        rect.x + 2.5 * cell.w,
        rect.y + 0.5 * cell.h
    )));

    soft.clear();
    t.render(&mut soft);
    let px = soft.pixels().to_vec();

    let normal = rgb_of(resolve(y901_core::ColorSpec::Default, DEFAULT_BG));
    // SGR 7 反转一次 + 选区反转一次 = 两次 swap = 恢复原色
    for col in 0..3 {
        assert_eq!(
            corner_px(&px, rect, cell, 0, col),
            normal,
            "反色与选区叠加应双重反转回默认背景（字符 {col}）"
        );
    }
    eprintln!("[sgr7+选区] 双重反转 = 原色，符合预期");
}
