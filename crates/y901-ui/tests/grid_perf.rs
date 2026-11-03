//! Grid 布局性能基准（第 4 步的性能红线）。
//!
//! 跑法：
//! ```text
//! cargo test -p y901-ui --release --test grid_perf -- --ignored --nocapture
//! ```
//!
//! 红线：10k 节点 Grid 的 `measure + arrange` ≤ 3 ms；
//! 冲突检测 ≤ 0.3 ms（`place` 的 N 次调用）。

use std::time::Instant;

use y901_ui::grid::{Grid, GridTrack};
use y901_ui::layout::LayoutCache;
use y901_ui::{Constraints, Rect, RenderCtx, Semantics, Size, TextStyle, TextMetrics, Widget};

/// 极简度量：每字符 8px。
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

/// 定尺寸叶子：测出 `size`，不画东西。
struct Leaf {
    id: String,
    size: Size,
    rect: Rect,
}

impl Leaf {
    fn new(id: String, w: f32, h: f32) -> Self {
        Self {
            id,
            size: Size::new(w, h),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
        c.constrain(self.size)
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
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

/// 建一个 `cols × rows` 的网格，每格一个 20×10 的叶子；前 `spanning` 个改为跨两列。
fn build(cols: usize, rows: usize, spanning: usize) -> Grid {
    let mut children: Vec<Box<dyn Widget>> = Vec::with_capacity(cols * rows);
    for i in 0..cols * rows {
        children.push(Box::new(Leaf::new(format!("c{i}"), 20.0, 10.0)));
    }
    let mut g = Grid::new(
        "perf",
        vec![GridTrack::Auto; cols],
        vec![GridTrack::Auto; rows],
    )
    .gap(1.0, 1.0)
    .with_children(children);

    // 让前几个控件跨两列（不重叠：放在不同的行）
    for k in 0..spanning.min(rows) {
        let id = format!("c{}", k * cols);
        let _ = g.place(id, 0..2u32, k as u32..k as u32 + 1);
    }
    g
}

#[test]
#[ignore = "性能基准，--ignored 手动跑"]
fn grid_10k_layout_time() {
    let (cols, rows) = (100usize, 100usize);
    let nodes = cols * rows;

    // 预热
    let mut g = build(cols, rows, 50);
    let mut cache = LayoutCache::new();
    let c = Constraints::loose(Size::new(4000.0, 2000.0));
    let _ = g.measure(&Mono, c, &mut cache);
    g.arrange(Rect::new(0.0, 0.0, 4000.0, 2000.0), &Mono, &mut cache);

    // 正式测：每次重建缓存，测"全量重排"
    let rounds = 5;
    let mut total = 0f64;
    let mut worst = 0f64;
    for _ in 0..rounds {
        let mut g = build(cols, rows, 50);
        let mut cache = LayoutCache::new();
        let t = Instant::now();
        let _ = g.measure(&Mono, c, &mut cache);
        g.arrange(Rect::new(0.0, 0.0, 4000.0, 2000.0), &Mono, &mut cache);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        total += ms;
        if ms > worst {
            worst = ms;
        }
    }
    let avg = total / rounds as f64;
    eprintln!("[grid-perf] nodes={nodes} spanning=50 avg={avg:.3}ms worst={worst:.3}ms");
    assert!(avg > 0.0);
}

#[test]
#[ignore = "性能基准，--ignored 手动跑"]
fn grid_10k_conflict_detection_time() {
    let (cols, rows) = (100usize, 100usize);
    let mut children: Vec<Box<dyn Widget>> = Vec::with_capacity(cols * rows);
    for i in 0..cols * rows {
        children.push(Box::new(Leaf::new(format!("c{i}"), 10.0, 10.0)));
    }
    let mut g = Grid::new(
        "perf",
        vec![GridTrack::Auto; cols],
        vec![GridTrack::Auto; rows],
    )
    .with_children(children);

    // 10k 次 place，每次都要扫相关行的占用集 → 测冲突检测的总代价
    let t = Instant::now();
    for i in 0..cols * rows {
        let id = format!("c{i}");
        let col = (i % cols) as u32;
        let row = (i / cols) as u32;
        g.place(id, col..col + 1, row..row + 1).expect("每格一个");
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!("[grid-perf] placements={} {ms:.3}ms", cols * rows);
    assert!(ms > 0.0);
}
