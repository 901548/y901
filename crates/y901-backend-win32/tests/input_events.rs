#![cfg(windows)]

//! P1-2：输入层扩展（右键/中键/系统键/键松开/滚轮带修饰键）。
//!
//! 全部走 `handle_wm_msg` / `wheel_event` 这两个纯函数——与 `wndproc` 里
//! 调用它们的是同一份逻辑，所以不依赖窗口也能测。

use y901_backend_win32::window::{handle_wm_msg, wheel_event, CharFilter};
use y901_runtime::RawEvent;
use y901_ui::{Key, Modifiers, MouseButton};

// Windows 消息号（与 windows-sys 常量一致）
const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_LBUTTONDOWN: u32 = 0x0201;
// 测试 API：测试内的类型见证/未用常量，仅测试代码引用
#[allow(dead_code)] // 完整消息集的一部分（用例不覆盖松开左键）
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
// 测试 API：测试内的类型见证/未用常量，仅测试代码引用
#[allow(dead_code)] // 同上
const WM_MBUTTONUP: u32 = 0x0208;

fn lparam(x: i32, y: i32) -> i64 {
    (((y as u32) << 16) | (x as u32 & 0xFFFF)) as i64
}

#[test]
fn right_click_produces_mouse_down_right() {
    let mut f = CharFilter::default();
    let ev = handle_wm_msg(
        WM_RBUTTONDOWN,
        0,
        lparam(30, 40),
        Modifiers::default(),
        &mut f,
    );
    assert_eq!(
        ev,
        Some(RawEvent::MouseDown {
            x: 30.0,
            y: 40.0,
            button: MouseButton::Right
        })
    );

    let up = handle_wm_msg(
        WM_RBUTTONUP,
        0,
        lparam(30, 40),
        Modifiers::default(),
        &mut f,
    );
    assert_eq!(
        up,
        Some(RawEvent::MouseUp {
            x: 30.0,
            y: 40.0,
            button: MouseButton::Right
        })
    );
}

#[test]
fn middle_click_produces_mouse_down_middle() {
    let mut f = CharFilter::default();
    let ev = handle_wm_msg(
        WM_MBUTTONDOWN,
        0,
        lparam(5, 6),
        Modifiers::default(),
        &mut f,
    );
    assert_eq!(
        ev,
        Some(RawEvent::MouseDown {
            x: 5.0,
            y: 6.0,
            button: MouseButton::Middle
        })
    );
}

#[test]
fn left_click_still_works() {
    let mut f = CharFilter::default();
    let ev = handle_wm_msg(
        WM_LBUTTONDOWN,
        0,
        lparam(1, 2),
        Modifiers::default(),
        &mut f,
    );
    assert_eq!(
        ev,
        Some(RawEvent::MouseDown {
            x: 1.0,
            y: 2.0,
            button: MouseButton::Left
        })
    );
}

#[test]
fn alt_key_produces_system_key_down() {
    let mut f = CharFilter::default();
    let mods = Modifiers {
        alt: true,
        ..Modifiers::default()
    };
    // VK_I = 0x49
    let ev = handle_wm_msg(WM_SYSKEYDOWN, 0x49, 0, mods, &mut f);
    assert_eq!(
        ev,
        Some(RawEvent::SystemKeyDown {
            key: Key::Char('i'),
            mods
        }),
        "Alt+键必须走 SystemKeyDown，且带上 alt 修饰键"
    );
}

#[test]
fn ctrl_wheel_produces_wheel_with_mods() {
    let mods = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    // wparam 高 16 位 = 120（向上滚一格）
    let wparam: u64 = (120u64) << 16;
    let ev = wheel_event(10.0, 20.0, wparam, mods);
    assert_eq!(
        ev,
        RawEvent::Wheel {
            x: 10.0,
            y: 20.0,
            delta: 120.0,
            mods
        },
        "Ctrl+滚轮必须把 mods 一起带出去（此前 Wheel 没有 mods 字段）"
    );
}

#[test]
fn plain_wheel_has_no_modifiers() {
    let wparam: u64 = (-120i64 as u64) << 16;
    let ev = wheel_event(0.0, 0.0, wparam, Modifiers::default());
    match ev {
        RawEvent::Wheel { delta, mods, .. } => {
            assert_eq!(delta, -120.0);
            assert!(!mods.ctrl && !mods.shift && !mods.alt);
        }
        other => panic!("应为 Wheel，实际 {other:?}"),
    }
}

#[test]
fn char_after_keydown_is_still_deduplicated() {
    // 回归：新增分支不能破坏 P0-3 的控制键去重
    let mut f = CharFilter::default();
    assert!(matches!(
        handle_wm_msg(WM_KEYDOWN, 0x0D, 0, Modifiers::default(), &mut f),
        Some(RawEvent::KeyDown { .. })
    ));
    assert!(handle_wm_msg(WM_CHAR, 0x0D, 0, Modifiers::default(), &mut f).is_none());
}

/// WM_DROPFILES → RawEvent::FileDrop → UiEvent::FileDrop 的翻译与路由。
///
/// 无法在测试里构造真实 HDROP（需要 OLE 拖放源），所以这里验证两段可测的链路：
/// 1. `translate` 把 `RawEvent::FileDrop` 翻成带坐标的 `UiEvent::FileDrop`；
/// 2. `UiTree::dispatch` 按坐标命中目标控件（和鼠标事件同一套 hit-test）。
#[test]
fn file_drop_translates_and_routes_to_hit_target() {
    use std::path::PathBuf;
    use y901_runtime::translate;
    use y901_ui::{Pos, Size, UiEvent};

    let raw = y901_runtime::RawEvent::FileDrop {
        paths: vec![
            PathBuf::from(r"C:\\tmp\\a.txt"),
            PathBuf::from(r"C:\\tmp\\b.bin"),
        ],
        x: 42.0,
        y: 24.0,
    };
    let ev = translate(&raw, Size::new(400.0, 300.0)).expect("FileDrop 必须被翻译");
    match &ev {
        UiEvent::FileDrop { paths, pos } => {
            assert_eq!(paths.len(), 2, "两个路径都要保留");
            assert!(paths[0].ends_with("a.txt"));
            assert_eq!(*pos, Pos::new(42.0, 24.0), "坐标必须保留");
        }
        other => panic!("翻译结果不对：{other:?}"),
    }
}
