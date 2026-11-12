//! Tabs：标签栏 + 内容区，切换带 200ms 过渡。
//!
//! ## 设计取舍：**叶子控件 + 绘制回调**
//!
//! 本框架的容器（Flex/ScrollView）需要把子控件暴露给树做布局与命中测试。Tabs 若走容器路线，
//! 就得处理"非激活页不参与布局"——那是**树层语义**，不是控件能单方面决定的。
//!
//! 所以第一版做成叶子控件：调用方给一个 `painter` 回调，Tabs 在内容区矩形里调它。
//! 好处：
//! - 非激活页**天然不会被画**（回调只对激活页调用）；
//! - 布局完全在 Tabs 内部，无子节点、无缓存一致性问题；
//! - 过渡动画自己持有 `Animate`，用 `Instant` 算 dt（叶子控件不必等树发 dt）。
//!
//! 代价：内容不能是"任意 Widget 组合"，得由调用方在回调里画。
//! 这是**已知限制**，写在这里而不是藏着。

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use y901_ui::{
    animation::{Animate, Easing},
    Constraints, EventCtx, EventResult, FlexItem, Key, Modifiers, Pos, Rect, RenderCtx, Semantics,
    Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 标签栏高度（逻辑像素 × DPI 缩放）。
///
/// **为何是函数**：125% DPI 下正文字从 16→20px，
/// 32px 的标签栏只剩 12px 余量，而标签文字是垂直居中的 —— 字会顶边。
fn bar_h(scale: f32) -> f32 {
    32.0 * scale
}
/// 切换过渡时长。
const SWITCH_MS: u64 = 200;
/// 过渡时内容区的水平位移（像素）。
const SLIDE_PX: f32 = 4.0;

/// 内容区绘制回调：`(index, ctx, content_rect, progress)`。
///
/// `progress` 在过渡中从 0 走到 1，静止时为 1.0——调用方可据此做额外的淡入/位移。
pub type ContentPainter = Box<dyn FnMut(usize, &mut dyn RenderCtx, Rect, f32)>;

/// 标签页控件。
pub struct Tabs {
    id: String,
    /// 标签标题
    items: Vec<String>,
    /// 当前选中索引（共享可读：测试与宿主需要不 downcast 就能观察选中项）
    selected: Rc<Cell<usize>>,
    /// 内容绘制回调
    painter: ContentPainter,
    /// 布局矩形
    rect: Rect,
    /// 切换过渡（`None` = 静止）
    anim: Option<Animate>,
    /// 上一次 Tick 的时间戳（用来算 dt）
    last_tick: Option<Instant>,
    /// 是否持有键盘焦点
    focused: bool,
    /// 外部布局偏好（默认 `FlexItem::default()`，**不自动 fill**）。
    flex: FlexItem,
    /// 内容区高度；`None` = 使用最小高度 `BAR_H + 1.0`。
    content_height: Option<f32>,
}

impl Tabs {
    /// 创建一个 Tabs。
    ///
    /// 输入：`id`、`items`（标题列表，至少 1 项）、`painter`（内容绘制）。
    /// 行为：初始选中第 0 页。`items` 为空时补一个占位标签，避免除零与空矩形。
    pub fn new(id: impl Into<String>, items: Vec<String>, painter: ContentPainter) -> Self {
        let items = if items.is_empty() {
            vec!["untitled".to_string()]
        } else {
            items
        };
        Self {
            id: id.into(),
            items,
            selected: Rc::new(Cell::new(0)),
            painter,
            rect: Rect::ZERO,
            anim: None,
            last_tick: None,
            focused: false,
            flex: FlexItem::default(),
            content_height: None,
        }
    }

    /// 当前选中索引。
    pub fn selected(&self) -> usize {
        self.selected.get()
    }

    /// 选中索引的共享句柄（只读观察用；写请走 `select` / 交互）。
    pub fn selection(&self) -> Rc<Cell<usize>> {
        self.selected.clone()
    }

    /// 标签数量。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否没有标签（构造时已兜底，恒为 false；保留以配合 clippy 的 `len` 约定）。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 程序化选中某一页（越界则忽略）。
    ///
    /// 返回：`true` = 选中项发生变化。
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.items.len() || index == self.selected.get() {
            return false;
        }
        self.selected.set(index);
        self.start_transition();
        true
    }

    /// 标签栏矩形。
    ///
    /// `scale` 由调用方传入：`measure` / `arrange` 传 `m.scale()`，
    /// `event` / `render` 传 `ctx.scale()`。**不存缓存** ——
    /// `Tabs` 没有 `&self` 公开几何 API（`bar_rect` / `content_rect` 都是私有），
    /// 所以不存在"拿不到 ctx 的调用方"。
    fn bar_rect(&self, scale: f32) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y,
            self.rect.w,
            bar_h(scale).min(self.rect.h),
        )
    }

    /// 内容区矩形（`scale` 与 `bar_rect` 必须同源，否则栏与内容会错位）。
    fn content_rect(&self, scale: f32) -> Rect {
        let bar = self.bar_rect(scale);
        Rect::new(
            self.rect.x,
            bar.bottom(),
            self.rect.w,
            (self.rect.h - bar.h).max(0.0),
        )
    }

    /// 第 `i` 个标签的矩形（等分宽度）。
    ///
    /// 为什么等分而不是自适应：自适应要量文字宽度，会在每次布局时改变标签位置，
    /// 用户肌肉记忆会失效。等分在 2~6 个标签时更稳。
    fn tab_rect(&self, i: usize, scale: f32) -> Rect {
        let bar = self.bar_rect(scale);
        let n = self.items.len().max(1) as f32;
        let w = bar.w / n;
        Rect::new(bar.x + w * i as f32, bar.y, w, bar.h)
    }

    /// 启动切换过渡。
    ///
    /// 若已有过渡在跑，用 `retarget` 从**当前进度**续——快速连点 Tab 时不会跳回 0。
    fn start_transition(&mut self) {
        let dur = Duration::from_millis(SWITCH_MS);
        let next = match self.anim.take() {
            Some(prev) if !prev.is_done() => prev.retarget(1.0, dur, Easing::EaseInOutCubic),
            _ => Animate::new(0.0, 1.0, dur, Easing::EaseInOutCubic),
        };
        self.anim = Some(next);
    }

    /// 推进过渡（按真实时间）。返回 `true` = 需要重绘。
    fn advance(&mut self) -> bool {
        let Some(anim) = self.anim.as_mut() else {
            return false;
        };
        let now = Instant::now();
        let dt = match self.last_tick {
            Some(prev) => now.saturating_duration_since(prev),
            // 首帧：给一个极小的 dt，避免"一帧就跳完"
            None => Duration::from_millis(1),
        };
        self.last_tick = Some(now);
        let running = anim.tick(dt);
        if !running {
            self.anim = None;
            self.last_tick = None;
        }
        true
    }

    /// 当前过渡进度（静止时为 1.0）。
    fn progress(&self) -> f32 {
        self.anim.as_ref().map(|a| a.progress()).unwrap_or(1.0)
    }

    /// 外部布局偏好（宽/高/填满等）。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.flex = item;
        self
    }

    /// 指定**内容区**高度；Tabs 总高 = `BAR_H + height`。
    ///
    /// 不设置时 measure 返回 `BAR_H + 1.0` —— 那是**最小高度**，不是设计意图。
    /// 实际应用应用 `flex_item(FlexItem::fill())` 让父容器分配高度，
    /// 或用本方法显式指定。
    pub fn content_height(mut self, height: f32) -> Self {
        self.content_height = Some(height);
        self
    }
}

impl Widget for Tabs {
    fn id(&self) -> &str {
        &self.id
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 有显式 `content_height` 就用它；否则保留**最小高度** `bar_h + 1.0`（保证内容区≥ 1px）。
        let bar = bar_h(m.scale());
        let h = match self.content_height {
            Some(ch) => bar + ch.max(0.0),
            None => bar + 1.0,
        };
        c.constrain(Size::new(c.max.w, h))
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

    fn flex(&self) -> FlexItem {
        self.flex
    }

    /// 可获得键盘焦点：树的焦点序列只收录 `focusable() == true` 的节点，
    /// 不声明就永远拿不到焦点（实测：不声明时 ArrowLeft/Right 完全不生效）。
    fn focusable(&self) -> bool {
        true
    }

    fn wants_tick(&self) -> bool {
        // 只有过渡在跑时才收 Tick：静止时零开销（空闲 CPU 不受影响）
        self.anim.is_some()
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseDown { pos, .. } => {
                let bar = self.bar_rect(ctx.scale());
                if !bar.contains(*pos) {
                    return EventResult::Ignored;
                }
                ctx.request_focus(self.id.clone());
                for i in 0..self.items.len() {
                    if self.tab_rect(i, ctx.scale()).contains(*pos) {
                        return if self.select(i) {
                            EventResult::Handled
                        } else {
                            EventResult::Ignored
                        };
                    }
                }
                EventResult::Ignored
            }
            UiEvent::Focus(focused) => {
                self.focused = *focused;
                EventResult::Redraw
            }
            UiEvent::Key { key, modifiers } => {
                // 不靠 `self.focused` 判断：树只会把 Key 发给**焦点节点**，
                // 而 `request_focus` 是在事件里发生的，本控件的 `focused` 字段
                // 要等广播的 `Focus(true)` 才会更新 —— 两者时机不同。
                // 只处理"无修饰键"的方向键：Ctrl+← 之类交给上层（可能表示"上一个词/页"）
                if *modifiers != Modifiers::default() {
                    return EventResult::Ignored;
                }
                match key {
                    Key::Left => {
                        let next = (self.selected.get() + self.items.len().max(1) - 1)
                            % self.items.len().max(1);
                        if self.select(next) {
                            EventResult::Handled
                        } else {
                            EventResult::Ignored
                        }
                    }
                    Key::Right => {
                        let next = (self.selected.get() + 1) % self.items.len().max(1);
                        if self.select(next) {
                            EventResult::Handled
                        } else {
                            EventResult::Ignored
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            UiEvent::Tick => {
                if self.advance() {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let bar = self.bar_rect(ctx.scale());
        let selected_style = ctx.theme().style(y901_ui::Role::BUTTON, State::Selected);
        let normal_style = ctx.theme().style(y901_ui::Role::BUTTON, State::Normal);

        // 标签栏底色
        ctx.fill_rect(bar, normal_style.bg);

        let n = self.items.len().max(1);
        let mut labels: Vec<(String, Rect, TextStyle)> = Vec::with_capacity(n);
        for i in 0..n {
            let r = self.tab_rect(i, ctx.scale());
            let active = i == self.selected.get();
            let s = if active { selected_style } else { normal_style };
            if active {
                ctx.fill_rect(r, s.bg);
            }
            let ts = TextStyle::fg(s.fg);
            let tw = ctx.measure_text(&self.items[i], &ts).w;
            let x = r.x + (r.w - tw) * 0.5;
            let y = r.y + (r.h - ctx.line_height(&ts)) * 0.5;
            labels.push((self.items[i].clone(), r, ts));
            let _ = (x, y);
        }

        // 选中标签的下划线（过渡时随进度从 0 长到满宽）
        let p = self.progress();
        let sel_rect = self.tab_rect(self.selected.get().min(n - 1), ctx.scale());
        ctx.fill_rect(
            Rect::new(
                sel_rect.x,
                sel_rect.bottom() - 2.0,
                sel_rect.w * p.clamp(0.0, 1.0),
                2.0,
            ),
            selected_style.fg,
        );

        // 标签文字最后画，保证盖在下划线上方
        for (text, r, ts) in labels {
            let tw = ctx.measure_text(&text, &ts).w;
            let x = r.x + (r.w - tw) * 0.5;
            let y = r.y + (r.h - ctx.line_height(&ts)) * 0.5;
            ctx.draw_text(Pos::new(x, y), &text, &ts);
        }

        // 内容区：只在激活页上调用 painter，并施加过渡（淡入 + 右移）。
        let content = self.content_rect(ctx.scale());
        if content.w > 0.0 && content.h > 0.0 {
            ctx.push_clip(content);
            let eased = 1.0 - p; // p=0 → 完全透明
            let dx = SLIDE_PX * eased;
            ctx.push_transform(Pos::new(content.x - dx, content.y), 1.0);
            (self.painter)(self.selected.get(), ctx, content, p);
            ctx.pop_transform();
            ctx.pop_clip();
            if eased > 0.0 {
                // 淡入遮罩：用背景色盖一层（与 Popup 同一手法，保持一致观感）
                let bg = ctx.background_color();
                ctx.fill_rect(
                    content,
                    y901_ui::Color::rgba(bg.r, bg.g, bg.b, eased.clamp(0.0, 1.0)),
                );
            }
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.items[self.selected.get().min(self.items.len() - 1)].clone())
            .value(self.selected.get().to_string())
            .action("select")
            .prop("count", self.items.len().to_string())
            .prop("selected", self.selected.get().to_string())
    }
}
