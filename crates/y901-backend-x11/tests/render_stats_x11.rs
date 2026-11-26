#![cfg(target_os = "linux")]

//! **X11 分阶段统计（`Y901_RENDER_STATS=1`）**的端到端验收（RenderStats A 阶段第 4 步）。
//!
//! 这个测试文件**自己设置环境变量**：`render_stats::enabled()` 是进程级 `OnceLock`，
//! 同一进程里只能有一种模式，因此"开启"与"关闭"分在两个测试二进制
//!（本文件 = 开启；`render_stats_x11_disabled.rs` = 关闭），避免并行用例互相污染。
//!
//! 没有 X server 的环境（Xvfb 之外的 CI）按仓库既有惯例**留痕跳过**：写
//! `target/skip-count.txt` + stderr，不 panic、不静默 return。
//!
//! 需要钉住的四件事（对应 `docs/RENDER_STATS_DESIGN_2026-11-25.md` §2 的 X11 列）：
//! 1. 可测阶段真的被测量了：`render_prepare` / `cpu_raster` / `pixel_copy` / `upload`；
//! 2. 不可测 / 不存在的阶段**绝不伪造**：`gpu_execute`、`compositor` 必须是
//!    `Unavailable` 且带非空原因；`backend_submit` / `present_blocking` 必须是
//!    `NotApplicable`（X11 的上传与 Present 是同一个动作）；
//! 3. 计数器自洽：脏区数 ≥ 1、受损像素 = 客户区面积、`frame_skipped` 与是否跳过一致；
//! 4. `validate()` 通过、`take_render_stats` 一次性消费、`presented` 恒为 false
//!    （真值由 Runtime 写，后端不得伪造）。

use std::time::{Duration, Instant};

use y901_backend_x11::window::{CreateArgs, X11Window};
use y901_runtime::render_stats::{BackendKind, Stage, StageId};
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree,
    Widget,
};

const W: u16 = 360;
const H: u16 = 200;

/// 同一进程内只允许一个 X 连接串行使用（与 `real_window.rs` 同款）。
static SERVER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 留痕跳过（与 `real_window.rs` 同一套约定）。
fn skip(reason: &str) {
    use std::io::Write as _;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/skip-count.txt");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "0\trender_stats_x11\t{reason}");
    }
    eprintln!("[stats-x11] 跳过：{reason}");
}

fn soft_for(w: u16, h: u16) -> y901_render::soft::SoftBackend {
    let size = 15.0f32;
    let (cw, ch) = y901_render::cell_metrics_for(size);
    let cell = Size::new(cw, ch);
    let engine = y901_render::MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("测试字体可读"),
        size,
        cell,
    )
    .expect("字形引擎");
    y901_render::soft::SoftBackend::new(w as u32, h as u32, Box::new(engine), cell, None)
        .expect("软光栅器")
}

fn create_window(d: &str) -> Option<X11Window> {
    for attempt in 0..3 {
        match X11Window::create(CreateArgs {
            display: d,
            width: W,
            height: H,
            title: "y901-stats-x11",
            soft: soft_for(W, H),
            fullscreen: false,
        }) {
            Ok(x) => return Some(x),
            Err(e) => {
                if attempt == 2 {
                    skip(&format!("建窗失败：{e}"));
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    None
}

/// 一个自己画两行字的叶子（x11 crate 不依赖 `y901-widgets`，所以这里手写）。
struct TwoLines {
    id: &'static str,
    rect: Rect,
}

impl Widget for TwoLines {
    fn id(&self) -> &str {
        self.id
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(W as f32, H as f32))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(
            Rect::new(0.0, 0.0, W as f32, H as f32),
            Color::from_rgb8(24, 24, 32),
        );
        ctx.draw_text(
            Pos::new(8.0, 24.0),
            "X11 RenderStats",
            &TextStyle::default(),
        );
        ctx.draw_text(Pos::new(8.0, 48.0), "A stage step 4", &TextStyle::default());
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 一棵"会真的画出东西"的小树：一块背景 + 两行文字（文字会触发字形光栅化）。
fn build() -> UiTree {
    let mut tree = UiTree::new(Box::new(TwoLines {
        id: "leaf",
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(W as f32, H as f32), &soft_for(W, H));
    tree.invalidate_all();
    tree
}

#[test]
fn x11_reports_measurable_stages_and_never_fakes_the_others() {
    std::env::set_var("Y901_RENDER_STATS", "1");
    let _guard = SERVER_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let d = std::env::var("DISPLAY").unwrap_or_else(|_| ":99".to_string());
    let Some(mut win) = create_window(&d) else {
        return;
    };

    // 前置：种类必须自报为 X11（决定能力表，进而决定 validate 的判据）。
    assert_eq!(
        win.render_stats_backend(),
        Some(BackendKind::X11),
        "X11Window 必须自报 BackendKind::X11"
    );

    let mut tree = build();
    win.relayout(&mut tree);

    // 第 1 帧：完整渲染。
    let t0 = Instant::now();
    let outcome = win.render(&mut tree).expect("render");
    let wall = t0.elapsed();
    assert!(
        matches!(outcome, RenderOutcome::Presented),
        "首帧应真正上屏，实际 {outcome:?}"
    );

    let stats = win.take_render_stats().expect("开启统计后必须交付统计");
    assert_eq!(stats.backend, BackendKind::X11);

    // 1) 四个可测阶段都必须有值（X11 走的是"清屏 → 整树 → 拷帧 → 上传"）。
    for id in [
        StageId::RenderPrepare,
        StageId::CpuRaster,
        StageId::PixelCopy,
        StageId::Upload,
    ] {
        assert!(
            stats.stages.get(id).is_some(),
            "{} 必须被测量（X11 走这条路径）",
            id.name()
        );
    }

    // 2) 不得伪造，且三态各归其位：
    //    - `compositor`：X server 绘制无回执 → **Unavailable**（带原因）；
    //    - `gpu_execute` / `backend_submit` / `present_blocking` / `command_gen`：
    //      CPU 后端没有这些阶段 / 已合并 → **NotApplicable**；
    //    - `pixel_copy`：X11 **有**（软光栅→暂存拷贝），上面已断言被测量。
    //      `compositor` 与 `command_gen` 都是 **Unavailable**（存在这个概念、但无法分离测量）；
    //      `command_gen` 之所以不是 NotApplicable：软渲染确实"生成绘制命令"，
    //      只是与写像素在同一次调用里，拆开就是伪造数据。
    for id in [StageId::Compositor, StageId::CommandGen] {
        match stats.stage(id) {
            Stage::Unavailable(reason) => assert!(
                !reason.trim().is_empty(),
                "{} 的不可用原因不能为空",
                id.name()
            ),
            other => panic!(
                "{} 必须是 Unavailable（不得伪造），实际 {other:?}",
                id.name()
            ),
        }
    }
    for id in [
        StageId::GpuExecute,
        StageId::BackendSubmit,
        StageId::PresentBlocking,
    ] {
        assert_eq!(
            stats.stage(id),
            Stage::NotApplicable,
            "{} 在 X11 上不存在/已合并，应为 NotApplicable",
            id.name()
        );
    }

    // 3) 计数器：整窗重绘 → 受损像素 = 客户区面积；脏区数 ≥ 1；本帧没跳过。
    assert_eq!(
        stats.counters.damaged_pixels,
        W as u64 * H as u64,
        "X11 每帧整窗重绘，受损像素应等于客户区面积"
    );
    assert!(stats.counters.dirty_rect_count >= 1, "脏区数应 ≥ 1");
    assert!(
        !stats.counters.frame_skipped,
        "本帧真的渲染了，不该标记跳过"
    );
    assert_eq!(
        stats.counters.atlas_upload_bytes, 0,
        "X11 没有 GPU 图集上传，必须写 0（不是伪造测量）"
    );

    // 4) 阶段之和不得超过 `window.render` 的墙钟（否则说明区间被重复计数）。
    let sum: Duration = [
        StageId::RenderPrepare,
        StageId::CpuRaster,
        StageId::PixelCopy,
        StageId::Upload,
    ]
    .iter()
    .filter_map(|id| stats.stages.get(*id))
    .sum();
    assert!(
        sum <= wall + Duration::from_millis(2),
        "四段之和 {sum:?} 超过 window.render 墙钟 {wall:?}（阶段重叠被重复计数）"
    );

    // 5) `presented` 归 Runtime，后端不得写。
    assert!(
        !stats.presented,
        "后端统计里的 presented 必须恒为 false（真值由 Runtime 写）"
    );

    // 6) 自检 + 一次性消费。
    assert_eq!(stats.validate(), Ok(()), "统计不得自相矛盾");
    assert!(
        win.take_render_stats().is_none(),
        "take_render_stats 必须是一次性消费"
    );
}

#[test]
fn skipped_frame_reports_no_stages_and_does_not_leak() {
    std::env::set_var("Y901_RENDER_STATS", "1");
    let _guard = SERVER_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let d = std::env::var("DISPLAY").unwrap_or_else(|_| ":99".to_string());
    let Some(mut win) = create_window(&d) else {
        return;
    };

    let mut tree = build();
    win.relayout(&mut tree);
    // 第 1 帧：完整渲染（消耗掉初始脏区）。
    win.render(&mut tree).expect("render 1");
    let _ = win.take_render_stats().expect("首帧统计");

    // 第 2 帧：没有脏区 → 后端跳过渲染。
    let outcome = win.render(&mut tree).expect("render 2");
    assert!(
        matches!(outcome, RenderOutcome::Skipped),
        "无脏区时不应上屏，实际 {outcome:?}"
    );
    let stats = win.take_render_stats().expect("跳过帧也必须交付统计");
    assert!(stats.counters.frame_skipped, "跳过帧必须标记 frame_skipped");
    for id in [
        StageId::RenderPrepare,
        StageId::CpuRaster,
        StageId::PixelCopy,
        StageId::Upload,
    ] {
        assert!(
            stats.stages.get(id).is_none(),
            "跳过帧不得有 {} 的测量值（否则就是把没跑过的阶段写成 0）",
            id.name()
        );
    }
    assert_eq!(stats.validate(), Ok(()), "跳过帧的统计也必须自洽");
    // 第三帧仍然可交付（每帧重置，不跨帧污染）。
    tree.invalidate_all();
    win.render(&mut tree).expect("render 3");
    let stats = win.take_render_stats().expect("第 3 帧统计");
    assert!(stats.stages.get(StageId::CpuRaster).is_some());
    assert!(!stats.counters.frame_skipped);
}
