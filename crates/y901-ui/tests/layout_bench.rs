//! 布局引擎基准（`#[ignore]`，手动跑）。
//!
//! 目的：给 `LayoutCache` 的键类型重构（String → u64）提供可对比的数字。
//! 用法：`cargo test -p y901-ui --release --test layout_bench -- --ignored --nocapture`

use std::time::Instant;

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, FlexItem, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiTree, Widget,
};

fn count_nodes(n: &dyn Widget) -> usize {
    1 + n
        .children()
        .iter()
        .map(|c| count_nodes(c.as_ref()))
        .sum::<usize>()
}

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
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut y901_ui::layout::LayoutCache) {
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

/// width^depth 个叶子（10^4 = 10k）。
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

#[test]
#[ignore = "基准测试：cargo test -p y901-ui --release --test layout_bench -- --ignored --nocapture"]
fn layout_10k_full_relayout() {
    let mut tree = build(10, 4);
    let nodes = count_nodes(tree.root());
    let rounds = 20;
    let mut total = 0f64;
    let mut worst = 0f64;
    for i in 0..rounds {
        // 交替两个视口尺寸：等价于窗口拉伸时"视口变了 → 全树重排"
        let w = if i % 2 == 0 { 2000.0 } else { 2001.0 };
        let t = Instant::now();
        tree.layout(Size::new(w, 2000.0), &Mono);
        let us = t.elapsed().as_secs_f64() * 1e6;
        total += us;
        if us > worst {
            worst = us;
        }
    }
    let avg = total / rounds as f64;
    eprintln!("[bench] nodes={nodes} rounds={rounds} avg={avg:.1}us worst={worst:.1}us");
    assert!(avg > 0.0);
}
