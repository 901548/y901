#![cfg(windows)]

//! **`WM_CLOSE` 不再立刻销毁窗口**（审计 #4）。
//!
//! 旧行为：点标题栏 X → `DefWindowProcW` → 同步 `DestroyWindow` → 宿主收到的
//! `RawEvent::Close` 到达时 HWND **已经不在了**。于是：
//! - 应用**无法否决**关闭（"还有未保存的改动，确定退出？"做不到）；
//! - `Window::close()` 也没有参与的机会。
//!
//! 新行为：`WM_CLOSE` 只投递 `RawEvent::Close`，窗口继续存在；宿主决定
//! 「否决（什么都不做）」还是「同意（调 `Window::close()`）」。
//! `run_loop` 保持"收到 Close 就退出"的语义，所以默认体验不变。
//!
//! 本文件钉住三件事：
//! 1. `WM_CLOSE` 之后窗口**仍然存在**（`IsWindow` 为真）；
//! 2. 事件队列里确实有 `RawEvent::Close`；
//! 3. 之后调 `close()` 才真正销毁，且重复 `close()` 不 panic（幂等）。

use y901_backend_win32::window::Window;
use y901_runtime::window::WindowBackend;
use y901_runtime::{Options, RawEvent};

const W: u32 = 320;
const H: u32 = 160;

/// 同一进程内只允许一个窗口（共享消息队列，串行更稳）。
static WIN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const WM_CLOSE: u32 = 0x0010;

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
            use std::io::Write as _;
            let path = std::env::temp_dir().join("y901-test-skips.txt");
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "SKIP: wm_close 建窗失败：{e}");
            }
            eprintln!("[wm-close] 跳过：建窗失败 {e}");
            None
        }
    }
}

fn is_window(hwnd: isize) -> bool {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as *mut core::ffi::c_void) != 0
    }
}

#[test]
fn wm_close_notifies_without_destroying_the_window() {
    let _guard = WIN_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-wm-close") else {
        return;
    };
    let hwnd = win.hwnd() as isize;
    assert!(is_window(hwnd), "前置：窗口已创建");

    // 先把创建过程中攒下的事件吃掉，避免误判。
    let _ = win.wait_events(0);

    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd as *mut core::ffi::c_void,
            WM_CLOSE,
            0,
            0,
        );
    }
    let events = win.wait_events(50).unwrap_or_default();
    assert!(
        events.iter().any(|e| matches!(e, RawEvent::Close)),
        "WM_CLOSE 必须投递 RawEvent::Close，实得 {events:?}"
    );
    assert!(
        is_window(hwnd),
        "WM_CLOSE 之后窗口**必须仍然存在**（旧实现在这里已经被 DestroyWindow 了，\
         应用根本没有否决关闭的机会）"
    );

    // 同意关闭：调 close() 才真正销毁。
    win.close();
    assert!(!is_window(hwnd), "close() 之后窗口应被销毁");
    // 幂等：再调一次不应 panic（内部 IsWindow 短路）。
    win.close();
}

/// 不调 `close()` 时，窗口一直活着 —— 这就是"否决关闭"。
#[test]
fn ignoring_the_close_event_keeps_the_window_alive() {
    let _guard = WIN_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-wm-close-veto") else {
        return;
    };
    let hwnd = win.hwnd() as isize;
    let _ = win.wait_events(0);
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd as *mut core::ffi::c_void,
            WM_CLOSE,
            0,
            0,
        );
    }
    let _ = win.wait_events(50);
    assert!(is_window(hwnd), "否决关闭后窗口必须继续存在");

    // 再泵几轮、甚至等一会儿，窗口仍然在（没有后台的 DestroyWindow 兜底）。
    std::thread::sleep(std::time::Duration::from_millis(60));
    let _ = win.wait_events(10);
    assert!(is_window(hwnd), "否决之后不得被偷偷销毁");

    win.close();
    assert!(!is_window(hwnd));
}
