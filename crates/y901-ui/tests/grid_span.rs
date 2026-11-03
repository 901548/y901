//! Grid 跨格：**裁剪**与**自动流落位**的端到端测试（第 3 步）。
//!
//! 这两个性质只有像素/整树能证明：
//! - 裁剪：`children_clip` 返回的矩形是否真的作用在绘制上；
//! - 自动流：`cell_of` 是否真的跳过被显式定位占用的格子。
//!
//! 全部用 `SoftBackend`（CPU 回读），不依赖 D3D11。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::grid::{Grid, GridTrack};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Edges, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, UiTree, Widget,
};

const W: u32 = 400;
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

/// 一个**故意画出界**的叶子：它无视自己的矩形，向右画 `extra` 像素。
///
/// 为什么需要它：只有"内容比格子大"时才能验证裁剪真的生效；
/// 规规矩矩的控件永远不会暴露裁剪切错的问题。
struct OverflowLeaf {
    id: String,
    rect: Rect,
    /// 额外向右溢出的像素。
    extra: f32,
}

impl OverflowLeaf {
    fn new(id: &str, extra: f32) -> Self {
        Self {
            id: id.to_string(),
            rect: Rect::ZERO,
            extra,
        }
    }
}

impl Widget for OverflowLeaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
        c.constrain(Size::new(self.rect.w.max(10.0), 10.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 从自己的左上角开始，画得比自己的矩形更宽（模拟"内容溢出"）
        ctx.fill_rect(
            Rect::new(self.rect.x, self.rect.y, self.rect.w + self.extra, self.rect.h),
            Color::from_rgb8(255, 0, 0),
        );
    }
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true).label(self.id.clone())
    }
}

/// 一个规规矩矩的叶子（自动流测试用）。
struct PlainLeaf {
    id: String,
    rect: Rect,
}

impl PlainLeaf {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for PlainLeaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
        c.constrain(Size::new(10.0, 10.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::from_rgb8(0, 200, 0));
    }
}


/// 本地等宽度量：每个字符 8px 宽、15px 行高。
struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 先跑一次 measure + arrange，把轨道尺寸填上，再取几何。
fn laid_out(mut g: Grid) -> Grid {
    let mut cache = LayoutCache::new();
    let c = Constraints::loose(Size::new(W as f32, H as f32));
    let _ = g.measure(&Mono, c, &mut cache);
    g.arrange(Rect::new(0.0, 0.0, W as f32, H as f32), &Mono, &mut cache);
    g
}

/// 2×2 网格，列 100、行 50、无间距、无内边距。
fn grid_2x2(children: Vec<Box<dyn Widget>>) -> Grid {
    Grid::new(
        "g",
        vec![GridTrack::Fixed(100.0), GridTrack::Fixed(100.0)],
        vec![GridTrack::Fixed(50.0), GridTrack::Fixed(50.0)],
    )
    .gap(0.0, 0.0)
    .padding(Edges::ZERO)
    .with_children(children)
}

#[test]
fn spanning_child_is_clipped_to_the_union_not_the_first_cell() {
    // 跨两列（0..2）：并集宽 200；控件画 200+150 = 350，超出 150 应被裁掉。
    let mut g = grid_2x2(vec![Box::new(OverflowLeaf::new("wide", 150.0))]);
    g.place("wide", 0..2, 0..1).expect("跨两列");

    let mut tree = UiTree::new(Box::new(g));
    let mut s = soft();
    tree.layout(Size::new(W as f32, H as f32), &s);
    s.clear();
    tree.render(&mut s);

    // 并集内部（第 0..200 列，第 0 行）应为红色
    let inside = s.pixel(100, 10).expect("px");
    let (r_in, g_in, b_in) = red_of(inside);
    assert!(
        r_in > 180 && g_in < 60 && b_in < 60,
        "并集内应为红色，实际 ({r_in},{g_in},{b_in})"
    );

    // 并集之外（x >= 200）应为背景（没有被溢出画到）
    let outside = s.pixel(250, 10).expect("px");
    let (r_out, g_out, b_out) = red_of(outside);
    assert!(
        r_out < 60 && g_out < 60 && b_out < 60,
        "并集之外应被裁剪（背景色），实际 ({r_out},{g_out},{b_out}) —— 说明裁剪切错了格子"
    );

    // 反向保证：如果没有裁剪，x=250 一定是红的；这条断言就是上面那条的"变异对照"
    assert!(
        outside != inside,
        "裁剪点与非裁剪点颜色相同，说明本次测量没有区分力"
    );
}

fn red_of(px: u32) -> (u8, u8, u8) {
    (
        ((px >> 16) & 0xFF) as u8,
        ((px >> 8) & 0xFF) as u8,
        (px & 0xFF) as u8,
    )
}

#[test]
fn auto_flow_skips_cells_taken_by_explicit_placement() {
    // 把 a 放在**右上**，则自动流必须让 b 落在 (0,0)、c 落在 (0,1)。
    let mut g = grid_2x2(vec![
        Box::new(PlainLeaf::new("a")),
        Box::new(PlainLeaf::new("b")),
        Box::new(PlainLeaf::new("c")),
    ]);
    g.place("a", 1..2, 0..1).expect("a 在右上");
    let rects = laid_out(g).child_rects();

    // 网格原点 (0,0)：列宽 100、行高 50
    assert_eq!(rects[0], Rect::new(100.0, 0.0, 100.0, 50.0), "a 在右上");
    assert_eq!(rects[1], Rect::new(0.0, 0.0, 100.0, 50.0), "b 应落在左上");
    assert_eq!(rects[2], Rect::new(0.0, 50.0, 100.0, 50.0), "c 应落在左下");
}

#[test]
fn auto_flow_matches_the_spec_example() {
    // 需求原文的例子：place("a", 0..1, 0..1)，children = [a, b, c]
    let mut g = grid_2x2(vec![
        Box::new(PlainLeaf::new("a")),
        Box::new(PlainLeaf::new("b")),
        Box::new(PlainLeaf::new("c")),
    ]);
    g.place("a", 0..1, 0..1).expect("a 在左上");
    let rects = laid_out(g).child_rects();
    assert_eq!(rects[0], Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(rects[1], Rect::new(100.0, 0.0, 100.0, 50.0), "b 在 (1,0)");
    assert_eq!(rects[2], Rect::new(0.0, 50.0, 100.0, 50.0), "c 在 (0,1)");
}

#[test]
fn without_placements_layout_is_unchanged() {
    // 向后兼容：一个都没 place 时，行为必须与旧版顺序排布逐像素一致。
    let g = grid_2x2(vec![
        Box::new(PlainLeaf::new("a")),
        Box::new(PlainLeaf::new("b")),
        Box::new(PlainLeaf::new("c")),
        Box::new(PlainLeaf::new("d")),
    ]);
    let rects = laid_out(g).child_rects();
    assert_eq!(rects[0], Rect::new(0.0, 0.0, 100.0, 50.0));
    assert_eq!(rects[1], Rect::new(100.0, 0.0, 100.0, 50.0));
    assert_eq!(rects[2], Rect::new(0.0, 50.0, 100.0, 50.0));
    assert_eq!(rects[3], Rect::new(100.0, 50.0, 100.0, 50.0));
}

// 让编译器知道这些导入被用到（`Pos`/`TextMetrics` 出现在签名与类型见证里）
#[allow(dead_code)]
fn _witness(_p: Pos, _m: &dyn TextMetrics) {}
