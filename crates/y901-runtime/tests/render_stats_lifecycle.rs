//! **A 阶段第 2 步的验收用例**：Runtime 层的统计生命周期。
//!
//! 这些用例用最小 mock 后端驱动真实的 `frame_step_with_stats` / 统计合并逻辑，
//! 不依赖任何平台窗口，因此可以在 CI 的 Windows job 里稳定跑。
//!
//! 覆盖指令要求的 9 项：
//! 1. 默认 Backend 返回 `None`；
//! 2. `FrameStats` 未被修改（编译期断言：字段仍是 4 个布尔）；
//! 3. `frame_step` 签名未变（本文件直接以 5 参数调用）；
//! 4. `total_frame` 不包含 `wait_events`（由 `run_loop` 的 `work_start` 位置保证；
//!    这里验证"统计只从进入 frame_step 之后开始计时"）；
//! 5. `present_ms` 不再代表整帧（`FrameMeta.present_us` 为 `None` 或真实值）；
//! 6. 后端没提供数据时不伪造；
//! 7. 连续两帧互不污染；
//! 8. 关闭统计时不读时钟（`mark(false)` / 不调用 `take_render_stats`）；
//! 9. 原有 runtime 测试仍通过（由 `cargo test -p y901-runtime` 覆盖）。

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use y901_runtime::render_stats::{self, BackendKind, RenderStats, StageId};
use y901_runtime::runtime::{frame_step, frame_step_with_stats};
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{Host, RawEvent};
use y901_ui::{Size, UiTree};

/// 极简后端：不画任何东西，只记录"渲染了几次"。
struct Mock {
    /// `take_render_stats` 被调用的次数（用于验证关闭统计时**根本不会**调用）。
    take_calls: Rc<Cell<u32>>,
    /// 下一帧要返回的统计（`None` = 未接入）。
    next_stats: Option<RenderStats>,
    /// 自报的后端种类（`None` = 未接入统计）。
    kind: Option<BackendKind>,
    /// `render` 里假装耗时（不参与断言，只是为了有非零区间）。
    spin_us: u64,
}

impl Mock {
    fn new() -> Self {
        Self {
            take_calls: Rc::new(Cell::new(0)),
            next_stats: None,
            kind: None,
            spin_us: 0,
        }
    }
}

impl WindowBackend for Mock {
    fn wait_events(&mut self, _wait_ms: u32) -> Option<Vec<RawEvent>> {
        Some(Vec::new())
    }
    fn viewport(&self) -> (u32, u32) {
        (320, 200)
    }
    fn relayout(&mut self, tree: &mut UiTree) {
        tree.layout(Size::new(320.0, 200.0), &Mono);
    }
    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String> {
        if tree.dirty().is_empty() {
            return Ok(RenderOutcome::Skipped);
        }
        // 消费脏区（真实后端会画），否则每帧都 dirty 会让断言难以对齐。
        tree.invalidate_all();
        let t = std::time::Instant::now();
        tree.render(&mut NullCtx);
        while (t.elapsed().as_micros() as u64) < self.spin_us {
            std::hint::spin_loop();
        }
        Ok(RenderOutcome::Presented)
    }
    fn set_theme(&mut self, _theme: y901_ui::theme::Theme) -> Result<(), String> {
        Ok(())
    }
    fn render_stats_backend(&self) -> Option<BackendKind> {
        self.kind
    }
    fn take_render_stats(&mut self) -> Option<RenderStats> {
        self.take_calls.set(self.take_calls.get() + 1);
        self.next_stats.take()
    }
}

struct QuietHost;
impl Host for QuietHost {
    fn idle(&mut self, _tree: &mut UiTree) {}
    fn command(&mut self, _tree: &mut UiTree, _cmd: y901_ui::Command) -> bool {
        false
    }
}

struct Mono;
impl y901_ui::TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

struct NullCtx;
impl y901_ui::TextMetrics for NullCtx {
    fn measure_text(&self, t: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}
impl y901_ui::RenderCtx for NullCtx {
    fn size(&self) -> Size {
        Size::new(320.0, 200.0)
    }
    fn fill_rect(&mut self, _r: y901_ui::Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: y901_ui::Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _p: y901_ui::Pos, text: &str, _s: &y901_ui::TextStyle) -> f32 {
        text.chars().count() as f32 * 8.0
    }
    fn push_clip(&mut self, _r: y901_ui::Rect) {}
    fn pop_clip(&mut self) {}
}

/// 最小 widget（与 `tests/run_many_contract.rs` 的 `NoopWidget` 同款）。
struct NoopWidget {
    rect: y901_ui::Rect,
}

impl y901_ui::Widget for NoopWidget {
    fn id(&self) -> &str {
        "root"
    }
    fn measure(
        &mut self,
        _m: &dyn y901_ui::TextMetrics,
        c: y901_ui::Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(
        &mut self,
        rect: y901_ui::Rect,
        _m: &dyn y901_ui::TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }
    fn rect(&self) -> y901_ui::Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut y901_ui::EventCtx, _e: &y901_ui::UiEvent) -> y901_ui::EventResult {
        y901_ui::EventResult::Ignored
    }
    fn children(&self) -> &[Box<dyn y901_ui::Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

fn make_tree() -> UiTree {
    let mut tree = UiTree::new(Box::new(NoopWidget {
        rect: y901_ui::Rect::ZERO,
    }));
    tree.layout(Size::new(320.0, 200.0), &Mono);
    tree
}

// ---------------------------------------------------------------------------
// 1) 默认后端返回 None（未接入统计的后端/测试替身无需改动）
// ---------------------------------------------------------------------------

#[test]
fn default_backend_reports_no_stats() {
    // 用一个**只实现必需方法**的后端：默认方法必须给出 None。
    struct Bare;
    impl WindowBackend for Bare {
        fn wait_events(&mut self, _w: u32) -> Option<Vec<RawEvent>> {
            Some(Vec::new())
        }
        fn viewport(&self) -> (u32, u32) {
            (1, 1)
        }
        fn relayout(&mut self, _tree: &mut UiTree) {}
        fn render(&mut self, _tree: &mut UiTree) -> Result<RenderOutcome, String> {
            Ok(RenderOutcome::Skipped)
        }
        fn set_theme(&mut self, _t: y901_ui::theme::Theme) -> Result<(), String> {
            Ok(())
        }
    }
    let mut bare = Bare;
    assert!(bare.take_render_stats().is_none(), "默认实现必须返回 None");
    assert!(
        bare.render_stats_backend().is_none(),
        "默认实现必须自报 None（不知道后端种类）"
    );
}

// ---------------------------------------------------------------------------
// 3) frame_step 签名未变（编译期即验证：5 参数调用）
// ---------------------------------------------------------------------------

#[test]
fn frame_step_public_signature_is_unchanged() {
    let mut mock = Mock::new();
    let mut tree = make_tree();
    let mut host = QuietHost;
    let stats = frame_step(&mut mock, &mut tree, &mut host, &[], Duration::ZERO)
        .expect("frame_step 不应失败");
    // `FrameStats` 仍是 4 个布尔字段（编译期验证 + 运行期抽查两个）。
    let _ = (
        stats.had_work,
        stats.need_layout,
        stats.presented,
        stats.close_requested,
    );
    assert!(!stats.close_requested);
}

// ---------------------------------------------------------------------------
// 4) 统计只覆盖 frame_step 之内：total_frame 由调用方按 work_start 计
// ---------------------------------------------------------------------------

#[test]
fn stats_cover_only_the_frame_step_span() {
    let mut mock = Mock::new();
    mock.kind = Some(BackendKind::D3d11);
    let mut tree = make_tree();
    let mut host = QuietHost;
    let mut stats = RenderStats::new(1, BackendKind::D3d11);

    let before = std::time::Instant::now();
    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        Some(&mut stats),
    )
    .unwrap();
    let elapsed = before.elapsed();

    // frame_step 里产生的阶段不得长于它自身的墙钟（否则说明把别的东西算进来了）。
    for id in [StageId::EventDispatch, StageId::Layout, StageId::AfterFrame] {
        if let Some(d) = stats.stages.get(id) {
            assert!(
                d <= elapsed + Duration::from_millis(1),
                "{} 的测量值 {:?} 超过了 frame_step 的实际耗时 {:?}",
                id.name(),
                d,
                elapsed
            );
        }
    }
    // `total_frame` 由 run_loop 在 wait_events 之后取起点，因此 frame_step 内**不填**它。
    assert!(
        stats.stages.get(StageId::TotalFrame).is_none(),
        "total_frame 必须由调用方（run_loop）填写，不能由 frame_step 自己估"
    );
}

// ---------------------------------------------------------------------------
// 6) 后端没提供数据时不伪造
// ---------------------------------------------------------------------------

#[test]
fn backend_stages_stay_unmeasured_when_backend_reports_nothing() {
    let mut mock = Mock::new();
    mock.kind = Some(BackendKind::D3d11); // 自报种类，但 take 返回 None
    let mut tree = make_tree();
    let mut host = QuietHost;
    let mut stats = RenderStats::new(1, BackendKind::D3d11);

    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        Some(&mut stats),
    )
    .unwrap();

    for id in [
        StageId::RenderPrepare,
        StageId::CommandGen,
        StageId::BackendSubmit,
        StageId::PresentBlocking,
        StageId::Upload,
    ] {
        assert!(
            stats.stages.get(id).is_none(),
            "后端未提供数据时 {} 必须保持未测量（不得伪造 0）",
            id.name()
        );
    }
    // 而 GPU / 合成器必须是 Unavailable，且带原因。
    for id in [StageId::GpuExecute, StageId::Compositor] {
        match stats.stage(id) {
            y901_runtime::render_stats::Stage::Unavailable(reason) => {
                assert!(
                    !reason.trim().is_empty(),
                    "{} 的不可用原因不能为空",
                    id.name()
                )
            }
            other => panic!("{} 应为 Unavailable，实际 {other:?}", id.name()),
        }
    }
}

// ---------------------------------------------------------------------------
// 7) 连续两帧互不污染
// ---------------------------------------------------------------------------

#[test]
fn consecutive_frames_do_not_leak_stats() {
    let mut mock = Mock::new();
    mock.kind = Some(BackendKind::D3d11);
    let mut tree = make_tree();
    let mut host = QuietHost;

    // 帧 1：后端上报 present_blocking = 3ms
    // 顺序与 `run_loop` 一致：frame_step → take_render_stats → merge_backend。
    let mut s1 = RenderStats::new(1, BackendKind::D3d11);
    let mut be1 = RenderStats::new(1, BackendKind::D3d11);
    be1.stages
        .set(StageId::PresentBlocking, Duration::from_millis(3));
    mock.next_stats = Some(be1);
    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        Some(&mut s1),
    )
    .unwrap();
    let taken1 = mock.take_render_stats();
    assert!(s1.merge_backend(taken1), "帧 1 应合并到后端数据");

    // 帧 2：先 reset（run_loop 每帧都做），后端什么都不上报
    let mut s2 = RenderStats::new(2, BackendKind::D3d11);
    s2.reset_for_next_frame(2);
    mock.next_stats = None;
    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        Some(&mut s2),
    )
    .unwrap();
    let taken2 = mock.take_render_stats();
    assert!(
        !s2.merge_backend(taken2),
        "帧 2 后端未提供数据时应返回 false"
    );

    // 帧 2 必须**看不到**帧 1 的 present_blocking。
    assert!(
        s2.stages.get(StageId::PresentBlocking).is_none(),
        "第二帧不得继承第一帧的 present_blocking（跨帧污染）"
    );
    // 而第一帧确实拿到了（证明上面的断言不是因为"从来没工作过"）。
    assert!(
        s1.stages.get(StageId::PresentBlocking).is_some(),
        "第一帧应拿到后端上报的 present_blocking"
    );
}

#[test]
fn reset_for_next_frame_is_what_prevents_leakage() {
    let mut s = RenderStats::new(1, BackendKind::D3d11);
    s.stages.set(StageId::Upload, Duration::from_micros(7));
    s.counters.damaged_pixels = 12345;
    s.presented = true;
    s.reset_for_next_frame(2);
    assert!(s.stages.get(StageId::Upload).is_none());
    assert_eq!(s.counters.damaged_pixels, 0);
    assert!(!s.presented);
    assert_eq!(s.frame_id, 2);
}

// ---------------------------------------------------------------------------
// 5) present 数据：只有后端上报才有值
// ---------------------------------------------------------------------------

#[test]
fn present_is_unavailable_without_backend_data_and_not_the_frame_total() {
    let mut mock = Mock::new();
    mock.kind = Some(BackendKind::D3d11);
    let mut tree = make_tree();
    let mut host = QuietHost;
    let mut stats = RenderStats::new(1, BackendKind::D3d11);

    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        Some(&mut stats),
    )
    .unwrap();
    // 未接入时 `present_blocking` 必须是 None；`stage()` 给出 Unavailable。
    assert!(stats.stages.get(StageId::PresentBlocking).is_none());
    match stats.stage(StageId::PresentBlocking) {
        y901_runtime::render_stats::Stage::Unavailable(_) => {}
        other => panic!("present_blocking 应为 Unavailable，实际 {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 8) 关闭统计：不读时钟、不创建对象、不消费后端统计
// ---------------------------------------------------------------------------

#[test]
fn disabled_stats_never_touch_the_clock_or_the_backend() {
    // `mark(false)` 不读时钟：StageTimes 保持默认。
    let mut times = y901_runtime::render_stats::StageTimes::default();
    render_stats::record(&mut times, StageId::Layout, render_stats::mark(false));
    assert_eq!(times, y901_runtime::render_stats::StageTimes::default());

    // 关闭时 run_loop 不会创建 `RenderStats`，因此**根本不会调用**
    // `take_render_stats()` —— 用 mock 的调用计数在 `frame_step` 之外的
    // 一层模拟：不传 stats 时后端不应被问。
    let mut mock = Mock::new();
    mock.kind = Some(BackendKind::D3d11);
    mock.next_stats = Some(RenderStats::new(1, BackendKind::D3d11));
    let mut tree = make_tree();
    let mut host = QuietHost;
    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        None, // 关闭统计
    )
    .unwrap();
    assert_eq!(
        mock.take_calls.get(),
        0,
        "统计关闭时不得消费后端统计（也不得让后端做统计相关构造）"
    );
}

#[test]
fn frame_step_never_consumes_backend_stats_itself() {
    // `frame_step_with_stats` 只负责 runtime 自己的阶段；消费后端统计是
    // `run_loop` 的职责。若它自己也取一次，`run_loop` 那次就会拿到 `None`
    //（"取走"语义），表现为后端数据永远丢失 —— 这就是"重复消费"的 bug。
    let mut mock = Mock::new();
    mock.kind = Some(BackendKind::D3d11);
    mock.next_stats = Some(RenderStats::new(1, BackendKind::D3d11));
    let mut tree = make_tree();
    let mut host = QuietHost;
    let mut stats = RenderStats::new(1, BackendKind::D3d11);

    let _ = frame_step_with_stats(
        &mut mock,
        &mut tree,
        &mut host,
        &[],
        Duration::ZERO,
        Some(&mut stats),
    )
    .unwrap();
    assert_eq!(
        mock.take_calls.get(),
        0,
        "frame_step 不得自己消费后端统计（会与 run_loop 重复消费）"
    );

    // 由调用方取，且**只取一次**：第二次必然是 None（取走语义）。
    assert!(mock.take_render_stats().is_some(), "第一次应取到");
    assert!(
        mock.take_render_stats().is_none(),
        "第二次必须为 None（每帧最多消费一次）"
    );
}
