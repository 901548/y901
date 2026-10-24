//! 通用滚动基座：`ScrollView` + `Scrollbar`。
//!
//! 坐标系契约：
//! - `ScrollView::arrange` 把子节点排在**内容坐标系**里（`y - offset`）；
//! - `ScrollView::clip()` 返回视口，框架据此裁剪绘制、约束命中；
//! - 滚动条是独立 Widget，通过 `Command::Node` 把自己的拖拽换算成比例发给滚动视图，
//!   两者互不持有对方类型。

use y901_ui::{
    Command, Constraints, Edges, EventCtx, EventResult, FlexItem, Json, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextStyle, TextMetrics, UiEvent, Widget,
};

/// 滚动条宽度（逻辑像素）。
pub const BAR_WIDTH: f32 = 10.0;

/// 竖向滚动容器：**主轴无界测量**子节点，超出部分靠视口裁剪。
pub struct ScrollView {
    id: String,
    kids: Vec<Box<dyn Widget>>,
    bar: Scrollbar,
    /// 当前滚动偏移（像素，0 = 顶部）。
    offset: f32,
    /// 内容总高度（measure 时得到）。
    content_h: f32,
    viewport: Rect,
    /// 视口（去掉滚动条）。
    inner: Rect,
    item: FlexItem,
    /// 是否显示滚动条（内容不足时自动隐藏）。
    show_bar: bool,
}

impl ScrollView {
    pub fn new(id: impl Into<String>, child: Box<dyn Widget>) -> Self {
        let id = id.into();
        Self {
            bar: Scrollbar::new(format!("{id}.bar"), id.clone()),
            id,
            kids: vec![child],
            offset: 0.0,
            content_h: 0.0,
            viewport: Rect::ZERO,
            inner: Rect::ZERO,
            item: FlexItem::default(),
            show_bar: true,
        }
    }

    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn bar_id(&self) -> &str {
        &self.bar.id
    }

    fn max_offset(&self) -> f32 {
        (self.content_h - self.inner.h).max(0.0)
    }

    fn set_offset(&mut self, value: f32) -> bool {
        let clamped = value.clamp(0.0, self.max_offset());
        if (clamped - self.offset).abs() < 0.01 {
            return false;
        }
        self.offset = clamped;
        true
    }
}

impl Widget for ScrollView {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("scroll_view")
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
        metrics: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 主轴无界：问内容到底多高
        let content = cache.measure(
            self.kids[0].as_mut(),
            metrics,
            Constraints::unbounded_main(y901_ui::Axis::Vertical, c.max),
        );
        self.content_h = content.h;
        // 视口自身取父容器给的尺寸
        c.constrain(Size::new(content.w.min(c.max.w), c.max.h))
    }

    fn arrange(&mut self, rect: Rect, metrics: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.viewport = rect;
        self.show_bar = self.content_h > rect.h + 0.5;
        let bar_w = if self.show_bar { BAR_WIDTH } else { 0.0 };
        self.inner = Rect::new(rect.x, rect.y, (rect.w - bar_w).max(0.0), rect.h);

        // 内容坐标系：子节点按未滚动的绝对高度排，屏幕位置靠平移得到
        let content_rect = Rect::new(self.inner.x, self.inner.y - self.offset, self.inner.w, self.content_h);
        self.kids[0].arrange(content_rect, metrics, cache);

        self.offset = self.offset.clamp(0.0, self.max_offset());

        // 同步滚动条几何
        self.bar.arrange_at(
            Rect::new(rect.right() - BAR_WIDTH, rect.y, BAR_WIDTH, rect.h),
            if self.content_h > 0.0 {
                (rect.h / self.content_h).min(1.0)
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
        // 三阶段意味着祖先会被调用两次（Capture + Bubble）。
        // 滚动是"响应"而非"拦截"，只在非捕获阶段处理，否则会滚两倍。
        if ctx.phase == y901_ui::EventPhase::Capture {
            return EventResult::Ignored;
        }
        match event {
            // 滚动条：按下即捕获，拖拽换算比例
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
            UiEvent::MouseMove { pos } if self.show_bar => {
                let was = self.bar.hovered;
                self.bar.hovered = self.bar.rect.contains(*pos);
                if was != self.bar.hovered {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::Wheel { pos, delta } => {
                if !self.inner.contains(*pos) {
                    return EventResult::Ignored;
                }
                let step = *delta / 120.0 * 48.0;
                if self.set_offset(self.offset - step) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Action { name, args } if name == "scroll_to_ratio" => {
                let ratio = args
                    .get("ratio")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                if self.set_offset(ratio * self.max_offset()) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Action { name, args } if name == "scroll_to" => {
                let y = args.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                if self.set_offset(y) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let style = ctx.theme().style(Role::PANEL, State::Normal);
        ctx.fill_rect(self.viewport, style.bg);
        self.bar.render_bar(ctx);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
            .label("scroll")
            .action_spec("scroll_to", y901_ui::InputKind::Number)
            .action_spec("scroll_to_ratio", y901_ui::InputKind::Number)
            .state("offset", self.offset)
            .state("content_height", self.content_h)
            .state("viewport_height", self.inner.h)
    }
}

/// 竖向滚动条。拖拽时把自己换算成比例交给目标滚动视图。
pub struct Scrollbar {
    pub(crate) id: String,
    #[allow(dead_code)] // 留作把滚动条拆成独立组件时用
    target: String,
    pub(crate) rect: Rect,
    pub(crate) visible: bool,
    /// 滑块占轨道的比例（viewport / content）
    pub(crate) thumb_ratio: f32,
    /// 滑块起点在轨道中的比例
    start_ratio: f32,
    pub(crate) dragging: bool,
    hovered: bool,
    /// 独立使用时的三个量：内容总高 / 视口高 / 当前偏移。
    total: f32,
    viewport: f32,
    offset: f32,
    /// 拖拽回调：参数是换算后的新 offset。
    on_change: Option<Box<dyn FnMut(f32)>>,
}

impl Scrollbar {
    pub fn new(id: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            target: target.into(),
            rect: Rect::ZERO,
            visible: false,
            thumb_ratio: 1.0,
            start_ratio: 0.0,
            dragging: false,
            hovered: false,
            total: 0.0,
            viewport: 0.0,
            offset: 0.0,
            on_change: None,
        }
    }

    pub(crate) fn arrange_at(&mut self, rect: Rect, thumb_ratio: f32, start_ratio: f32, visible: bool) {
        self.rect = rect;
        self.visible = visible;
        self.thumb_ratio = thumb_ratio.clamp(0.05, 1.0);
        self.start_ratio = start_ratio.clamp(0.0, 1.0 - self.thumb_ratio);
    }

    pub(crate) fn thumb(&self) -> Rect {
        let h = (self.rect.h * self.thumb_ratio).max(24.0);
        let y = self.rect.y + (self.rect.h - h) * self.start_ratio;
        Rect::new(self.rect.x + 2.0, y, (self.rect.w - 4.0).max(2.0), h)
    }

    pub(crate) fn ratio_at(&self, y: f32) -> f32 {
        if self.rect.h <= 0.0 {
            return 0.0;
        }
        ((y - self.rect.y) / self.rect.h).clamp(0.0, 1.0)
    }
}

impl Scrollbar {
    pub(crate) fn render_bar(&self, ctx: &mut dyn RenderCtx) {
        if !self.visible {
            return;
        }
        let state = if self.dragging {
            State::Pressed
        } else if self.hovered {
            State::Hovered
        } else {
            State::Normal
        };
        let track = ctx.theme().style(Role("scrollbar"), State::Disabled).bg;
        ctx.fill_rect(self.rect, track);
        let thumb = ctx.theme().style(Role("scrollbar"), state).bg;
        ctx.fill_rect(self.thumb(), thumb);
    }
}
/// 便捷：命令行式滚动（组件外部调用）。
pub fn scroll_command(target: &str, y: f32) -> Command {
    Command::node(target, "scroll_to", Json::obj().set("y", y))
}

/// 供测试/调试：按行高构造内容。
pub fn rows(metrics: &dyn TextMetrics, count: usize) -> f32 {
    metrics.line_height(&TextStyle::default()) * count as f32
}

/// 边距常量（无滚动条时用）。
pub const NO_PADDING: Edges = Edges::ZERO;

/// 兼容 `Pos` 的显式导入（某些平台构建需要）。
#[allow(dead_code)]
fn _pos(_: Pos) {}

impl Scrollbar {
    /// 独立使用：设置内容总高、视口高、当前偏移。
    pub fn tracker(mut self, total: f32, viewport: f32, offset: f32) -> Self {
        self.total = total.max(0.0);
        self.viewport = viewport.max(0.0);
        self.offset = offset.max(0.0);
        self.refresh_ratios();
        self
    }

    /// 拖拽回调：拖动时把新 offset 交给业务代码。
    pub fn on_change(mut self, f: impl FnMut(f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    pub fn offset(&self) -> f32 {
        self.offset
    }

    fn refresh_ratios(&mut self) {
        let max = (self.total - self.viewport).max(0.0);
        self.thumb_ratio = if self.total > 0.0 {
            (self.viewport / self.total).clamp(0.05, 1.0)
        } else {
            1.0
        };
        self.start_ratio = if max > 0.0 {
            (self.offset / max).clamp(0.0, 1.0 - self.thumb_ratio)
        } else {
            0.0
        };
    }

    /// 根据拖拽位置更新 offset 并触发回调。
    fn drag_to(&mut self, y: f32) -> bool {
        let max = (self.total - self.viewport).max(0.0);
        if max <= 0.0 {
            return false;
        }
        let ratio = self.ratio_at(y) - self.thumb_ratio * 0.5;
        let next = (ratio * max).clamp(0.0, max);
        if (next - self.offset).abs() < 0.01 {
            return false;
        }
        self.offset = next;
        self.refresh_ratios();
        if let Some(cb) = self.on_change.as_mut() {
            cb(self.offset);
        }
        true
    }
}

impl Widget for Scrollbar {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("scrollbar")
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(Size::new(BAR_WIDTH, c.max.h))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = rect;
        self.visible = self.total > self.viewport;
        self.refresh_ratios();
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseDown { pos, .. } if self.visible && self.rect.contains(*pos) => {
                ctx.set_pointer_capture(self.id.clone());
                self.dragging = true;
                self.drag_to(pos.y);
                EventResult::Redraw
            }
            UiEvent::MouseMove { pos } if self.dragging => {
                if self.drag_to(pos.y) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::MouseUp { .. } if self.dragging => {
                self.dragging = false;
                ctx.release_pointer_capture();
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        self.render_bar(ctx);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
            .label("scrollbar")
            .state("offset", self.offset)
            .state("total", self.total)
            .state("viewport", self.viewport)
    }
}