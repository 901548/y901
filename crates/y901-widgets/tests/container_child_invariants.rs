//! **结构性容器的子节点不变量：外部增删必须干净失败，不能 panic / 静默无效**。
//!
//! `Scroll` 的模型是"恰好一个内容节点"、`Split` 是"恰好两个面板"。
//! 但 `UiTree::remove_node` / `add_node` 只要能拿到 `children_mut()` 就会动手：
//! * 删掉唯一/第一个子节点 → 下一次 `measure`/`arrange` 里 `kids[0]` 越界 panic；
//! * 加一个子节点 → 它永远不被布局（静默无效，用户以为加上了）。
//!
//! 所以这两个容器对"外部可变子节点"返回 `None`（接口干净失败），
//! 只对框架内部的**只读遍历**暴露借用。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Axis, Constraints, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree, Widget,
};
use y901_widgets::ScrollView;
use y901_widgets::SplitView;

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
    size: Size,
    rect: Rect,
}

impl Leaf {
    fn new(id: &str, w: f32, h: f32) -> Self {
        Self {
            id: id.to_string(),
            size: Size::new(w, h),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(self.size)
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn scroll_refuses_to_lose_its_content_child() {
    let scroll = ScrollView::new("s", Box::new(Leaf::new("content", 100.0, 400.0)));
    let mut tree = UiTree::new(Box::new(scroll));
    let viewport = Size::new(300.0, 200.0);
    tree.layout(viewport, &Mono);

    // 结构性容器：外部不得摘掉内容节点（返回 false，而不是删掉）。
    assert!(
        !tree.remove_node("content"),
        "Scroll 必须拒绝摘掉唯一内容节点（否则下一次布局会 panic）"
    );

    // 再布局一次：不得 panic，内容仍在。
    tree.layout(Size::new(300.0, 210.0), &Mono);
    assert!(tree.rect_of("content").is_some(), "内容节点应仍然在树里");
}

#[test]
fn split_refuses_extra_children_and_keeps_layout_sane() {
    let split = SplitView::new(
        "sp",
        Axis::Horizontal,
        Box::new(Leaf::new("left", 100.0, 100.0)),
        Box::new(Leaf::new("right", 100.0, 100.0)),
    );
    let mut tree = UiTree::new(Box::new(split));
    let viewport = Size::new(600.0, 400.0);
    tree.layout(viewport, &Mono);

    assert!(
        !tree.remove_node("left"),
        "Split 必须拒绝摘掉面板（否则 arrange 会越界）"
    );
    tree.layout(Size::new(620.0, 420.0), &Mono);
    assert!(tree.rect_of("left").is_some());
    assert!(tree.rect_of("right").is_some());
}

#[test]
fn scroll_with_content_still_lays_out_and_scrolls() {
    // 反向证据：拒绝外部增删之后，正常路径必须仍然工作（内容确实被排布）。
    let scroll = ScrollView::new("s", Box::new(Leaf::new("content", 100.0, 400.0)));
    let mut tree = UiTree::new(Box::new(scroll));
    tree.layout(Size::new(300.0, 200.0), &Mono);
    let r = tree.rect_of("content").expect("内容应被布局");
    assert!(
        r.h >= 400.0 - 0.01,
        "内容应按自身高度排布（实际 h={}）",
        r.h
    );
    // 内容比视口高 → 出现滚动条 → 内容宽度 = 视口 - 滚动条宽（不是铺满）。
    assert!(
        r.w > 0.0 && r.w < 300.0,
        "出现滚动条时内容应窄于视口（实际 w={}）",
        r.w
    );
}
