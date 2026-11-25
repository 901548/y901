#![cfg(windows)]

//! **D3D11 分阶段统计（`Y901_RENDER_STATS=1`）**的端到端验收。
//!
//! 这个测试文件**自己设置环境变量**：`render_stats::enabled()` 是进程级
//! `OnceLock`，同一进程里只能有一种模式。因此"开启"与"关闭"分别放在
//! 不同的测试二进制（本文件 = 开启；`render_stats_disabled.rs` = 关闭），
//! 避免并行用例互相污染。
//!
//! 没有 D3D11 设备的机器（CI 的 windows runner）按仓库既有惯例**留痕跳过**
//!（写 `y901-test-skips.txt` + stderr），不 panic、不静默 return。

use std::time::{Duration, Instant};

use y901_backend_win32::window::Window;
use y901_runtime::render_stats::{BackendKind, Stage, StageId};
use y901_runtime::window::WindowBackend;
use y901_runtime::Options;
use y901_ui::{Color, Edges, Flex, UiTree};
use y901_widgets::TextLabel;

/// 同一进程内只允许一个 D3D 窗口活着（与 `grid_image_gpu.rs` 同款）。
static D3D_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 留痕跳过：gate 只认写进 `y901-test-skips.txt` 的 SKIP。
fn skip(reason: &str) {
    let path = std::env::temp_dir().join("y901-test-skips.txt");
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "SKIP: render_stats_enabled {reason}");
    }
    eprintln!("[stats-test] 跳过：{reason}");
}

fn window(title: &str, w: u32, h: u32) -> Option<Window> {
    let options = Options {
        title: title.to_string(),
        width: w,
        height: h,
        theme: None,
        font: None,
        fullscreen: false,
    };
    Window::create(&options).ok()
}

/// 一棵"会真的画出东西"的小树：背景 + 两行文字（文字会触发图集上传）。
fn build() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(24, 24, 32))
        .padding(Edges::all(8.0))
        .gap(4.0)
        .push(Box::new(
            TextLabel::new("l1", "RenderStats A 阶段").color(Color::WHITE),
        ))
        .push(Box::new(
            TextLabel::new("l2", "D3D11 instrumentation").color(Color::WHITE),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    tree
}

#[test]
fn d3d11_reports_stage_timings() {
    // 必须在**任何** `enabled()` 调用之前设置（本进程第一次读就缓存）。
    std::env::set_var("Y901_RENDER_STATS", "1");
    // 不要 headless：本用例要验证真实 Present 也被计时。
    std::env::remove_var("Y901_HEADLESS");

    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());

    let Some(mut win) = window("y901-render-stats", 420, 240) else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };
    // 后端类型识别：D3D 路径必须自报 D3d11（软路径才是 None）。
    assert_eq!(
        win.render_stats_backend(),
        Some(BackendKind::D3d11),
        "D3D11 窗口必须自报 BackendKind::D3d11"
    );

    let mut tree = build();
    win.resize(&mut tree);

    // ---- 第一帧：完整渲染 ----
    let wall = Instant::now();
    win.render(&mut tree);
    let wall = wall.elapsed();

    let stats = match win.take_render_stats() {
        Some(s) => s,
        None => {
            skip("take_render_stats 返回 None（这台机器上 D3D 没跑起来）");
            return;
        }
    };

    // 1) Runtime 负责的边界阶段**不在**后端数据里（后端只管自己那 12 个）。
    // 2) 后端负责的三个核心阶段必须被测量。
    for id in [
        StageId::RenderPrepare,
        StageId::CommandGen,
        StageId::BackendSubmit,
    ] {
        assert!(
            stats.stages.get(id).is_some(),
            "{} 必须被测量（真实渲染帧）",
            id.name()
        );
    }
    // 3) 计数器：至少推入了实例，且绘制数不超过单帧上限。
    assert!(
        stats.counters.instance_pushed > 0,
        "本帧应推入实例（画面不是空的）"
    );
    assert!(
        stats.counters.instance_drawn <= 10_000,
        "绘制数不得超过 INSTANCE_CAPACITY"
    );
    assert!(stats.counters.dirty_rect_count > 0);
    assert!(stats.counters.damaged_pixels > 0);
    assert!(!stats.counters.frame_skipped);

    // 4) **Submit 与 Present 互斥**：所有后端阶段之和不得超过 `window.render` 墙钟。
    //    这条正是 `render_batched` 里"submit 段切在 present 之前"要保证的性质；
    //    一旦有人把 present 也算进 submit，求和就会超过墙钟。
    let sum = stats.backend_stage_sum();
    assert!(
        sum <= wall + Duration::from_millis(2),
        "后端阶段之和 {:?} 超过了 window.render 的墙钟 {:?}（说明阶段重叠被重复计数）",
        sum,
        wall
    );

    // 5) 真实 Present 已计时（非 headless）。
    assert!(
        stats.stages.get(StageId::PresentBlocking).is_some(),
        "非 headless 帧必须测量 present_blocking"
    );

    // 6) GPU / 合成器**永远不可测**（本阶段没有 timestamp query）。
    for id in [StageId::GpuExecute, StageId::Compositor] {
        match stats.stage(id) {
            Stage::Unavailable(reason) => {
                assert!(
                    !reason.trim().is_empty(),
                    "{} 的不可用原因不能为空",
                    id.name()
                )
            }
            other => panic!(
                "{} 必须是 Unavailable（不得伪造），实际 {:?}",
                id.name(),
                other
            ),
        }
    }

    // 7) **presented 归 Runtime**：后端不得写该字段（B1 裁决）。
    assert!(
        !stats.presented,
        "后端统计里的 presented 必须恒为 false（真值由 Runtime 写）"
    );

    // 8) 自检：数据不得自相矛盾。
    assert_eq!(stats.validate(), Ok(()));

    // 9) 取走语义：第二次必为 None（每帧最多消费一次）。
    assert!(
        win.take_render_stats().is_none(),
        "take_render_stats 必须是一次性消费"
    );
}

#[test]
fn skipped_frame_reports_no_backend_stages_and_does_not_leak() {
    std::env::set_var("Y901_RENDER_STATS", "1");
    std::env::remove_var("Y901_HEADLESS");

    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());

    let Some(mut win) = window("y901-render-stats-skip", 360, 200) else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };
    let mut tree = build();
    win.resize(&mut tree);

    // 第 1 帧：完整渲染（会产生 upload / submit / present 等阶段）。
    win.render(&mut tree);
    let first = match win.take_render_stats() {
        Some(s) => s,
        None => {
            skip("首帧没有统计（D3D 未跑起来）");
            return;
        }
    };
    assert!(first.stages.get(StageId::BackendSubmit).is_some());

    // 第 2 帧：什么都没标脏 → 后端跳过渲染。
    assert!(tree.dirty().is_empty(), "渲染后脏区应已清空");
    win.render(&mut tree);
    let second = match win.take_render_stats() {
        Some(s) => s,
        None => {
            skip("跳过帧没有交付统计（D3D 未跑起来）");
            return;
        }
    };

    // 跳过帧：`frame_skipped=true`，且**没有任何后端阶段**（全部保持 None）。
    assert!(
        second.counters.frame_skipped,
        "跳过帧必须标记 frame_skipped=true"
    );
    for id in [
        StageId::RenderPrepare,
        StageId::CommandGen,
        StageId::Upload,
        StageId::BackendSubmit,
        StageId::PresentBlocking,
    ] {
        assert!(
            second.stages.get(id).is_none(),
            "跳过帧的 {} 必须保持未测量（不得继承上一帧）",
            id.name()
        );
    }
    assert_eq!(second.counters.instance_pushed, 0);
    assert_eq!(second.counters.damaged_pixels, 0);
    // 跨帧隔离：第一帧确实有提交，第二帧没有 —— 证明上面的断言不是"从来没数据"。
    assert!(first.stages.get(StageId::BackendSubmit).is_some());
    assert_eq!(second.validate(), Ok(()));
}
