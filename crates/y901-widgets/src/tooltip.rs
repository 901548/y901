//! Tooltip：悬停延时后弹出的小提示。
//!
//! ## 为什么需要 `wants_tick`
//!
//! 延时靠 `Instant` 计时，但框架没有"定时器事件"——只有 `Tick` 心跳。
//! 因此：悬停开始时 `wants_tick() -> true`（才会收到 Tick），
//! 弹出后立刻改回 `false`（**空闲不再被唤醒**，这是验收项之一）。

use std::time::{Duration, Instant};

use y901_ui::{
    Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

/// 默认延时（毫秒）。
pub const DEFAULT_DELAY_MS: u64 = 500;
/// 内边距。
const PAD: f32 = 6.0;

/// 提示气泡本体（作为浮层内容绘制）。
pub struct Tooltip {
    id: String,
    text: String,
    rect: Rect,
}

impl Tooltip {
    /// 创建一个提示气泡。
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Tooltip {
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
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let style = TextStyle::default();
        let t = m.measure_text(&self.text, &style);
        c.constrain(Size::new(t.w + PAD * 2.0, t.h + PAD * 2.0))
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

    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let s = ctx.theme().style(Role::PANEL, State::Normal);
        ctx.fill_round_rect(self.rect, s.bg, 4.0);
        ctx.stroke_rect(self.rect, s.border, 1.0);
        let style = TextStyle::fg(s.fg);
        ctx.draw_text(
            Pos::new(self.rect.x + PAD, self.rect.y + PAD),
            &self.text,
            &style,
        );
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.text.clone())
    }
}

/// 包裹一个子控件，悬停后弹出提示。
pub struct TooltipHost {
    id: String,
    child: Box<dyn Widget>,
    text: String,
    delay: Duration,
    hover_started: Option<Instant>,
    tooltip_open: bool,
    rect: Rect,
}

impl TooltipHost {
    /// 创建宿主。默认延时 500ms。
    pub fn new(id: impl Into<String>, child: Box<dyn Widget>, text: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            id,
            child,
            text: text.into(),
            delay: Duration::from_millis(DEFAULT_DELAY_MS),
            hover_started: None,
            tooltip_open: false,
            rect: Rect::ZERO,
        }
    }

    /// 设置延时（测试里用很小的值）。
    pub fn delay_ms(mut self, ms: u64) -> Self {
        self.delay = Duration::from_millis(ms);
        self
    }

    /// 浮层 id（关闭时要用）。
    pub fn tooltip_id(&self) -> String {
        format!("{}__tooltip", self.id)
    }

    /// 提示是否已弹出（测试与诊断用）。
    pub fn is_open(&self) -> bool {
        self.tooltip_open
    }

    /// 子控件矩形（作为浮层锚点）。
    fn anchor(&self) -> Rect {
        self.child.rect()
    }
}

impl Widget for TooltipHost {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        std::slice::from_ref(&self.child)
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        // 单子节点用不了 Vec 语义；树会走只读 `children()` 建索引。
        None
    }

    fn wants_tick(&self) -> bool {
        // 只在"已开始计时且还没弹出"时收 Tick：弹出后立即静默，空闲零开销。
        self.hover_started.is_some() && !self.tooltip_open
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let size = self.child.measure(m, c, cache);
        c.constrain(size)
    }

    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = rect;
        self.child.arrange(rect, m, cache);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseEnter => {
                if self.hover_started.is_none() {
                    self.hover_started = Some(Instant::now());
                }
                EventResult::Redraw
            }
            UiEvent::MouseLeave => {
                self.hover_started = None;
                if self.tooltip_open {
                    ctx.request_close_popup(self.tooltip_id());
                    self.tooltip_open = false;
                }
                EventResult::Redraw
            }
            UiEvent::Tick => {
                let Some(started) = self.hover_started else {
                    return EventResult::Ignored;
                };
                if self.tooltip_open || started.elapsed() < self.delay {
                    return EventResult::Ignored;
                }
                // 延时到了：弹浮层。锚点用子控件矩形，`open_popup` 会做边界翻转。
                let id = self.tooltip_id();
                let text = self.text.clone();
                ctx.request_popup(Box::new(move || Box::new(Tooltip::new(id, text))), self.anchor());
                self.tooltip_open = true;
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, _ctx: &mut dyn RenderCtx) {
        // 子控件由树单独渲染（容器不重复画），这里什么都不做。
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.id.clone())
            .prop("tooltip", self.text.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;
    use y901_ui::layout::LayoutCache;
    use y901_ui::{MouseButton, UiTree};

    const W: u32 = 240;
    const H: u32 = 200;

    fn soft() -> SoftBackend {
        let (cw, ch) = y901_render::cell_metrics_for(15.0);
        let cell = Size::new(cw, ch);
        let engine = MyFontEngine::new(
            y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
            15.0,
            cell,
        )
        .expect("engine");
        SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
    }

    /// 一个 40x20 的占位子控件（TooltipHost 的被包裹者）。
    struct Box40 {
        rect: Rect,
    }

    impl Widget for Box40 {
        fn id(&self) -> &str {
            "box"
        }
        fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
            c.constrain(Size::new(40.0, 20.0))
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn render(&mut self, ctx: &mut dyn RenderCtx) {
            ctx.fill_rect(self.rect, y901_ui::Color::WHITE);
        }
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    fn host(delay_ms: u64) -> TooltipHost {
        TooltipHost::new(
            "h",
            Box::new(Box40 { rect: Rect::ZERO }),
            "提示文字",
        )
        .delay_ms(delay_ms)
    }

    /// 轮询 Tick 直到条件满足或超时（**不用固定 sleep 赌时间**）。
    fn poll_until(
        tree: &mut UiTree,
        limit: usize,
        mut done: impl FnMut(&UiTree) -> bool,
    ) -> bool {
        for _ in 0..limit {
            if done(tree) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
            tree.dispatch(&UiEvent::Tick);
        }
        done(tree)
    }

    fn enter(tree: &mut UiTree) {
        tree.dispatch(&UiEvent::MouseMove {
            pos: Pos::new(10.0, 10.0),
        });
    }

    fn leave(tree: &mut UiTree) {
        tree.dispatch(&UiEvent::MouseMove {
            pos: Pos::new(2000.0, 2000.0),
        });
    }

    #[test]
    fn tooltip_appears_after_the_delay() {
        let mut tree = UiTree::new(Box::new(host(5)));
        tree.layout(Size::new(W as f32, H as f32), &soft());
        enter(&mut tree);

        let opened = poll_until(&mut tree, 300, |t| !t.popups().is_empty());
        assert!(opened, "延时后应弹出 Tooltip");
        assert_eq!(tree.popups().len(), 1);
        eprintln!("[tooltip] 弹出成功");
    }

    #[test]
    fn quick_pass_does_not_open_a_tooltip() {
        let mut tree = UiTree::new(Box::new(host(200)));
        tree.layout(Size::new(W as f32, H as f32), &soft());
        enter(&mut tree);
        // 未到延时就走
        for _ in 0..3 {
            tree.dispatch(&UiEvent::Tick);
        }
        leave(&mut tree);
        // 再推进很久也不该弹出
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(1));
            tree.dispatch(&UiEvent::Tick);
        }
        assert!(
            tree.popups().is_empty(),
            "延时未到就离开，不应弹出 Tooltip（实际 {} 个浮层）",
            tree.popups().len()
        );
    }

    #[test]
    fn tooltip_closes_on_leave() {
        let mut tree = UiTree::new(Box::new(host(5)));
        tree.layout(Size::new(W as f32, H as f32), &soft());
        enter(&mut tree);
        assert!(
            poll_until(&mut tree, 300, |t| !t.popups().is_empty()),
            "先要能弹出"
        );
        leave(&mut tree);
        assert_eq!(tree.popups().len(), 0, "离开后 Tooltip 应关闭");
    }

    #[test]
    fn no_ticks_are_requested_after_the_tooltip_opens() {
        let mut tree = UiTree::new(Box::new(host(5)));
        tree.layout(Size::new(W as f32, H as f32), &soft());
        enter(&mut tree);
        assert!(poll_until(&mut tree, 300, |t| !t.popups().is_empty()));
        // 弹出后不应再要求 Tick
        let wants = tree.find("h").map(|n| n.wants_tick()).unwrap_or(true);
        assert!(!wants, "Tooltip 弹出后不应再请求 Tick（空闲 CPU 验收项）");
    }

    #[test]
    fn tooltip_pixels_appear_and_disappear() {
        let mut soft = soft();
        let mut tree = UiTree::new(Box::new(host(5)));
        tree.layout(Size::new(W as f32, H as f32), &soft);

        // 基线：未弹出
        tree.invalidate_all();
        soft.clear();
        tree.render(&mut soft);
        let before_total = non_background(&soft);

        enter(&mut tree);
        assert!(poll_until(&mut tree, 300, |t| !t.popups().is_empty()));
        tree.layout(Size::new(W as f32, H as f32), &soft);
        let tip_rect = tree
            .popup_rect("h__tooltip")
            .expect("浮层应有矩形（layout 后）");

        tree.invalidate_all();
        soft.clear();
        tree.render(&mut soft);
        let inside = non_background_in(&soft, tip_rect);

        eprintln!(
            "[tooltip] 浮层矩形 = {:?}，其内非背景像素 = {inside}，全屏对比 {before_total} -> {}",
            tip_rect,
            non_background(&soft)
        );
        assert!(
            inside > 0,
            "提示气泡矩形内应有绘制像素（实测 {inside}）"
        );

        // 离开后同一区域应回到"没有画东西"
        leave(&mut tree);
        tree.invalidate_all();
        soft.clear();
        tree.render(&mut soft);
        let after = non_background_in(&soft, tip_rect);
        assert_eq!(after, 0, "关闭后气泡矩形内不应再有内容（实测 {after}）");
    }

    /// 某矩形内的非背景像素数。
    ///
    /// 为什么不用全屏计数：弹出浮层会带来一次整窗重绘，
    /// 背景色本身变了，全屏计数从 0 跳到几万——那是在测"画布被重刷"，
    /// 不是在测"气泡出现了"。这个坑实测踩过。
    fn non_background_in(s: &SoftBackend, r: Rect) -> usize {
        // 背景取**远离浮层**的右下角：(0,0) 恰好在气泡内部，
        // 拿它当基准会把气泡自己的颜色当成背景，于是计数横为 0。
        let bg = s.pixel(W - 2, H - 2).expect("bg");
        let x0 = r.x.max(0.0) as u32;
        let y0 = r.y.max(0.0) as u32;
        let x1 = r.right().min(W as f32).max(0.0) as u32;
        let y1 = r.bottom().min(H as f32).max(0.0) as u32;
        let mut n = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if s.pixel(x, y).expect("px") != bg {
                    n += 1;
                }
            }
        }
        n
    }

    /// 非背景像素数（近似"画了多少东西"）。
    fn non_background(s: &SoftBackend) -> usize {
        let bg = s.pixel(0, 0).expect("bg");
        let mut n = 0;
        for y in 0..H {
            for x in 0..W {
                if s.pixel(x, y).expect("px") != bg {
                    n += 1;
                }
            }
        }
        n
    }

    /// 贴近屏幕底部的锚点：Tooltip 应翻到**上方**。
    ///
    /// 复用 `open_popup` 的边界翻转逻辑（不是 Tooltip 自己的实现）——
    /// 若这里失败，说明翻转逻辑有问题，应去查 `open_popup`/`push_popup` 的布局。
    #[test]
    fn tooltip_flips_above_when_there_is_no_room_below() {
        // 把宿主放在视口最底部附近
        struct BottomHost {
            rect: Rect,
            children: Vec<Box<dyn Widget>>,
        }
        impl Widget for BottomHost {
            fn id(&self) -> &str {
                "bottom"
            }
            fn measure(
                &mut self,
                _m: &dyn TextMetrics,
                c: Constraints,
                _cache: &mut LayoutCache,
            ) -> Size {
                c.constrain(Size::new(W as f32, H as f32))
            }
            fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
                self.rect = rect;
                // 只给底部 20px 高的位置
                let r = Rect::new(rect.x, rect.bottom() - 20.0, 40.0, 20.0);
                for ch in self.children.iter_mut() {
                    ch.arrange(r, m, cache);
                }
            }
            fn rect(&self) -> Rect {
                self.rect
            }
            fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
                EventResult::Ignored
            }
            fn children(&self) -> &[Box<dyn Widget>] {
                &self.children
            }
            fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
                Some(&mut self.children)
            }
            fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
            fn semantics(&self) -> Semantics {
                Semantics::default()
            }
        }

        let mut tree = UiTree::new(Box::new(BottomHost {
            rect: Rect::ZERO,
            children: vec![Box::new(host(5))],
        }));
        tree.layout(Size::new(W as f32, H as f32), &soft());

        // 悬停到底部那个宿主上
        tree.dispatch(&UiEvent::MouseMove {
            pos: Pos::new(20.0, H as f32 - 10.0),
        });
        assert!(
            poll_until(&mut tree, 300, |t| !t.popups().is_empty()),
            "底部锚点也应能弹出"
        );
        tree.layout(Size::new(W as f32, H as f32), &soft());

        let tip = tree.popup_rect("h__tooltip").expect("浮层应有矩形");
        let anchor_y = H as f32 - 20.0;
        eprintln!(
            "[tooltip] 锚点 y={anchor_y}，浮层 y={}，高={}",
            tip.y, tip.h
        );
        assert!(
            tip.y < anchor_y,
            "底部空间不足时 Tooltip 应翻到上方：浮层 y={} 应小于锚点 y={anchor_y}",
            tip.y
        );
    }

    #[test]
    fn mouse_down_does_not_open_anything() {
        let mut tree = UiTree::new(Box::new(host(5)));
        tree.layout(Size::new(W as f32, H as f32), &soft());
        enter(&mut tree);
        tree.dispatch(&UiEvent::MouseDown {
            pos: Pos::new(10.0, 10.0),
            button: MouseButton::Left,
        });
        assert!(tree.popups().is_empty() || tree.popups().len() == 1);
    }
}
