//! 复现：`cursor_at` 与 `hit_index` 在跨层重叠时的语义差异。
//!
//! **这是一个待修复 bug 的复现测试，当前 `#[ignore]`。**
//!
//! - `hit_index` 扫 `paint_order`（按 layer 稳定排序：Base → Overlay → Modal），
//!   用 `rposition` 取最后一个匹配 → 命中**最上层**节点。
//! - `cursor_at` 递归 `children()`（纯树序），后访问的覆盖先访问的 → 命中
//!   **树序最后**的节点，与层级无关。
//!
//! 当树里 Overlay 节点排在 Base 节点**之前**时，两者结论相反。

use y901_ui::{
    Constraints, Cursor, EventCtx, EventResult, Layer, Pos, Rect, RenderCtx, Semantics, Size,
    TextStyle, TextMetrics, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::new(8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 固定矩形 + 固定光标的叶子。
struct Probe {
    id: &'static str,
    rect: Rect,
    layer: Layer,
    cursor: Cursor,
}

impl Widget for Probe {
    fn id(&self) -> &str {
        self.id
    }
    fn layer(&self) -> Layer {
        self.layer
    }
    fn cursor(&self) -> Cursor {
        self.cursor
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.max
    }
    fn arrange(&mut self, _r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {}
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

/// 最小容器：按给定顺序持有子节点。
struct Col {
    kids: Vec<Box<dyn Widget>>,
    rect: Rect,
}

impl Widget for Col {
    fn id(&self) -> &str {
        "root"
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.max
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        let kids = std::mem::take(&mut self.kids);
        for mut k in kids {
            k.arrange(r, _m, _c);
            self.kids.push(k);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
}

fn tree_overlay_first() -> UiTree {
    // 注意顺序：Overlay 在**前**，Base 在**后**（两者都覆盖 (10,10)）
    let root = Col {
        kids: vec![
            Box::new(Probe {
                id: "overlay",
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
                layer: Layer::Overlay,
                cursor: Cursor::Pointer,
            }),
            Box::new(Probe {
                id: "base",
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
                layer: Layer::Base,
                cursor: Cursor::Text,
            }),
        ],
        rect: Rect::ZERO,
    };
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(120.0, 120.0), &Mono);
    tree
}

#[test]
#[ignore = "已知 bug：cursor_at 不看 layer，跨层重叠时与 hit_index 结论相反；修复后移除 ignore"]
fn cursor_at_matches_hit_index_across_layers() {
    let tree = tree_overlay_first();
    let pos = Pos::new(10.0, 10.0);

    let hit = tree.hit_test(pos).expect("应命中");
    let cursor = tree.cursor_at(pos);

    eprintln!("[repro] hit_test = {hit}, cursor_at = {cursor:?}");

    assert_eq!(
        hit, "overlay",
        "前置：paint_order 按 layer 排序后，Overlay 应在 Base 之上"
    );
    assert_eq!(
        cursor,
        Cursor::Pointer,
        "光标应来自最上层（overlay），实际 {cursor:?}"
    );
}
