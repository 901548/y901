#![cfg(windows)]

//! **统计关闭时不产生任何统计**（`Y901_RENDER_STATS` 未设置）。
//!
//! 与 `render_stats_enabled.rs` 分成两个测试二进制是**必须的**：
//! `render_stats::enabled()` 是进程级 `OnceLock`，同进程只能有一种模式。

use y901_backend_win32::window::Window;
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
        let _ = writeln!(f, "SKIP: render_stats_disabled {reason}");
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

#[test]
fn disabled_stats_report_nothing() {
    // 显式清掉：即使外部 shell 设过，也让本进程走"关闭"路径。
    std::env::remove_var("Y901_RENDER_STATS");

    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());

    let Some(mut win) = window("y901-render-stats-off", 320, 200) else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };

    // 后端类型仍然可报（种类与"是否开启统计"是两件事）。
    assert_eq!(
        win.render_stats_backend(),
        Some(y901_runtime::render_stats::BackendKind::D3d11)
    );

    let root = Flex::column("root")
        .background(Color::from_rgb8(20, 20, 26))
        .padding(Edges::all(8.0))
        .push(Box::new(
            TextLabel::new("t", "disabled").color(Color::WHITE),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    win.resize(&mut tree);
    win.render(&mut tree);

    // 关闭统计：**不产生统计对象**（因此也没有任何计时/分配发生）。
    assert!(
        win.take_render_stats().is_none(),
        "统计关闭时 take_render_stats 必须返回 None"
    );
    // 再取一次仍为 None（不是"被取走了"，而是"从来没有"）。
    assert!(win.take_render_stats().is_none());

    // 再渲染一帧也依然没有。
    tree.invalidate_all();
    win.render(&mut tree);
    assert!(win.take_render_stats().is_none());
}
