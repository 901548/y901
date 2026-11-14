//! ComboBox：下拉选择（走框架 popup 浮层）。
//!
//! 闭合态：输入框样式 + 当前值 + 右侧 ▾。
//! 展开态：**由 `ctx.request_popup` 弹出的独立列表 widget（`ComboList`）**，
//! 因此**不受父容器裁剪**，且自带框架的 150ms 弹出动画。
//!
//! 已知代价（框架 popup 的固有限制，已接受）：popup 内容与父组件**不共享状态**，
//! 所以选中结果通过 `Rc<Cell<Option<usize>>>` 回传，ComboBox 在**下一次事件**
//! （`Tick` / `MouseUp`）里读取并落库、再触发回调。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::widget::FlexItem;
use y901_ui::{
    Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

/// 闭合框高度。
const BOX_H: f32 = 32.0;
/// 列表项高度。
const ITEM_H: f32 = 28.0;
/// 列表内边距。
const LIST_PAD: f32 = 4.0;
/// 列表最小宽度。
const LIST_MIN_W: f32 = 180.0;

/// 下拉选择。
pub struct ComboBox {
    id: String,
    items: Vec<String>,
    selected: Option<usize>,
    on_change: Option<Box<dyn Fn(usize)>>,
    /// popup 侧写入的选中项（由 `ComboList` 设置）。
    shared: Rc<Cell<Option<usize>>>,
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
            on_change: None,
            shared: Rc::new(Cell::new(None)),
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

    /// 当前选中项。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 浮层 id（`push_popup` 用它去重）。
    fn popup_id(&self) -> String {
        format!("{}-list", self.id)
    }

    /// 闭合框矩形。
    fn box_rect(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y, self.rect.w, BOX_H.min(self.rect.h))
    }

    /// 当前显示的文字。
    fn display_text(&self) -> &str {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(String::as_str)
            .unwrap_or("请选择")
    }

    /// 把 popup 回传的选中项落库；返回 true = 有变化（需要重绘）。
    fn absorb_shared(&mut self) -> bool {
        let Some(i) = self.shared.take() else {
            return false;
        };
        if i >= self.items.len() {
            return false;
        }
        self.selected = Some(i);
        if let Some(cb) = self.on_change.as_ref() {
            cb(i);
        }
        true
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
        // 先吸收 popup 的选中结果（上次点击写入的）
        if matches!(event, UiEvent::Tick | UiEvent::MouseUp { .. }) && self.absorb_shared() {
            return EventResult::Redraw;
        }
        match event {
            UiEvent::MouseUp { pos, .. } if self.box_rect().contains(*pos) => {
                ctx.request_focus(self.id.clone());
                let popup_id = self.popup_id();
                let items = self.items.clone();
                let selected = self.selected;
                let shared = self.shared.clone();
                let list = ComboList {
                    id: popup_id,
                    items,
                    selected,
                    shared,
                    width: self.box_rect().w.max(LIST_MIN_W),
                    rect: Rect::ZERO,
                };
                ctx.request_popup(
                    Box::new(move || Box::new(list)),
                    self.box_rect(),
                );
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (box_bg, box_border, text_fg, muted) = {
            let theme = ctx.theme();
            let input = theme.style(Role::TEXT_INPUT, State::Normal);
            (
                input.bg,
                input.border,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.muted_text_color(),
            )
        };
        let b = self.box_rect();
        ctx.fill_round_rect(b, box_bg, 4.0);
        ctx.stroke_round_rect(b, box_border, 1.0, 4.0);

        let style = TextStyle::fg(text_fg);
        let ty = b.y + (b.h - ctx.line_height(&style)) * 0.5;
        ctx.draw_text(Pos::new(b.x + 10.0, ty), self.display_text(), &style);

        let arrow = "▾";
        let astyle = TextStyle::fg(muted);
        let aw = ctx.measure_text(arrow, &astyle).w;
        ctx.draw_text(Pos::new(b.right() - 10.0 - aw, ty), arrow, &astyle);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("combobox {}", self.id))
            .value(self.display_text().to_string())
            .prop("items", self.items.len().to_string())
    }
}

/// **私有**：ComboBox 弹出的列表（由 popup 层承载，因此不受父容器裁剪）。
struct ComboList {
    id: String,
    items: Vec<String>,
    selected: Option<usize>,
    shared: Rc<Cell<Option<usize>>>,
    width: f32,
    rect: Rect,
}

impl ComboList {
    /// 第 i 项矩形（与闭合框的位置无关：popup 层会按 anchor 摆放整块）。
    fn item_rect(&self, i: usize) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y + LIST_PAD + i as f32 * ITEM_H,
            self.rect.w,
            ITEM_H,
        )
    }
}

impl Widget for ComboList {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        _c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        Size::new(self.width, self.items.len() as f32 * ITEM_H + LIST_PAD * 2.0)
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        let UiEvent::MouseUp { pos, .. } = event else {
            return EventResult::Ignored;
        };
        for i in 0..self.items.len() {
            if self.item_rect(i).contains(*pos) {
                self.shared.set(Some(i));
                ctx.request_close_popup(self.id.clone());
                return EventResult::Redraw;
            }
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (bg, border, text_fg, hover_bg, sel_bg) = {
            let theme = ctx.theme();
            let input = theme.style(Role::TEXT_INPUT, State::Normal);
            (
                input.bg,
                input.border,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.style(Role::LIST_ITEM, State::Hovered).bg,
                theme.style(Role::LIST_ITEM, State::Pressed).bg,
            )
        };
        ctx.fill_round_rect(self.rect, bg, 6.0);
        ctx.stroke_round_rect(self.rect, border, 1.0, 6.0);
        for i in 0..self.items.len() {
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
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("combobox list {}", self.id))
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

    #[test]
    fn click_box_requests_popup() {
        let mut cb = ComboBox::new("c", vec!["a", "b", "c"]).selected(Some(1));
        let mut cache = LayoutCache::new();
        cb.arrange(Rect::new(0.0, 0.0, 200.0, BOX_H), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert!(ctx.popup_requests.is_empty());
        let r = cb.event(
            &mut ctx,
            &UiEvent::MouseUp { pos: Pos::new(20.0, 10.0), button: y901_ui::MouseButton::Left },
        );
        assert_eq!(r, EventResult::Redraw);
        assert_eq!(ctx.popup_requests.len(), 1, "点击闭合框应请求弹出列表");
        assert!(ctx.popup_requests[0].anchor.w > 0.0);
    }

    #[test]
    fn popup_writes_selection_back_to_combo() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let hits = Arc::new(AtomicUsize::new(0));
        let h2 = hits.clone();
        let mut cb = ComboBox::new("c", vec!["a", "b"]).on_change(move |_| {
            h2.fetch_add(1, Ordering::SeqCst);
        });
        let mut cache = LayoutCache::new();
        cb.arrange(Rect::new(0.0, 0.0, 200.0, BOX_H), &Metrics, &mut cache);
        // 模拟 popup 侧写入 index 1
        cb.shared.set(Some(1));
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert_eq!(cb.event(&mut ctx, &UiEvent::Tick), EventResult::Redraw);
        assert_eq!(cb.selected_index(), Some(1));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // 第二次 Tick：已吸收，无变化
        assert_eq!(cb.event(&mut ctx, &UiEvent::Tick), EventResult::Ignored);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}