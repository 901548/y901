//! dispatch 路径的分配计数基准（审查 #18 的 tree.rs 候选）。
//!
//! **必须单线程**：计数器是进程全局的，并行跑会把别的用例的分配也算进来。
//!
//! 跑法：
//! ```text
//! cargo test -p y901-ui --release --test perf_dispatch_alloc -- --ignored --nocapture --test-threads=1
//! ```
//!
//! 回答一个问题：**一次事件派发（dispatch）分配多少次？** 其中多少来自
//! `focus.clone()` / `id.to_string()` / `path.clone()` 这三类。
//!
//! 为什么用计数分配器而不是 profiler：本轮只需要"每帧 N 次"这个量级数字，
//! 计数分配器没有采样误差、可复现，且不需要符号化堆栈。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn take_allocs() -> usize {
    ALLOCS.swap(0, Ordering::Relaxed)
}

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, Key, Modifiers, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
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
        c.constrain(Size::new(40.0, 20.0))
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
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Handled
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn flat(n: usize) -> Box<dyn Widget> {
    let mut col = Flex::column("root");
    for i in 0..n {
        col = col.push(Box::new(Leaf {
            id: format!("f{i}"),
            rect: Rect::ZERO,
        }));
    }
    Box::new(col)
}

/// 一次 `dispatch` 的分配次数（100 节点树，键盘事件，取 1000 次平均）。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn dispatch_allocs_per_event() {
    let mut tree = UiTree::new(flat(100));
    tree.layout(Size::new(400.0, 400.0), &Mono);

    // \u9884\u70ed\uff1a\u9996\u6b21 dispatch \u4f1a\u5efa\u7acb focus/\u7d22\u5f15\u7b49\u4e00\u6b21\u6027\u72b6\u6001
    let ev = UiEvent::Key {
        key: Key::Char('a'),
        modifiers: Modifiers::default(),
    };
    for _ in 0..10 {
        let _ = tree.dispatch(&ev);
    }

    let _ = take_allocs();
    const N: usize = 1000;
    for _ in 0..N {
        let _ = tree.dispatch(&ev);
    }
    let total = take_allocs();
    eprintln!(
        "[dispatch] 100 节点树，键盘事件：每帧 {:.2} 次分配（{N} 次共 {total} 次）",
        total as f64 / N as f64
    );
    assert!(
        total > 0,
        "dispatch 至少应有若干次分配，否则说明计数器没接上"
    );
}

/// 一次鼠标移动的分配次数（悬停目标变化会走 `focus`/`path` 相关路径）。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn hover_allocs_per_event() {
    let mut tree = UiTree::new(flat(100));
    tree.layout(Size::new(400.0, 400.0), &Mono);

    let ev = UiEvent::MouseMove {
        pos: y901_ui::Pos::new(20.0, 20.0),
    };
    for _ in 0..10 {
        let _ = tree.dispatch(&ev);
    }
    let _ = take_allocs();
    const N: usize = 1000;
    for _ in 0..N {
        let _ = tree.dispatch(&ev);
    }
    let total = take_allocs();
    eprintln!(
        "[dispatch] 100 节点树，同位置 MouseMove：每帧 {:.2} 次分配",
        total as f64 / N as f64
    );
    assert!(total > 0);
}

/// 分诊：同一只标在**空白处**移动（命中无控件、目标不变）时的分配。
///
/// 与 `hover_allocs_per_event`（命中控件）相减，就能把"命中/目标切换"
/// 相关的分配（`focus.clone()` 与 `ctx.target = id.to_string()`）单独分离出来。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn hover_blank_area_allocs_per_event() {
    let mut tree = UiTree::new(flat(100));
    tree.layout(Size::new(400.0, 400.0), &Mono);

    let ev = UiEvent::MouseMove {
        pos: y901_ui::Pos::new(395.0, 395.0),
    };
    for _ in 0..10 {
        let _ = tree.dispatch(&ev);
    }
    let _ = take_allocs();
    const N: usize = 1000;
    for _ in 0..N {
        let _ = tree.dispatch(&ev);
    }
    let total = take_allocs();
    eprintln!(
        "[dispatch] 100 节点树，空白处 MouseMove：每帧 {:.2} 次分配",
        total as f64 / N as f64
    );
    assert!(total < 100_000, "计数器应接上");
}
