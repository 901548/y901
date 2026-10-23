//! 虚拟化列表：只实例化/排布可见行。
//!
//! - `measure` 不测量任何子节点，内容高度 = `行数 × 行高`（解析式，O(1)）；
//! - `arrange` 只把可见区间内的行放进池子（池子大小 = 视口行数 + 1）；
//! - 行池是 `Vec<Box<dyn Widget>>`，通过 `as_any_mut` 就地改内容，实现行复用。

use std::any::Any;

use y901_ui::{
    Constraints, EventCtx, EventResult, FlexItem, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

use crate::scroll::{Scrollbar, BAR_WIDTH};

/// 一行。
struct Row {
    id: String,
    text: String,
    rect: Rect,
}

impl Row {
    fn set(&mut self, parent: &str, index: usize, text: &str, rect: Rect) {
        self.id.clear();
        self.id.push_str(parent);
        self.id.push_str(".row");
        self.id.push_str(&index.to_string());
        self.text.clear();
        self.text.push_str(text);
        self.rect = rect;
    }
}

impl Widget for Row {
    fn id(&self) -> &str {
        &self.id
    }
    fn role(&self) -> Role {
        Role::LIST_ITEM
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        let h = m.line_height(&TextStyle::default());
        c.constrain(Size::new(c.max.w, h))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let style = ctx.theme().style(Role::LIST_ITEM, State::Normal);
        ctx.fill_rect(self.rect, style.bg);
        ctx.draw_text(
            Pos::new(self.rect.x + 8.0, self.rect.y + 2.0),
            &self.text,
            &TextStyle::fg(style.fg),
        );
    }
    fn semantics(&self) -> Semantics {
        Semantics::default().label(self.text.clone())
    }
}

pub struct ListView {
    id: String,
    items: Vec<String>,
    row_height: f32,
    /// 行池：长度 = 可见行数，随滚动复用。
    pool: Vec<Box<dyn Widget>>,
    offset: f32,
    viewport: Rect,
    inner: Rect,
    item: FlexItem,
    show_bar: bool,
    bar: Scrollbar,
    arranged_rows: usize,
}

impl ListView {
    pub fn new(id: impl Into<String>, row_height: f32) -> Self {
        let id = id.into();
        Self {
            bar: Scrollbar::new(format!("{id}.bar"), id.clone()),
            id,
            items: Vec::new(),
            row_height: row_height.max(1.0),
            pool: Vec::new(),
            offset: 0.0,
            viewport: Rect::ZERO,
            inner: Rect::ZERO,
            item: FlexItem::default(),
            show_bar: false,
            arranged_rows: 0,
        }
    }

    pub fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        self.offset = 0.0;
    }

    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// 诊断：上次 arrange 排布的行数（虚拟化的直接证据）。
    pub fn arranged_rows(&self) -> usize {
        self.arranged_rows
    }

    pub fn content_height(&self) -> f32 {
        self.items.len() as f32 * self.row_height
    }

    pub fn row_count(&self) -> usize {
        self.items.len()
    }

    fn max_offset(&self) -> f32 {
        (self.content_height() - self.inner.h).max(0.0)
    }

    fn set_offset(&mut self, v: f32) -> bool {
        let clamped = v.clamp(0.0, self.max_offset());
        if (clamped - self.offset).abs() < 0.01 {
            return false;
        }
        self.offset = clamped;
        true
    }
}

impl Widget for ListView {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.pool
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.pool)
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 不测子节点：内容高度解析可得
        c.constrain(Size::new(c.max.w, c.max.h))
    }

    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.viewport = rect;
        self.show_bar = self.content_height() > rect.h;
        let bar_w = if self.show_bar { BAR_WIDTH } else { 0.0 };
        self.inner = Rect::new(rect.x, rect.y, (rect.w - bar_w).max(0.0), rect.h);
        self.offset = self.offset.clamp(0.0, self.max_offset());

        // ---- 虚拟化：只处理可见区间 ----
        let first = (self.offset / self.row_height).floor().max(0.0) as usize;
        let visible = ((self.inner.h / self.row_height).ceil() as usize + 1)
            .min(self.items.len().saturating_sub(first));
        let count = visible.max(1).min(self.items.len().saturating_sub(first));

        self.pool.truncate(count);
        while self.pool.len() < count {
            self.pool.push(Box::new(Row {
                id: String::new(),
                text: String::new(),
                rect: Rect::ZERO,
            }));
        }

        self.arranged_rows = 0;
        for (k, cell) in self.pool.iter_mut().enumerate() {
            let index = first + k;
            let y = self.inner.y + index as f32 * self.row_height - self.offset;
            let r = Rect::new(self.inner.x, y, self.inner.w, self.row_height);
            if let Some(row) = cell.as_any_mut().and_then(|a| a.downcast_mut::<Row>()) {
                let text = self.items.get(index).cloned().unwrap_or_default();
                row.set(&self.id, index, &text, r);
            }
            cell.arrange(r, m, cache);
            self.arranged_rows += 1;
        }

        self.bar.arrange_at(
            Rect::new(rect.right() - BAR_WIDTH, rect.y, BAR_WIDTH, rect.h),
            if self.content_height() > 0.0 {
                (rect.h / self.content_height()).min(1.0)
            } else {
                1.0
            },
            if self.max_offset() > 0.0 {
                self.offset / self.max_offset()
            } else {
                0.0
            },
            self.show_bar,
        );
    }

    fn rect(&self) -> Rect {
        self.viewport
    }

    fn clip(&self) -> Option<Rect> {
        Some(self.inner)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if ctx.phase == y901_ui::EventPhase::Capture {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::Wheel { pos, delta } => {
                if !self.inner.contains(*pos) {
                    return EventResult::Ignored;
                }
                let step = *delta / 120.0 * self.row_height * 3.0;
                if self.set_offset(self.offset - step) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Action { name, args } if name == "scroll_to_ratio" => {
                let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                if self.set_offset(ratio * self.max_offset()) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::MouseDown { pos, .. } if self.show_bar && self.bar.rect.contains(*pos) => {
                ctx.set_pointer_capture(self.id.clone());
                self.bar.dragging = true;
                let ratio = self.bar.ratio_at(pos.y) - self.bar.thumb_ratio * 0.5;
                if self.set_offset(ratio * self.max_offset()) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::MouseMove { pos } if self.bar.dragging => {
                let ratio = self.bar.ratio_at(pos.y) - self.bar.thumb_ratio * 0.5;
                if self.set_offset(ratio * self.max_offset()) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::MouseUp { .. } if self.bar.dragging => {
                self.bar.dragging = false;
                ctx.release_pointer_capture();
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let style = ctx.theme().style(Role::LIST, State::Normal);
        ctx.fill_rect(self.viewport, style.bg);
        self.bar.render_bar(ctx);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(format!("list({} rows)", self.items.len()))
            .action("scroll_to_ratio")
            .state("rows", self.items.len() as u32)
            .state("offset", self.offset)
            .state("arranged", self.arranged_rows as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::{UiTree, Widget};

    struct Mono;
    impl TextMetrics for Mono {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.len() as f32 * 8.0, 16.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            20.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    fn ten_thousand() -> ListView {
        let items: Vec<String> = (0..10_000).map(|i| format!("row {i}")).collect();
        ListView::new("list", 20.0).items(items).flex_item(FlexItem::fill())
    }

    #[test]
    fn virtualized_list_only_arranges_visible_rows() {
        let mut list = ten_thousand();
        let mut cache = y901_ui::layout::LayoutCache::new();
        list.measure(&Mono, Constraints::loose(Size::new(300.0, 100.0)), &mut cache);
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);

        assert_eq!(list.content_height(), 200_000.0, "内容高度是解析值");
        assert_eq!(list.row_count(), 10_000);
        assert!(
            list.arranged_rows() <= 7,
            "10k 行只应排布可见的几行，实际 {}",
            list.arranged_rows()
        );
        assert_eq!(list.children().len(), list.arranged_rows());
    }

    #[test]
    fn scrolling_recycles_pool_and_keeps_count_small() {
        let mut list = ten_thousand();
        let mut cache = y901_ui::layout::LayoutCache::new();
        list.measure(&Mono, Constraints::loose(Size::new(300.0, 100.0)), &mut cache);
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);

        let mut ctx = y901_ui::EventCtx::new(None, Size::new(300.0, 100.0));
        for _ in 0..20 {
            list.event(
                &mut ctx,
                &UiEvent::Wheel {
                    pos: Pos::new(10.0, 10.0),
                    delta: -120.0,
                },
            );
            list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);
        }

        assert!(list.offset() > 0.0, "应真的滚动了");
        assert!(
            list.arranged_rows() <= 7,
            "滚动后仍只排布可见行，实际 {}",
            list.arranged_rows()
        );
    }

    #[test]
    fn list_ids_reflect_data_index() {
        let list = ten_thousand();
        let mut tree = UiTree::new(Box::new(list).flex_item_helper());
        tree.layout(Size::new(300.0, 100.0), &Mono);
        // 第一行可见
        assert!(tree.find("list.row0").is_some(), "可见行应有稳定 id");
    }

    /// 便于把 ListView 塞进 UiTree 的小工具。
    trait FlexHelper {
        fn flex_item_helper(self) -> Box<dyn Widget>;
    }
    impl FlexHelper for ListView {
        fn flex_item_helper(mut self) -> Box<dyn Widget> {
            self = self.flex_item(FlexItem::fill());
            Box::new(self)
        }
    }
}
