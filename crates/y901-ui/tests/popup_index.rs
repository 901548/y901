//! **弹出层内部控件也必须能被 `rect_of` / `invalidate` 找到**。
//!
//! 浮层不在 Widget 树里（`paint_order`、`find`、`find_path` 都走不到），
//! 但 `rect_of(id)` 与 `invalidate(id)` 是按**索引表**查的。索引里没有它们时：
//!
//! - `rect_of("<浮层里的控件>")` 返回 `None`（自动化/测试拿不到几何）；
//! - `invalidate("<浮层里的控件>")` **静默 no-op** —— 局部重绘下这个控件永远不刷新，
//!   只能靠无关的整窗重绘偶然救回来；
//! - 浮层被重新摆放时 `mark_moved_map` 看不到它的位移，旧位置留残影。
//!
//! 现在浮层子树的 rect 也进索引表（**但不进 `paint_order`**：命中测试与路径重建
//! 依赖树路径，浮层路径不在树里，混进去会让 `hit_index`/`path_of` 对不上号）。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
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

/// 记录 `render` 次数的叶子，用来断言"局部重绘确实重画了它"。
struct Leaf {
    id: &'static str,
    size: Size,
    rect: Rect,
    paints: Rc<Cell<usize>>,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(self.size)
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
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        self.paints.set(self.paints.get() + 1);
        ctx.fill_rect(self.rect, Color::WHITE);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 只摊开子节点的容器。
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

fn leaf(id: &'static str, w: f32, h: f32, paints: &Rc<Cell<usize>>) -> Box<dyn Widget> {
    Box::new(Leaf {
        id,
        size: Size::new(w, h),
        rect: Rect::ZERO,
        paints: paints.clone(),
    })
}

fn tree() -> UiTree {
    UiTree::new(Box::new(Host {
        children: vec![],
        rect: Rect::ZERO,
    }))
}

/// 浮层里的控件：`rect_of` 必须能查到，且矩形与浮层实际摆放一致。
#[test]
fn popup_children_are_indexed_for_rect_of() {
    let paints = Rc::new(Cell::new(0));
    let mut t = tree();
    t.layout(Size::new(300.0, 200.0), &Mono);
    assert!(
        t.open_popup(
            Box::new(Host {
                children: vec![leaf("inner", 40.0, 20.0, &paints)],
                rect: Rect::ZERO,
            }),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ),
        "前提：浮层应能打开"
    );
    t.layout(Size::new(300.0, 200.0), &Mono);

    let r = t.rect_of("inner").expect("浮层里的控件应能查到 rect");
    assert!(r.w > 0.0 && r.h > 0.0, "rect 应为有效尺寸：{r:?}");
}

/// 对浮层内控件调 `invalidate`：必须产生**局部**脏区（而不是静默 no-op）。
#[test]
fn invalidate_inside_popup_produces_a_dirty_rect() {
    let paints = Rc::new(Cell::new(0));
    let mut t = tree();
    t.layout(Size::new(300.0, 200.0), &Mono);
    t.open_popup(
        Box::new(Host {
            children: vec![leaf("inner", 40.0, 20.0, &paints)],
            rect: Rect::ZERO,
        }),
        Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    t.layout(Size::new(300.0, 200.0), &Mono);

    // 先渲染一帧把脏区清空
    struct Sink;
    impl TextMetrics for Sink {
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
    impl RenderCtx for Sink {
        fn size(&self) -> Size {
            Size::new(300.0, 200.0)
        }
        fn fill_rect(&mut self, _r: Rect, _c: Color) {}
        fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
        fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
            0.0
        }
        fn push_clip(&mut self, _r: Rect) {}
        fn pop_clip(&mut self) {}
    }
    t.render(&mut Sink);
    assert!(t.dirty().is_empty(), "前提：渲染后脏区应清空");

    t.invalidate("inner");
    let rects = t.dirty().rects();
    assert!(
        !rects.is_empty(),
        "对浮层内控件 invalidate 必须产生脏区（旧实现静默 no-op）"
    );
    assert!(
        rects.iter().any(|r| r.w > 0.0 && r.h > 0.0),
        "脏区应有有效尺寸：{rects:?}"
    );
}
