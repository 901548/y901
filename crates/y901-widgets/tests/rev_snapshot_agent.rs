//! `public_tree` 在 10k 节点上的性能（防 O(n²) 回归）。

use std::time::Instant;

use y901_agent::api::{dispatch_request, Policy};
use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, Json, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
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
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 16.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
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
        Semantics::default().exposed(true).label(self.id.clone())
    }
}

fn tree_10k() -> UiTree {
    // 10 层 × 10 层 × 10 层 + 10 层叶子 = 11110 节点
    fn layer(prefix: &str, width: usize, depth: usize) -> Box<dyn Widget> {
        if depth == 0 {
            return Box::new(Leaf {
                id: format!("{prefix}_leaf"),
                rect: Rect::ZERO,
            });
        }
        let mut col = Flex::column(prefix);
        for i in 0..width {
            col = col.push(layer(&format!("{prefix}_{i}"), width, depth - 1));
        }
        Box::new(col)
    }
    let mut tree = UiTree::new(layer("n", 10, 4));
    tree.layout(Size::new(4000.0, 4000.0), &Mono);
    tree
}

#[test]
#[ignore = "性能测试：手动运行 -- --ignored --nocapture"]
fn public_tree_is_linear_on_10k_nodes() {
    let mut tree = tree_10k();
    let t = Instant::now();
    let out = dispatch_request(&mut tree, &Policy::default(), "get_tree", &Json::obj())
        .expect("get_tree");
    let elapsed = t.elapsed();
    let children = match out.get("children") {
        Some(Json::Arr(a)) => a.len(),
        _ => 0,
    };
    eprintln!("[perf] public_tree 10k: {elapsed:?} (top children={children})");
    assert!(
        elapsed.as_millis() < 200,
        "10k 节点 get_tree 应 < 200ms（O(n²) 会是秒级），实际 {elapsed:?}"
    );
}
