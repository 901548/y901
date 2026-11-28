//! 通用滚动基座：`ScrollView` + `Scrollbar`。
//!
//! 坐标系契约：
//! - `ScrollView::arrange` 把子节点排在**内容坐标系**里（`y - offset`）；
//! - `ScrollView::clip()` 返回视口，框架据此裁剪绘制、约束命中；
//! - 滚动条是独立 Widget，通过 `Command::Node` 把自己的拖拽换算成比例发给滚动视图，
//!   两者互不持有对方类型。

use y901_ui::{
    Command, Constraints, Edges, EventCtx, EventResult, FlexItem, Json, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 滚动条宽度（逻辑像素）。
pub const BAR_WIDTH: f32 = 10.0;

/// 竖向滚动容器：**主轴无界测量**子节点，超出部分靠视口裁剪。
pub struct ScrollView {
    id: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
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
    /// 粘底：内容变高时自动跟到最底（聊天/日志场景）。
    stick_to_bottom: bool,
    /// 粘底是否处于激活状态：用户上滚后失效，回到底部自动重新激活。
    stick_armed: bool,
}

impl ScrollView {
    /// 新建滚动容器：单个子节点 + 右侧自动出现的滚动条。
    ///
    /// 输入：`id`（树内唯一，滚动条 id 为 `{id}.bar`）、`child`（内容）。
    /// 返回：未挂载的容器，`offset = 0`。
    /// 副作用：无（滚动条是子组件，随容器一起挂载/卸载）。
    pub fn new(id: impl Into<String>, child: Box<dyn Widget>) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            bar: Scrollbar::new(format!("{id}.bar"), id.clone()),
            id,
            key,
            kids: vec![child],
            offset: 0.0,
            content_h: 0.0,
            viewport: Rect::ZERO,
            inner: Rect::ZERO,
            item: FlexItem::default(),
            show_bar: true,
            stick_to_bottom: false,
            stick_armed: false,
        }
    }

    /// 开启/关闭粘底；开启时立即滚到最底并激活粘底。
    pub fn set_stick_to_bottom(&mut self, on: bool) -> bool {
        if self.stick_to_bottom == on {
            return false;
        }
        self.stick_to_bottom = on;
        self.stick_armed = on;
        if on {
            // 开启时直接贴到底（不管之前在哪）
            self.offset = self.max_offset();
        }
        true
    }

    /// 构造期开启粘底。
    pub fn stick_to_bottom(mut self, on: bool) -> Self {
        self.stick_to_bottom = on;
        self.stick_armed = on;
        self
    }

    /// 是否已经在最底部（容差 0.5px）。
    pub fn is_at_bottom(&self) -> bool {
        self.offset >= self.max_offset() - 0.5
    }

    /// 滚到最底部（并重新激活粘底）。返回是否真的改变了偏移。
    pub fn scroll_to_end(&mut self) -> bool {
        self.stick_armed = true;
        let end = self.max_offset();
        self.set_offset(end)
    }

    /// 设置 flex 伸缩参数（滚动容器常用 `FlexItem::fill()`）。
    ///
    /// 返回：自身。副作用：无。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 当前滚动偏移（像素，往下的距离）。
    ///
    /// 返回：`0..=max_offset()`；内容不足一屏时恒为 0。副作用：无。
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// 伴随滚动条的 id（`{容器 id}.bar`），供外部工具 `query` / `invoke` 定位。
    ///
    /// 返回：借用字符串。副作用：无。
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
        // 用户（或代码）改了偏移：只有停在底部时才继续粘底。
        self.stick_armed = self.is_at_bottom();
        true
    }

    /// 转发到内部滚动条（轨道/滑块颜色、宽度、圆角）。
    pub fn set_bar_style(&mut self, style: Option<crate::style::WidgetStyle>) -> bool {
        self.bar.set_style(style)
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
        // **结构性容器：子节点槽位固定（恰好 1 个内容节点）**。
        //
        // 为什么返回 `None`：`UiTree::add_node` / `remove_node` 会走这条路径。
        // 允许外部增删会破坏不变量 —— 删掉唯一子节点后 `measure`/`arrange` 里的
        // `kids[0]` 直接越界 panic；加第二个子节点则永远不会被布局（静默无效）。
        // 返回 `None` 让这些接口**干净地失败**（返回 false），而不是留下坏状态。
        //
        // 换内容请整棵替换 Scroll，或改内容控件自身的状态。
        None
    }

    /// 框架内部只读遍历仍可下钻（`node_at_mut` / `render_pass` 需要）。
    fn children_mut_for_traversal(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 主轴无界：问内容到底多高
        // （`kids` 正常恰好 1 个；这里用 `first_mut` 而不是 `[0]`，
        //   把"不变量被破坏"从 panic 降级成"内容高度为 0"。）
        let content = match self.kids.first_mut() {
            Some(kid) => cache.measure(
                kid.as_mut(),
                metrics,
                Constraints::unbounded_main(y901_ui::Axis::Vertical, c.max),
            ),
            None => Size::new(c.max.w.min(0.0), 0.0),
        };
        self.content_h = content.h;
        // 视口自身取父容器给的尺寸
        c.constrain(Size::new(content.w.min(c.max.w), c.max.h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        metrics: &dyn TextMetrics,
        cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.viewport = rect;
        self.show_bar = self.content_h > rect.h + 0.5;
        let bar_w = if self.show_bar { BAR_WIDTH } else { 0.0 };
        self.inner = Rect::new(rect.x, rect.y, (rect.w - bar_w).max(0.0), rect.h);

        // **滚动条出现后要按更窄的宽度重新量一次内容**。
        //
        // 为什么：`measure` 是按"没有滚动条"的宽度量的，而 `arrange` 让内容少 10px。
        // 依赖宽度的内容（`TextLabel::wrap` 按可用宽度重新折行）会因此**变高**，
        // 但 `max_offset` 用的是旧的 `content_h` —— 最后一段永远滚不到。
        // 代价可接受：约束不变时 `cache.measure` 直接命中，只有宽度真的变了才重测。
        if self.show_bar {
            if let Some(kid) = self.kids.first_mut() {
                let again = cache.measure(
                    kid.as_mut(),
                    metrics,
                    Constraints::unbounded_main(y901_ui::Axis::Vertical, self.inner.size()),
                );
                self.content_h = again.h;
            }
        }

        // **先把偏移算成"本帧最终值"，再摆内容**。
        //
        // 旧实现分两步：先钳制 → 摆内容 → 再处理粘底。粘底那一步会**在摆完内容之后**
        // 改 `offset`，于是同一帧里"子节点位置"和"offset()"各说各话：
        //
        // ```text
        // 流式追加一行（粘底开着）：
        //   摆内容用 offset=300 → 再把 offset 推到 360
        //   子节点按 300 摆好 → 最后 60px 落到视口外 → 下一帧才跳上来
        // ```
        //
        // 表现就是"流式输出时画面一顿一顿地跳"。这是 `arrange` 的同一类错误：
        // 凡是会影响子节点位置的状态，都必须在 `kid.arrange(...)` **之前**定稿。
        self.offset = self.offset.clamp(0.0, self.max_offset());
        if self.stick_to_bottom {
            if self.stick_armed {
                // 内容变高后跟到最底。
                self.offset = self.max_offset();
            } else if self.is_at_bottom() {
                // 用户上滚过 → `stick_armed = false`；回到底部后自动重新激活。
                self.stick_armed = true;
            }
        }

        // 内容坐标系：子节点按未滚动的绝对高度排，屏幕位置靠平移得到
        let content_rect = Rect::new(
            self.inner.x,
            self.inner.y - self.offset,
            self.inner.w,
            self.content_h,
        );
        if let Some(kid) = self.kids.first_mut() {
            kid.arrange(content_rect, metrics, cache);
        }

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

    /// 向下转型：运行时调 `scroll_to_end` / `set_stick_to_bottom` 的唯一路径。
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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
            UiEvent::Wheel { pos, delta, .. } => {
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
                let ratio = args.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
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
        Semantics::default()
            .exposed(true)
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
    // 诊断 API：`target`：ScrollCommand 的目标容器名，仅记录（拆成独立组件前保留）
    // 归类：内部字段 —— `target` 参与滚动目标计算，无直接读取点
    #[allow(dead_code)]
    // 预留 API：当前未接线，留作未来拆分/扩展使用 // 留作把滚动条拆成独立组件时用
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
    /// 外部注入的滚动条样式（`None` = 走主题）。
    style: Option<crate::style::WidgetStyle>,
}

impl Scrollbar {
    /// 新建独立滚动条（也可以被 `ScrollView` 内部创建）。
    ///
    /// 输入：`id`、`target`（它控制哪个容器的 offset；仅记录，供拆分为独立
    /// 组件时使用）。
    /// 返回：不可见（`visible = false`，等 `arrange_at` 决定）。
    /// 副作用：无。
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
            style: None,
        }
    }

    pub(crate) fn arrange_at(
        &mut self,
        rect: Rect,
        thumb_ratio: f32,
        start_ratio: f32,
        visible: bool,
    ) {
        self.rect = rect;
        self.visible = visible;
        self.thumb_ratio = thumb_ratio.clamp(0.05, 1.0);
        // **`start_ratio` 的语义是"滚动进度"（0 = 顶、1 = 底），不是"剩余行程比例"**。
        // 旧实现把它夹到 `1 - thumb_ratio`，于是滚到底也只到 0.9 —— 滑块永远差一截
        // 到不了轨道末端（轨道 100、内容 10 屏时差 7.6px），拖拽也拖不到真正的底部。
        self.start_ratio = start_ratio.clamp(0.0, 1.0);
    }

    pub(crate) fn thumb(&self) -> Rect {
        // **下限与位置夹紧必须用同一个高度**。
        // 旧实现：滑块高取 `max(比例高, 24)`，但位置用 `start_ratio`（按**比例**夹到
        // `1 - thumb_ratio`）算 —— 轨道 100、内容 10 屏时 `start_ratio` 最大 0.9，
        // 滑块实际高 24 → 底边 = 0.9 × (100-24) + 24 = 92.4，**差 7.6px 到不了底**；
        // 轨道比 24 还矮时滑块会直接画出轨道外。
        let h = (self.rect.h * self.thumb_ratio)
            .max(24.0)
            .min(self.rect.h.max(0.0));
        let travel = (self.rect.h - h).max(0.0);
        // 把"比例位置"换算成"该比例下恰好贴底的像素位置"，再用像素夹紧保证不越界。
        let ratio = self.start_ratio.clamp(0.0, 1.0);
        let y = self.rect.y + (travel * ratio).clamp(0.0, travel);
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
    /// 注入滚动条样式（轨道色/滑块色/宽度/圆角）。
    pub fn set_style(&mut self, style: Option<crate::style::WidgetStyle>) -> bool {
        if self.style == style {
            return false;
        }
        self.style = style;
        true
    }

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
        let thumb = ctx.theme().style(Role("scrollbar"), state).bg;
        match self.style {
            Some(s) => {
                // 宽度：有注入就紧贴右侧画指定宽度（不改布局）
                let w = if s.bar_width > 0.0 {
                    s.bar_width.min(self.rect.w)
                } else {
                    self.rect.w
                };
                let x = self.rect.right() - w;
                let track_rect = Rect::new(x, self.rect.y, w, self.rect.h);
                let color = s.bar_track.unwrap_or(track);
                ctx.fill_round_rect(track_rect, color, s.bar_radius);
                let thumb_full = self.thumb();
                let thumb_rect = Rect::new(x, thumb_full.y, w, thumb_full.h);
                let color = s.bar_thumb.unwrap_or(thumb);
                ctx.fill_round_rect(thumb_rect, color, s.bar_radius);
            }
            None => {
                ctx.fill_rect(self.rect, track);
                ctx.fill_rect(self.thumb(), thumb);
            }
        }
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
// 测试 API：`_pos`：类型见证，保证 `Pos` 仍被引用
// 归类：编译见证 —— `_pos` 保持 Pos 进入本模块的引用
#[allow(dead_code)] // 预留 API：当前未接线，留作未来拆分/扩展使用
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

    /// 独立使用时记录的滚动偏移（由 `tracker()` 的初始值或拖拽更新）。
    ///
    /// 返回：像素偏移。副作用：无。
    /// 注意：作为 `ScrollView` 的子组件时，真实偏移在父容器里，
    /// 这里的值可能未被同步（两者通过 `scroll_to` 命令协作）。
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
            // 进度就是 offset / max，夹到 [0,1]；不是 `1 - thumb_ratio`
            //（那会让"滚到底"永远差一截，见 `arrange_at` 的说明）。
            (self.offset / max).clamp(0.0, 1.0)
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

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(BAR_WIDTH, c.max.h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
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
        Semantics::default()
            .exposed(true)
            .label("scrollbar")
            .state("offset", self.offset)
            .state("total", self.total)
            .state("viewport", self.viewport)
    }
}

#[cfg(test)]
mod thumb_geometry_tests {
    use super::*;

    fn bar(track_h: f32, thumb_ratio: f32, start_ratio: f32) -> Scrollbar {
        let mut b = Scrollbar::new("x.bar", "x");
        b.arrange_at(
            Rect::new(0.0, 0.0, 10.0, track_h),
            thumb_ratio,
            start_ratio,
            true,
        );
        b
    }

    /// 滚到底时滑块**必须贴住轨道底边**（旧实现差 7.6px）。
    #[test]
    fn thumb_reaches_the_track_bottom_when_fully_scrolled() {
        // 轨道 100、内容 10 屏 → 比例高 0.1，但画出来的最小高度是 24
        let b = bar(100.0, 0.1, 1.0);
        let t = b.thumb();
        assert!(
            (t.bottom() - 100.0).abs() < 0.01,
            "滚到底时滑块底边应贴住轨道底边（100），实际 {}",
            t.bottom()
        );
        assert!(
            (t.y - (100.0 - 24.0)).abs() < 0.01,
            "滑块顶部应为 76，实际 {}",
            t.y
        );
    }

    /// 顶部时贴顶。
    #[test]
    fn thumb_reaches_the_track_top_at_zero_scroll() {
        let b = bar(100.0, 0.1, 0.0);
        assert!((b.thumb().y - 0.0).abs() < 0.01);
    }

    /// 轨道比 24px 下限还矮时，滑块不能画到轨道外面。
    #[test]
    fn thumb_never_exceeds_a_short_track() {
        for track in [10.0f32, 20.0, 24.0, 30.0] {
            let b = bar(track, 1.0, 1.0);
            let t = b.thumb();
            assert!(
                t.y >= -0.01 && t.bottom() <= track + 0.01,
                "轨道高 {track}：滑块 {t:?} 超出了轨道"
            );
        }
    }

    /// 中间比例：位置与比例成正比，且始终落在 [0, travel] 内。
    #[test]
    fn thumb_position_is_monotonic_and_bounded() {
        let mut last = -1.0;
        for step in 0..=10 {
            let r = step as f32 / 10.0;
            let t = bar(100.0, 0.3, r).thumb();
            assert!(t.y >= -0.01 && t.bottom() <= 100.01, "r={r}: {t:?} 越界");
            assert!(t.y >= last, "r={r}: 位置应单调不减");
            last = t.y;
        }
    }
}
