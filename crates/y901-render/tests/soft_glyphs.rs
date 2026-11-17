//! SoftBackend 的字形渲染回归测试。
//!
//! 起因有两件（都真实发生过）：
//! 1. 一次误判——我以为 SoftBackend 不画字，实际是**默认颜色透明**加上**测试字体没有中文字形**；
//! 2. **行序约定不一致**——光栅化的覆盖率原本是“自下而上”（字体轮廓坐标 y 向上），
//!    而绘制方按“第 0 行 = 顶部”逐行贴下去。结果每个字形都按**自身高度**整体错位：
//!    一行里高的字低、矮的字高（`L` 与 `o` 的底边差 5px），整行还压到下一行
//!    （“字距发紧”）。本文件逐像素钉住这条约定。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::{rasterize_glyph, FontEngine, MyFontEngine};
use y901_ui::{Color, Pos, RenderCtx, Size, TextStyle};

fn font_bytes() -> Vec<u8> {
    y901_render::load_font(&y901_render::default_test_font_path()).expect("font")
}

fn engine() -> (MyFontEngine, Size) {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let e = MyFontEngine::new(font_bytes(), 15.0, cell).expect("engine");
    (e, cell)
}

fn canvas(px: f32) -> (SoftBackend, Size) {
    let (cw, ch) = y901_render::cell_metrics_for(px);
    let cell = Size::new(cw, ch);
    let e = MyFontEngine::new(font_bytes(), px, cell).expect("engine");
    let s = SoftBackend::new_headless(200, 90, Box::new(e), cell).expect("soft");
    (s, cell)
}

/// 整块画布的墨迹包围盒 `(x0, y0, x1, y1)`（左上角原点，y 向下）。
fn ink_bbox(s: &SoftBackend) -> Option<(u32, u32, u32, u32)> {
    let bg = s.pixel(0, s.height() - 1).expect("bg");
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..s.height() {
        for x in 0..s.width() {
            if s.pixel(x, y).expect("px") != bg {
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    bbox
}

fn ink(s: &SoftBackend) -> usize {
    let bg = s.pixel(0, s.height() - 1).expect("bg");
    let mut n = 0;
    for y in 0..s.height() {
        for x in 0..s.width() {
            if s.pixel(x, y).expect("px") != bg {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn latin_glyph_really_renders() {
    let (mut s, _cell) = canvas(15.0);
    s.clear();
    s.draw_text(Pos::new(10.0, 10.0), "A", &TextStyle::fg(Color::WHITE));
    let n = ink(&s);
    eprintln!("[glyph] 'A' 墨迹像素 = {n}");
    assert!(n > 20, "拉丁字母应有明显墨迹，实测 {n}");
}

#[test]
fn default_text_style_is_visible() {
    // 默认样式若前景透明，文字就"画了但看不见"——这是真实发生过的坑。
    let (mut s, _cell) = canvas(15.0);
    s.clear();
    s.draw_text(Pos::new(10.0, 10.0), "A", &TextStyle::default());
    let n = ink(&s);
    eprintln!("[glyph] 默认样式 'A' 墨迹像素 = {n}");
    assert!(n > 20, "TextStyle::default() 应可见，实测 {n}");
}

/// `GlyphBitmap::coverage` 必须是**自上而下**的行序。
///
/// 用 `'L'` 判定：自上而下时第 0 行只有竖笔（窄），最后一行是横脚（宽）。
/// 自下而上时正好相反——这正是“字高低不齐”的根源。
#[test]
fn glyph_bitmap_rows_start_at_the_top() {
    let b = rasterize_glyph(&font_bytes(), 'L', 40.0).expect("L bitmap");
    let row_width = |y: u32| (0..b.width).filter(|&x| b.at(x, y) > 0).count();
    let first = (0..b.height).find(|&y| row_width(y) > 0).expect("有墨");
    let last = (0..b.height)
        .rev()
        .find(|&y| row_width(y) > 0)
        .expect("有墨");
    let top = row_width(first);
    let bottom = row_width(last);
    eprintln!(
        "[glyph] 'L'@40 {}x{}: 首行墨宽={}, 末行墨宽={}",
        b.width, b.height, top, bottom
    );
    assert!(
        bottom > top * 3,
        "'L' 末行应是横脚（宽 ≫ 竖笔），实测 首={top} 末={bottom}；行序反了"
    );

    // `'T'` 是反方向的同一个判据：第一行是**宽**的横杠，末行是窄的竖笔。
    let t = rasterize_glyph(&font_bytes(), 'T', 40.0).expect("T bitmap");
    let row_width = |y: u32| (0..t.width).filter(|&x| t.at(x, y) > 0).count();
    let first = (0..t.height).find(|&y| row_width(y) > 0).expect("有墨");
    let last = (0..t.height)
        .rev()
        .find(|&y| row_width(y) > 0)
        .expect("有墨");
    assert!(
        row_width(first) > row_width(last) * 3,
        "'T' 首行应是横杠、末行应是竖笔，实测 首={} 末={}；行序反了",
        row_width(first),
        row_width(last)
    );
}

/// 一行里**不同高度的字形必须共用同一条基线**。
///
/// 这是用户报的“有的高有的低”：`L`（高）、`o`（矮）、`8`（数字）底边必须对齐；
/// 有下伸的 `g` 才允许更低。旧行序下 `o` 的底边比 `L` 高 5px。
#[test]
fn glyphs_of_different_heights_share_one_baseline() {
    let mut bottoms = Vec::new();
    for ch in ['L', 'o', '8', 'H'] {
        let (mut s, _cell) = canvas(16.0);
        s.clear();
        s.draw_text(Pos::new(8.0, 8.0), &ch.to_string(), &TextStyle::fg(Color::WHITE));
        let (_, _, _, y1) = ink_bbox(&s).unwrap_or_else(|| panic!("{ch:?} 应有墨迹"));
        eprintln!("[glyph] {ch:?} 墨迹底边 y = {y1}");
        bottoms.push((ch, y1));
    }
    let base = bottoms[0].1;
    for (ch, y1) in &bottoms {
        assert!(
            (*y1 as i32 - base as i32).abs() <= 1,
            "{ch:?} 的底边 {y1} 与基线 {base} 相差超过 1px（字高低不齐）"
        );
    }

    // 下伸字形必须**低于**基线，否则说明 bearing 被抹平了。
    let (mut s, _cell) = canvas(16.0);
    s.clear();
    s.draw_text(Pos::new(8.0, 8.0), "g", &TextStyle::fg(Color::WHITE));
    let (_, _, _, gy1) = ink_bbox(&s).expect("g 应有墨迹");
    assert!(
        gy1 > base,
        "'g' 的下伸应低于基线：g={gy1}, 基线={base}"
    );
}

/// 字形必须落在自己的行盒里（不整体下移一个行高）。
#[test]
fn text_stays_inside_its_line_box() {
    let px = 16.0;
    let origin_y = 20.0;
    let (mut s, cell) = canvas(px);
    s.clear();
    // `cell` 是行盒高度；字形的墨迹不得越过盒子的底边太多（下伸除外）。
    let _ = cell;
    s.draw_text(
        Pos::new(8.0, origin_y),
        "Lo8",
        &TextStyle::fg(Color::WHITE),
    );
    let (_, y0, _, y1) = ink_bbox(&s).expect("应有墨迹");
    let box_bottom = origin_y + px;
    eprintln!("[glyph] 行盒 y {origin_y}..{box_bottom}，墨迹 y {y0}..{y1}");
    assert!(
        y0 as f32 >= origin_y - 2.0,
        "墨迹顶端 {y0} 不应高于行盒顶端 {origin_y}"
    );
    assert!(
        (y1 as f32) <= box_bottom + 2.0,
        "墨迹底端 {y1} 溢出行盒底端 {box_bottom}（整体下移了一个行高）"
    );
}

#[test]
fn proportional_advances_preserve_font_spacing() {
    let (e, cell) = engine();
    let narrow = e.advance('i', 15.0);
    let wide = e.advance('m', 15.0);
    assert!(
        narrow > 0.0 && wide > narrow,
        "expected hmtx advances i={narrow}, m={wide}"
    );

    let mut s = SoftBackend::new_headless(200, 60, Box::new(e), cell).expect("soft");
    s.set_proportional_text(true);
    s.clear();
    let width = s.draw_text(Pos::new(10.0, 10.0), "im", &TextStyle::fg(Color::WHITE));
    assert!(
        (width - (narrow + wide)).abs() < 0.01,
        "drawn width {width} != font advances"
    );
}

#[test]
fn latin_test_font_has_no_cjk_glyphs() {
    // 测试字体是 segoeui.ttf（拉丁）：中文字形取不到。
    // 这是**字体限制**，不是渲染器缺陷：中文界面应选中文字体（如 msyh/simsun）。
    let (mut e, _cell) = engine();
    assert!(
        e.glyph('父').is_none(),
        "如果这里变成 Some，说明测试字体已换成支持中文的，请更新本测试"
    );
    assert!(e.glyph('A').is_some(), "拉丁字母应存在");
}
