//! ComboBox：下拉选择（最小可用版）。
//!
//! 闭合态：输入框样式 + 当前值 + 右侧 ▾。
//! 展开态：闭合框下面**接着画**一个列表（最多 8 项）。
//!
//! ⚠️ **已知限制**：展开列表画在**自身矩形之外**，因此
//! 1) 需要下方留出空间（示例里每行留了 220px）；
//! 2) 后续兄弟控件会盖住它。真正需要浮层的场景请用 `Popover` 承载。
//!    要做成真浮层需要框架的 overlay 层（`Menu`/`Dialog` 走的是那条路径）。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Align, Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size,
    State, TextStyle, TextMetrics, UiEvent, Widget,
};

/// 闭合框高度。
const BOX_H: f32 = 32.0;
/// 列表项高度。
const ITEM_H: f32 = 28.0;
/// 最多同屏项数。
const MAX_VISIBLE: usize = 8;

/// 下拉选择。
pub struct ComboBox {
    id: String,
    items: Vec<String>,
    selected: Option<usize>,
    open: bool,
    on_change: Option<Box<dyn Fn(usize)>>,
    rect: Rect,
    item: FlexItem,
}

impl ComboBox {
    /// 创建下拉框。
    pub fn new(id: impl Into<String>, items: Vec<impl Into<String>>) -> Self {
        Self {
            id: id.into(),
            items: items.into_iter().map(Into::into).collect(),
            selected: None,
            open: false,
            on_change: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    /// 预选中项。
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index.filter(|i| *i < self.items.len());
        self
    }

    /// 选中变化回调。
    pub fn on_change(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 是否展开（测试用）。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 当前选中项。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 闭合框矩形。
    fn box_rect(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y, self.rect.w, BOX_H.min(self.rect.h))
    }

    /// 第 i 项的矩形（在框的下方）。
    fn item_rect(&self, i: usize) -> Rect {
        let b = self.box_rect();
        Rect::new(b.x, b.bottom() + 2.0 + i as f32 * ITEM_H, b.w, ITEM_H)
    }

    /// 当前显示的文字。
    fn display_text(&self) -> &str {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(String::as_str)
            .unwrap_or("请选择")
    }
}

impl Widget for ComboBox {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::BUTTON
    }

    fn focusable(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, BOX_H))
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
        match event {
            UiEvent::MouseUp { pos, .. } => {
                // 先看列表项（列表在框下面，优先级更高）
                if self.open {
                    let n = self.items.len().min(MAX_VISIBLE);
                    for i in 0..n {
                        if self.item_rect(i).contains(*pos) {
                            self.selected = Some(i);
                            self.open = false;
                            if let Some(cb) = self.on_change.as_ref() {
                                cb(i);
                            }
                            return EventResult::Redraw;
                        }
                    }
                }
                if self.box_rect().contains(*pos) {
                    self.open = !self.open;
                    ctx.request_focus(self.id.clone());
                    return EventResult::Redraw;
                }
                if self.open {
                    self.open = false;
                    return EventResult::Redraw;
                }
                EventResult::Ignored
            }
            UiEvent::Key { key, .. } if matches!(key, y901_ui::Key::Escape) && self.open => {
                self.open = false;
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (box_bg, box_border, text_fg, hover_bg, sel_bg, muted) = {
            let theme = ctx.theme();
            let input = theme.style(Role::TEXT_INPUT, State::Normal);
            (
                input.bg,
                input.border,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.style(Role::LIST_ITEM, State::Hovered).bg,
                theme.style(Role::LIST_ITEM, State::Pressed).bg,
                theme.muted_text_color(),
            )
        };
        let b = self.box_rect();
        ctx.fill_round_rect(b, box_bg, 4.0);
        ctx.stroke_round_rect(b, box_border, 1.0, 4.0);

        let style = TextStyle::fg(text_fg);
        let ty = b.y + (b.h - ctx.line_height(&style)) * 0.5;
        ctx.draw_text(Pos::new(b.x + 10.0, ty), self.display_text(), &style);
        // 右侧指示符（用文字，不新增图元）
        let arrow = if self.open { "▴" } else { "▾" };
        let astyle = TextStyle::fg(muted);
        let aw = ctx.measure_text(arrow, &astyle).w;
        ctx.draw_text(
            Pos::new(b.right() - 10.0 - aw, ty),
            arrow,
            &astyle,
        );

        if !self.open {
            return;
        }
        // 展开列表（画在自身矩形之外，见模块文档的限制说明）
        let n = self.items.len().min(MAX_VISIBLE);
        let area = Rect::new(b.x, b.bottom() + 2.0, b.w, n as f32 * ITEM_H);
        ctx.fill_round_rect(area, box_bg, 4.0);
        ctx.stroke_round_rect(area, box_border, 1.0, 4.0);
        for i in 0..n {
            let r = self.item_rect(i);
            if self.selected == Some(i) {
                ctx.fill_rect(r, sel_bg);
            } else if i % 2 == 1 {
                ctx.fill_rect(r, hover_bg);
            }
            let s = TextStyle::fg(text_fg);
            let ly = r.y + (r.h - ctx.line_height(&s)) * 0.5;
            ctx.draw_text(Pos::new(r.x + 10.0, ly), &self.items[i], &s);
        }
        let _ = Align::Start;
        let _ = Color::WHITE;
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("combobox {}", self.id))
            .value(self.display_text().to_string())
            .prop("open", self.open.to_string())
            .prop("items", self.items.len().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    fn arranged() -> (ComboBox, EventCtx) {
        let mut cb = ComboBox::new("c", vec!["a", "b", "c"]).selected(Some(1));
        let mut cache = LayoutCache::new();
        cb.arrange(Rect::new(0.0, 0.0, 200.0, BOX_H), &Metrics, &mut cache);
        let ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        (cb, ctx)
    }

    #[test]
    fn click_toggles_open_then_selects_item() {
        let (mut cb, mut ctx) = arranged();
        assert!(!cb.is_open());
        let click_box = UiEvent::MouseUp {
            pos: Pos::new(20.0, 10.0),
            button: y901_ui::MouseButton::Left,
        };
        assert_eq!(cb.event(&mut ctx, &click_box), EventResult::Redraw);
        assert!(cb.is_open(), "点击合闭框应展开");
        // 点第 3 项
        let r = cb.item_rect(2);
        let click_item = UiEvent::MouseUp {
            pos: Pos::new(r.x + 4.0, r.y + 4.0),
            button: y901_ui::MouseButton::Left,
        };
        assert_eq!(cb.event(&mut ctx, &click_item), EventResult::Redraw);
        assert!(!cb.is_open());
        assert_eq!(cb.selected_index(), Some(2));
    }

    #[test]
    fn selecting_fires_callback_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicUsize::new(0));
        let h2 = hits.clone();
        let mut cb = ComboBox::new("c", vec!["a", "b"])
            .on_change(move |_| {
                h2.fetch_add(1, Ordering::SeqCst);
            });
        let mut cache = LayoutCache::new();
        cb.arrange(Rect::new(0.0, 0.0, 200.0, BOX_H), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        cb.event(
            &mut ctx,
            &UiEvent::MouseUp { pos: Pos::new(10.0, 10.0), button: y901_ui::MouseButton::Left },
        );
        let r = cb.item_rect(0);
        cb.event(
            &mut ctx,
            &UiEvent::MouseUp { pos: Pos::new(r.x + 2.0, r.y + 2.0), button: y901_ui::MouseButton::Left },
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}