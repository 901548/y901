//! revision 框架实现（方案 1'）的证伪测试。
//!
//! **探针约定**：`ProbeWidget::revision()` 恒返回 0。
//! 这样测试里看到的非零 revision **必然来自框架的计数器**——
//! 如果框架的 bump 逻辑被删掉，测试会立刻变红。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Constraints, EventCtx, EventResult, Json, Rect, RenderCtx, Semantics, Signal, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 测试探针：状态可被测试或事件改变；`revision()` **恒为 0**。
struct Probe {
    id: String,
    label: String,
    disabled: bool,
    /// 只在 props 里出现，不进 semantics —— 模拟 hover。
    hovered: bool,
    /// 可选的数据源：模拟 TextLabel 绑定 Signal。
    signal: Option<Signal<String>>,
    rect: Rect,
}

impl Probe {
    fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            disabled: false,
            hovered: false,
            signal: None,
            rect: Rect::ZERO,
        }
    }

    fn bind(mut self, sig: Signal<String>) -> Self {
        self.label = sig.get();
        self.signal = Some(sig);
        self
    }

    fn current_label(&self) -> String {
        match &self.signal {
            Some(s) => s.get(),
            None => self.label.clone(),
        }
    }
}

impl Widget for Probe {
    fn id(&self) -> &str {
        &self.id
    }
    /// **恒为 0**：任何非零 revision 都只能来自框架计数。
    fn revision(&self) -> u64 {
        0
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::Action { name, .. } if name == "toggle_disabled" => {
                self.disabled = !self.disabled;
                EventResult::Redraw
            }
            UiEvent::Action { name, .. } if name == "relabel" => {
                self.label = format!("{}!", self.label);
                EventResult::Redraw
            }
            UiEvent::MouseMove { .. } => {
                // hover 只改内部字段，不碰 semantics
                self.hovered = true;
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.current_label())
            .disabled(self.disabled)
            // hover 只在 props：不进语义、不进哈希
            .prop("hovered", self.hovered.to_string())
    }
}

fn tree_with(probe: Probe) -> UiTree {
    let mut tree = UiTree::new(Box::new(probe));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    tree
}

fn action(name: &str) -> UiEvent {
    UiEvent::Action {
        name: name.into(),
        args: Json::obj(),
    }
}

// ---------- 1. dispatch 改 state → bump ----------

#[test]
fn dispatch_changes_state_bumps_revision() {
    let mut tree = tree_with(Probe::new("p", "A"));
    assert_eq!(tree.effective_revision("p"), 0, "初始应为 0");

    tree.dispatch_to("p", &action("toggle_disabled"));
    tree.layout(Size::new(200.0, 100.0), &Mono);

    assert!(
        tree.effective_revision("p") >= 1,
        "disabled 变化后 revision 必须增加（框架 bump）"
    );
}

// ---------- 2. Signal 改 label → bump ----------

#[test]
fn signal_changes_label_bumps_revision() {
    let sig = Signal::new("A".to_string());
    let probe = Probe::new("p", "A").bind(sig.clone());
    let mut tree = tree_with(probe);
    assert_eq!(tree.effective_revision("p"), 0);

    sig.subscribe("p", tree.invalidation());
    sig.set("B".to_string());
    tree.layout(Size::new(200.0, 100.0), &Mono);

    assert!(
        tree.effective_revision("p") >= 1,
        "Signal 驱动的 label 变化也必须 bump（不走 dispatch 的路径）"
    );
}

// ---------- 3. hover（只改 props）→ 不 bump ----------

#[test]
fn hover_does_not_bump_revision() {
    let mut tree = tree_with(Probe::new("p", "A"));

    let result = tree.dispatch_to(
        "p",
        &UiEvent::MouseMove {
            pos: y901_ui::Pos::new(10.0, 10.0),
        },
    );
    assert!(result.needs_redraw(), "hover 应要求重绘");
    tree.layout(Size::new(200.0, 100.0), &Mono);

    assert_eq!(
        tree.effective_revision("p"),
        0,
        "hover 只改 props，不应触发 revision（否则 Agent 会被 409 风暴淹没）"
    );
}

// ---------- 4. disabled 变化 → bump ----------

#[test]
fn disabled_change_bumps_revision() {
    let mut tree = tree_with(Probe::new("p", "A"));
    tree.dispatch_to("p", &action("toggle_disabled"));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert!(tree.effective_revision("p") >= 1);

    // 再翻回去，revision 必须继续增长且单调
    let after_first = tree.effective_revision("p");
    tree.dispatch_to("p", &action("toggle_disabled"));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert!(
        tree.effective_revision("p") > after_first,
        "revision 必须单调递增"
    );
}

// ---------- 5. 删节点清理 ----------

#[test]
fn remove_node_clears_last_semantic() {
    let root = y901_ui::Flex::column("root").push(Box::new(Probe::new("p", "A")));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &Mono);

    tree.dispatch_to("p", &action("toggle_disabled"));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert!(tree.effective_revision("p") >= 1);

    assert!(tree.remove_node("p"));

    // 同 id 重新添加：必须从 0 开始
    tree.add_node("root", Box::new(Probe::new("p", "B")));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(
        tree.effective_revision("p"),
        0,
        "删除后重建的同 id 节点应重置为 0"
    );
}

// ---------- 6. id 复用 + generation ----------

#[test]
fn id_reuse_with_different_generation_does_not_false_bump() {
    let root = y901_ui::Flex::column("root").push(Box::new(Probe::new("x", "A")));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert_eq!(tree.effective_revision("x"), 0);

    assert!(tree.remove_node("x"));
    // 同 id、不同内容、不同 generation
    tree.add_node("root", Box::new(Probe::new("x", "B")));
    tree.layout(Size::new(200.0, 100.0), &Mono);

    assert_eq!(
        tree.effective_revision("x"),
        0,
        "新实例不应继承旧实例的哈希比较（否则会假 bump）"
    );
    // 但新实例之后的变化仍要正常 bump
    tree.dispatch_to("x", &action("toggle_disabled"));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    assert!(tree.effective_revision("x") >= 1);
}

// 防止未使用告警的占位（Cell/Rc 用于未来扩展）
#[allow(dead_code)]
fn _unused(_: Rc<Cell<usize>>) {}
