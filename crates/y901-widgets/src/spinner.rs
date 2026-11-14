//! Spinner：8 点旋转圆环，表示"正在等待"。
//!
//! 为什么是 8 个点而不是 SVG 圆弧：`RenderCtx` 只有矩形/圆角矩形/文字，
//! 8 个圆点绕圈既不需要新图元，也能表达"转"的方向感。
//!
//! 动画由 `UiEvent::Tick` 驱动：每次 Tick 推进一格，返回 `Redraw` 让主循环继续出帧
//! （`run_loop` 在"有工作"时不睡，因此能连续转动）。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, State,
    TextMetrics, UiEvent, Widget,
};

/// 圆点数量。
const DOTS: usize = 8;
/// 默认直径。
const DEFAULT_SIZE: f32 = 24.0;

/// 旋转指示器。
pub struct Spinner {
    id: String,
    size: f32,
    /// 当前高亮的点下标（每 Tick +1）。
    phase: usize,
    rect: Rect,
    item: FlexItem,
    /// 缩放后的直径（0.0 = 尚未测量）。
    size_scaled: f32,
}

impl Spinner {
    /// 创建 Spinner（默认直径 24）。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            size: DEFAULT_SIZE,
            phase: 0,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            size_scaled: 0.0,
        }
    }

    /// 设置直径（逻辑像素）。
    pub fn size(mut self, size: f32) -> Self {
        self.size = size.max(8.0);
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 当前相位（测试用）。
    pub fn phase(&self) -> usize {
        self.phase
    }
}

impl Widget for Spinner {
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
        self.size_scaled = self.size * m.scale();
        c.constrain(Size::new(self.size_scaled, self.size_scaled))
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

    /// **声明需要每帧 `Tick`**：框架只给 `wants_tick() == true` 的节点发 Tick
    /// （`tree.rs` 的广播门控），否则轮转/脉冲类组件永远不会动。
    fn wants_tick(&self) -> bool {
        true
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if matches!(event, UiEvent::Tick) {
            self.phase = (self.phase + 1) % DOTS;
            return EventResult::Redraw;
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let want = if self.size_scaled > 0.0 { self.size_scaled } else { self.size * ctx.scale() };
        let side = want.min(self.rect.w).min(self.rect.h);
        if side <= 0.0 {
            return;
        }
        let cx = self.rect.x + self.rect.w * 0.5;
        let cy = self.rect.y + self.rect.h * 0.5;
        let radius = side * 0.5 - side * 0.12;
        let dot = (side * 0.18).max(2.0);
        let accent = ctx.theme().style(Role::BUTTON, State::Pressed).bg;

        for i in 0..DOTS {
            // 从 12 点方向开始，顺时针
            let angle = (i as f32) * std::f32::consts::TAU / DOTS as f32
                - std::f32::consts::FRAC_PI_2;
            let px = cx + radius * angle.cos();
            let py = cy + radius * angle.sin();
            // 越接近当前相位越亮：0.15 ..= 1.0
            let steps = (i + DOTS - self.phase) % DOTS;
            let alpha = 1.0 - (steps as f32) / (DOTS as f32);
            let color = Color::rgba(accent.r, accent.g, accent.b, alpha.clamp(0.15, 1.0));
            ctx.fill_round_rect(
                Rect::new(px - dot * 0.5, py - dot * 0.5, dot, dot),
                color,
                dot * 0.5,
            );
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("spinner {}", self.id))
            .prop("busy", "true")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, _t: &str, _s: &y901_ui::TextStyle) -> Size {
            Size::new(0.0, 14.0)
        }
        fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    #[test]
    fn measure_is_square() {
        let mut s = Spinner::new("sp").size(32.0);
        let mut cache = LayoutCache::new();
        let size = s.measure(&Metrics, Constraints::loose(Size::new(100.0, 100.0)), &mut cache);
        assert_eq!(size.w, 32.0);
        assert_eq!(size.h, 32.0);
    }

    #[test]
    fn tick_advances_phase_and_asks_for_redraw() {
        let mut s = Spinner::new("sp");
        let mut cache = LayoutCache::new();
        s.arrange(Rect::new(0.0, 0.0, 24.0, 24.0), &Metrics, &mut cache);
        let ctx = &mut EventCtx::new(None, Size::new(100.0, 100.0));
        assert_eq!(s.event(ctx, &UiEvent::Tick), EventResult::Redraw);
        assert_eq!(s.phase(), 1);
    }
}