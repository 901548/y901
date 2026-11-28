//! **`frame_step` 状态机的不变量**（属性测试 + 两条定向回归）。
//!
//! 事件循环是"单看每一行都对、组合起来才错"的典型：`FrameStats` 的四个布尔量
//! （`had_work` / `need_layout` / `presented` / `close_requested`）分别驱动不同的行为，
//! 组合错了就是"关不掉窗口"、"空闲烧 CPU"或"后台数据不刷新"。
//!
//! 这里随机混 6 种 `RawEvent`（关窗/改尺寸/鼠标移动/按下/字符/光标形状）× 200 组 × 15 帧，
//! 每帧检查：
//!
//! 1. `close_requested` **恰好**由 `Close` 事件触发（多一个 → 莫名退出；少一个 → 关不掉）；
//! 2. 有 `Resize` 就必须 `need_layout` 且真的调了 `relayout`；
//! 3. `presented` 为真 ⇒ 后端确实渲染过；
//! 4. 空闲帧（无事件、无脏区、无请求）不得上屏。
//!
//! 另加两条定向回归：**外部标脏同帧重排+上屏**（审计 #7 的运行时侧）、**空闲帧一次都不画**。

use std::cell::Cell;
use std::rc::Rc;

use y901_runtime::runtime::frame_step;
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{Host, RawEvent};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree,
    Widget,
};

struct Mono;
impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

struct NullCtx;
impl TextMetrics for NullCtx {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Mono.measure_text(t, s)
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        Mono.line_height(s)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}
impl RenderCtx for NullCtx {
    fn size(&self) -> Size {
        Size::new(320.0, 200.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

struct Leaf {
    id: &'static str,
    rect: Rect,
}
impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        cons: Constraints,
        _cache: &mut LayoutCache,
    ) -> Size {
        cons.constrain(Size::new(100.0, 40.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::from_rgb8(10, 20, 30));
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

struct Counting {
    renders: Rc<Cell<u32>>,
    relayouts: Rc<Cell<u32>>,
    viewport: (u32, u32),
}
impl WindowBackend for Counting {
    fn wait_events(&mut self, _w: u32) -> Option<Vec<RawEvent>> {
        Some(Vec::new())
    }
    fn viewport(&self) -> (u32, u32) {
        self.viewport
    }
    fn relayout(&mut self, tree: &mut UiTree) {
        self.relayouts.set(self.relayouts.get() + 1);
        tree.layout(
            Size::new(self.viewport.0 as f32, self.viewport.1 as f32),
            &Mono,
        );
    }
    fn set_theme(&mut self, _theme: y901_ui::theme::Theme) -> Result<(), String> {
        Ok(())
    }
    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String> {
        if tree.dirty().is_empty() {
            return Ok(RenderOutcome::Skipped);
        }
        self.renders.set(self.renders.get() + 1);
        tree.invalidate_all();
        tree.render(&mut NullCtx);
        Ok(RenderOutcome::Presented)
    }
}

struct QuietHost;
impl Host for QuietHost {
    fn idle(&mut self, _tree: &mut UiTree) {}
    fn command(&mut self, _tree: &mut UiTree, _cmd: y901_ui::Command) -> bool {
        false
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn make() -> (UiTree, Counting, QuietHost) {
    let mut tree = UiTree::new(Box::new(Leaf {
        id: "root",
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(320.0, 200.0), &Mono);
    tree.invalidate_all();
    tree.render(&mut NullCtx);
    let win = Counting {
        renders: Rc::new(Cell::new(0)),
        relayouts: Rc::new(Cell::new(0)),
        viewport: (320, 200),
    };
    (tree, win, QuietHost)
}

#[test]
fn frame_stats_reflect_the_event_batch() {
    let mut rng = Lcg(0x0f_4a11);
    for case in 0..200 {
        let (mut tree, mut win, mut host) = make();
        for step in 0..15 {
            let n = (rng.next() % 4) as usize;
            let mut events: Vec<RawEvent> = Vec::new();
            let mut has_close = false;
            let mut has_resize = false;
            for _ in 0..n {
                match rng.next() % 6 {
                    0 => {
                        events.push(RawEvent::Close);
                        has_close = true;
                    }
                    1 => {
                        let w = 100 + (rng.next() % 200) as u32;
                        let h = 60 + (rng.next() % 200) as u32;
                        events.push(RawEvent::Resize { w, h });
                        win.viewport = (w, h);
                        has_resize = true;
                    }
                    2 => events.push(RawEvent::MouseMove { x: 10.0, y: 10.0 }),
                    3 => events.push(RawEvent::MouseDown {
                        x: 5.0,
                        y: 5.0,
                        button: y901_ui::MouseButton::Left,
                    }),
                    4 => events.push(RawEvent::Char { c: 'a' }),
                    _ => events.push(RawEvent::SetCursor { x: 3.0, y: 4.0 }),
                }
            }
            let stats = frame_step(
                &mut win,
                &mut tree,
                &mut host,
                &events,
                std::time::Duration::from_millis(16),
            )
            .expect("frame_step 不应失败");

            // 1) close_requested 必须**恰好**由 Close 事件触发
            assert_eq!(
                stats.close_requested,
                has_close,
                "[case{case} step{step}] close_requested 与事件不符（events={}个）",
                events.len()
            );
            // 2) 有 Resize 就必须重排
            if has_resize {
                assert!(
                    stats.need_layout,
                    "[case{case} step{step}] 有 Resize 事件却没请求重排"
                );
                assert!(
                    win.relayouts.get() > 0,
                    "[case{case} step{step}] 有 Resize 却没真的 relayout"
                );
            }
            // 3) 没有事件、没有脏区 → 不得重绘（空闲帧不许烧 CPU）
            if events.is_empty() && tree.dirty().is_empty() && !stats.had_work {
                assert!(!stats.presented, "[case{case} step{step}] 空闲帧不应上屏");
            }
            // 4) presented 为真 ⇒ 本帧确实画过
            if stats.presented {
                assert!(
                    win.renders.get() > 0,
                    "[case{case} step{step}] presented 为真但后端没渲染"
                );
            }
        }
    }
}
/// **外部标脏（后台线程）必须在同一帧内变成重排 + 上屏**。
///
/// 这条是 2026-11-27 审计 #7 修复的运行时侧回归：以前 `InvalidationHandle::invalidate(id)`
/// 只标一块脏区、不请求重排，尺寸变化要等下一次别的事件；现在 `One` 会置 `layout_requested`，
/// `frame_step` 读到 `external > 0` 也走重排。这里从 runtime 层确认这三点连在一起。
#[test]
fn external_invalidation_causes_relayout_and_present_in_the_same_frame() {
    let (mut tree, mut win, mut host) = make();
    // 先跑一帧把脏区清干净
    let _ = frame_step(
        &mut win,
        &mut tree,
        &mut host,
        &[],
        std::time::Duration::from_millis(16),
    )
    .unwrap();
    tree.take_layout_request();
    let renders_before = win.renders.get();
    let relayouts_before = win.relayouts.get();

    tree.invalidation_handle().invalidate("root");
    let stats = frame_step(
        &mut win,
        &mut tree,
        &mut host,
        &[],
        std::time::Duration::from_millis(16),
    )
    .expect("frame_step");

    assert!(
        win.relayouts.get() > relayouts_before,
        "外部标脏后必须在本帧重排（relayout 次数没变）"
    );
    assert!(
        stats.presented,
        "外部标脏后必须在本帧上屏（presented=false 说明只标脏没画）"
    );
    assert!(win.renders.get() > renders_before, "后端没有被要求渲染");
}

/// **真正的空闲帧不许画**：无事件、无脏区、无请求 → `presented == false` 且后端一次都不画。
#[test]
fn idle_frames_never_paint() {
    let (mut tree, mut win, mut host) = make();
    let _ = frame_step(
        &mut win,
        &mut tree,
        &mut host,
        &[],
        std::time::Duration::from_millis(16),
    )
    .unwrap();
    let renders = win.renders.get();
    for i in 0..5 {
        let stats = frame_step(
            &mut win,
            &mut tree,
            &mut host,
            &[],
            std::time::Duration::from_millis(16),
        )
        .unwrap();
        assert!(
            !stats.presented,
            "第 {i} 个空闲帧不该上屏（空闲时烧 CPU 是最常见的能效 bug）"
        );
        assert!(!stats.need_layout, "第 {i} 个空闲帧不该重排");
        assert!(!stats.close_requested);
    }
    assert_eq!(win.renders.get(), renders, "空闲帧期间后端不应被要求渲染");
}
