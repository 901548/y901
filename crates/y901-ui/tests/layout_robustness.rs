//! **panic 之后仍可用** + **缓存键契约**。
//!
//! 这两条都属于"不会天天踩、踩了很难查"的健壮性问题：
//!
//! 1. `UiTree::layout` 曾经在布局**之前**就把 `rects` 索引表 `mem::take` 走。
//!    只要控件在 `measure`/`arrange` 里 panic（三方控件里很常见），表就永久空着：
//!    `rect_of()` 全变 `None`、`invalidate(id)` 静默 no-op，下一轮布局还会把整棵树
//!    当成"全新节点"（于是又没有旧∪新标脏 —— 残影）。
//!    现在布局期间不动 `rects`，只在中途用**备用缓冲**交换。
//!
//! 2. `Widget::cache_key()` 只能是 `id_key(id())` 的预算缓存。返回别的值会让
//!    "按 id 查"（`rect_of`/`invalidate`）与"按节点查"（`measure`/索引表）指向不同桶，
//!    表现是"某些节点永远不刷新"。现在统一走 `layout::key_of()`，契约违反在
//!    调试构建里直接断言失败。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle,
    UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 可挂子节点的容器；`boom` 为真时在 `measure` 里 panic。
struct Host {
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
}

impl Widget for Host {
    fn id(&self) -> &str {
        "root"
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        for ch in &mut self.children {
            cache.measure(ch.as_mut(), m, Constraints::loose(c.max));
        }
        c.constrain(c.max)
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = r;
        let mut y = r.y;
        for ch in &mut self.children {
            let size = cache.size_for_arrange(ch.as_mut(), m, Constraints::loose(r.size()));
            ch.arrange(Rect::new(r.x, y, size.w, size.h), m, cache);
            y += size.h;
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 叶子：宽度固定；`boom` 为真时 `measure` panic。
struct Leaf {
    id: String,
    width: f32,
    rect: Rect,
    boom: Rc<Cell<bool>>,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        if self.boom.get() {
            panic!("leaf measure 故意 panic");
        }
        c.constrain(Size::new(self.width, 10.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
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

fn leaf(id: &str, width: f32, boom: &Rc<Cell<bool>>) -> Box<dyn Widget> {
    Box::new(Leaf {
        id: id.to_string(),
        width,
        rect: Rect::ZERO,
        boom: boom.clone(),
    })
}

/// **回归**：控件在 measure 里 panic 之后，上一轮的 rect 索引必须还在。
#[test]
fn rect_of_survives_a_panicking_layout() {
    let boom = Rc::new(Cell::new(false));
    let mut tree = UiTree::new(Box::new(Host {
        children: vec![leaf("a", 10.0, &boom)],
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    let before = tree.rect_of("a").expect("前提：第一轮有 rect");
    assert_eq!(before.w, 10.0);

    // 让下一轮布局在 measure 里 panic
    boom.set(true);
    tree.mark_layout_dirty("a");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.layout(Size::new(200.0, 100.0), &Mono);
    }))
    .is_err();
    assert!(panicked, "前提：这一轮布局确实 panic 了");

    // **关键断言**：索引表没有被搬空
    assert_eq!(
        tree.rect_of("a"),
        Some(before),
        "布局 panic 后上一轮的 rect 应仍然可查（旧实现会永久空表）"
    );
}

/// **回归**：panic 之后再来一轮正常布局必须能自愈（不能留下半条祖先链）。
#[test]
fn layout_recovers_after_a_panic() {
    let boom = Rc::new(Cell::new(false));
    let mut tree = UiTree::new(Box::new(Host {
        children: vec![leaf("a", 10.0, &boom)],
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(tree.rect_of("a").map(|r| r.w), Some(10.0));

    boom.set(true);
    tree.mark_layout_dirty("a");
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tree.layout(Size::new(200.0, 100.0), &Mono);
    }));

    // 修好之后再布局：新尺寸必须生效，且不能被"跳过的快路径"骗过去
    boom.set(false);
    tree.mark_layout_dirty("a");
    // 换个宽度才能观察到重新布局（叶子宽度来自外部状态）
    tree.layout(Size::new(200.0, 100.0), &Mono);
    let stats = tree.layout_stats();
    assert!(
        !stats.skipped,
        "被标脏之后必须真的重新布局，而不是走 O(1) 跳过"
    );
    assert!(tree.rect_of("a").is_some(), "恢复后索引表必须重建");
}

/// **契约**：`cache_key()` 返回不是 `id_key(id)` 的值 ⇒ 调试构建里直接断言失败。
#[test]
#[should_panic(expected = "cache_key")]
fn custom_cache_key_is_rejected_loudly() {
    struct Liar {
        rect: Rect,
    }
    impl Widget for Liar {
        fn id(&self) -> &str {
            "liar"
        }
        fn cache_key(&self) -> Option<u64> {
            Some(0xDEAD_BEEF) // 不是 id_key("liar")
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(Size::new(10.0, 10.0))
        }
        fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
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

    let mut tree = UiTree::new(Box::new(Liar { rect: Rect::ZERO }));
    tree.layout(Size::new(100.0, 100.0), &Mono);
}
