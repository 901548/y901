//! 分配计数基准（独立二进制：计数分配器有固定开销，不能混进计时基准）。
//!
//! ```text
//! cargo test -p y901-ui --release --test perf_alloc -- --ignored --nocapture
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// 计数分配器：只在这个基准二进制里生效。
struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn take_allocs() -> (usize, usize) {
    (
        ALLOCS.swap(0, Ordering::Relaxed),
        BYTES.swap(0, Ordering::Relaxed),
    )
}


use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiEvent, UiTree, Widget,
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
    key: u64,
    rect: Rect,
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
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
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

/// 一次全量重排会分配多少次？（测试 id 字符串 / 路径副本的代价）
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_layout_allocations_10k() {
    let mut tree = UiTree::new(flat(10_000));
    tree.layout(Size::new(2000.0, 2000.0), &Mono);

    let mut rows = Vec::new();
    for i in 0..10 {
        let w = if i % 2 == 0 { 2000.0 } else { 2001.0 };
        let _ = take_allocs();
        tree.layout(Size::new(w, 2000.0), &Mono);
        rows.push(take_allocs());
    }
    let avg_allocs = rows.iter().map(|(a, _)| *a).sum::<usize>() / rows.len();
    let avg_bytes = rows.iter().map(|(_, b)| *b).sum::<usize>() / rows.len();
    eprintln!(
        "[alloc] 10k nodes full relayout: avg_allocs={avg_allocs} avg_bytes={avg_bytes}  \
         (每节点 {:.2} 次分配)",
        avg_allocs as f64 / 10_001.0
    );
}
