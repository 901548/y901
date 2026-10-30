//! `rev_snapshot` 的正确性与性能。
//!
//! 正确性：快照必须与逐个 `effective_revision` 的结果**完全一致**。
//! 性能：一次遍历 O(n)，10k 节点应在毫秒级；O(n²) 会是秒级。

use std::time::Instant;

use y901_ui::{
    Constraints, EventCtx, EventResult, Flex, Json, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, UiTree, Widget,
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

/// 能让 revision 变化的探针：`bump` 动作会让自身 revision +1。
struct Probe {
    id: String,
    rect: Rect,
    rev: u64,
}

impl Widget for Probe {
    fn id(&self) -> &str {
        &self.id
    }
    fn revision(&self) -> u64 {
        self.rev
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
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::Action { name, .. } if name == "bump" => {
                self.rev += 1;
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true).action("bump")
    }
}

/// 构造 `width × depth` 的树，节点名 `n{i}_{j}`。
fn build(width: usize, depth: usize) -> UiTree {
    fn layer(prefix: &str, width: usize, depth: usize) -> Box<dyn Widget> {
        if depth == 0 {
            return Box::new(Probe {
                id: format!("{prefix}_leaf"),
                rect: Rect::ZERO,
                rev: 0,
            });
        }
        let mut col = Flex::column(prefix);
        for i in 0..width {
            col = col.push(layer(&format!("{prefix}_{i}"), width, depth - 1));
        }
        Box::new(col)
    }
    let mut tree = UiTree::new(layer("n", width, depth));
    tree.layout(Size::new(4000.0, 4000.0), &Mono);
    tree
}

fn all_ids(tree: &UiTree) -> Vec<String> {
    fn walk(n: &dyn Widget, out: &mut Vec<String>) {
        out.push(n.id().to_string());
        for c in n.children() {
            walk(c.as_ref(), out);
        }
    }
    let mut out = Vec::new();
    walk(tree.root(), &mut out);
    out
}

/// 正确性：快照与逐个 `effective_revision` 完全一致（含 bump 之后）。
#[test]
fn rev_snapshot_matches_effective_revision() {
    let mut tree = build(3, 2);
    // 制造一些非零计数：bump 两个叶子
    let ids = all_ids(&tree);
    for id in ids.iter().filter(|id| id.ends_with("_leaf")).take(2) {
        tree.dispatch_to(
            id,
            &UiEvent::Action {
                name: "bump".into(),
                args: Json::obj(),
            },
        );
    }
    tree.layout(Size::new(4000.0, 4000.0), &Mono);

    let snapshot = tree.rev_snapshot();
    assert_eq!(snapshot.len(), ids.len(), "快照必须覆盖所有节点");
    for id in &ids {
        assert_eq!(
            snapshot.get(id).copied(),
            Some(tree.effective_revision(id)),
            "节点 {id} 的快照值与逐个查询不一致"
        );
    }
    assert!(
        snapshot.values().any(|v| *v > 0),
        "前置：至少有一个节点 revision > 0（否则测试无效）"
    );
}

/// 性能：10k 节点快照必须远快于 O(n²)（后者是秒级）。
#[test]
#[ignore = "性能测试：手动运行 cargo test -p y901-ui --test rev_snapshot -- --ignored --nocapture"]
fn rev_snapshot_is_linear_on_10k_nodes() {
    // width^depth 个叶子：10^4 = 10k（注意 100^3 是 100 万，会挂起）
    let tree = build(10, 4);
    let n = all_ids(&tree).len();
    let t = Instant::now();
    let snapshot = tree.rev_snapshot();
    let elapsed = t.elapsed();
    eprintln!("[perf] rev_snapshot: {n} nodes in {elapsed:?}");
    assert_eq!(snapshot.len(), n);
    assert!(
        elapsed.as_millis() < 10,
        "10k 节点快照应 < 10ms（O(n²) 会是秒级），实际 {elapsed:?}"
    );
}
