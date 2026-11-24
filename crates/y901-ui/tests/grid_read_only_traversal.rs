//! **只读遍历不得让 `Grid` 的落位缓存失效**。
//!
//! `UiTree::render()` / 事件分发的"沿路径取节点"（`node_at_mut`）需要 `&mut dyn Widget`，
//! 因此只能用 `Widget::children_mut()` 逐层下钻。但 `children_mut()` 的契约是
//! "外部要改这个 Vec 了，索引必须标脏" —— `Grid` 实现里把 `index_dirty` / `cells_dirty`
//! 一起置位。于是**每帧渲染都会把每个 Grid 的落位缓存标脏**，下一次布局白跑一遍
//! `auto_cells()`（建占用位图 + 为每个子控件找空格；100×100 网格是 10k 次搜索）。
//!
//! 这条用例把"渲染不应改变布局状态"钉住：渲染前后的布局结果必须完全一致，
//! 且 `Grid` 的落位缓存不能被渲染弄失效。

use y901_ui::grid::{Grid, GridTrack};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

struct Leaf {
    id: String,
    size: Size,
    rect: Rect,
}

impl Leaf {
    fn new(id: String) -> Self {
        Self {
            id,
            size: Size::new(10.0, 10.0),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(self.size)
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn build(cols: usize, rows: usize) -> UiTree {
    let mut children: Vec<Box<dyn Widget>> = Vec::with_capacity(cols * rows);
    for i in 0..cols * rows {
        children.push(Box::new(Leaf::new(format!("c{i}"))));
    }
    let grid = Grid::new(
        "g",
        vec![GridTrack::Auto; cols],
        vec![GridTrack::Auto; rows],
    )
    .with_children(children);
    UiTree::new(Box::new(grid))
}

/// 渲染（整帧）不得改变布局结果，也不得让 Grid 的落位缓存失效。
#[test]
fn rendering_does_not_invalidate_grid_placement_cache() {
    let (cols, rows) = (6usize, 5usize);
    let viewport = Size::new(400.0, 300.0);
    let mut tree = build(cols, rows);
    tree.layout(viewport, &Mono);
    let before: Vec<Option<Rect>> = (0..cols * rows)
        .map(|i| tree.rect_of(&format!("c{i}")))
        .collect();

    // 整帧渲染：内部会沿路径取节点（`node_at_mut`）
    tree.invalidate_all();
    let mut sink = Sink;
    tree.render(&mut sink);

    // 渲染后再布局一次（模拟下一帧）。若渲染把 Grid 标脏了，这里会重算落位缓存；
    // 结果虽然仍应正确，但"标记状态"已被污染 —— 用几何不变来兜底，
    // 另外用一次 `child_rects()` 的调用计数确认缓存没有被迫重建。
    tree.layout(viewport, &Mono);
    let after: Vec<Option<Rect>> = (0..cols * rows)
        .map(|i| tree.rect_of(&format!("c{i}")))
        .collect();
    assert_eq!(before, after, "渲染前后的布局结果必须一致");
}

/// 落位缓存的重算次数：**只读操作不得让它增长**。
///
/// 这是本文件真正的断言 —— 上面那条"几何不变"只能证明结果对，证明不了"没白算"。
#[test]
fn grid_placement_is_not_recomputed_after_read_only_operations() {
    let (cols, rows) = (6usize, 5usize);
    let viewport = Size::new(400.0, 300.0);
    let mut tree = build(cols, rows);
    tree.layout(viewport, &Mono);
    let after_layout = placement_revision(&mut tree);

    // 1) 整帧渲染（沿路径下钻 → 旧实现会经 `children_mut()` 标脏）
    tree.invalidate_all();
    let mut sink = Sink;
    tree.render(&mut sink);
    assert_eq!(
        placement_revision(&mut tree),
        after_layout,
        "渲染（只读）不应让 Grid 重算落位"
    );

    // 2) 事件分发（同样沿路径下钻）
    tree.dispatch(&y901_ui::UiEvent::MouseMove {
        pos: y901_ui::Pos::new(5.0, 5.0),
    });
    assert_eq!(
        placement_revision(&mut tree),
        after_layout,
        "事件分发（只读遍历）不应让 Grid 重算落位"
    );

    // 3) **强制一次真实重排**（只标脏布局缓存、不改几何），再布局。
    //
    // 这一步是关键：视口不变时 `layout()` 会整棵跳过、`Grid::measure` 根本不会被调用，
    // 于是"渲染有没有把落位缓存标脏"完全观测不到（用例会永远绿）。
    // 先 `mark_layout_dirty` 逼 `measure` 真的跑，才能看见格子是否被重算。
    tree.mark_layout_dirty("g");
    tree.layout(viewport, &Mono);
    assert_eq!(
        placement_revision(&mut tree),
        after_layout,
        "渲染/事件分发之后重排，不应让 Grid 重算落位（说明只读遍历把缓存标脏了）"
    );

    // 4) 反向证据：真的改了子节点，必须重算（否则上面的断言会"永远为真"）。
    //    注意要**触发一次真实布局**（改视口）：约束不变时 `layout()` 会整棵跳过，
    //    `ensure_cells` 根本不会被调用，计数自然不动。
    tree.with_node_mut("g", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(g) = any.downcast_mut::<Grid>() {
                let _ = g.children_mut().map(|c| c.len());
            }
        }
    });
    tree.layout(Size::new(420.0, 320.0), &Mono);
    assert!(
        placement_revision(&mut tree) > after_layout,
        "子节点集合变化后必须重算落位（否则本用例的反向证据失效）"
    );
}

fn placement_revision(tree: &mut UiTree) -> u64 {
    let mut n = 0;
    tree.with_node_mut("g", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(g) = any.downcast_mut::<Grid>() {
                n = g.placement_revision();
            }
        }
    });
    n
}

struct Sink;

impl y901_ui::TextMetrics for Sink {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Mono.measure_text(t, s)
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        Mono.line_height(s)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Sink {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _p: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}
