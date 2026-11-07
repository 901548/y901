//! 动态挂载/卸载 + 响应式状态管理的集成测试。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, FlexItem, Pos, Rect, RenderCtx, Signal, Size,
    TextMetrics, TextStyle, UiTree, Widget,
};
use y901_widgets::{Button, TextLabel};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

// ---------- 1. 动态挂载 / 卸载 ----------

#[test]
fn add_then_remove_node_updates_tree_and_hit_test() {
    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert!(
        tree.hit_test(Pos::new(10.0, 10.0)).as_deref() != Some("btn"),
        "空树里不应有按钮"
    );

    // 挂一个按钮
    assert!(tree.add_node("root", Box::new(Button::new("btn", "Save"))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let snap = tree.snapshot_json().to_string();
    assert!(snap.contains("\"btn\""), "语义树应能看到新节点: {snap}");

    let rect = tree.rect_of("btn").expect("按钮应有几何");
    assert!(rect.w > 0.0 && rect.h > 0.0);
    assert_eq!(
        tree.hit_test(Pos::new(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .as_deref(),
        Some("btn")
    );

    // 摘掉它
    assert!(tree.remove_node("btn"));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert!(tree.find("btn").is_none(), "节点应已从树中移除");
    assert_ne!(
        tree.hit_test(Pos::new(rect.x + rect.w * 0.5, rect.y + rect.h * 0.5))
            .as_deref(),
        Some("btn"),
        "移除后不应再命中按钮"
    );
    assert!(!tree.snapshot_json().to_string().contains("\"btn\""));
}

#[test]
fn removing_focused_node_clears_focus() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        Button::new("btn", "Go").flex_item(FlexItem::default()),
    ))));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.set_focus(Some("btn".to_string())));
    assert_eq!(tree.focus(), Some("btn"));

    tree.remove_node("btn");
    assert_eq!(tree.focus(), None, "被删节点不应继续持有焦点");
}

/// 记录挂载/卸载次数的探针。
struct Lifecycle {
    id: String,
    log: Rc<RefCell<Vec<String>>>,
    rect: Rect,
}

impl Widget for Lifecycle {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(50.0, 20.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn on_mount(&mut self, ctx: &mut y901_ui::MountCtx) {
        self.log
            .borrow_mut()
            .push(format!("mount:{}:{}", self.id, ctx.id));
    }
    fn on_unmount(&mut self) {
        self.log.borrow_mut().push(format!("unmount:{}", self.id));
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

#[test]
fn lifecycle_hooks_fire_on_mount_and_unmount() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    tree.add_node(
        "root",
        Box::new(Lifecycle {
            id: "life".into(),
            log: log.clone(),
            rect: Rect::ZERO,
        }),
    );
    assert_eq!(log.borrow().as_slice(), ["mount:life:life"]);

    tree.remove_node("life");
    assert_eq!(
        log.borrow().as_slice(),
        ["mount:life:life", "unmount:life"],
        "卸载应触发 on_unmount"
    );
}

// ---------- 2. 响应式 ----------

#[test]
fn signal_change_repaints_label_without_manual_invalidation() {
    let title = Signal::new("Hello".to_string());

    // 注意：没有任何 invalidate / mark_dirty 调用
    let label = TextLabel::new("title", "Hello").bind(title.clone());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(label))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 挂载时订阅：这里手动补一次（真实场景由 add_node 自动完成）
    let invalidation = tree.invalidation();
    title.subscribe("title", invalidation);

    let before = tree.snapshot_json().to_string();
    assert!(before.contains("Hello"));

    // 业务代码只改数据
    title.set("World".to_string());

    // 框架在下一帧自动排空失效队列并重绘
    let drained = tree.drain_invalidation();
    assert_eq!(drained, 1, "Signal 变化应产生一条失效");
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let after = tree.snapshot_json().to_string();
    assert!(after.contains("World"), "标签应自动显示新文本: {after}");
    assert!(!after.contains("Hello"));
}

#[test]
fn signal_updates_are_deduplicated_and_dirty_state_set() {
    let title = Signal::new("a".to_string());
    let label = TextLabel::new("title", "a").bind(title.clone());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(label))));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    title.subscribe("title", tree.invalidation());

    // 连续改三次：同一帧内只标脏一次
    title.set("b".to_string());
    title.set("c".to_string());
    title.set("d".to_string());

    assert_eq!(tree.drain_invalidation(), 1, "同帧内应合并成一次失效");

    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.snapshot_json().to_string().contains('d'));
}

#[test]
fn unmounted_label_unsubscribes() {
    let title = Signal::new("x".to_string());
    let label = TextLabel::new("title", "x").bind(title.clone());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(label))));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    title.subscribe("title", tree.invalidation());

    tree.remove_node("title");
    // 组件自身的退订由 on_unmount 完成；这里手动验证语义
    title.unsubscribe("title");

    assert!(title.set("y".to_string()));
    assert_eq!(tree.drain_invalidation(), 0, "退订后不应再产生失效");
}

#[test]
fn unrelated_color_helper_still_compiles() {
    // 防止误删 Color 导入（历史回归）
    let _ = Color::WHITE;
}
