#![cfg(windows)]

//! **Win32 `--force-soft`（纯 CPU 路径）的分阶段统计**验收（RenderStats A 阶段第 6 步）。
//!
//! 这个测试二进制自己设置两个环境变量：
//! - `Y901_FORCE_SOFT=1`：走 GDI 软件路径（不创建 D3D11 设备）；
//! - `Y901_RENDER_STATS=1`：打开统计。
//!
//! 两者都是进程级 `OnceLock` 缓存，因此必须独占一个测试二进制（与
//! `render_stats_enabled.rs` / `render_stats_disabled.rs` 同一套约定）。
//!
//! 钉住四件事（对应 `docs/RENDER_STATS_DESIGN_2026-11-25.md` §2 的 Win32/Soft 列）：
//! 1. 种类自报为 `BackendKind::Soft`（**不是 D3d11**——能力表不同，混了就会误判）；
//! 2. 三个可测阶段真的被测量：`render_prepare` / `cpu_raster` / `present_blocking`；
//! 3. 不存在/不可测的阶段**绝不伪造**：`backend_submit` / `gpu_execute` /
//!    `pixel_convert` / `image_alloc` / `pixel_copy` / `canvas_blit` 必须 `NotApplicable`，
//!    `command_gen` / `compositor` 必须 `Unavailable` 且原因非空；
//! 4. 计数器自洽 + `validate()` 通过 + `take_render_stats` 一次性消费。

use std::time::Duration;

use y901_backend_win32::window::Window;
use y901_runtime::render_stats::{BackendKind, Stage, StageId};
use y901_runtime::window::WindowBackend;
use y901_runtime::Options;
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree,
    Widget,
};

const W: u32 = 360;
const H: u32 = 200;

/// 同一进程内只允许一个窗口（共享 GDI/消息队列，串行更稳）。
static WIN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn window(title: &str) -> Option<Window> {
    let options = Options {
        title: title.to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,
        fullscreen: false,
    };
    match Window::create(&options) {
        Ok(w) => Some(w),
        Err(e) => {
            // 软路径只需 GDI，理论上永远可用；真要失败就留痕跳过而不是假绿。
            use std::io::Write as _;
            let path = std::env::temp_dir().join("y901-test-skips.txt");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "SKIP: render_stats_soft 建窗失败：{e}");
            }
            eprintln!("[stats-soft] 跳过：建窗失败 {e}");
            None
        }
    }
}

/// 一个自己画背景 + 两行字的叶子（软路径会真的光栅化它）。
struct Painted {
    id: &'static str,
    rect: Rect,
}

impl Widget for Painted {
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
            Color::from_rgb8(20, 22, 28),
        );
        ctx.draw_text(Pos::new(8.0, 24.0), "soft stats", &TextStyle::default());
        ctx.draw_text(Pos::new(8.0, 48.0), "A stage 6", &TextStyle::default());
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn build() -> UiTree {
    let mut tree = UiTree::new(Box::new(Painted {
        id: "leaf",
        rect: Rect::ZERO,
    }));
    tree.invalidate_all();
    tree
}

#[test]
fn soft_backend_reports_its_own_stages_and_never_fakes_others() {
    std::env::set_var("Y901_FORCE_SOFT", "1");
    std::env::set_var("Y901_RENDER_STATS", "1");
    let _guard = WIN_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-stats-soft") else {
        return;
    };

    assert_eq!(
        win.render_stats_backend(),
        Some(BackendKind::Soft),
        "`--force-soft` 必须自报 Soft（不是 D3d11）：两条路径的能力表不同"
    );

    let mut tree = build();
    win.resize(&mut tree);

    let t0 = std::time::Instant::now();
    let outcome = WindowBackend::render(&mut win, &mut tree).expect("render");
    let wall = t0.elapsed();
    assert!(
        matches!(outcome, y901_runtime::window::RenderOutcome::Presented),
        "首帧应上屏，实际 {outcome:?}"
    );

    let stats = win.take_render_stats().expect("开启统计后必须交付统计");
    assert_eq!(stats.backend, BackendKind::Soft);

    // 1) 三个可测阶段都必须有值。
    for id in [
        StageId::RenderPrepare,
        StageId::CpuRaster,
        StageId::PresentBlocking,
    ] {
        assert!(
            stats.stages.get(id).is_some(),
            "{} 必须被测量（软路径走这条）",
            id.name()
        );
    }

    // 2) 不存在 / 不可测的阶段不得伪造。
    for id in [
        StageId::BackendSubmit,
        StageId::GpuExecute,
        StageId::PixelConvert,
        StageId::ImageAlloc,
        StageId::PixelCopy,
        StageId::CanvasBlit,
    ] {
        assert_eq!(
            stats.stage(id),
            Stage::NotApplicable,
            "{} 在 Win32 软路径上不存在，应为 NotApplicable",
            id.name()
        );
    }
    for id in [StageId::CommandGen, StageId::Compositor] {
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

    // 3) 计数器：软路径每帧整窗重绘。
    let (vw, vh) = win.viewport();
    eprintln!(
        "[stats-soft] 客户区 {vw}x{vh}、damaged_pixels={}、dirty_rect_count={}",
        stats.counters.damaged_pixels, stats.counters.dirty_rect_count
    );
    assert_eq!(
        stats.counters.damaged_pixels,
        vw as u64 * vh as u64,
        "软路径整窗重绘，受损像素应等于**实际**客户区面积"
    );
    assert!(stats.counters.dirty_rect_count >= 1);
    assert!(!stats.counters.frame_skipped);
    assert_eq!(stats.counters.atlas_upload_bytes, 0);

    // 4) 三段之和 ≤ `window.render` 墙钟（防区间重复计数）。
    let sum: Duration = [
        StageId::RenderPrepare,
        StageId::CpuRaster,
        StageId::PresentBlocking,
    ]
    .iter()
    .filter_map(|id| stats.stages.get(*id))
    .sum();
    assert!(
        sum <= wall + Duration::from_millis(2),
        "三段之和 {sum:?} 超过 window.render 墙钟 {wall:?}"
    );

    // 5) `presented` 归 Runtime，后端不得写。
    assert!(!stats.presented);
    assert_eq!(stats.validate(), Ok(()), "统计不得自相矛盾");
    assert!(
        win.take_render_stats().is_none(),
        "take_render_stats 必须是一次性消费"
    );
}

#[test]
fn soft_skipped_frame_reports_no_stages() {
    std::env::set_var("Y901_FORCE_SOFT", "1");
    std::env::set_var("Y901_RENDER_STATS", "1");
    let _guard = WIN_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-stats-soft-skip") else {
        return;
    };

    let mut tree = build();
    win.resize(&mut tree);
    WindowBackend::render(&mut win, &mut tree).expect("render 1");
    let _ = win.take_render_stats().expect("首帧统计");

    // 无脏区 → 软路径直接返回：不得有任何阶段测量值。
    let outcome = WindowBackend::render(&mut win, &mut tree).expect("render 2");
    assert!(
        matches!(outcome, y901_runtime::window::RenderOutcome::Skipped),
        "无脏区时不应上屏，实际 {outcome:?}"
    );
    let stats = win.take_render_stats().expect("跳过帧也必须交付统计");
    assert!(stats.counters.frame_skipped, "跳过帧必须标记 frame_skipped");
    for id in [
        StageId::RenderPrepare,
        StageId::CpuRaster,
        StageId::PresentBlocking,
    ] {
        assert!(
            stats.stages.get(id).is_none(),
            "跳过帧不得有 {} 的测量值（否则就是把没跑过的阶段写成 0）",
            id.name()
        );
    }
    // **把统计打成一行 JSON**（`perf-gate.ps1 -Only stats` 抓的就是这一行）。
    // 为什么放在测试里而不是只走 `run_loop`：基准要能在无人值守的 CI/本机命令里
    // 直接拿到数字，不依赖启动一个真窗口应用。
    let mut json = String::new();
    stats.write_json(&mut json);
    eprintln!("{json}");
    assert!(json.starts_with('{'), "JSON 行必须以 {{ 开头：{json}");
    assert!(
        json.contains("\"backend\"") && json.contains("\"stages\":"),
        "JSON 行必须含 backend 与 stages：{json}"
    );

    assert_eq!(stats.validate(), Ok(()));

    // 下一帧仍能正常交付（每帧重置，不跨帧污染）。
    tree.invalidate_all();
    WindowBackend::render(&mut win, &mut tree).expect("render 3");
    let stats = win.take_render_stats().expect("第 3 帧统计");
    assert!(stats.stages.get(StageId::CpuRaster).is_some());
    assert!(!stats.counters.frame_skipped);
}
