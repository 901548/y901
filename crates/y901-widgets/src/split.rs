//! 通用分栏：可拖拽的两栏容器（左右 / 上下）。
//!
//! 与现有机制的关系：
//! - 拖拽用**指针捕获**，鼠标移出分栏也能继续拖；
//! - 比例变化只影响 `arrange`，不触发 `measure` 失效（尺寸约束没变）；
//! - `cursor()` 让应用层在悬浮分隔条时把鼠标变成左右/上下箭头。

use y901_ui::{
    Axis, Constraints, Cursor, Edges, EventCtx, EventResult, FlexItem, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, UiEvent, Widget,
};

/// 分隔条命中宽度（视觉 1px，热区 ±HIT_SLOP）。
const DIVIDER: f32 = 1.0;
const HIT_SLOP: f32 = 4.0;

/// 两栏分隔容器：可拖动分隔条，两侧各有最小尺寸。
pub struct SplitView {
    id: String,
    /// 两栏，顺序固定：first 在左/上。
    kids: Vec<Box<dyn Widget>>,
    axis: Axis,
    /// 第一栏占比（0..1）。
    ratio: f32,
    /// 第一栏最小像素宽/高。
    min_first: f32,
    /// 第二栏最小像素宽/高。
    min_second: f32,
    rect: Rect,
    item: FlexItem,
    dragging: bool,
    hover_divider: bool,
}

impl SplitView {
    /// 新建分栏容器：两栏 + 中间可拖拽的分隔条。
    ///
    /// 输入：`id`、`axis`（水平=左右分栏，垂直=上下分栏）、两个子节点。
    /// 返回：默认 `ratio = 0.28`、最小宽 120 / 200 像素。
    /// 副作用：无。子节点顺序即"第一栏 / 第二栏"。
    pub fn new(
        id: impl Into<String>,
        axis: Axis,
        first: Box<dyn Widget>,
        second: Box<dyn Widget>,
    ) -> Self {
        Self {
            id: id.into(),
            kids: vec![first, second],
            axis,
            ratio: 0.28,
            min_first: 120.0,
            min_second: 200.0,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            dragging: false,
            hover_divider: false,
        }
    }

    /// `new(id, Axis::Horizontal, …)` 的快捷方式：左右分栏。
    ///
    /// 返回：未挂载的分栏容器。副作用：无。
    pub fn horizontal(
        id: impl Into<String>,
        first: Box<dyn Widget>,
        second: Box<dyn Widget>,
    ) -> Self {
        Self::new(id, Axis::Horizontal, first, second)
    }

    /// `new(id, Axis::Vertical, …)` 的快捷方式：上下分栏。
    ///
    /// 返回：未挂载的分栏容器。副作用：无。
    pub fn vertical(
        id: impl Into<String>,
        first: Box<dyn Widget>,
        second: Box<dyn Widget>,
    ) -> Self {
        Self::new(id, Axis::Vertical, first, second)
    }

    /// 设置第一栏占比（0..1）。
    ///
    /// 输入：`ratio`，**会被夹到 [0.05, 0.95]**（防止把某一栏拖没）。
    /// 返回：自身。副作用：无（下一次布局生效）。
    pub fn ratio(mut self, ratio: f32) -> Self {
        self.ratio = ratio.clamp(0.05, 0.95);
        self
    }

    /// 两栏的最小像素尺寸（拖拽时不会低于它）。
    ///
    /// 输入：`first` / `second`，**负数会被夹到 0**。
    /// 返回：自身。副作用：无。
    pub fn min_sizes(mut self, first: f32, second: f32) -> Self {
        self.min_first = first.max(0.0);
        self.min_second = second.max(0.0);
        self
    }

    /// 设置 flex 伸缩参数。
    ///
    /// 返回：自身。副作用：无（父容器测量时读取）。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 当前实际占比（拖拽后的真实值，可能因最小尺寸约束而与 `ratio()` 不同）。
    ///
    /// 返回：`0.05..=0.95` 之间的浮点。副作用：无。
    pub fn current_ratio(&self) -> f32 {
        self.ratio
    }

    fn along(&self) -> f32 {
        match self.axis {
            Axis::Horizontal => self.rect.w,
            Axis::Vertical => self.rect.h,
        }
    }

    /// 第一块的实际主轴长度。
    ///
    /// **必须只有这一份实现**：`arrange` 用它摆两块，`divider_pos` 用它画分隔条，
    /// 命中测试与拖拽起点也用它。旧版 `arrange` 把结果按 `min_first` 钳过，
    /// 而 `divider_pos` 直接用 `ratio * total` —— 一旦 `min_first` 生效
    /// （例如 400px 宽的默认 0.28 → 112 < 120），分隔条就被画到**第一块内部**，
    /// 而真正 1px 的间隙没人画（露出背景），鼠标也得点在错的位置才能拖动。
    ///
    /// `min_second` 也在这里落实：不能只顾第一块把第二块挤成 0。
    fn first_len(&self) -> f32 {
        let total = self.along();
        if total <= 0.0 {
            return 0.0;
        }
        let want = total * self.ratio;
        let min_first = self.min_first.min(total);
        let min_second = self.min_second.min(total);
        let upper = (total - DIVIDER - min_second).max(min_first).min(total);
        want.max(min_first).min(upper).max(0.0)
    }

    /// 分隔条中心（主轴方向的绝对坐标）。
    fn divider_pos(&self) -> f32 {
        let start = match self.axis {
            Axis::Horizontal => self.rect.x,
            Axis::Vertical => self.rect.y,
        };
        start + self.first_len()
    }

    fn on_divider(&self, pos: Pos) -> bool {
        let center = self.divider_pos();
        let along = match self.axis {
            Axis::Horizontal => pos.x,
            Axis::Vertical => pos.y,
        };
        let cross_ok = match self.axis {
            Axis::Horizontal => pos.y >= self.rect.y && pos.y <= self.rect.bottom(),
            Axis::Vertical => pos.x >= self.rect.x && pos.x <= self.rect.right(),
        };
        cross_ok && (along - center).abs() <= HIT_SLOP
    }

    /// 依据拖拽位置更新比例（受最小尺寸约束）。
    fn update_ratio(&mut self, pos: Pos) {
        let total = self.along();
        if total <= 0.0 {
            return;
        }
        let along = match self.axis {
            Axis::Horizontal => pos.x - self.rect.x,
            Axis::Vertical => pos.y - self.rect.y,
        };
        let min = (self.min_first / total).clamp(0.0, 0.95);
        let max = (1.0 - self.min_second / total).clamp(0.05, 1.0);
        let next = (along / total).clamp(min.min(max), max.max(min));
        self.ratio = next;
    }
}

impl Widget for SplitView {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("split_view")
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 两栏都按"能占满"来测，实际尺寸在 arrange 里按比例分
        for k in self.kids.iter_mut() {
            let _ = cache.measure(k.as_mut(), m, c);
        }
        c.constrain(c.max)
    }

    fn arrange(
        &mut self,
        rect: Rect,
        m: &dyn TextMetrics,
        cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        let first_len = self.first_len();
        let total = self.along();
        let second_len = (total - first_len - DIVIDER).max(0.0);

        let (a, b) = match self.axis {
            Axis::Horizontal => (
                Rect::new(rect.x, rect.y, first_len, rect.h),
                Rect::new(rect.x + first_len + DIVIDER, rect.y, second_len, rect.h),
            ),
            Axis::Vertical => (
                Rect::new(rect.x, rect.y, rect.w, first_len),
                Rect::new(rect.x, rect.y + first_len + DIVIDER, rect.w, second_len),
            ),
        };

        self.kids[0].arrange(a, m, cache);
        self.kids[1].arrange(b, m, cache);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn cursor(&self) -> Cursor {
        if self.hover_divider || self.dragging {
            match self.axis {
                Axis::Horizontal => Cursor::ResizeHorizontal,
                Axis::Vertical => Cursor::ResizeVertical,
            }
        } else {
            Cursor::Default
        }
    }

    fn cursor_at(&self, pos: y901_ui::Pos) -> Cursor {
        if self.dragging || self.on_divider(pos) {
            match self.axis {
                Axis::Horizontal => Cursor::ResizeHorizontal,
                Axis::Vertical => Cursor::ResizeVertical,
            }
        } else {
            Cursor::Default
        }
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseDown { pos, .. } if self.on_divider(*pos) || self.dragging => {
                ctx.set_pointer_capture(self.id.clone());
                self.dragging = true;
                self.update_ratio(*pos);
                ctx.request_relayout();
                EventResult::Relayout
            }
            UiEvent::MouseMove { pos } => {
                if self.dragging {
                    self.update_ratio(*pos);
                    EventResult::Relayout
                } else {
                    let was = self.hover_divider;
                    self.hover_divider = self.on_divider(*pos);
                    if was != self.hover_divider {
                        EventResult::Redraw
                    } else {
                        EventResult::Ignored
                    }
                }
            }
            UiEvent::MouseUp { .. } if self.dragging => {
                self.dragging = false;
                ctx.release_pointer_capture();
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let theme = ctx.theme();
        let state = if self.dragging {
            State::Pressed
        } else if self.hover_divider {
            State::Hovered
        } else {
            State::Normal
        };
        let color = theme.style(Role("split_view"), state).bg;
        let bar = match self.axis {
            Axis::Horizontal => Rect::new(self.divider_pos(), self.rect.y, DIVIDER, self.rect.h),
            Axis::Vertical => Rect::new(self.rect.x, self.divider_pos(), self.rect.w, DIVIDER),
        };
        ctx.fill_rect(bar, color);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label("split")
            .state("ratio", self.ratio)
            .state("dragging", self.dragging)
    }

    fn padding(&self) -> Edges {
        Edges::ZERO
    }
}
