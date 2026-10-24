//! Spike：测量 `semantics()` → 语义面 JSON 的成本，以及序列化确定性。
//!
//! 只服务设计决策，不进产品路径。两种跑法：
//! ```text
//! cargo run --release -p y901-ui --example hash_bench
//! cargo test --release -p y901-ui --example hash_bench -- --ignored --nocapture
//! ```

use std::time::Instant;

use y901_ui::{
    Constraints, EventCtx, EventResult, Json, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiTree, Widget,
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

/// 带一点"真实"语义的节点：label / value / state / actions。
struct Node {
    id: String,
    rect: Rect,
    children: Vec<Box<dyn Widget>>,
    n: u32,
}

impl Widget for Node {
    fn id(&self) -> &str {
        &self.id
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
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
        Semantics::default()
            .label(format!("node-{}", self.id))
            .value(format!("v{}", self.n))
            .action_spec("set", y901_ui::InputKind::Text)
            .state("n", self.n)
            .state("flag", self.n.is_multiple_of(2))
            .exposed(true)
    }
}

/// 100 × 100 + 1 ≈ 10k 的树。
fn big_tree() -> UiTree {
    let mut root = Node {
        id: "root".into(),
        rect: Rect::ZERO,
        children: Vec::new(),
        n: 0,
    };
    for i in 0..100 {
        let mut mid = Node {
            id: format!("mid{i}"),
            rect: Rect::ZERO,
            children: Vec::new(),
            n: i,
        };
        for j in 0..100 {
            mid.children.push(Box::new(Node {
                id: format!("leaf{i}_{j}"),
                rect: Rect::ZERO,
                children: Vec::new(),
                n: j,
            }));
        }
        root.children.push(Box::new(mid));
    }
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(1000.0, 1000.0), &Mono);
    tree
}

/// 框架将要对"语义面"做哈希的候选实现：把这些字段拼成 Json 再序列化。
/// 这就是被测量的对象（`Semantics` 本身没有 Display）。
fn semantic_surface(sem: &Semantics) -> String {
    Json::obj()
        .set("label", sem.label.clone())
        .set("value", sem.value.clone())
        .set(
            "actions",
            Json::Arr(sem.actions.iter().map(|a| a.to_json()).collect()),
        )
        .set("state", sem.state.clone())
        .set("disabled", sem.disabled)
        .set("sensitive", sem.sensitive)
        .set("exposed", sem.exposed_to_ai)
        .to_string()
}

fn run_bench() {
    let tree = big_tree();
    let ids = ["leaf7_7", "leaf42_42", "leaf99_99"];

    for id in ids {
        if let Some(n) = tree.find(id) {
            let _ = semantic_surface(&n.semantics());
        }
    }

    let iters = 1000u32;
    let t0 = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        for id in ids {
            if let Some(node) = tree.find(id) {
                sink += semantic_surface(&node.semantics()).len();
            }
        }
    }
    let elapsed = t0.elapsed();
    let per_frame_us = elapsed.as_secs_f64() * 1_000_000.0 / iters as f64;
    eprintln!(
        "[bench] 3 nodes x {iters} frames: total {elapsed:?}, per-frame {per_frame_us:.1} us, sink={sink}"
    );

    let one = tree.find("leaf7_7").expect("node");
    let t1 = Instant::now();
    for _ in 0..10_000 {
        let _ = semantic_surface(&one.semantics());
    }
    let d = t1.elapsed();
    eprintln!(
        "[bench] 1 node x 10000: {d:?} ({:.2} us/次)",
        d.as_secs_f64() * 1e6 / 10_000.0
    );

    assert!(
        per_frame_us < 500.0,
        "单帧 3 节点哈希应 < 0.5ms（500us），实测 {per_frame_us:.1}us"
    );
}

fn check_determinism() {
    let tree = big_tree();
    let node = tree.find("leaf3_3").expect("node");
    let first = semantic_surface(&node.semantics());
    for i in 0..100 {
        assert_eq!(
            first,
            semantic_surface(&node.semantics()),
            "第 {i} 次序列化与首次不一致"
        );
    }
    let a = Json::obj().set("b", 1u32).set("a", 2u32).to_string();
    let b = Json::obj().set("b", 1u32).set("a", 2u32).to_string();
    assert_eq!(a, b, "Json 对象键序必须稳定（当前是插入序 Vec）");
    eprintln!("[bench] deterministic ok: {first}");
}

fn main() {
    run_bench();
    check_determinism();
}

#[test]
#[ignore = "spike：需要 release 构建，手动运行"]
fn bench_hash() {
    run_bench();
}

#[test]
#[ignore = "spike：需要 release 构建，手动运行"]
fn serialization_is_deterministic() {
    check_determinism();
}
