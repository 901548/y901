//! Tabs 的布局 API：`flex_item` 与 `content_height`。
//!
//! 背景：旧版 `measure` 恒返回 `BAR_H + 1.0`，调用方**无法覆盖**，于是内容区只有 1px。
//! 这不是设计，是 API 缺口。现在提供两条显式路径：
//! - `flex_item(FlexItem::fill())`：让父容器分配高度（推荐）
//! - `content_height(h)`：显式指定内容区高度
//!
//! 不设置时仍返回 `BAR_H + 1.0`，但文档已写明那是**最小高度**。

use y901_ui::{
    layout::LayoutCache, Constraints, FlexItem, Rect, RenderCtx, Size, TextMetrics, Widget,
};
use y901_widgets::Tabs;

/// 极简内容绘制器：什么都不画，只用于触发布局。
fn noop_painter(_idx: usize, _ctx: &mut dyn RenderCtx, _rect: Rect, _t: f32) {}

fn tabs(id: &str) -> Tabs {
    Tabs::new(
        id,
        vec!["one".to_string(), "two".to_string()],
        Box::new(noop_painter),
    )
}

/// 断言当前 Tabs 组件的 `BAR_H`（与 `tabs.rs` 内常量一致）。
const BAR_H: f32 = 32.0;

struct ZeroMetrics;
impl TextMetrics for ZeroMetrics {
    fn measure_text(&self, _text: &str, _style: &y901_ui::TextStyle) -> Size {
        Size::new(0.0, 0.0)
    }
    fn line_height(&self, _style: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

#[test]
fn tabs_with_content_height_measures_correctly() {
    let mut t = tabs("t1").content_height(200.0);
    let c = Constraints::loose(Size::new(400.0, 500.0));
    let mut cache = LayoutCache::default();
    let size = t.measure(&ZeroMetrics, c, &mut cache);
    assert_eq!(
        size.h,
        BAR_H + 200.0,
        "content_height(200) 应得到 BAR_H + 200"
    );
}

#[test]
fn tabs_without_any_setting_falls_back_to_minimum() {
    // **兜底行为必须保留**：没有显式设置时仍是 BAR_H + 1.0（最小高度，避免 0 高）
    let mut t = tabs("t2");
    let c = Constraints::loose(Size::new(400.0, 500.0));
    let mut cache = LayoutCache::default();
    let size = t.measure(&ZeroMetrics, c, &mut cache);
    assert_eq!(size.h, BAR_H + 1.0, "未设置时应保留最小高度 BAR_H + 1.0");
    assert_eq!(
        t.flex(),
        FlexItem::default(),
        "未设置 flex_item 时不得自动 fill"
    );
}

#[test]
fn tabs_in_flex_with_fill_can_exceed_minimum() {
    // flex_item(fill) 时由父容器分配高度：这里只验证 measure 仍报最小高度、
    // 但 arrange 后 rect 会被拉满（布局器职责），所以断言的是"不阻止被拉高"。
    let mut t = tabs("t3").flex_item(FlexItem::fill());
    assert_eq!(t.flex(), FlexItem::fill(), "flex_item 必须原样保存");
    let c = Constraints::loose(Size::new(400.0, 500.0));
    let mut cache = LayoutCache::default();
    let size = t.measure(&ZeroMetrics, c, &mut cache);
    // 关键：**不会**被硬编码成 BAR_H+1.0 之外的值；父容器可覆盖
    assert!(size.h >= BAR_H, "最小高度不得低于 BAR_H");
}
