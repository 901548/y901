#![cfg(target_os = "linux")]
//! 证明**两个 `X11Window` 在同一进程里互不干扰**。
//!
//! 为何要有它：多窗口盘点的结论是"X11 天然支持"——
//! 每个 `X11Window` 有自己的 `Connection`、自己的 `wid`、自己的事件队列。
//! 但那是**推断**；这条测试把它变成**证据**：给 A 发一个真实 X11 按键事件，
//! 断言 A 收到、B 没收到。
//!
//! 运行：
//! ```bash
//! xvfb-run -a cargo test -p y901-backend-x11 --test multi_window -- --ignored --nocapture
//! ```
//! 需要 `xdotool`（向指定窗口注入按键）。缺少时测试会**显式跳过并打印原因**，不假绿。

use std::process::Command;
use std::time::{Duration, Instant};

use y901_backend_x11::window::{CreateArgs, X11Window};
use y901_render::soft::SoftBackend;
use y901_runtime::window::WindowBackend;
use y901_ui::Size;

/// 建窗用的软光栅器：文字要真画，所以给一个真实存在的字体。
fn soft_for(w: u16, h: u16, size: f32) -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(size);
    let cell = Size::new(cw, ch);
    let path = y901_render::default_test_font_path();
    let engine = y901_render::MyFontEngine::new(
        y901_render::load_font(&path).expect("字体可读"),
        size,
        cell,
    )
    .expect("字形引擎");
    SoftBackend::new(w as u32, h as u32, Box::new(engine), cell, None).expect("软光栅器")
}

/// 把某个窗口在一段时间内收到的原始事件**全部**拉完（直到连续空轮）。
fn drain_events(win: &mut X11Window) -> Vec<y901_runtime::RawEvent> {
    let mut out = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        match win.wait_events(50) {
            None => break,
            Some(evs) if evs.is_empty() => break, // 空轮表示已经挤干
            Some(evs) => out.extend(evs),
        }
    }
    out
}

#[test]
#[ignore = "需要 X server + xdotool（向指定窗口注入按键）"]
fn two_windows_receive_their_own_events() {
    // xdotool 不在就显式跳过——不能让"不能测"看起来像"测过了"。
    if Command::new("xdotool").arg("version").output().is_err() {
        eprintln!("[SKIP] 缺少 xdotool，无法向指定窗口注入按键，跳过");
        return;
    }
    let d = std::env::var("DISPLAY").unwrap_or_default();

    let build = |title: &str| {
        X11Window::create(CreateArgs {
            display: &d,
            width: 320,
            height: 200,
            title,
            soft: soft_for(320, 200, 15.0),
            fullscreen: false,
        })
        .expect("建窗")
    };
    let mut a = build("multi-a");
    let mut b = build("multi-b");

    assert_ne!(a.wid, b.wid, "两个窗口必须有不同的 X11 window id");

    // 先把两个窗口在建窗期间积压的事件（Expose 等）拉完，
    // 否则后面无法区分"新事件"与"建窗带来的旧事件"。
    let _ = drain_events(&mut a);
    let _ = drain_events(&mut b);

    // 只给 A 发 Esc。`--window` 让 xdotool 直接向目标窗口发送，
    // 不依赖"谁拥有焦点"（这正是测试要排除的干扰）。
    let sent = Command::new("xdotool")
        .args([
            "key",
            "--window",
            &a.wid.to_string(),
            "--clearmodifiers",
            "Escape",
        ])
        .output()
        .expect("xdotool key");
    assert!(
        sent.status.success(),
        "xdotool 发送失败：stderr={:?}",
        String::from_utf8_lossy(&sent.stderr)
    );

    let a_events = drain_events(&mut a);
    let b_events = drain_events(&mut b);

    eprintln!(
        "[multi] A(wid=0x{:X}) 收到 {} 个事件；B(wid=0x{:X}) 收到 {} 个",
        a.wid,
        a_events.len(),
        b.wid,
        b_events.len()
    );

    assert!(
        !a_events.is_empty(),
        "目标窗口 A 应收到至少一个事件（按键）"
    );
    assert!(
        b_events.is_empty(),
        "另一个窗口 B 不应收到 A 的事件，实际收到 {} 个",
        b_events.len()
    );
}

/// **回归守卫**：全屏状态不得再回到进程级 `static`。
///
/// 为何用"结构断言"而非运行时测试：两个窗口的全屏状态差异在外部**无法直接读取**
/// （字段私有，`set_fullscreen` 又是状态机 no-op）。但这个改动的回归风险很具体：
/// **有人把 `static` 加回来**，那会让两个窗口共享标志（A 的 Esc 被 B 抢走）。
///
/// 旧实现：`static FULLSCREEN` + `static WANT_EXIT_FS` + `fn is_fullscreen_window()`。
/// 现实现：每个 `X11Window` 自带 `fullscreen: bool` + `want_exit_fs: Cell<bool>`。
#[test]
fn fullscreen_state_is_per_window_not_process_wide() {
    let src = include_str!("../src/window.rs");
    assert!(
        !src.contains("static FULLSCREEN"),
        "不得回到进程级 FULLSCREEN：多窗口下它会让两个窗口的全屏状态互相覆盖"
    );
    assert!(
        !src.contains("static WANT_EXIT_FS"),
        "不得回到进程级 WANT_EXIT_FS：A 窗口的 Esc 会被 B 窗口的 wait_events 抢走"
    );
    assert!(
        src.contains("want_exit_fs: std::cell::Cell<bool>"),
        "应以每窗口的 Cell 承载退出全屏请求"
    );
}
