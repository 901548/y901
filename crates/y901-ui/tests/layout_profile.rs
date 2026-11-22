//! 布局剖析台（`#[ignore]`，手动跑）。
//!
//! 与 `layout_bench` 的分工：那个只回答"全量重排多少微秒"，这个回答
//! **时间花在哪** —— 冷/热布局对比、缓存命中率、以及"每次布局的分配次数"。
//!
//! 用法：
//! ```text
//! cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture
//! ```
//!
//! 为什么要"冷 vs 热"两组数：
//! - **热**（同一视口连跑）= 命中 `LayoutCache`，成本几乎全是哈希查找 + 树遍历；
//! - **冷**（视口交替 2000/2001）= 每轮全树重测，成本是真正的 measure/arrange 数学。
//!
//! 两者差距告诉我们"缓存还有没有价值"以及"优化该打哈希还是打数学"。

use std::alloc::{GlobalAlloc, Layout as AllocLayout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiTree, Widget,
};

// ---------------------------------------------------------------- 分配计数

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: AllocLayout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) > 0 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: AllocLayout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: AllocLayout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) > 0 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// ---------------------------------------------------------------- 合成树

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
    rect: Rect,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 16.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn build(width: usize, depth: usize) -> UiTree {
    fn layer(prefix: &str, width: usize, depth: usize) -> Box<dyn Widget> {
        if depth == 0 {
            return Box::new(Leaf {
                id: format!("{prefix}_leaf"),
                rect: Rect::ZERO,
            });
        }
        let mut col = Flex::column(prefix).flex_item(FlexItem::default());
        for i in 0..width {
            col = col.push(layer(&format!("{prefix}_{i}"), width, depth - 1));
        }
        Box::new(col)
    }
    let mut tree = UiTree::new(layer("n", width, depth));
    tree.layout(Size::new(2000.0, 2000.0), &Mono);
    tree
}

fn time_layout(tree: &mut UiTree, viewport: Size, rounds: usize) -> (f64, f64) {
    let mut total = 0.0;
    let mut worst = 0.0f64;
    for i in 0..rounds {
        // 交替 1px：等价于"视口变了 → 必须全量重排"（冷路径）。
        let w = if i % 2 == 0 {
            viewport.w
        } else {
            viewport.w + 1.0
        };
        let t = Instant::now();
        tree.layout(Size::new(w, viewport.h), &Mono);
        let us = t.elapsed().as_secs_f64() * 1e6;
        total += us;
        if us > worst {
            worst = us;
        }
    }
    (total / rounds as f64, worst)
}

fn time_layout_warm(tree: &mut UiTree, viewport: Size, rounds: usize) -> (f64, f64) {
    let mut total = 0.0;
    let mut worst = 0.0f64;
    tree.layout(viewport, &Mono);
    for _ in 0..rounds {
        let t = Instant::now();
        tree.layout(viewport, &Mono);
        let us = t.elapsed().as_secs_f64() * 1e6;
        total += us;
        if us > worst {
            worst = us;
        }
    }
    (total / rounds as f64, worst)
}

#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_skip_by_size() {
    // 缓存是否真的生效，随规模变化吗？——「热布局实测每秒都在重测」若成立，
    // 说明 skip 在某个规模上失效（小树测试却是绿的），这里按规模二分。
    for (w, d) in [(2usize, 2usize), (4, 3), (10, 3), (10, 4)] {
        let mut tree = build(w, d);
        let viewport = Size::new(2000.0, 2000.0);
        tree.layout(viewport, &Mono);
        tree.layout(viewport, &Mono);
        let s = tree.layout_stats();
        eprintln!(
            "[skip] nodes≈{}^{}={} measured={} arranged={} skipped={}",
            w,
            d,
            w.pow(d as u32),
            s.measured,
            s.arranged,
            s.skipped
        );
    }
}

#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_shape_costs() {
    // 同样的节点量、不同的形状：区分"每容器开销"与"每节点开销"。
    // - 宽树（1 个容器 + 10k 叶子）：几乎全是叶子路径（缓存哈希 + measure/arrange）
    // - 深树（本项目默认 10^4）：1111 个容器，每个 10 个子件
    let viewport = Size::new(2000.0, 2000.0);
    for (label, w, d) in [("wide-10k", 10000usize, 1usize), ("deep-10k", 10, 4)] {
        let mut tree = build(w, d);
        tree.layout(viewport, &Mono);
        let (avg, _) = time_layout(&mut tree, viewport, 10);
        let s = tree.layout_stats();
        eprintln!(
            "[shape] {label} nodes={} cold_avg={avg:.1}us measured={} arranged={}",
            w.pow(d as u32),
            s.measured,
            s.arranged
        );
    }
}

#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_10k_layout() {
    let viewport = Size::new(2000.0, 2000.0);

    // 冷：视口交替 → 每轮全树重测
    let mut tree = build(10, 4);
    let (cold_avg, cold_worst) = time_layout(&mut tree, viewport, 20);

    // 热：同一视口 → 命中缓存
    let (warm_avg, _) = time_layout_warm(&mut tree, viewport, 20);

    // 分配：一次冷布局的分配次数/字节（计数窗口内只包含 layout 调用）
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    COUNTING.store(1, Ordering::Relaxed);
    tree.layout(Size::new(viewport.w + 2.0, viewport.h), &Mono);
    COUNTING.store(0, Ordering::Relaxed);
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);

    let stats = tree.layout_stats();
    eprintln!(
        "[profile] cold avg={cold_avg:.1}us worst={cold_worst:.1}us | warm avg={warm_avg:.1}us \
         | cold_allocs={allocs} cold_bytes={bytes} | last_stats measured={} arranged={} skipped={}",
        stats.measured, stats.arranged, stats.skipped
    );

    // 热路径（视口不变）单独计分配：这条路径上理论上是"什么都不该重算"，
    // 若分配量仍然与节点数同阶，说明每帧都在做 O(n) 的无谓工作。
    //
    // **必须先在同一视口上连跑两次**再开始计数：上一段冷测量结束时 tree 的视口
    // 是 `w+2`，直接测会测到一次冷布局（这个坑真踩过：数字与冷布局一模一样）。
    tree.layout(viewport, &Mono);
    tree.layout(viewport, &Mono);
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    COUNTING.store(1, Ordering::Relaxed);
    tree.layout(viewport, &Mono);
    COUNTING.store(0, Ordering::Relaxed);
    let warm_allocs = ALLOCS.load(Ordering::Relaxed);
    let warm_bytes = BYTES.load(Ordering::Relaxed);
    let wstats = tree.layout_stats();
    eprintln!(
        "[profile] warm_allocs={warm_allocs} warm_bytes={warm_bytes} \
         warm_stats measured={} arranged={} skipped={}",
        wstats.measured, wstats.arranged, wstats.skipped
    );

    assert!(cold_avg > 0.0);
}
