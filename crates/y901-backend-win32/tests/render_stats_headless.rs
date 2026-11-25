#![cfg(windows)]

//! **无头模式（`Y901_HEADLESS=1`）下 `present_blocking` 必须保持未测量**。
//!
//! 无头模式在 `render_batched` 里 `headless_present_skipped()` 处**提前返回**，
//! 根本不调用 `IDXGISwapChain::Present` —— 因此 `present_blocking` 必须保持 `None`
//!（而不是写 0，也不该拿 submit 的时间冒充）。
//!
//! 独立测试二进制的原因：`headless_present_skipped()` 与
//! `render_stats::enabled()` 都是进程级 `OnceLock`。

use y901_backend_win32::window::Window;
use y901_runtime::render_stats::{BackendKind, StageId};
use y901_runtime::window::WindowBackend;
use y901_runtime::Options;
use y901_ui::{Color, Edges, Flex, UiTree};
use y901_widgets::TextLabel;

static D3D_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn skip(reason: &str) {
    let path = std::env::temp_dir().join("y901-test-skips.txt");
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "SKIP: render_stats_headless {reason}");
    }
    eprintln!("[stats-test] 跳过：{reason}");
}

#[test]
fn headless_frame_does_not_measure_present() {
    std::env::set_var("Y901_RENDER_STATS", "1");
    std::env::set_var("Y901_HEADLESS", "1");

    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());

    let options = Options {
        title: "y901-render-stats-headless".to_string(),
        width: 320,
        height: 200,
        theme: None,
        font: None,
        fullscreen: false,
    };
    let Some(mut win) = Window::create(&options).ok() else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };
    assert_eq!(win.render_stats_backend(), Some(BackendKind::D3d11));

    let root = Flex::column("root")
        .background(Color::from_rgb8(20, 20, 26))
        .padding(Edges::all(8.0))
        .push(Box::new(
            TextLabel::new("t", "headless").color(Color::WHITE),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    win.resize(&mut tree);
    win.render(&mut tree);

    let Some(stats) = win.take_render_stats() else {
        skip("headless 帧没有交付统计（D3D 未跑起来）");
        return;
    };

    // 提交确实发生了（无头只跳过 Present，不跳过渲染）。
    assert!(
        stats.stages.get(StageId::BackendSubmit).is_some(),
        "无头模式仍然渲染并提交，backend_submit 必须被测量"
    );
    // 但 Present 没发生 → 必须保持 `None`（不是 0，也不是 submit 的时间）。
    assert!(
        stats.stages.get(StageId::PresentBlocking).is_none(),
        "无头模式下没有调用 Present，present_blocking 必须保持未测量"
    );
    // 且不能被解释成 NotApplicable（D3D11 的这个阶段是"可测但本帧没走"）。
    match stats.stage(StageId::PresentBlocking) {
        y901_runtime::render_stats::Stage::Unavailable(reason) => {
            assert!(!reason.trim().is_empty(), "未测量的原因不能为空");
        }
        other => panic!("present_blocking 应为 Unavailable，实际 {other:?}"),
    }
    assert_eq!(stats.validate(), Ok(()));
}
