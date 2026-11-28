//! 通用虚拟列表：**只管滚动、可见区间、虚拟化裁剪**，不规定行长什么样。
//!
//! ```text
//! let list = ListView::new("log", 20.0)
//!     .len(entries.len())
//!     .painter(move |index, rect, ctx| {
//!         ctx.draw_text(Pos::new(rect.x + 8.0, rect.y + 2.0), &entries[index], &style);
//!     });
//! ```
//!
//! 为什么不存数据：早期的 `ListView` 存 `RowData { cells, icon }`，
//! 那是"文件表格"这一个业务的形状。聊天气泡、图片流、日志行
//! 都装不进去。现在框架只提供"**第 i 行在这个矩形里，你自己画**"。

use y901_ui::{
    Constraints, EventCtx, EventPhase, EventResult, FlexItem, InputKind, Pos, Rect, RenderCtx,
    Role, Semantics, Size, State, TextMetrics, UiEvent, Widget,
};

use crate::scroll::Scrollbar;

/// 行绘制回调：`(index, rect, ctx)`。
///
/// 框架已经把 `ctx` 裁剪到可见区，回调里只需直接画。
pub type RowPainter = Box<dyn FnMut(usize, Rect, &mut dyn RenderCtx)>;

const BAR_WIDTH: f32 = 6.0;

/// 虚拟化列表（数据由回调提供）。
pub struct ListView {
    id: String,
    /// 无障碍名称（读屏器念它）。空 = 不提供名称。
    label: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
    row_height: f32,
    /// 总行数（由调用方告知，框架不存数据）。
    len: usize,
    offset: f32,
    viewport: Rect,
    inner: Rect,
    item: FlexItem,
    show_bar: bool,
    bar: Scrollbar,
    arranged_rows: usize,
    selected: Option<usize>,
    hovered: Option<usize>,
    on_select: Option<Box<dyn FnMut(usize)>>,
    stick_to_bottom: bool,
    stick_armed: bool,
    painter: Option<RowPainter>,
    /// 动态行高（可选）；未设置时用 `row_height`。
    row_height_provider: Option<Box<dyn Fn(usize) -> f32>>,
    /// 行顶部累加高度（长度 `len + 1`）；用于 O(log n) 查行。
    prefix: Vec<f32>,
    /// `prefix` 是否已失效（len/提供者变了）。
    prefix_dirty: bool,
}

impl ListView {
    /// 无障碍名称（读屏器念的名字；空 = 不提供）。
    ///
    /// 语义里曾经把内部 id（和行数）拼进名字，读屏器会把实现细节念出来。
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// 新建空列表。
    ///
    /// 输入：`id`（树内唯一；滚动条 id 为 `{id}.bar`）、`row_height`（像素，最小 1）。
    /// 返回：未设行数/回调的实例。副作用：无。
    pub fn new(id: impl Into<String>, row_height: f32) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            bar: Scrollbar::new(format!("{id}.bar"), id.clone()),
            label: String::new(),
            id,
            key,
            row_height: row_height.max(1.0),
            len: 0,
            offset: 0.0,
            viewport: Rect::ZERO,
            inner: Rect::ZERO,
            item: FlexItem::default(),
            show_bar: false,
            arranged_rows: 0,
            selected: None,
            hovered: None,
            on_select: None,
            stick_to_bottom: false,
            stick_armed: false,
            painter: None,
            row_height_provider: None,
            prefix: vec![0.0],
            prefix_dirty: true,
        }
    }

    /// 设置动态行高提供者（例：聊天气泡高低不一）。
    ///
    /// 输入：`provider(index) -> 像素高度`。传入 `None` 或不调用就用固定 `row_height`。
    /// 副作用：作废高度缓存（下次 `measure`/`arrange` 重算前缀和）。
    pub fn set_row_height_provider(&mut self, provider: impl Fn(usize) -> f32 + 'static) {
        self.row_height_provider = Some(Box::new(provider));
        self.prefix_dirty = true;
    }

    /// 清除动态行高，回到固定行高。
    pub fn clear_row_height_provider(&mut self) {
        self.row_height_provider = None;
        self.prefix_dirty = true;
    }

    /// 第 `index` 行的高度（动态提供者优先，否则固定值）。
    ///
    /// 返回：至少 1。
    pub fn row_height_at(&self, index: usize) -> f32 {
        match &self.row_height_provider {
            Some(p) => p(index).max(1.0),
            None => self.row_height,
        }
    }

    /// 重算前缀和（长度 `len + 1`）。
    fn ensure_prefix(&mut self) {
        if !self.prefix_dirty && self.prefix.len() == self.len + 1 {
            return;
        }
        self.prefix.clear();
        self.prefix.reserve(self.len + 1);
        let mut acc = 0.0;
        self.prefix.push(0.0);
        for i in 0..self.len {
            acc += self.row_height_at(i);
            self.prefix.push(acc);
        }
        self.prefix_dirty = false;
    }

    /// 第 `index` 行顶部相对于内容顶部的偏移。
    fn row_top(&self, index: usize) -> f32 {
        self.prefix.get(index).copied().unwrap_or_else(|| {
            if index >= self.len {
                self.prefix.last().copied().unwrap_or(0.0)
            } else {
                0.0
            }
        })
    }

    /// 用二分搜索把"内容坐标 y"换成行下标。
    ///
    /// 返回：`None` = 超出内容范围。
    fn index_at_content_y(&self, y: f32) -> Option<usize> {
        if y < 0.0 || self.len == 0 {
            return None;
        }
        let total = self.prefix.last().copied().unwrap_or(0.0);
        if y >= total {
            return None;
        }
        // 找第一个 prefix[i+1] > y 的 i
        let mut lo = 0usize;
        let mut hi = self.len;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.prefix[mid + 1] <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some(lo)
    }

    /// 设置总行数（构建器版）。
    pub fn len(mut self, n: usize) -> Self {
        self.len = n;
        self
    }

    /// 设置总行数（运行时版）。
    pub fn set_len(&mut self, n: usize) -> bool {
        if self.len == n {
            return false;
        }
        self.len = n;
        // 选中项必须夹进新长度：否则 `selected_index()` 报一个不存在的行，
        // 语义面与 `state("selected", …)` 一起说谎，`invalidate_row` 静默无效。
        if let Some(i) = self.selected {
            if i >= n {
                self.selected = None;
            }
        }
        self.prefix_dirty = true;
        self.ensure_prefix();
        self.offset = self.offset.clamp(0.0, self.max_offset());
        if self.stick_armed {
            self.offset = self.max_offset();
        }
        true
    }

    /// 设置行绘制回调（构建器版）。
    pub fn painter(mut self, f: impl FnMut(usize, Rect, &mut dyn RenderCtx) + 'static) -> Self {
        self.painter = Some(Box::new(f));
        self
    }

    /// 设置行绘制回调（运行时版）。
    pub fn set_painter(
        &mut self,
        f: impl FnMut(usize, Rect, &mut dyn RenderCtx) + 'static,
    ) -> bool {
        self.painter = Some(Box::new(f));
        true
    }

    /// 便利构造：纯文本行（每行居左画一行字）。
    ///
    /// 数据被捕获在回调里，**不进** `ListView` 的字段——
    /// 所以它依然是通用的虚拟列表。
    pub fn from_strings(id: impl Into<String>, items: Vec<String>) -> Self {
        Self::new(id, 20.0)
            .len(items.len())
            .painter(move |i, rect, ctx| {
                if let Some(text) = items.get(i) {
                    let fg = ctx.theme().style(Role::TEXT, State::Normal).fg;
                    let style = y901_ui::TextStyle::fg(fg);
                    ctx.draw_text(Pos::new(rect.x + 8.0, rect.y + 2.0), text, &style);
                }
            })
    }

    /// 开启/关闭粘底（日志/聊天场景）。
    pub fn set_stick_to_bottom(&mut self, on: bool) -> bool {
        self.stick_to_bottom = on;
        self.stick_armed = on;
        if on {
            self.offset = self.max_offset();
        }
        true
    }

    /// 构建期开启粘底。
    pub fn stick_to_bottom(mut self, on: bool) -> Self {
        self.stick_to_bottom = on;
        self.stick_armed = on;
        self
    }

    /// 是否已在底部（容差 0.5px）。
    pub fn is_at_bottom(&self) -> bool {
        self.offset >= self.max_offset() - 0.5
    }

    /// 滚到底部并重新激活粘底。
    pub fn scroll_to_end(&mut self) {
        self.stick_armed = true;
        self.offset = self.max_offset();
    }

    /// 滚到指定行（尽量让它在可见区顶部）。
    pub fn scroll_to_row(&mut self, index: usize) -> bool {
        let target = self.row_top(index).min(self.max_offset()).max(0.0);
        if (self.offset - target).abs() < 0.01 {
            return false;
        }
        self.set_offset(target);
        true
    }

    /// 某行当前的屏幕矩形（不可见返回 `None`）。
    ///
    /// 用于配合 `UiTree::invalidate_rect()` 做**单行**标脏。
    pub fn row_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.len {
            return None;
        }
        let y = self.inner.y + self.row_top(index) - self.offset;
        let h = self.row_height_at(index);
        let r = Rect::new(self.inner.x, y, self.inner.w, h);
        let visible = r.intersect(self.inner);
        if visible.is_empty() {
            None
        } else {
            Some(visible)
        }
    }

    /// 单行标脏矩形（已裁到可见区）。
    pub fn invalidate_row(&self, index: usize) -> Option<Rect> {
        self.row_rect(index)
    }

    /// 当前可见的行区间 `[start, end)`。
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        if self.len == 0 || self.inner.h <= 0.0 || self.prefix.len() != self.len + 1 {
            return 0..0;
        }
        let Some(first) = self.index_at_content_y(self.offset.max(0.0)) else {
            // 偏移落在内容之外（空列表/越界）
            return 0..0;
        };
        let bottom = self.offset + self.inner.h;
        let mut end = first;
        while end < self.len && self.prefix[end] < bottom {
            end += 1;
        }
        first..end
    }

    /// 设置迪归属的 flex 参数。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 设置选中行。
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index;
        self
    }

    /// 选中回调（应用层自己决定含义）。
    pub fn on_row_selected(mut self, f: impl FnMut(usize) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// 当前选中行。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 当前滚动偏移（像素）。
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// 本次布局中参与绘制的可见行数。
    pub fn arranged_rows(&self) -> usize {
        self.arranged_rows
    }

    /// 内容总高度（解析值）。
    pub fn content_height(&self) -> f32 {
        if self.prefix.len() == self.len + 1 {
            self.prefix.last().copied().unwrap_or(0.0)
        } else if let Some(p) = &self.row_height_provider {
            (0..self.len).map(|i| p(i).max(1.0)).sum()
        } else {
            self.len as f32 * self.row_height
        }
    }

    /// 总行数。
    pub fn row_count(&self) -> usize {
        self.len
    }

    /// 设置滚动条样式。
    pub fn set_bar_style(&mut self, style: Option<crate::style::WidgetStyle>) -> bool {
        self.bar.set_style(style)
    }

    fn max_offset(&self) -> f32 {
        (self.content_height() - self.inner.h).max(0.0)
    }

    fn set_offset(&mut self, v: f32) -> bool {
        let v = v.clamp(0.0, self.max_offset());
        if (self.offset - v).abs() < 0.01 {
            return false;
        }
        self.offset = v;
        self.stick_armed = self.stick_to_bottom && self.is_at_bottom();
        true
    }

    /// 屏幕坐标命中的行下标（已考虚滚动偏移）。
    ///
    /// 输入：`pos`（客户区坐标）。
    /// 返回：`Some(i)` = 命中第 i 行；`None` = 在可见区外或超出行数。
    pub fn row_at(&self, pos: Pos) -> Option<usize> {
        if !self.inner.contains(pos) {
            return None;
        }
        let y = pos.y - self.inner.y + self.offset;
        self.index_at_content_y(y)
    }
}

impl Widget for ListView {
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 动态行高需要在布局前就知道内容高度
        self.ensure_prefix();
        c.constrain(Size::new(c.max.w, c.max.h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.ensure_prefix();
        self.viewport = rect;
        self.show_bar = self.content_height() > rect.h;
        let bar_w = if self.show_bar { BAR_WIDTH } else { 0.0 };
        self.inner = Rect::new(rect.x, rect.y, (rect.w - bar_w).max(0.0), rect.h);
        self.offset = self.offset.clamp(0.0, self.max_offset());
        if self.stick_armed {
            self.offset = self.max_offset();
        }
        self.arranged_rows = self.visible_range().len();

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
        if ctx.phase == EventPhase::Capture {
            return EventResult::Ignored;
        }
        // 顺序即优先级：滚动条必须抢在行点击之前
        match event {
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
            UiEvent::MouseDown { pos, .. } => {
                if let Some(index) = self.row_at(*pos) {
                    self.selected = Some(index);
                    if let Some(cb) = self.on_select.as_mut() {
                        cb(index);
                    }
                    return EventResult::Relayout;
                }
                EventResult::Ignored
            }
            UiEvent::MouseMove { pos } => {
                let was = self.hovered;
                self.hovered = self.row_at(*pos);
                if was != self.hovered {
                    EventResult::Relayout
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::Wheel { pos, delta, .. } => {
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
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let bg = ctx.theme().style(Role::LIST, State::Normal).bg;
        ctx.fill_rect(self.viewport, bg);

        // 可见行：框架画选中/悬停底色，内容交给回调。
        let range = self.visible_range();
        let sel_bg = ctx.theme().style(Role::LIST, State::Focused).bg;
        let hov_bg = ctx.theme().style(Role::LIST, State::Hovered).bg;
        let mut painter = self.painter.take();
        for index in range {
            let Some(rect) = self.row_rect(index) else {
                continue;
            };
            if Some(index) == self.selected {
                ctx.fill_rect(rect, sel_bg);
            } else if Some(index) == self.hovered {
                ctx.fill_rect(rect, hov_bg);
            }
            if let Some(f) = painter.as_mut() {
                f(index, rect, ctx);
            }
        }
        self.painter = painter;

        self.bar.render_bar(ctx);
    }

    fn semantics(&self) -> Semantics {
        let visible = self.visible_range();
        Semantics::default()
            .exposed(true)
            // 名字里不念行数（读屏器改成播报"列表，N 项"由 state 里的 rows 驱动）。
            .label(self.label.clone())
            .action_spec("scroll_to_ratio", InputKind::Number)
            .state("rows", self.len as u32)
            .state("offset", self.offset)
            .state("arranged", self.arranged_rows as u32)
            .state("first_visible", visible.start as u32)
            .state(
                "selected",
                self.selected.map(|i| i as u32).unwrap_or(u32::MAX),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use y901_ui::{layout::LayoutCache, TextStyle, UiTree};

    struct Mono;
    impl TextMetrics for Mono {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 8.0, 16.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            20.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    fn big_list(painted: Rc<Cell<usize>>) -> ListView {
        let n = 10_000;
        ListView::new("list", 20.0)
            .len(n)
            .painter(move |_i, _r, _ctx| painted.set(painted.get() + 1))
            .flex_item(FlexItem::fill())
    }

    #[test]
    fn only_visible_rows_are_painted() {
        let painted = Rc::new(Cell::new(0));
        let mut list = big_list(painted.clone());
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 100.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);

        assert_eq!(list.content_height(), 200_000.0, "内容高度是解析值");
        assert_eq!(list.row_count(), 10_000);
        assert!(
            list.arranged_rows() <= 7,
            "10k 行只应算可见的几行，实际 {}",
            list.arranged_rows()
        );
        assert_eq!(list.children().len(), 0, "行不再是子节点（由回调画）");
    }

    #[test]
    fn scroll_keeps_visible_window_small() {
        let painted = Rc::new(Cell::new(0));
        let mut list = big_list(painted);
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 100.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);

        let mut ctx = EventCtx::new(None, Size::new(300.0, 100.0));
        for _ in 0..20 {
            list.event(
                &mut ctx,
                &UiEvent::Wheel {
                    pos: Pos::new(10.0, 10.0),
                    delta: -120.0,
                    mods: y901_ui::Modifiers::default(),
                },
            );
            list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);
        }

        assert!(list.offset() > 0.0, "应真的滚动了");
        assert!(
            list.arranged_rows() <= 7,
            "滚动后仍只算可见行：{}",
            list.arranged_rows()
        );
    }

    #[test]
    fn visible_range_follows_offset() {
        let mut list = big_list(Rc::new(Cell::new(0)));
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 100.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);
        assert_eq!(list.visible_range().start, 0);

        // 滚到第 50 行（=1000px）
        list.scroll_to_row(50);
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);
        assert_eq!(list.visible_range().start, 50, "可见区应从第 50 行开始");
        assert!(
            list.row_rect(50).is_some() && list.row_rect(49).is_none(),
            "第 50 行可见、第 49 行不可见"
        );
    }

    #[test]
    fn sticky_bottom_follows_len_growth() {
        let mut list = ListView::new("log", 20.0)
            .len(5)
            .stick_to_bottom(true)
            .flex_item(FlexItem::fill());
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 100.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);
        assert!(list.is_at_bottom());

        // 新行进来：应自动跟到底
        list.set_len(50);
        list.arrange(Rect::new(0.0, 0.0, 300.0, 100.0), &Mono, &mut cache);
        assert!(list.is_at_bottom(), "粘底应跟着新内容");
        assert!(
            list.visible_range().end == 50,
            "可见区应到最后一行：{:?}",
            list.visible_range()
        );
    }

    #[test]
    fn dynamic_row_heights_shape_total_and_rects() {
        // 三行，高度 10/30/50
        let mut list = ListView::new("chat", 20.0)
            .len(3)
            .flex_item(FlexItem::fill());
        let heights = [10.0f32, 30.0, 50.0];
        list.set_row_height_provider(move |i| heights.get(i).copied().unwrap_or(20.0));

        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 400.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 400.0), &Mono, &mut cache);

        assert_eq!(list.content_height(), 90.0, "总高度 = 10+30+50");
        assert_eq!(list.row_height_at(1), 30.0);
        assert_eq!(list.row_rect(0).unwrap().h, 10.0);
        assert_eq!(list.row_rect(1).unwrap().y, 10.0, "第二行顶部 = 10");
        assert_eq!(list.row_rect(2).unwrap().y, 40.0);
        assert_eq!(list.row_rect(2).unwrap().h, 50.0);
    }

    #[test]
    fn row_at_matches_row_rect_with_variable_heights() {
        let mut list = ListView::new("chat", 20.0)
            .len(3)
            .flex_item(FlexItem::fill());
        let heights = [10.0f32, 30.0, 50.0];
        list.set_row_height_provider(move |i| heights.get(i).copied().unwrap_or(20.0));
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 400.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 400.0), &Mono, &mut cache);

        for i in 0..3 {
            let r = list.row_rect(i).unwrap();
            // 行中心和上沿都应命中该行
            assert_eq!(list.row_at(Pos::new(r.x + 1.0, r.y + r.h * 0.5)), Some(i));
            assert_eq!(list.row_at(Pos::new(r.x + 1.0, r.y + 0.5)), Some(i));
            // 下沿（y + h - 0.5）仍属于该行
            assert_eq!(list.row_at(Pos::new(r.x + 1.0, r.y + r.h - 0.5)), Some(i));
        }
        // 内容之外（> 90px）不命中
        assert_eq!(list.row_at(Pos::new(1.0, 95.0)), None);
    }

    #[test]
    fn visible_range_with_variable_heights() {
        // 100 行，每行 10px；视口 95px → 可见约 10 行
        let mut list = ListView::new("chat", 20.0)
            .len(100)
            .flex_item(FlexItem::fill());
        list.set_row_height_provider(|_| 10.0);
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 95.0)),
            &mut cache,
        );
        list.arrange(Rect::new(0.0, 0.0, 300.0, 95.0), &Mono, &mut cache);

        let r = list.visible_range();
        assert_eq!(r.start, 0);
        assert!(
            (9..=11).contains(&r.len()),
            "95px 视口、每行 10px 应约 10 行，实际 {:?}",
            r
        );

        // 滚到第 50 行：起点应是 50
        list.scroll_to_row(50);
        list.arrange(Rect::new(0.0, 0.0, 300.0, 95.0), &Mono, &mut cache);
        assert_eq!(list.visible_range().start, 50);
    }

    #[test]
    fn height_provider_change_recomputes_layout() {
        let mut list = ListView::new("chat", 20.0)
            .len(4)
            .flex_item(FlexItem::fill());
        list.set_row_height_provider(|_| 10.0);
        let mut cache = LayoutCache::new();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 200.0)),
            &mut cache,
        );
        assert_eq!(list.content_height(), 40.0);

        // 换一个提供者：内容高度必须变
        list.set_row_height_provider(|_| 25.0);
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 200.0)),
            &mut cache,
        );
        assert_eq!(list.content_height(), 100.0, "新提供者应生效");

        // 清掉提供者：回到固定 20px
        list.clear_row_height_provider();
        list.measure(
            &Mono,
            Constraints::loose(Size::new(300.0, 200.0)),
            &mut cache,
        );
        assert_eq!(list.content_height(), 80.0);
    }

    #[test]
    fn tree_owns_list_and_hit_tests_rows() {
        let mut tree = UiTree::new(Box::new(big_list(Rc::new(Cell::new(0)))));
        tree.layout(Size::new(300.0, 100.0), &Mono);
        assert_eq!(tree.hit_test(Pos::new(10.0, 10.0)).as_deref(), Some("list"));
    }
}
