//! **id 复用不得继承旧实例的布局缓存**。
//!
//! 背景：`LayoutCache` 的键是 `(id_key, constraints, layout_version)`。
//! 新实例的 `layout_version()` 从 0 开始，与旧实例相同；若视口没变、约束相同，
//! 命中缓存就会**直接返回旧实例的尺寸、连 `measure` 都不调** ——
//! 于是"删掉一行再用同一个 id 建一个新的"会画出旧尺寸（内容更长却被截断，
//! 或容器内部状态没初始化，如 `ScrollView` 根本没跑过 measure ⇒ 滚不动）。
//!
//! `remove_node` 已经在 `forget_recursive` 里清了 revision 相关的 map，
//! 这里确保**布局缓存也在同一批清理里**。

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

/// 自报宽度的叶子：宽度来自外部 `Cell`，`measure` 是否被调用会被计数。
struct Probe {
    id: String,
    width: Rc<Cell<f32>>,
    measured: Rc<Cell<u32>>,
    rect: Rect,
}

impl Widget for Probe {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        self.measured.set(self.measured.get() + 1);
        c.constrain(Size::new(self.width.get(), 10.0))
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

/// 容器：只摊开子节点（交给 `Flex` 会引入额外变量，这里直接用最简单的自定义容器）。
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

fn probe(id: &str, width: f32, measured: Rc<Cell<u32>>) -> Box<dyn Widget> {
    Box::new(Probe {
        id: id.to_string(),
        width: Rc::new(Cell::new(width)),
        measured,
        rect: Rect::ZERO,
    })
}

/// 删掉一个节点，再用**同一个 id** 建一个新实例（宽度不同）：
/// 新实例必须被真正 `measure`，矩形宽度必须是新值。
#[test]
fn readded_node_with_same_id_is_measured_again() {
    let measured = Rc::new(Cell::new(0u32));
    let mut tree = UiTree::new(Box::new(Host {
        children: vec![probe("x", 10.0, measured.clone())],
        rect: Rect::ZERO,
    }));

    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(
        tree.rect_of("x").map(|r| r.w),
        Some(10.0),
        "前置：第一次布局宽度应为 10"
    );

    // 同一个 id 换一个新实例（宽度 50）
    assert!(tree.remove_node("x"), "应能摘掉 x");
    let measured2 = Rc::new(Cell::new(0u32));
    assert!(tree.add_node("root", probe("x", 50.0, measured2.clone())));

    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(
        tree.rect_of("x").map(|r| r.w),
        Some(50.0),
        "新实例必须是新宽度（若为 10 说明命中了旧实例的布局缓存）"
    );
    assert!(
        measured2.get() > 0,
        "新实例的 measure 必须被调用过（命中缓存时会跳过）"
    );
}

/// 对照：**不删直接改**同 id 的实例状态时，只要 version 变了也必须重测。
/// （这条本来就该通过，放在这里是为了区分"缓存太松"与"缓存太紧"。）
#[test]
fn version_bump_still_invalidates() {
    struct Versioned {
        id: &'static str,
        version: Rc<Cell<u64>>,
        rect: Rect,
    }
    impl Widget for Versioned {
        fn id(&self) -> &str {
            self.id
        }
        fn layout_version(&self) -> u64 {
            self.version.get()
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            let w = if self.version.get() == 0 { 10.0 } else { 50.0 };
            c.constrain(Size::new(w, 10.0))
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

    let version = Rc::new(Cell::new(0u64));
    let mut tree = UiTree::new(Box::new(Host {
        children: vec![Box::new(Versioned {
            id: "v",
            version: version.clone(),
            rect: Rect::ZERO,
        })],
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(tree.rect_of("v").map(|r| r.w), Some(10.0));

    version.set(1);
    tree.mark_layout_dirty("v");
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(
        tree.rect_of("v").map(|r| r.w),
        Some(50.0),
        "version 变了必须重测"
    );
}
