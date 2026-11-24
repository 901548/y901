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
    /// `Some` = 像真实控件那样在构造时预算好 `id_key`（`cache_key()`），
    /// `None` = 每次布局都现算字符串 FNV（本合成树的默认，最悲观）。
    key: Option<u64>,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn cache_key(&self) -> Option<u64> {
        self.key
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
    build_with(width, depth, false)
}

/// 同 [`build`]，但可以让叶子像**真实控件**那样在构造时预算好 `cache_key()`。
///
/// 用途：量化"每帧现算字符串 FNV"在 11k 节点上占多少 —— 真实控件
/// （`Flex`/`Label`/`Button`…）都覆盖了 `cache_key()`，而本合成树的 `Leaf`
/// 默认不覆盖，所以默认数字是**偏悲观**的上界。
fn build_with(width: usize, depth: usize, precomputed_keys: bool) -> UiTree {
    fn layer(prefix: &str, width: usize, depth: usize, precomputed_keys: bool) -> Box<dyn Widget> {
        if depth == 0 {
            let id = format!("{prefix}_leaf");
            let key = precomputed_keys.then(|| y901_ui::layout::id_key(&id));
            return Box::new(Leaf {
                id,
                rect: Rect::ZERO,
                key,
            });
        }
        let mut col = Flex::column(prefix).flex_item(FlexItem::default());
        for i in 0..width {
            col = col.push(layer(
                &format!("{prefix}_{i}"),
                width,
                depth - 1,
                precomputed_keys,
            ));
        }
        Box::new(col)
    }
    let mut tree = UiTree::new(layer("n", width, depth, precomputed_keys));
    tree.layout(Size::new(2000.0, 2000.0), &Mono);
    tree
}

/// **量化哈希占比**：同一棵树，叶子分别"每帧现算字符串哈希"与"构造时预算键"。
#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_hash_share() {
    let viewport = Size::new(2000.0, 2000.0);
    for (label, pre) in [("leaf-现算字符串哈希", false), ("leaf-预算cache_key", true)] {
        let mut tree = build_with(10, 4, pre);
        // 预热一轮，让缓存表就位
        let _ = time_layout(&mut tree, viewport, 2);
        let (avg, _) = time_layout(&mut tree, viewport, 6);
        eprintln!("[hash-share] {label}: cold_avg={avg:.1}us");
    }
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

/// 空渲染上下文：只统计调用次数，不做任何像素工作 ——
/// 这样测到的是**绘制遍历**本身（clip 计算、脏区裁剪、节点遍历），与光栅化无关。
#[derive(Default)]
struct Sink {
    fills: usize,
    texts: usize,
    clips: usize,
}

impl TextMetrics for Sink {
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

impl RenderCtx for Sink {
    fn size(&self) -> Size {
        Size::new(2000.0, 2000.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {
        self.fills += 1;
    }
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {
        self.fills += 1;
    }
    fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
        self.texts += 1;
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {
        self.clips += 1;
    }
    fn pop_clip(&mut self) {}
}

#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_render_walk() {
    // 每帧的绘制遍历：全量脏 vs 只脏一个叶子。
    // 这一步在生产里是**每帧都跑**的（只要有脏区），比布局更常触发。
    let viewport = Size::new(2000.0, 2000.0);
    let mut tree = build(10, 4);
    tree.layout(viewport, &Mono);
    tree.invalidate_all();

    // 预热一次，让 paint_order 等结构就位
    let mut sink = Sink::default();
    tree.render(&mut sink);

    let rounds = 20;
    let mut total = 0.0;
    for _ in 0..rounds {
        tree.invalidate_all();
        let t = Instant::now();
        tree.render(&mut sink);
        total += t.elapsed().as_secs_f64() * 1e6;
    }
    let full_avg = total / rounds as f64;
    let full_fills = sink.fills;
    let full_clips = sink.clips;

    // 局部：只脏一个叶子的矩形
    let leaf_rect = tree
        .rect_of("n_0_0_0_0_leaf")
        .unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
    let mut total_partial = 0.0;
    for _ in 0..rounds {
        tree.invalidate_rect(leaf_rect);
        let t = Instant::now();
        tree.render(&mut sink);
        total_partial += t.elapsed().as_secs_f64() * 1e6;
    }
    let partial_avg = total_partial / rounds as f64;

    eprintln!(
        "[render] full_frame avg={full_avg:.1}us fills={full_fills} clips={full_clips} | \
         one_leaf avg={partial_avg:.1}us"
    );
    assert!(full_avg > 0.0);
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
/// 渲染遍历的**代价模型**：整帧 vs 局部（不同脏区大小）。
/// 用来判断"遍历本身"与"每个节点实际 draw"各占多少。
#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_render_cost_model() {
    let viewport = Size::new(2000.0, 2000.0);
    let mut tree = build(10, 4);
    tree.layout(viewport, &Mono);
    let mut sink = Sink::default();
    tree.invalidate_all();
    tree.render(&mut sink);

    // (a) 整帧
    let mut full = 0.0;
    for _ in 0..10 {
        tree.invalidate_all();
        let t = Instant::now();
        tree.render(&mut sink);
        full += t.elapsed().as_secs_f64() * 1e6;
    }
    full /= 10.0;

    // (b) 局部：脏区大小不同
    let cases = [
        ("1x1", Rect::new(100.0, 100.0, 1.0, 1.0)),
        ("50x50", Rect::new(100.0, 100.0, 50.0, 50.0)),
        ("全视口", Rect::new(0.0, 0.0, 2000.0, 2000.0)),
    ];
    let mut out = format!("[render-model] full={full:.1}us");
    for (label, r) in cases {
        let mut acc = 0.0;
        for _ in 0..10 {
            tree.invalidate_rect(r);
            let t = Instant::now();
            tree.render(&mut sink);
            acc += t.elapsed().as_secs_f64() * 1e6;
        }
        out.push_str(&format!(" {label}={:.1}us", acc / 10.0));
    }
    eprintln!("{out}");
}
/// **Grid 在真实树里的代价**：`grid_perf` 只测 `Grid::measure/arrange`，
/// 不含 `UiTree` 的索引重建（那里会逐子调用 `children_clip`）。
#[test]
#[ignore = "剖析台：cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture"]
fn profile_grid_in_tree() {
    use y901_ui::grid::{Grid, GridTrack};

    fn make() -> UiTree {
        let (cols, rows) = (100usize, 100usize);
        let mut children: Vec<Box<dyn Widget>> = Vec::with_capacity(cols * rows);
        for i in 0..cols * rows {
            children.push(Box::new(Leaf {
                id: format!("c{i}"),
                rect: Rect::ZERO,
                key: None,
            }));
        }
        let grid = Grid::new(
            "grid",
            vec![GridTrack::Auto; cols],
            vec![GridTrack::Auto; rows],
        )
        .gap(1.0, 1.0)
        .with_children(children);
        UiTree::new(Box::new(grid))
    }

    let viewport = Size::new(4000.0, 2000.0);
    let mut tree = make();
    tree.layout(viewport, &Mono);

    // 冷路径（视口交替 1px → 必须全量重排）
    let (cold, worst) = time_layout(&mut tree, viewport, 10);
    eprintln!("[grid-tree] nodes=10001 cold_avg={cold:.1}us worst={worst:.1}us");

    // 整帧渲染
    let mut sink = Sink::default();
    tree.invalidate_all();
    tree.render(&mut sink);
    let mut full = 0.0;
    for _ in 0..10 {
        tree.invalidate_all();
        let t = Instant::now();
        tree.render(&mut sink);
        full += t.elapsed().as_secs_f64() * 1e6;
    }
    eprintln!("[grid-tree] render_full_avg={:.1}us", full / 10.0);
}
