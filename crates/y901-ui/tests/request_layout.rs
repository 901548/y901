//! `UiTree::request_layout` 的**契约测试**。
//!
//! 为什么需要它：结构性变更（增删节点 / 切换布局模式）**不会**自动触发布局 ——
//! `mark_layout_dirty` 只清布局缓存、`invalidate*` 只标脏区，两者都**不**置
//! `layout_requested`。若调用方忘了请求布局，新子节点会**从未被 arrange**，
//! 表现成"控件画在了旧位置/别的列上"。
//!
//! 契约：`request_layout()` 后 `take_layout_request()` 返回 `true`；`take` 是消耗式的。

use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, UiEvent,
    UiTree, Widget,
};

/// 最小控件：只占位，不绘制。
struct Leaf {
    id: &'static str,
    rect: Rect,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(10.0, 10.0))
    }
    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn leaf(id: &'static str) -> Box<dyn Widget> {
    Box::new(Leaf {
        id,
        rect: Rect::ZERO,
    })
}

#[test]
fn request_layout_sets_flag_and_take_consumes_it() {
    let mut tree = UiTree::new(leaf("root"));
    // 构造时可能已请求过一次布局：先取走并归零，确保下面的断言只反映我们这次调用。
    let _ = tree.take_layout_request();
    assert!(!tree.take_layout_request(), "取走之后标志必须是 false");

    tree.request_layout();
    assert!(
        tree.take_layout_request(),
        "request_layout() 之后必须能取到 true"
    );
    assert!(
        !tree.take_layout_request(),
        "take_layout_request 是消耗式的：第二次必须为 false（否则主循环会每帧重排）"
    );
}

#[test]
fn request_layout_is_idempotent_within_one_frame() {
    let mut tree = UiTree::new(leaf("root"));
    let _ = tree.take_layout_request();
    // 多次请求应当合并成一次（标志是 bool，不是计数器）。
    tree.request_layout();
    tree.request_layout();
    tree.request_layout();
    assert!(tree.take_layout_request(), "多次请求仍应只体现为一次");
    assert!(!tree.take_layout_request(), "取走后归零");
}
