//! 订阅的生命周期：**节点被删后，订阅必须在下一次 poll 被清掉**。
//!
//! 为何要这条测试：旧行为是 `poll_subscriptions` 遇到不存在的节点**跳过但保留**，
//! 于是动态创建/销毁带订阅的节点时 `subscriptions` 会**持续增长**（长期运行的内存泄漏）。

use std::time::{Duration, Instant};

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, UiEvent,
    UiTree, Widget,
};

/// 叶子节点：只占位，不做事。
struct Leaf {
    id: String,
    rect: Rect,
}

impl Leaf {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
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

/// 容器：**必须实现 `children_mut`**，否则 `remove_node` 无法摘子节点。
struct Container {
    id: String,
    rect: Rect,
    kids: Vec<Box<dyn Widget>>,
}

impl Container {
    fn new(id: &str, kids: Vec<Box<dyn Widget>>) -> Self {
        Self {
            id: id.to_string(),
            rect: Rect::ZERO,
            kids,
        }
    }
}

impl Widget for Container {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(200.0, 100.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
        for (i, k) in self.kids.iter_mut().enumerate() {
            k.arrange(
                Rect::new(rect.x, rect.y + i as f32 * 24.0, rect.w, 20.0),
                _m,
                _cache,
            );
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 只用于 layout 的度量实现（不画字）。
struct Dummy;

impl TextMetrics for Dummy {
    fn measure_text(&self, text: &str, _style: &y901_ui::TextStyle) -> Size {
        // 固定字号：本测试只需布局能算出尺寸，不关心真实字体。
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _style: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn tree_with_child() -> UiTree {
    let root = Container::new("root", vec![Box::new(Leaf::new("a"))]);
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &Dummy);
    tree
}

#[test]
fn subscription_of_removed_node_is_cleaned_up_on_next_poll() {
    let mut tree = tree_with_child();
    assert!(
        tree.subscribe("a", Duration::from_millis(16)),
        "存在的节点应能订阅"
    );
    assert_eq!(tree.subscription_count(), 1);

    assert!(tree.remove_node("a"), "摘除 a 应成功");

    // **懒清理设计**：remove_node 不做立即清理（它不知道子树里哪些 id 有订阅，
    // 而 poll 本来就要逐个查节点）。所以这里**仍应为 1**。
    assert_eq!(
        tree.subscription_count(),
        1,
        "remove_node 不做立即清理（懒清理设计）"
    );

    // 一次 poll 就应把它清掉。
    let _ = tree.poll_subscriptions(Instant::now() + Duration::from_secs(1));

    assert_eq!(
        tree.subscription_count(),
        0,
        "节点已被删的订阅必须在下一次 poll 被剔除（否则就是泄漏）"
    );
}

#[test]
fn live_subscriptions_survive_poll() {
    // 对照：节点还在时，订阅**不能**被误删。
    let mut tree = tree_with_child();
    assert!(tree.subscribe("a", Duration::from_millis(16)));

    let _ = tree.poll_subscriptions(Instant::now() + Duration::from_secs(1));
    assert_eq!(tree.subscription_count(), 1, "节点存在时，订阅必须保留");

    let _ = tree.poll_subscriptions(Instant::now() + Duration::from_secs(2));
    assert_eq!(tree.subscription_count(), 1, "重复 poll 也不应删掉有效订阅");
}
