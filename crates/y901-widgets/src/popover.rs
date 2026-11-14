//! Popover：锚定浮层，可放任意 widget，点击外部 / Esc 关闭。
//!
//! 与 `Tooltip` 的区别：Tooltip 只显示一段文字、无交互；Popover 承载**任意控件**。
//! 与 `Menu` / `Dialog` 的区别：那两个走框架的 overlay 层（真浮层、可超出父容器）；
//! 本组件是**普通布局节点**，内容作为子控件参与布局（由框架递归渲染），
//! 因此它不会覆盖兄弟控件——需要真浮层请用 `Menu`/`Dialog`。
//!
//! 定位策略（`arrange` 时计算）：优先放在锚点**下方 8px**；下方空间不足则放上方；
//! 水平方向默认左对齐锚点，右侧越界则右对齐。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, State,
    TextMetrics, UiEvent, Widget,
};

/// 与锚点的间距。
const OFFSET: f32 = 8.0;
/// 内边距。
const PAD: f32 = 12.0;

/// 锚定浮层。
pub struct Popover {
    id: String,
    anchor: Rect,
    content: Vec<Box<dyn Widget>>,
    open: bool,
    on_dismiss: Option<Box<dyn Fn()>>,
    rect: Rect,
    item: FlexItem,
}

impl Popover {
    /// 创建浮层：`anchor` 为锚点矩形（相对父容器），`content` 为内容控件。
    pub fn new(id: impl Into<String>, anchor: Rect, content: Box<dyn Widget>) -> Self {
        Self {
            id: id.into(),
            anchor,
            content: vec![content],
            open: true,
            on_dismiss: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    /// 初始是否展开。
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// 关闭回调。
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 是否展开。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 计算浮层矩形（内容尺寸 + padding，按锚点定位）。
    fn popup_rect(&self, content: Size, bounds: Rect) -> Rect {
        let w = content.w + PAD * 2.0;
        let h = content.h + PAD * 2.0;
        let below = self.anchor.bottom() + OFFSET;
        let above = self.anchor.y - OFFSET - h;
        let y = if below + h <= bounds.bottom() {
            below
        } else if above >= bounds.y {
            above
        } else {
            below
        };
        let x = if self.anchor.x + w <= bounds.right() {
            self.anchor.x
        } else if self.anchor.right() - w >= bounds.x {
            self.anchor.right() - w
        } else {
            bounds.x
        };
        Rect::new(x, y, w, h)
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
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        if !self.open {
            return c.constrain(Size::new(0.0, 0.0));
        }
        let inner = Constraints {
            min: Size::ZERO,
            max: Size::new(
                (c.max.w - PAD * 2.0).max(0.0),
                (c.max.h - PAD * 2.0).max(0.0),
            ),
            ..Default::default()
        };
        let content = self.content[0].measure(m, inner, cache);
        c.constrain(Size::new(content.w + PAD * 2.0, content.h + PAD * 2.0))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        m: &dyn TextMetrics,
        cache: &mut y901_ui::layout::LayoutCache,
    ) {
        if !self.open {
            self.rect = Rect::ZERO;
            return;
        }
        let inner = Constraints {
            min: Size::ZERO,
            max: Size::new((rect.w - PAD * 2.0).max(0.0), (rect.h - PAD * 2.0).max(0.0)),
            ..Default::default()
        };
        let content_size = self.content[0].measure(m, inner, cache);
        let target = self.popup_rect(content_size, rect);
        self.rect = target;
        let inner_rect = Rect::new(
            target.x + PAD,
            target.y + PAD,
            content_size.w,
            content_size.h,
        );
        self.content[0].arrange(inner_rect, m, cache);
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.content
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.content)
    }

    fn event(&mut self, _ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if !self.open {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::Key { key, .. } if matches!(key, y901_ui::Key::Escape) => {
                self.open = false;
                if let Some(f) = self.on_dismiss.as_ref() {
                    f();
                }
                EventResult::Redraw
            }
            UiEvent::MouseUp { pos, .. } => {
                if self.rect.contains(*pos) {
                    EventResult::Ignored // 内部点击交给子控件
                } else if self.anchor.contains(*pos) {
                    // 点锚点：收起（与弹出互为开关）
                    self.open = false;
                    if let Some(f) = self.on_dismiss.as_ref() {
                        f();
                    }
                    EventResult::Redraw
                } else {
                    self.open = false;
                    if let Some(f) = self.on_dismiss.as_ref() {
                        f();
                    }
                    EventResult::Redraw
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if !self.open || self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (bg, border) = {
            let theme = ctx.theme();
            (
                theme.style(Role::PANEL, State::Hovered).bg,
                theme.divider_color(),
            )
        };
        ctx.fill_round_rect(self.rect, bg, 8.0);
        ctx.stroke_round_rect(self.rect, border, 1.0, 8.0);
        let _ = Color::WHITE;
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("popover {}", self.id))
            .prop("open", self.open.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use crate::TextLabel;

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
    fn positions_below_anchor_and_reserves_padding() {
        let content: Box<dyn Widget> = Box::new(TextLabel::new("t", "hello"));
        let mut p = Popover::new("p", Rect::new(40.0, 40.0, 80.0, 24.0), content);
        let mut cache = LayoutCache::new();
        let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
        let size = p.measure(&Metrics, Constraints::loose(bounds.size()), &mut cache);
        p.arrange(bounds, &Metrics, &mut cache);
        assert!(size.w >= PAD * 2.0 && size.h >= PAD * 2.0);
        assert_eq!(p.rect.x, 40.0, "左对齐锚点");
        assert!(p.rect.y > 40.0 + 24.0, "在锚点下方");
    }

    #[test]
    fn closed_popover_measures_zero() {
        let content: Box<dyn Widget> = Box::new(TextLabel::new("t", "x"));
        let mut p = Popover::new("p", Rect::new(0.0, 0.0, 10.0, 10.0), content).open(false);
        let mut cache = LayoutCache::new();
        let size = p.measure(&Metrics, Constraints::loose(Size::new(100.0, 100.0)), &mut cache);
        assert_eq!((size.w, size.h), (0.0, 0.0));
    }
}