//! **`FlexItem` 的 min/max 必须与 `width`/`height` 用同一套 DPI 语义**。
//!
//! 缺陷：`width: Px(100)` 在 measure 里乘了 `metrics.scale()`，而 `max_width: Px(120)`
//! 是按原始数字 clamp 的 —— "尺寸按物理像素、上下界按逻辑像素"：125% DPI 下
//! `width(100).max_width(120)` 本意是 125px，却被 120 卡住；150%/200% 更明显
//! （尺寸被悄悄压回逻辑像素值）。
//!
//! 同时钉住 arrange 侧：它会对**已缩放**的尺寸再 clamp 一次，若用未缩放的上下界，
//! 一次重排就会把尺寸压回去。

use y901_ui::layout::{Flex, LayoutCache};
use y901_ui::{
    Align, Axis, Constraints, Dimension, Edges, FlexItem, Justify, Rect, RenderCtx, Semantics,
    Size, TextMetrics, TextStyle, Widget,
};

/// 带缩放的度量：`scale` 就是 DPI 因子。
struct Scaled(f32);

impl TextMetrics for Scaled {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0 * self.0, 16.0 * self.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0 * self.0
    }
    fn scale(&self) -> f32 {
        self.0
    }
}

struct Leaf {
    id: String,
    rect: Rect,
    size: Size,
    item: FlexItem,
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
    fn flex(&self) -> FlexItem {
        self.item
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 在给定 DPI 下布一次局，返回第一个子件的 rect。
fn place(scale: f32, content: Size, item: FlexItem) -> Rect {
    let mut flex = Flex::row("row").push(Box::new(Leaf {
        id: "leaf".into(),
        rect: Rect::ZERO,
        size: content,
        item,
    }));
    let mut cache = LayoutCache::new();
    let m = Scaled(scale);
    let viewport = Size::new(1200.0 * scale, 400.0 * scale);
    let _ = cache.measure(&mut flex, &m, Constraints::tight(viewport));
    flex.arrange(Rect::from_size(viewport), &m, &mut cache);
    let _ = (
        Axis::Horizontal,
        Align::Stretch,
        Justify::Start,
        Edges::ZERO,
    );
    flex.children()[0].rect()
}

#[test]
fn max_width_is_scaled_with_dpi() {
    // 宽 100 逻辑像素、上限 120 逻辑像素 → 各 DPI 下都应是 100×scale
    let item = FlexItem::width(100.0).max_width(120.0);
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        let r = place(scale, Size::new(400.0, 20.0), item);
        let want = 100.0 * scale;
        assert!(
            (r.w - want).abs() < 0.01,
            "scale={scale}: 宽度应为 {want}，实际 {}（上限被当成物理像素卡住了）",
            r.w
        );
    }
}

#[test]
fn min_width_is_scaled_with_dpi() {
    // 内容只有 10，下界 100 逻辑像素 → 各 DPI 下都应是 100×scale
    let item = FlexItem {
        min_width: Dimension::Px(100.0),
        ..FlexItem::NO_LIMITS
    };
    for scale in [1.0f32, 1.25, 2.0] {
        let r = place(scale, Size::new(10.0, 20.0), item);
        let want = 100.0 * scale;
        assert!(
            (r.w - want).abs() < 0.01,
            "scale={scale}: 宽度下界应为 {want}，实际 {}",
            r.w
        );
    }
}

#[test]
fn max_height_clamps_scaled_content() {
    // 内容高 100，上限 30 逻辑像素 → 应夹到 30×scale
    let item = FlexItem {
        max_height: Dimension::Px(30.0),
        ..FlexItem::NO_LIMITS
    };
    for scale in [1.0f32, 1.25, 2.0] {
        let r = place(scale, Size::new(50.0, 100.0), item);
        let want = 30.0 * scale;
        assert!(
            (r.h - want).abs() < 0.01,
            "scale={scale}: 高度应被 max_height 夹到 {want}，实际 {}",
            r.h
        );
    }
}

/// 反向证据：min/max 不能**永远**不生效（否则上面的用例不构成约束）。
#[test]
fn clamps_still_apply_at_scale_one() {
    let r = place(
        1.0,
        Size::new(200.0, 10.0),
        FlexItem::width(200.0).max_width(80.0),
    );
    assert_eq!(r.w, 80.0, "1.0 缩放下 max_width(80) 必须生效");
    let r = place(
        1.0,
        Size::new(20.0, 10.0),
        FlexItem::width(20.0).min_width(100.0),
    );
    assert_eq!(r.w, 100.0, "1.0 缩放下 min_width(100) 必须生效");
}
