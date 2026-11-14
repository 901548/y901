//! Popover：锚定的浮层容器（内容走框架 popup 层，**不受父容器裁剪**）。
//!
//! 内容由 `factory` 构造并交给 `EventCtx::request_popup`，由 `UiTree` 的 popup 层承载：
//! 可以超出父容器、自带 150ms 弹出动画、点外部/`Esc` 由框架关闭。
//! 内容可以是容器（`Flex` 等）—— 框架的 popup 渲染已改为递归 `render_pass`。
//!
//! 锚点坐标系：构造期的 `anchor` 只作为**布局尺寸/命中兜底**；
//! 真正弹出用的是**布局后的 `self.rect`**（避免父容器拉伸导致的偏移）。
//!
//! 为什么 `measure` 返回**锚点尺寸**而不是 0×0：命中测试按 `rect()` 走，
//! 零尺寸节点永远收不到点击。代价：本组件在布局里占一块与锚点同样大的空白
//! （`render` 不画任何东西）。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, TextMetrics,
    UiEvent, Widget,
};

/// 锚定浮层。
pub struct Popover {
    id: String,
    anchor: Rect,
    factory: Box<dyn Fn() -> Box<dyn Widget>>,
    rect: Rect,
    item: FlexItem,
    /// DPI 缩放（`measure` 里唯一写入；未测过时 1.0）。
    scale: f32,
}

impl Popover {
    /// 创建浮层。
    pub fn new(
        id: impl Into<String>,
        anchor: Rect,
        factory: impl Fn() -> Box<dyn Widget> + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            anchor,
            factory: Box::new(factory),
            rect: Rect::ZERO,
            item: FlexItem::default(),
            scale: 1.0,
        }
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 锚点矩形。
    pub fn anchor(&self) -> Rect {
        self.anchor
    }
}

impl Widget for Popover {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        c.constrain(Size::new(self.anchor.w, self.anchor.h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        let UiEvent::MouseUp { pos, .. } = event else {
            return EventResult::Ignored;
        };
        // **用布局后的 `self.rect`**（而不是构造期传入的 `anchor`）：
        // 父容器（如 `Flex`）会把子节点拉伸/定位，构造期坐标与实际位置不一致。
        let hit = if self.rect.w > 0.0 && self.rect.h > 0.0 {
            self.rect
        } else {
            self.anchor
        };
        if !hit.contains(*pos) {
            return EventResult::Ignored;
        }
        let content = (self.factory)();
        ctx.request_popup(Box::new(move || content), hit);
        EventResult::Redraw
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        // **关闭态要有可见锚点**：之前这里是空函数，
        // 于是没点开时整个触发区完全不可见，用户不知道点哪里。
        // 给一个**低强度描边**（分隔线色），不影响内容布局，
        // 也不会在弹出时遮挡浮层。
        let border = ctx.theme().divider_color();
        let radius = 6.0 * ctx.scale();
        ctx.stroke_round_rect(self.rect, border, 1.0, radius);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("popover anchor {}", self.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextLabel;
    use y901_ui::layout::LayoutCache;
    use y901_ui::Pos;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &y901_ui::TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    #[test]
    fn measure_returns_anchor_size_so_it_can_be_hit() {
        let mut p = Popover::new("p", Rect::new(10.0, 10.0, 80.0, 24.0), || {
            Box::new(TextLabel::new("t", "hello")) as Box<dyn Widget>
        });
        let mut cache = LayoutCache::new();
        let s = p.measure(&Metrics, Constraints::loose(Size::new(400.0, 400.0)), &mut cache);
        assert_eq!((s.w, s.h), (80.0, 24.0), "锚点尺寸必须非零，否则收不到事件");
    }

    #[test]
    fn click_on_anchor_requests_popup() {
        let mut p = Popover::new("p", Rect::new(10.0, 10.0, 80.0, 24.0), || {
            Box::new(TextLabel::new("t", "hello")) as Box<dyn Widget>
        });
        let mut cache = LayoutCache::new();
        p.arrange(Rect::new(10.0, 10.0, 80.0, 24.0), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(200.0, 200.0));
        p.event(
            &mut ctx,
            &UiEvent::MouseUp { pos: Pos::new(150.0, 150.0), button: y901_ui::MouseButton::Left },
        );
        assert!(ctx.popup_requests.is_empty(), "锚点外不应请求");
        p.event(
            &mut ctx,
            &UiEvent::MouseUp { pos: Pos::new(20.0, 20.0), button: y901_ui::MouseButton::Left },
        );
        assert_eq!(ctx.popup_requests.len(), 1);
        assert_eq!(ctx.popup_requests[0].anchor.w, 80.0);
    }
}
