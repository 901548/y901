//! `Flex::gap` 家族的后设置者优先契约。
//!
//! 契约：**后设置的生效**。`gap()` / `gap_scaled()` / `gap_fixed()` 三者互斥。
//!
//! DPI 语义（`docs/design/dpi-policy.md`）：`gap()` 现在是 **DIP**，实际值 × `metrics.scale()`；
//! `gap_scaled()` 与它等价（保留为兼容别名）；`gap_fixed()` 才是"不缩放的物理像素"。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, Edges, EventCtx, EventResult, Flex, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, Widget,
};

/// 等宽的假文字度量：每个字符 8px，行高 16px；`scale` 可配（用于验证 ×scale）。
struct Metrics {
    scale: f32,
}

impl TextMetrics for Metrics {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        self.scale
    }
}

/// 固定 10×10 的叶子，位置由父容器决定。
struct Leaf {
    id: &'static str,
    rect: Rect,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(10.0, 10.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
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

/// 布局一个 row，返回两个子项的 x。
fn child_xs(flex: &mut Flex, metrics: &dyn TextMetrics) -> (f32, f32) {
    let mut cache = LayoutCache::default();
    let constraints = Constraints::loose(Size::new(200.0, 100.0));
    let size = flex.measure(metrics, constraints, &mut cache);
    flex.arrange(Rect::new(0.0, 0.0, size.w, size.h), metrics, &mut cache);
    let kids = flex.children();
    (kids[0].rect().x, kids[1].rect().x)
}

fn row() -> Flex {
    Flex::row("t")
        .padding(Edges::ZERO)
        .push(Box::new(Leaf {
            id: "a",
            rect: Rect::ZERO,
        }))
        .push(Box::new(Leaf {
            id: "b",
            rect: Rect::ZERO,
        }))
}

/// `.gap_scaled(8).gap(4)` → 用 4 × scale = 5（后设置者生效）。
#[test]
fn gap_scaled_then_gap_uses_gap() {
    let metrics = Metrics { scale: 1.25 };
    let mut flex = row().gap_scaled(8.0).gap(4.0);
    let (x0, x1) = child_xs(&mut flex, &metrics);
    let gap = x1 - (x0 + 10.0);
    assert!(
        (gap - 5.0).abs() < 0.01,
        "expected 4.0 * 1.25 = 5.0 (later .gap wins, DIP), got {gap}"
    );
}

/// `.gap(4).gap_scaled(8)` → 用 8 × scale（后设置者生效）。
#[test]
fn gap_then_gap_scaled_uses_scaled() {
    let metrics = Metrics { scale: 1.25 };
    let mut flex = row().gap(4.0).gap_scaled(8.0);
    let (x0, x1) = child_xs(&mut flex, &metrics);
    let gap = x1 - (x0 + 10.0);
    assert!(
        (gap - 10.0).abs() < 0.01,
        "expected gap 8.0*1.25 = 10.0 (later .gap_scaled wins), got {gap}"
    );
}

/// 对照 1：`gap(4)` 是 DIP —— 125% 下实际 5.0。
#[test]
fn plain_gap_scales_with_dpi() {
    let metrics = Metrics { scale: 1.25 };
    let mut flex = row().gap(4.0);
    let (x0, x1) = child_xs(&mut flex, &metrics);
    let gap = x1 - (x0 + 10.0);
    assert!(
        (gap - 5.0).abs() < 0.01,
        "expected 4.0 * 1.25 = 5.0 (DIP), got {gap}"
    );
}

/// 对照 2：`gap_fixed(4)` 不随 DPI 变 —— 125% 下仍是 4.0。
#[test]
fn gap_fixed_is_scale_independent() {
    let metrics = Metrics { scale: 1.25 };
    let mut flex = row().gap_fixed(4.0);
    let (x0, x1) = child_xs(&mut flex, &metrics);
    let gap = x1 - (x0 + 10.0);
    assert!((gap - 4.0).abs() < 0.01, "expected 4.0 (fixed), got {gap}");
}
