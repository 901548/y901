//! 性能基准（不进默认测试）：布局引擎 + 脏区合并 + **分配计数**。
//!
//! 运行：
//! ```text
//! cargo test -p y901-ui --release --test perf_bench -- --ignored --nocapture
//! cargo test -p y901-ui --test perf_bench -- --ignored --nocapture        # debug
//! ```

use std::time::Instant;

use y901_ui::{
    Constraints, DirtyRegion, EventCtx, EventResult, Flex, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiEvent, UiTree, Widget,
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
    /// 模拟"热路径组件"：构造时预算 key，布局时不再哈希字符串。
    key: u64,
    rect: Rect,
}

/// 对照组：**不**缓存 key 的叶子（老行为，用来量化 cache_key 的收益）。
struct PlainLeaf {
    id: String,
    rect: Rect,
}

impl Widget for PlainLeaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn count(n: &dyn Widget) -> usize {
    1 + n.children().iter().map(|c| count(c.as_ref())).sum::<usize>()
}

/// 窄而深：`width^depth` 个叶子。
fn deep(width: usize, depth: usize) -> Box<dyn Widget> {
    fn go(prefix: &str, width: usize, depth: usize) -> Box<dyn Widget> {
        if depth == 0 {
            let id = format!("{prefix}l");
            return Box::new(Leaf {
                key: y901_ui::layout::id_key(&id),
                id,
                rect: Rect::ZERO,
            });
        }
        let mut col = Flex::column(prefix);
        for i in 0..width {
            col = col.push(go(&format!("{prefix}{i}_"), width, depth - 1));
        }
        Box::new(col)
    }
    go("n", width, depth)
}

/// 宽而浅：一个 Flex 挂 `n` 个叶子。
fn flat(n: usize) -> Box<dyn Widget> {
    let mut col = Flex::column("root");
    for i in 0..n {
        let id = format!("f{i}");
        col = col.push(Box::new(Leaf {
            key: y901_ui::layout::id_key(&id),
            id,
            rect: Rect::ZERO,
        }));
    }
    Box::new(col)
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

fn bench_layout(name: &str, root: Box<dyn Widget>, rounds: usize) {
    let mut tree = UiTree::new(root);
    tree.layout(Size::new(2000.0, 2000.0), &Mono);
    let nodes = count(tree.root());

    let mut samples = Vec::with_capacity(rounds);
    for i in 0..rounds {
        // 交替视口尺寸：等价于“视口变了 → 全树重排”
        let w = if i % 2 == 0 { 2000.0 } else { 2001.0 };
        let t = Instant::now();
        tree.layout(Size::new(w, 2000.0), &Mono);
        samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = samples.iter().sum::<f64>() / samples.len() as f64;
    eprintln!(
        "[layout] {name}: nodes={nodes} rounds={rounds} avg={avg:.0}us p50={:.0}us p99={:.0}us max={:.0}us",
        pct(&sorted, 0.50),
        pct(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.0)
    );
    assert!(avg > 0.0);
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_layout_10k_deep() {
    bench_layout("10k deep", deep(10, 4), 15);
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_layout_10k_flat() {
    bench_layout("10k flat", flat(10_000), 15);
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_layout_50k_flat() {
    bench_layout("50k flat", flat(50_000), 10);
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_dirty_region_streaming() {
    // 模拟流式输出：每次往尾部追加一行小矩形（与上一行贴着，所以会不断合并）
    const N: usize = 10_000;
    let mut d = DirtyRegion::default();
    let mut samples = Vec::with_capacity(N);
    let t_all = Instant::now();
    for i in 0..N {
        let r = Rect::new(100.0, 100.0 + i as f32 * 18.0, 400.0, 18.0);
        let t = Instant::now();
        d.add(r);
        samples.push(t.elapsed().as_secs_f64() * 1e9);
    }
    let total = t_all.elapsed().as_secs_f64() * 1e6;
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg_ns = samples.iter().sum::<f64>() / samples.len() as f64;
    eprintln!(
        "[dirty] streaming {} adds: total={:.1}us avg={:.0}ns p99={:.0}ns max={:.0}ns final_rects={} is_full={}",
        N,
        total,
        avg_ns,
        pct(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.0),
        d.rects().len(),
        d.is_full()
    );

    // 散点式：不相交，看矩形数是否爆炸
    let mut d2 = DirtyRegion::default();
    let t = Instant::now();
    for i in 0..N {
        d2.add(Rect::new((i % 100) as f32 * 4.0, (i / 100) as f32 * 4.0, 2.0, 2.0));
    }
    let scattered = t.elapsed().as_secs_f64() * 1e6;
    eprintln!(
        "[dirty] scattered {} adds: total={:.1}us final_rects={}",
        N,
        scattered,
        d2.rects().len()
    );
    let _ = EventCtx::new(None, Size::ZERO);
}

/// 初始化开销定位：同一棵树形，只把 id 长度从 4 改到 48。
///
/// 如果耗时随 id 长度线性增长，说明热点在 **字符串哈希 / 分配**
/// （`rebuild_indices` 里每节点一次 `id.to_string()` + `HashMap` 插入），
/// 而不是 `measure`/`arrange` 的虚函数调用。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_layout_id_length_sensitivity() {
    fn flat_pad(n: usize, pad: usize) -> Box<dyn Widget> {
        let mut col = Flex::column("root");
        for i in 0..n {
            let mut id = format!("f{i}");
            while id.len() < pad {
                id.push('_');
            }
            col = col.push(Box::new(Leaf {
                key: y901_ui::layout::id_key(&id),
                id,
                rect: Rect::ZERO,
            }));
        }
        Box::new(col)
    }
    for pad in [4usize, 16, 48] {
        let mut tree = UiTree::new(flat_pad(10_000, pad));
        tree.layout(Size::new(2000.0, 2000.0), &Mono);
        let mut samples = Vec::new();
        for i in 0..10 {
            let w = if i % 2 == 0 { 2000.0 } else { 2001.0 };
            let t = Instant::now();
            tree.layout(Size::new(w, 2000.0), &Mono);
            samples.push(t.elapsed().as_secs_f64() * 1e6);
        }
        let avg = samples.iter().sum::<f64>() / samples.len() as f64;
        eprintln!("[layout] id_len={pad:2}: 10k nodes avg={avg:.0}us");
    }
}

/// 对照：全部叶子**不**缓存 key（每次布局现场哈希字符串）。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_layout_50k_plain_leaf() {
    fn flat_plain(n: usize) -> Box<dyn Widget> {
        let mut col = Flex::column("root");
        for i in 0..n {
            col = col.push(Box::new(PlainLeaf {
                id: format!("f{i}"),
                rect: Rect::ZERO,
            }));
        }
        Box::new(col)
    }
    bench_layout("50k plain-leaf", flat_plain(50_000), 10);
}
