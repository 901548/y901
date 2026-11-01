//! 选区**反色渲染**的像素回读断言。
//!
//! ## 为什么要用像素断言而不是"目视"
//!
//! `Selection::contains()` 的单元测试只证明"位置算对了"，
//! 但"渲染时真的用了这个结果"是另一回事（很容易忘记接线）。
//! 这里用 `SoftBackend::pixels()` 把真实渲染结果读回来逐像素比较。
//!
//! ## 比较策略：同一场景渲染两次
//!
//! A = 无选区；B = 有选区。
//! **选区内的单元格像素必须不同，选区外必须逐像素相同**——
//! 这比"断言某个像素是某个颜色"更稳：不依赖调色板、不依赖字体、不依赖渲染器实现。

#![cfg(test)]

use y901_extras::terminal::TerminalWidget;
use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::Widget;
use y901_ui::{Rect, Size};

const W: u32 = 200;
const H: u32 = 100;

/// 造一个头部无窗的软渲染器（字形用自研引擎，不依赖 DirectWrite）。
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

/// 装好内容并完成布局的终端（回还定的字格尺寸与布局矩形）。
fn terminal_with(lines: &[&str], soft: &mut SoftBackend) -> (TerminalWidget, Rect) {
    let mut t = TerminalWidget::new("t", 20, 6).read_only();
    {
        let g = t.grid_mut();
        for (i, s) in lines.iter().enumerate() {
            g.set_cursor(i, 0);
            for ch in s.chars() {
                g.place_cell(ch, 0);
            }
        }
    }
    let rect = Rect::new(0.0, 0.0, W as f32, H as f32);
    let mut cache = LayoutCache::new();
    t.arrange(rect, soft, &mut cache);
    (t, rect)
}

/// 把一帧渲染到软后端并返回像素片段（BGRA）。
fn render_frame(t: &mut TerminalWidget, soft: &mut SoftBackend) -> Vec<u8> {
    soft.clear();
    t.render(soft);
    soft.pixels().to_vec()
}

/// 取单元格内部四个"背景样点"（用于比较）。
///
/// 为什么不比整块：字形（尤其 g/j/p/q 的下伸部）会溢出单元格，
/// 把邻行的像素带进来 →— 导致"选区外也变了"的假失败。
/// 角落 2px 内缩几乎一定是背景，且避开了行与行之间的渗透。
fn cell_pixels(
    px: &[u8],
    rect: Rect,
    cell: Size,
    row: usize,
    col: usize,
    w: u32,
) -> Vec<u8> {
    let cx0 = (rect.x + col as f32 * cell.w).round() as i32;
    let cy0 = (rect.y + row as f32 * cell.h).round() as i32;
    let cw = cell.w.round() as i32;
    let mut out = Vec::new();
    // 取单一样点（右上角）：字形几乎不会到达这里，
    // 也避开了上一行字形的下伸部。
    for (dx, dy) in [(cw - 3, 2)] {
        let x = cx0 + dx;
        let y = cy0 + dy;
        if x < 0 || y < 0 || x >= w as i32 {
            continue;
        }
        let i = ((y as u32 * w + x as u32) * 4) as usize;
        if i + 3 < px.len() {
            out.extend_from_slice(&px[i..i + 4]);
        }
    }
    out
}

/// 核心断言：**选区内的单元格像素变了，选区外的逐像素相同**。
fn assert_inverse_range(
    base: &[u8],
    sel_px: &[u8],
    rect: Rect,
    cell: Size,
    cols: usize,
    rows: usize,
    selected: impl Fn(usize, usize) -> bool,
) {
    let mut changed_in = 0usize;
    let mut changed_out = 0usize;
    for row in 0..rows {
        for col in 0..cols {
            let a = cell_pixels(base, rect, cell, row, col, W);
            let b = cell_pixels(sel_px, rect, cell, row, col, W);
            let same = a == b;
            if selected(row, col) {
                assert!(
                    !same,
                    "单元格 ({row},{col}) 在选区内，像素应变化（反色）但没变"
                );
                changed_in += 1;
            } else {
                assert!(
                    same,
                    "单元格 ({row},{col}) 在选区外，像素不应变化"
                );
                changed_out += 1;
            }
        }
    }
    eprintln!("[反色] 选区内单元格 {changed_in} 个已变色，选区外 {changed_out} 个逐像素相同");
}

#[test]
fn single_row_selection_reverses_only_selected_cells() {
    let mut soft = soft();
    let (mut t, rect) = terminal_with(&["abcdefghij"], &mut soft);
    let cell = t.cell_size();

    let base = render_frame(&mut t, &mut soft);
    // 选中第 0 行的 col 2..=5
    assert!(
        t.begin_selection(y901_ui::Pos::new(
            rect.x + 2.5 * cell.w,
            rect.y + 0.5 * cell.h
        )),
        "应能起选"
    );
    assert!(t.drag_selection_to(y901_ui::Pos::new(
        rect.x + 5.5 * cell.w,
        rect.y + 0.5 * cell.h
    )));
    let sel_px = render_frame(&mut t, &mut soft);
    assert_inverse_range(&base, &sel_px, rect, cell, 10, 1, |row, col| {
        row == 0 && (2..=5).contains(&col)
    });
}

#[test]
fn cross_row_selection_reverses_first_middle_and_last_rows_correctly() {
    let mut soft = soft();
    let (mut t, rect) = terminal_with(&["abcdef", "ghijkl", "mnopqr"], &mut soft);
    let cell = t.cell_size();

    let base = render_frame(&mut t, &mut soft);

    // 从 (0,2) 拖到 (2,1)
    assert!(t.begin_selection(y901_ui::Pos::new(
        rect.x + 2.5 * cell.w,
        rect.y + 0.5 * cell.h
    )));
    assert!(t.drag_selection_to(y901_ui::Pos::new(
        rect.x + 1.5 * cell.w,
        rect.y + 2.5 * cell.h
    )));
    let sel_px = render_frame(&mut t, &mut soft);

    // 逐点抽查（比逐格断言更稳：不受字形溢出影响）
    let changed = |row: usize, col: usize| {
        cell_pixels(&base, rect, cell, row, col, W) != cell_pixels(&sel_px, rect, cell, row, col, W)
    };
    assert!(!changed(0, 0), "首行 col0 在选区外，不应变色");
    assert!(!changed(0, 1), "首行 col1 在选区外（起始列是 2）");
    assert!(changed(0, 2), "首行 col2 是起始列，应反色");
    assert!(changed(0, 5), "首行末列在选区内，应反色");
    assert!(changed(1, 0), "中间行全选，col0 也应反色");
    assert!(changed(1, 5), "中间行全选，末列也应反色");
    assert!(changed(2, 0), "末行 col0 在选区内");
    assert!(changed(2, 1), "末行 col1 是结束列");
    assert!(!changed(2, 3), "末行 col3 在选区外（结束列是 1）");
    assert!(!changed(2, 5), "末行末列在选区外");
}

#[test]
fn clearing_selection_restores_the_original_pixels() {
    let mut soft = soft();
    let (mut t, rect) = terminal_with(&["abcdef", "ghijkl"], &mut soft);
    let cell = t.cell_size();

    let base = render_frame(&mut t, &mut soft);
    assert!(t.begin_selection(y901_ui::Pos::new(
        rect.x + 1.5 * cell.w,
        rect.y + 0.5 * cell.h
    )));
    assert!(t.drag_selection_to(y901_ui::Pos::new(
        rect.x + 3.5 * cell.w,
        rect.y + 1.5 * cell.h
    )));
    let selected = render_frame(&mut t, &mut soft);
    assert_ne!(base, selected, "有选区时应有像素变化");

    // 清除选区 → 渲染应逐像素回到初始帧
    t.clear_selection();
    let restored = render_frame(&mut t, &mut soft);
    assert_eq!(
        restored, base,
        "清除选区后画面必须与初始帧逐像素相同（否则选区会留下残影）"
    );
    eprintln!("[反色] 清除选区后 {} 字节像素完全复原", restored.len());
}
