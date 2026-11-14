//! Skeleton：骨架屏占位（文本条 / 圆形 / 矩形）。
//!
//! 用途：数据到达前的"形状预告"，比空白或 Spinner 更少引起布局跳动。
//!
//! 脉冲动画：由 `UiEvent::Tick` 驱动 alpha 在 0.35↔0.55 之间呼吸（组件内自驱动，
//! 因为框架尚无"组件内启动动画"的入口——见 README 已知限制）。

use y901_ui::widget::{Dimension, FlexItem};
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, TextMetrics,
    UiEvent, Widget,
};

/// 一个占位形状。
#[derive(Clone, Copy, Debug)]
pub enum SkeletonShape {
    /// 文字条占位。
    Text {
        /// 宽度策略（固定 / 填满）。
        width: Dimension,
        /// 高度。
        height: f32,
    },
    /// 圆形占位（头像）。
    Circle {
        /// 直径。
        size: f32,
    },
    /// 矩形：宽度策略 + 高度 + 圆角。
    Rect {
        /// 宽度策略。
        width: Dimension,
        /// 高度。
        height: f32,
        /// 圆角半径。
        radius: f32,
    },
}

impl SkeletonShape {
    /// 高度。
    fn height(self) -> f32 {
        match self {
            SkeletonShape::Text { height, .. } => height,
            SkeletonShape::Circle { size } => size,
            SkeletonShape::Rect { height, .. } => height,
        }
    }
}

/// 骨架屏。
pub struct Skeleton {
    id: String,
    shapes: Vec<SkeletonShape>,
    gap: f32,
    rect: Rect,
    item: FlexItem,
    pulse: bool,
    up: bool,
}

impl Skeleton {
    /// 创建空骨架。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            shapes: Vec::new(),
            gap: 8.0,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            pulse: true,
            up: true,
        }
    }

    /// 追加一条文字占位。
    pub fn text(mut self, width: Dimension, height: f32) -> Self {
        self.shapes.push(SkeletonShape::Text { width, height });
        self
    }

    /// 追加一个圆形占位（头像）。
    pub fn circle(mut self, size: f32) -> Self {
        self.shapes.push(SkeletonShape::Circle { size });
        self
    }

    /// 追加一个矩形占位。
    pub fn rect(mut self, width: Dimension, height: f32, radius: f32) -> Self {
        self.shapes
            .push(SkeletonShape::Rect { width, height, radius });
        self
    }

    /// 形状间距。
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.0);
        self
    }

    /// 关闭脉冲动画（静态）。
    pub fn static_placeholder(mut self) -> Self {
        self.pulse = false;
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 总高度。
    fn content_height(&self) -> f32 {
        let n = self.shapes.len();
        if n == 0 {
            return 0.0;
        }
        let sum: f32 = self.shapes.iter().map(|s| s.height()).sum();
        sum + self.gap * (n - 1) as f32
    }
}

impl Widget for Skeleton {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, self.content_height()))
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
        if self.pulse && matches!(event, UiEvent::Tick) {
            self.up = !self.up;
            return EventResult::Redraw;
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let base = ctx.theme().divider_color();
        let alpha = if !self.pulse {
            0.45
        } else if self.up {
            0.55
        } else {
            0.35
        };
        let color = Color::rgba(base.r, base.g, base.b, alpha);

        let mut y = self.rect.y;
        for shape in &self.shapes {
            match *shape {
                SkeletonShape::Text { width, height } => {
                    let w = match width {
                        Dimension::Px(v) => v,
                        Dimension::Flex(_) => self.rect.w,
                        _ => self.rect.w * 0.7,
                    };
                    ctx.fill_round_rect(
                        Rect::new(self.rect.x, y, w.min(self.rect.w), height),
                        color,
                        height * 0.5,
                    );
                    y += height;
                }
                SkeletonShape::Circle { size } => {
                    ctx.fill_round_rect(
                        Rect::new(self.rect.x, y, size, size),
                        color,
                        size * 0.5,
                    );
                    y += size;
                }
                SkeletonShape::Rect {
                    width,
                    height,
                    radius,
                } => {
                    let w = match width {
                        Dimension::Px(v) => v,
                        Dimension::Flex(_) => self.rect.w,
                        _ => self.rect.w,
                    };
                    ctx.fill_round_rect(
                        Rect::new(self.rect.x, y, w.min(self.rect.w), height),
                        color,
                        radius,
                    );
                    y += height;
                }
            }
            y += self.gap;
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("loading placeholder {}", self.id))
            .prop("shapes", self.shapes.len().to_string())
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
    fn height_sums_shapes_and_gaps() {
        let mut sk = Skeleton::new("s")
            .circle(32.0)
            .text(Dimension::Px(120.0), 14.0)
            .rect(Dimension::Px(80.0), 20.0, 4.0)
            .gap(8.0);
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(300.0, 300.0));
        let size = sk.measure(&Metrics, c, &mut cache);
        // 32 + 14 + 20 + 2 * 8
        assert_eq!(size.h, 82.0);
    }

    #[test]
    fn works_with_no_shapes() {
        let mut sk = Skeleton::new("s");
        let mut cache = LayoutCache::new();
        let size = sk.measure(&Metrics, Constraints::loose(Size::new(100.0, 100.0)), &mut cache);
        assert_eq!(size.h, 0.0);
    }
}