//! Win32 窗口与消息泵。
//!
//! **纪律**：`wndproc` 只把平台事件翻译成 `RawEvent` 推进队列，
//! 绝不触碰 `UiTree` / `Host`。这样窗口层可以被任何宿主复用，
//! 也让"事件翻译"成为可单测的纯函数（见 `y901_runtime::translate`）。

use std::sync::{Mutex, OnceLock};

/// 脏区列表的上限：超过就降级为整窗重绘。
///
/// 为什么要有上限：每块脏区都要一次 `scissor + clear + DrawInstanced`，
/// 碎片化时 draw 次数会翻倍，反而比一次整帧更慢。
const MAX_DIRTY_RECTS: usize = 16;

/// 把 `DirtyRegion` 转成后端要的矩形列表。
///
/// 返回：`None` = 整窗重绘（`full` 或脏区碎片超过上限）；
/// `Some(list)` = 逐块重绘。
///
/// 抽成独立函数是为了可测：`Window::render` 需要真实窗口，
/// 而这个决策本身只依赖 `DirtyRegion`。
fn plan_dirty_rects(dirty: &y901_ui::DirtyRegion) -> Option<Vec<(i32, i32, i32, i32)>> {
    if dirty.is_full() {
        return None;
    }
    let list: Vec<(i32, i32, i32, i32)> = dirty
        .rects()
        .iter()
        .map(|r| {
            (
                r.x.floor() as i32,
                r.y.floor() as i32,
                (r.x + r.w).ceil() as i32,
                (r.y + r.h).ceil() as i32,
            )
        })
        .collect();
    if list.is_empty() || list.len() > MAX_DIRTY_RECTS {
        return None;
    }
    Some(list)
}

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForSystem, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, DragQueryPoint};
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, LoadCursorW,
    MsgWaitForMultipleObjectsEx, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetCursor, SetWindowLongPtrW, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTCLIENT, IDC_ARROW, IDC_HAND, IDC_IBEAM,
    WM_DROPFILES,
    IDC_SIZEALL, IDC_SIZENS, IDC_SIZEWE, MSG, PM_REMOVE, QS_ALLINPUT, SW_SHOW, WM_CHAR, WM_DESTROY,
    WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_NULL, WM_PAINT, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE,
    WM_SYSCHAR, WM_SYSKEYDOWN, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

use y901_runtime::{Options, RawEvent};
use y901_ui::{Cursor, Key, Modifiers, MouseButton, Pos, Size, UiTree};

use crate::backend::D3dBackend;
use crate::ime::{handle_ime_message, ImeAction, ImeState};

/// 跨线程唤醒句柄：后台线程（PTY 读、RPC）用它打断主循环的阻塞等待。
///
/// 为什么需要：主循环空闲时阻塞在 `MsgWaitForMultipleObjectsEx` 上（0% CPU），
/// 此时"网络/管道里来了数据"不会产生 Windows 消息，必须有人显式唤醒。
/// 该句柄是 `Send + Sync`，可以安全地clone到任意线程。
#[derive(Clone)]
pub struct WakeHandle {
    hwnd: HWND,
}

// SAFETY: `PostMessageW` 是线程安全的；HWND 在窗口存活期间有效，
// 调用方需保证不在窗口销毁后继续使用（与 `Window` 同一个生命周期约束）。
unsafe impl Send for WakeHandle {}
unsafe impl Sync for WakeHandle {}

impl WakeHandle {
    /// 唤醒主循环（可在任意线程调用）。
    ///
    /// 原理：往窗口投一条 `WM_NULL`，`MsgWaitForMultipleObjectsEx` 会立刻返回。
    /// 副作用：主循环跑一帧（若无脏区则立刻回到等待）。
    /// 失败：窗口已销毁时静默忽略。
    pub fn wake(&self) {
        if !self.hwnd.is_null() {
            // SAFETY: 句柄来自 Window::create；wndproc 对 WM_NULL 走 DefWindowProc。
            unsafe {
                PostMessageW(self.hwnd, WM_NULL, 0, 0);
            }
        }
    }
}

/// 平台事件队列：wndproc 写，宿主读。
static QUEUE: Mutex<Vec<RawEvent>> = Mutex::new(Vec::new());
/// 最近一次客户区尺寸。
static SIZE: Mutex<(u32, u32)> = Mutex::new((0, 0));
/// IME 状态属于窗口（输入法进程的镜像），与 UI 无关。
static IME: OnceLock<Mutex<ImeState>> = OnceLock::new();
static CLASS_READY: OnceLock<bool> = OnceLock::new();

fn queue() -> std::sync::MutexGuard<'static, Vec<RawEvent>> {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner())
}

fn push(ev: RawEvent) {
    queue().push(ev);
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn ime_state() -> std::sync::MutexGuard<'static, ImeState> {
    IME.get_or_init(|| Mutex::new(ImeState::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn modifiers() -> Modifiers {
    // SAFETY：`GetKeyState` 是 user32 的查询函数，无指针参数、无可失败前置条件；
    // 它读的是**调用线程**的键盘状态队列，所以必须在本 GUI 线程调用（跨线程读到的是别的队列）。
    // 违反后果：跨线程读到错误/恒为 0 的状态 —— 修饰键永远不放映，界面表现为按键行为异常。
    unsafe {
        Modifiers {
            shift: GetKeyState(0x10) < 0,
            ctrl: GetKeyState(0x11) < 0,
            alt: GetKeyState(0x12) < 0,
        }
    }
}

fn vk_to_key(vk: u32) -> Key {
    match vk {
        0x0D => Key::Enter,
        0x1B => Key::Escape,
        0x08 => Key::Backspace,
        0x09 => Key::Tab,
        0x2E => Key::Delete,
        0x26 => Key::Up,
        0x28 => Key::Down,
        0x25 => Key::Left,
        0x27 => Key::Right,
        0x24 => Key::Home,
        0x23 => Key::End,
        0x21 => Key::PageUp,
        0x22 => Key::PageDown,
        0x70..=0x87 => Key::Function((vk - 0x70 + 1) as u8),
            // SAFETY：同上（`GetKeyState` 无指针参数）；这里额外用它判断 CapsLock 是否开启，
            // 只有在 GUI 线程才有意义。违反后果：大写锁定判断恒错 —— 字母大小写翻转。
        0x41..=0x5A if unsafe { GetKeyState(0x11) < 0 } => char::from_u32(vk + 0x20)
            .map(Key::Char)
            .unwrap_or(Key::Unknown),
        _ => Key::Unknown,
    }
}

/// `WM_SYSKEYDOWN`（Alt+键）专用的键码映射。
///
/// 与 [`vk_to_key`] 的区别：Alt 组合键**不会**产生 `WM_CHAR`（字符被系统吞掉），
/// 因此字母/数字必须在这里翻译成 `Key::Char`，否则 `SystemKeyDown` 永远只有
/// `Key::Unknown`，宿主无法知道用户按了哪个键。
///
/// 说明：普通 `WM_KEYDOWN` 仍走 [`vk_to_key`]——字母字符由 `WM_CHAR` 投递，
/// 若这里也返回 `Key::Char`，组件（`TextInput` 的 Ctrl+A/C 分支）会处理两次。
fn vk_to_key_for_system(vk: u32) -> Key {
    match vk {
        0x41..=0x5A => char::from_u32(vk + 0x20)
            .map(Key::Char)
            .unwrap_or(Key::Unknown),
        0x30..=0x39 => char::from_u32(vk).map(Key::Char).unwrap_or(Key::Unknown),
        _ => vk_to_key(vk),
    }
}

/// # Safety
///
/// `hwnd` 必须仍是**有效且未销毁**的窗口句柄；本函数只读光标位置与客户区矩形。
/// 违反后果：对已销毁的 HWND 调用 `GetClientRect`/`ScreenToClient` 会失败并留下未初始化的 `POINT`，
/// 返回的坐标是垃圾值（鼠标命中会偏移到别处）。
unsafe fn client_pos(hwnd: HWND) -> Pos {
    let mut pt = POINT { x: 0, y: 0 };
    let _ = GetCursorPos(&mut pt);
    let _ = ScreenToClient(hwnd, &mut pt);
    Pos::new(pt.x as f32, pt.y as f32)
}

/// "下一帧必须整窗重绘"标记：由 `WM_PAINT` 置位，由 runtime 取走。
///
/// 为什么需要：D3D 交换链的内容**不受** Windows 保护——
/// 遮挡/最小化恢复后，窗口客户区是未定义的（黑或花）。
/// 而我们的渲染是"有脏区才画"，恢复时通常没有脏区，所以必须显式请求一次全量。
static NEED_FULL_REDRAW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 请求下一帧整窗重绘（`WM_PAINT` 走这里）。
///
/// 副作用：置位一次性标志；不会被覆盖、不会重复触发（多次调用等价于一次）。
#[doc(hidden)]
pub fn request_full_redraw() {
    NEED_FULL_REDRAW.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// 取走"整窗重绘"请求（runtime 每帧调一次）。
///
/// 返回：`true` = 有人在上一帧之后请求过；取走即清零。
#[doc(hidden)]
pub fn take_full_redraw() -> bool {
    NEED_FULL_REDRAW.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// 全局按键去重状态（`wndproc` 是无状态的 C 回调，只能放静态）。
///
/// 用 `Mutex` 而不是 `AtomicU32`：`CharFilter` 有"取走并清空"的复合语义，
/// 用原子实现要两次 CAS，容易写错；窗口消息本来就在同一线程串行处理。
static CHAR_FILTER: std::sync::Mutex<Option<CharFilter>> = std::sync::Mutex::new(None);

/// 控制键的 VK → 它同时会产生的那个控制字符。
///
/// 只覆盖 `TranslateMessage` 会重发 `WM_CHAR` 的四个键：
/// Enter(0x0D) / Tab(0x09) / Backspace(0x08) / Escape(0x1B)。
/// 其它键没有"双写"问题，返回 `None`。
fn control_char_for_vk(vk: u32) -> Option<char> {
    match vk {
        0x0D => Some('\r'),
        0x09 => Some('\t'),
        0x08 => Some('\u{8}'),
        0x1B => Some('\u{1b}'),
        _ => None,
    }
}

/// 这个 `WM_CHAR` 是否是紧邻 `WM_KEYDOWN` 的重复投递（应当丢弃）。
///
/// 输入：`last_vk`（上一次 WM_KEYDOWN 的键码，0 表示没有）、`c`（WM_CHAR 的字符）。
/// 返回：`true` = 已由 `WM_KEYDOWN` 处理过，丢弃。
/// 失败：无（纯判断）。
fn is_duplicate_control_char(last_vk: u32, c: char) -> bool {
    control_char_for_vk(last_vk) == Some(c)
}

/// 按键去重的状态机：`WM_KEYDOWN` 记键码，`WM_CHAR` 判断是否重复。
///
/// 抽成结构体是为了**可测**：`wndproc` 本身需要真窗口才能跑，
/// 但"按键 → 字符"的序列语义可以直接单测（见 `key_filter_tests`）。
#[derive(Default)]
/// 见 [handle_wm_msg]：按键去重状态机（跨 crate 可见以便集成测试）。
#[doc(hidden)]
pub struct CharFilter {
    last_vk: u32,
}

/// 键盘消息 → 平台事件（纯函数，**可单测**）。
///
/// 输入：`msg`（只处理 `WM_KEYDOWN` / `WM_CHAR`）、`wparam`、去重状态。
/// 返回：`Some(event)` 表示应当投递；`None` 表示丢弃（重复的控制字符、
/// 非法码点、或与该函数无关的消息）。
/// 失败：无（纯逻辑；除 `modifiers()` 读键盘状态外无副作用）。
/// 见模块文档：键盘/鼠标消息 → 平台事件（跨 crate 可见以便集成测试）。
#[doc(hidden)]
pub fn handle_wm_msg(
    msg: u32,
    wparam: u64,
    lparam: i64,
    mods: Modifiers,
    filter: &mut CharFilter,
) -> Option<RawEvent> {
    // 鼠标消息的坐标在 lparam 里（客户区坐标，低 16 位 x、高 16 位 y）
    let (mx, my) = (
        (lparam as u32 & 0xFFFF) as i16 as f32,
        (((lparam as u32) >> 16) & 0xFFFF) as i16 as f32,
    );
    match msg {
        WM_KEYDOWN => {
            filter.on_key_down(wparam as u32);
            Some(RawEvent::KeyDown {
                key: vk_to_key(wparam as u32),
                mods,
            })
        }
        // Alt+键：系统键单独成事件，避免与普通按键混淆
        WM_SYSKEYDOWN => Some(RawEvent::SystemKeyDown {
            key: vk_to_key_for_system(wparam as u32),
            mods,
        }),

        WM_CHAR => {
            let c = char::from_u32(wparam as u32)?;
            filter.on_char(c).map(|c| RawEvent::Char { c })
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => Some(RawEvent::MouseDown {
            x: mx,
            y: my,
            button: button_of(msg),
        }),
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => Some(RawEvent::MouseUp {
            x: mx,
            y: my,
            button: button_of(msg),
        }),
        _ => None,
    }
}

/// 鼠标消息 → 按键枚举。
fn button_of(msg: u32) -> MouseButton {
    match msg {
        WM_RBUTTONDOWN | WM_RBUTTONUP => MouseButton::Right,
        WM_MBUTTONDOWN | WM_MBUTTONUP => MouseButton::Middle,
        _ => MouseButton::Left,
    }
}

/// 滚轮事件（纯函数，可单测）：`wparam` 高 16 位是 ±120 的倍数。
/// 滚轮事件构造（跨 crate 可见以便集成测试）。
#[doc(hidden)]
pub fn wheel_event(x: f32, y: f32, wparam: u64, mods: Modifiers) -> RawEvent {
    let delta = ((wparam >> 16) as u16 as i16) as f32;
    RawEvent::Wheel { x, y, delta, mods }
}
impl CharFilter {
    /// 记录一次 `WM_KEYDOWN`。
    fn on_key_down(&mut self, vk: u32) {
        self.last_vk = vk;
    }

    /// 处理一次 `WM_CHAR`：返回 `Some(c)` 表示应当投递，`None` 表示丢弃。
    ///
    /// 无论是否丢弃，键码都会被清空（`WM_CHAR` 只跟随一个 `WM_KEYDOWN`）。
    fn on_char(&mut self, c: char) -> Option<char> {
        let vk = std::mem::take(&mut self.last_vk);
        if is_duplicate_control_char(vk, c) {
            None
        } else {
            Some(c)
        }
    }
}

/// # Safety
///
/// 这是 `extern "system"` 回调，由 Windows 消息循环在**创建窗口的那个 GUI 线程**上调用：
/// - 返回值必须是合法 `LRESULT`；未处理的消息必须交给 `DefWindowProcW` 兜底，
/// - 不得跨线程触碰 `Window` 的内部状态；
/// - 通过 `GetWindowLongPtrW(GWLP_USERDATA)` 取出的 `Box<Window>` 指针**只在窗口存活期间有效**，
///   且必须在 `WM_NCCREATE` 之后才可能非空。
///
/// 违反后果：返回错误的 `LRESULT` 会让系统消息处理异常（窗口不响应或无法关闭）；
/// 跨线程触碰状态会在 `Box` 已释放后读写它 —— 典型的 use-after-free，崩溃点通常在消息回调深处。
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            if create.is_null() {
                return 0;
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, hwnd as isize);
            1
        }
        WM_SIZE => {
            let w = (lparam as u32) & 0xFFFF;
            let h = ((lparam as u32) >> 16) & 0xFFFF;
            *SIZE.lock().unwrap_or_else(|e| e.into_inner()) = (w, h);
            push(RawEvent::Resize { w, h });
            0
        }
        WM_MOUSEMOVE => {
            let x = (lparam as u32 & 0xFFFF) as i16 as f32;
            let y = ((lparam as u32 >> 16) & 0xFFFF) as i16 as f32;
            push(RawEvent::MouseMove { x, y });
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN
        | WM_MBUTTONUP => {
            if let Ok(mut slot) = CHAR_FILTER.lock() {
                let filter = slot.get_or_insert_with(CharFilter::default);
                if let Some(ev) =
                    handle_wm_msg(msg, wparam as u64, lparam as i64, modifiers(), filter)
                {
                    push(ev);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            let pos = client_pos(hwnd);
            push(wheel_event(pos.x, pos.y, wparam as u64, modifiers()));
            0
        }
        WM_DROPFILES => {
            // OS 级文件拖放：取路径 + 松手点，再 DragFinish 释放
            let mut paths = Vec::new();
            let drop = wparam as *mut std::ffi::c_void;
            let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
            for i in 0..count {
                let len = DragQueryFileW(drop, i, std::ptr::null_mut(), 0);
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; len as usize + 1];
                let written = DragQueryFileW(drop, i, buf.as_mut_ptr(), buf.len() as u32);
                if written == 0 {
                    continue;
                }
                buf.truncate(written as usize);
                if let Ok(text) = String::from_utf16(&buf) {
                    paths.push(std::path::PathBuf::from(text));
                }
            }
            let mut pt = POINT { x: 0, y: 0 };
            DragQueryPoint(drop, &mut pt);
            DragFinish(drop);
            if !paths.is_empty() {
                push(RawEvent::FileDrop {
                    paths,
                    x: pt.x as f32,
                    y: pt.y as f32,
                });
                // 拖放后要求重绘（高亮状态可能需要更新）
                request_full_redraw();
            }
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_CHAR => {
            // 键盘消息统一走纯函数 `handle_wm_msg`（可单测）：它负责去重，
            // wndproc 只做"取状态 + 投递"。
            if let Ok(mut slot) = CHAR_FILTER.lock() {
                let filter = slot.get_or_insert_with(CharFilter::default);
                if let Some(ev) =
                    handle_wm_msg(msg, wparam as u64, lparam as i64, modifiers(), filter)
                {
                    push(ev);
                }
            }
            0
        }
        // WM_SYSCHAR 是 Windows 在 WM_SYSKEYDOWN 之后发的"系统菜单输入"；
        // 框架不处理系统菜单，直接吞掉（返回 0），避免 Alt+键触发系统提示音。
        WM_SYSCHAR => 0,
        WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION => {
            let action = {
                let mut ime = ime_state();
                handle_ime_message(hwnd, msg, lparam, &mut ime)
            };
            match action {
                ImeAction::Unhandled => DefWindowProcW(hwnd, msg, wparam, lparam),
                ImeAction::Redraw => {
                    let text = ime_state().composing_text();
                    let composing = !text.is_empty();
                    push(RawEvent::Ime {
                        text,
                        composition: composing,
                    });
                    0
                }
                ImeAction::Commit(text) => {
                    push(RawEvent::Ime {
                        text,
                        composition: false,
                    });
                    0
                }
            }
        }
        WM_SETCURSOR => {
            let hit = (lparam as u32) & 0xFFFF;
            if hit == HTCLIENT {
                let pos = client_pos(hwnd);
                push(RawEvent::SetCursor { x: pos.x, y: pos.y });
                1
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_PAINT => {
            let mut ps = windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT {
                hdc: std::ptr::null_mut(),
                fErase: 0,
                rcPaint: RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                fRestore: 0,
                fIncUpdate: 0,
                rgbReserved: [0; 32],
            };
            windows_sys::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            windows_sys::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            // 系统要求重画（遮挡后恢复、最小化恢复、首帧）：**不能**只 Begin/EndPaint，
            // D3D 的交换链内容不会被 Windows 恢复，必须让 runtime 下一帧整窗重绘，
            // 否则窗口是黑的/花的。这里只置标记，真正的 invalidate_all 由 runtime 做
            // （wndproc 拿不到 UiTree）。
            request_full_redraw();
            0
        }
        WM_DESTROY => {
            push(RawEvent::Close);
            PostQuitMessage(0);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// 一个 Win32 窗口。持有 HWND 与 D3D11 后端，不认识 UI 树。
pub struct Window {
    hwnd: HWND,
    backend: Option<D3dBackend>,
    /// 纯 CPU 后端（`Y901_FORCE_SOFT=1` 时启用；与 `backend` 互斥）。
    soft: Option<y901_render::soft::SoftBackend>,
    viewport: (u32, u32),
}

impl Window {
    /// 创建窗口与渲染后端。`Options.width/height` 是**窗口尺寸**（含边框与标题栏），实际渲染用的是 `GetClientRect` 得到的客户区，后者会小一圈。
    pub fn create(options: &Options) -> Result<Self, String> {
    // SAFETY：`create_inner` 的唯一前置条件是“在 GUI 线程、且 `options` 在调用期间有效”，
    // 两者在这里都成立（本函数由调用方在准备窗口的线程上执行，`options` 是借用参数）。
    // 违反后果：跨线程创建会让消息循环绑定到错误的线程，窗口收不到输入事件。
        unsafe { Self::create_inner(options) }
    }

/// # Safety
///
/// 只在 **GUI 线程**调用；`options` 的所有字段在调用期间保持有效（本函数只借用它）。
/// `CreateWindowExW` 的 `lpParam` 必须指向一个能活到 `WM_NCCREATE` 的 `Box` ——
/// 回调在 `WM_NCCREATE` 里把它取回并接管所有权；若提前释放，`wndproc` 会读到悬垂指针。
///
/// 违反后果：跨线程创建让消息循环绑定错误线程；`lpParam` 悬垂 → `WM_NCCREATE` 里 use-after-free；
/// 若中途失败而不 `DestroyWindow`，窗口类句柄泄漏并最终耗尽。
    unsafe fn create_inner(options: &Options) -> Result<Self, String> {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        // `Options.width/height` 是**客户区**尺寸，不是窗口尺寸。
        //
        // 为什么要反算：直接把它们传给 CreateWindowExW 时，它们是**窗口**尺寸，
        // 客户区要减掉边框与标题栏，而那个差值随 DPI / 主题 / DWM 状态变化——
        // 实测在同一台机器上来回在 378x164 与 382x173 之间摆动，
        // 直接让像素哈希门禁在“重建基线”与“报回归”之间反复横跳。
        // 现在用 AdjustWindowRectExForDpi 反算：**客户区恒为调用者说的大小**。
        let dpi = GetDpiForSystem();
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: options.width as i32,
            bottom: options.height as i32,
        };
        AdjustWindowRectExForDpi(&mut frame, WS_OVERLAPPEDWINDOW, 0, 0, dpi);
        let win_w = (frame.right - frame.left).max(1);
        let win_h = (frame.bottom - frame.top).max(1);

        let instance = GetModuleHandleW(std::ptr::null());
        if instance.is_null() {
            return Err("GetModuleHandleW failed".into());
        }

        let class_name = wide("Y901WindowClass");
        let ready = CLASS_READY.get_or_init(|| {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class_name.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            let atom = RegisterClassExW(&wc);
            atom != 0
        });
        if !*ready {
            return Err("RegisterClassExW failed".into());
        }

        let title = wide(&options.title);
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            win_w,
            win_h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("CreateWindowExW failed".into());
        }
        // 接受 OS 级文件拖放：没这一步 WM_DROPFILES 永远不会来。
        DragAcceptFiles(hwnd, 1);

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let _ = GetClientRect(hwnd, &mut rect);
        let viewport = (
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
        );

        let theme = options.theme.unwrap_or(y901_ui::theme::Theme::DARK);
        let font = options.font.clone().unwrap_or_default();

        // 纯 CPU 模式：不创建 D3D11 设备。
        // 为什么用环境变量而不是 `Options` 字段：`Options` 是公开结构体，
        // 加字段会破坏所有现有构造点（测试里有十几处）。
        // `--force-soft` 在 app 层译成这个环境变量。
        let force_soft = std::env::var_os("Y901_FORCE_SOFT").is_some();
        let soft = if force_soft {
            Some(
                {
                    // 字形引擎：自研（Y901_FONT=self）或 DirectWrite（默认）
                    let (cw, ch) = crate::backend::font::cell_metrics_for(font.size);
                    let cell = y901_ui::Size::new(cw, ch);
                    let engine: Box<dyn crate::backend::font::FontEngine> =
                        if std::env::var("Y901_FONT").map(|v| v == "self").unwrap_or(false) {
                            Box::new(
                                y901_render::MyFontEngine::new(
                                    y901_render::load_font(&y901_render::resolve_font_path(&font.family))
                                        .unwrap_or_default(),
                                    font.size,
                                    cell,
                                )
                                .map_err(|e| format!("MyFontEngine init failed: {e}"))?,
                            )
                        } else {
                            Box::new(
                                crate::backend::font::TextEngine::new(
                                    &font.family,
                                    font.size,
                                    cw,
                                    ch,
                                )?,
                            )
                        };
                    y901_render::soft::SoftBackend::new(viewport.0, viewport.1, engine, cell, Some(theme))
                        .map_err(|e| format!("soft backend init failed: {e}"))?
                },
            )
        } else {
            None
        };
        if force_soft {
            eprintln!("[y901] 纯 CPU 渲染后端已启用（Y901_FORCE_SOFT）：不创建 D3D11 设备");
        }
        let backend = if force_soft {
            None
        } else {
            Some(
                D3dBackend::new(hwnd, viewport.0, viewport.1, theme, &font)
                    .map_err(|e| format!("D3D11 init failed: {e}"))?,
            )
        };

        ShowWindow(hwnd, SW_SHOW);

        Ok(Self {
            hwnd,
            backend,
            soft,
            viewport,
        })
    }

    /// 运行时切换主题（转发到后端）。
    ///
    /// 返回：`Ok(())` 已切换；`Err` 表示后端尚未就绪。
    /// 副作用：主题字段更新。**全量重绘需要调用方做**
    /// （`UiTree::invalidate_all`）——窗口不认识 UI 树。
    pub fn set_theme(&mut self, theme: y901_ui::theme::Theme) -> Result<(), String> {
        if let Some(soft) = self.soft.as_mut() {
            return soft.set_theme(theme);
        }
        match self.backend.as_mut() {
            Some(b) => b.set_theme(theme),
            None => Err("渲染后端尚未就绪".to_string()),
        }
    }

    /// 运行时切换字体（转发到后端）。
    ///
    /// 返回：`Ok(())` 已切换；`Err` = 后端未就绪或字体加载失败（此时旧字体保持不变）。
    /// 副作用：图集将在下一帧整张重传；**重排需要调用方做**
    /// （cell 尺寸变了 → `mark_layout_dirty` + 重新 `layout`）。
    pub fn set_font(&mut self, font: &y901_runtime::FontConfig) -> Result<(), String> {
        match self.backend.as_mut() {
            Some(b) => b.set_font(font),
            None => Err("渲染后端尚未就绪".to_string()),
        }
    }

    /// 原始 `HWND`。
    ///
    /// **业务/组件不要用它**——请用 [`Window::handle`] 拿平台无关句柄。
    /// 保留公开仅为同仓库的集成测试（热重建纯 CPU 后端时需要真实 hwnd）。
    #[doc(hidden)]
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 窗口句柄的**平台无关包装**（业务与组件只能拿到这个）。
    pub fn handle(&self) -> y901_runtime::WindowHandle {
        y901_runtime::WindowHandle::new(
            y901_runtime::gfx::PlatformWindowHandle::Win32(self.hwnd),
            self.viewport.0,
            self.viewport.1,
            1.0,
        )
    }

    /// 当前客户区尺寸（像素）；布局与渲染都以它为准。
    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    /// 泵一轮消息。返回 false 表示窗口已退出。
    pub fn pump(&mut self) -> bool {
    // SAFETY：`PeekMessageW` 只在本 GUI 线程的消息队列上操作；`msg` 是栈上的 `MSG`，
    // 由系统写入、对齐正确。违反后果：跨线程抽消息会拿到别的线程队列（本线程永远收不到输入）。
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if msg.message == WM_QUIT {
                    return false;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // 尺寸可能在本轮变化
        self.viewport = *SIZE.lock().unwrap_or_else(|e| e.into_inner());
        true
    }

    /// 空闲等待：**没有消息时阻塞**，避免主循环空转吃满 CPU。
    ///
    /// 输入：`timeout_ms`（毫秒）。期间若有消息到达会提前返回。
    /// 返回：`true` = 等到了输入或超时；`false` = 收到退出信号（WM_QUIT）。
    /// 失败：无（等待失败按超时处理）。
    ///
    /// 为什么不能无限等：PTY 输出走的是 channel 而不是 Windows 消息，
    /// 主循环必须周期性醒来 `pump` 一次，所以这里用**有限超时**。
    pub fn wait_message(&self, timeout_ms: u32) -> bool {
    // SAFETY：`MsgWaitForMultipleObjectsEx` 不接收句柄数组（`null` + count 0），
    // 只等待本线程的消息队列；超时参数是纯数值。
    // 违反后果：跨线程调用会等错队列 —— 表现为“卡住不动”或空转占满 CPU。
        unsafe {
            // 第 1 个参数 0 表示不等待任何句柄，只看消息队列。
            let r = MsgWaitForMultipleObjectsEx(0, std::ptr::null(), timeout_ms, QS_ALLINPUT, 0);
            // WAIT_OBJECT_0..WAIT_OBJECT_0+1 都可能出现；超时是 WAIT_TIMEOUT(258)
            r != 258
        }
    }

    /// 取走本轮平台事件。
    pub fn drain_events(&mut self) -> Vec<RawEvent> {
        std::mem::take(&mut queue())
    }

    /// 把 UI 树画到交换链。
    pub fn render(&mut self, tree: &mut UiTree) {
        self.render_inner(tree, 1);
    }

    /// 渲染一帧，**显式指定 `sync_interval`**（仅供基准测试）。
    ///
    /// 注意：**窗口模式下 DWM 合成器会将帧节奏限制在 ~1/刷新率**，
    /// 因此 `sync = 0` 并不会更快（实测 p50 差仅 18µs）。要测纯提交耗时需独占全屏。
    pub fn render_with_sync(&mut self, tree: &mut UiTree, sync_interval: u32) {
        self.render_inner(tree, sync_interval);
    }

    /// `render` 与 `render_with_sync` 的共同实现。
    fn render_inner(&mut self, tree: &mut UiTree, sync_interval: u32) {
        // **没有脏区就不清屏、不 Present**：保留上一帧。
        //
        // 之前每帧都 begin()+present()：`begin` 清空实例，`tree.render` 在脏区为空时
        // 直接返回，于是 Present 出一个"零实例"的空帧 → 屏幕变黑；
        // 下次输入产生脏区才画一帧，然后又被下一帧冲掉（表现为"闪一下又黑"）。
        if tree.dirty().is_empty() {
            return;
        }
        if let Some(soft) = self.soft.as_mut() {
            // 纯 CPU：先清整张缓冲，再强制整树重绘。
            // 为什么不能只重绘脏区：软缓冲每帧都被清掉，
            // 没被标脏的节点不重画就会出现空洞（这一点与 D3D 的自持 RT 相反）。
            tree.invalidate_all();
            soft.clear();
            tree.render(soft);
            let (w, h) = self.viewport;
            let _ = (w, h);
            gdi_present(self.hwnd, soft.pixels(), soft.width(), soft.height());
            return;
        }
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        // 必须在 tree.render 之前取脏区（render 会清空它）
        let dirty = tree.dirty();
        // 按**列表**传给后端：不相邻的脏区不能合并成包围盒
        // （包围盒会把中间空隙也清掉，而 CPU 侧剪枝已经跳过了那些节点）。
        let rects = plan_dirty_rects(dirty);
        if rects.is_none() && !dirty.is_full() {
            // 脏区碎片化（> MAX_DIRTY_RECTS）：升级为整窗重绘，
            // 并重新标脏以保证所有节点都被画。
            tree.invalidate_all();
        }
        backend.set_dirty_bounds(rects);
        backend.begin();
        tree.render(backend);
        let (w, h) = self.viewport;
    // SAFETY：`backend` 与 `self.hwnd` 都来自同一个 `Window`，`w`/`h` 取自当前客户区；
    // `present` 内部会重新校验设备状态，失败时返回 HRESULT 而不是 UB。
    // 违反后果：若在窗口已销毁后 present，交换链仍持有失效的 HWND —— 表现为黑屏或 Present 失败。
        unsafe {
            let _ = backend.present(self.hwnd, w, h, sync_interval);
        }
    }

    /// 回读当前后备缓冲（BGRA），供像素数学验证使用。
    ///
    /// 返回：`(pixels, width, height)`；未初始化或回读失败时 `None`。
    /// 说明：必须在 `render` 之后调用（且下一帧 `render` 之前）。
    pub fn read_back_pixels(&self) -> Option<(Vec<u8>, u32, u32)> {
        if let Some(soft) = self.soft.as_ref() {
            let (w, h) = (soft.width(), soft.height());
            return Some((soft.pixels().to_vec(), w, h));
        }
        // SAFETY: 已在拥有窗口的线程上，backend 已初始化。
        unsafe { self.backend.as_ref()?.read_back_pixels() }
    }

    /// 请求在下一帧 present 之前快照（回读像素）。
    ///
    /// 为什么不直接 `read_back_pixels`：`FLIP_DISCARD` 在 present 后内容未定义，
    /// 帧后读会得到全 0。
    pub fn request_snapshot(&mut self) {
        if self.soft.is_some() {
            return; // 纯 CPU：帧缓冲一直在内存里，随时可取
        }
        if let Some(b) = self.backend.as_mut() {
            b.request_snapshot();
        }
    }

    /// 取走上次快照的像素（BGRA）。
    pub fn take_snapshot(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        if let Some(soft) = self.soft.as_ref() {
            let (w, h) = (soft.width(), soft.height());
            return Some((soft.pixels().to_vec(), w, h));
        }
        self.backend.as_mut()?.take_snapshot()
    }

    /// 峙区列表的上限：超过就降级为整窗重绘。
///
/// 取一个跨线程唤醒句柄（`Send + Sync`），交给后台线程在"有数据了"时调用。
    ///
    /// 返回：可 clone 的 [`WakeHandle`]。
    /// 用途：主循环空闲时阻塞等待，只有 Windows 消息能唤醒它；
    /// 网络/管道数据到达时必须显式 `wake()`，否则要等到下一次输入。
    pub fn wake_handle(&self) -> WakeHandle {
        WakeHandle { hwnd: self.hwnd }
    }

    /// 取走"系统要求重画"的请求（`WM_PAINT` 置位）。
    ///
    /// 返回：`true` = 窗口被遮挡/最小化后恢复，或 Windows 要求重画，
    /// 调用方应 `tree.invalidate_all()` 触发一次整窗重绘。
    /// 这是**一次性**标志，取走即清零。
    pub fn take_repaint_request(&self) -> bool {
        take_full_redraw()
    }

    /// 重新布局（尺寸变化后调用）。
    pub fn resize(&mut self, tree: &mut UiTree) {
        let (w, h) = self.viewport;
        if let Some(soft) = self.soft.as_mut() {
            let rebuilt = soft.resize(w, h);
            tree.layout(Size::new(w as f32, h as f32), soft);
            if rebuilt {
                tree.invalidate_all();
            }
            return;
        }
        if let Some(backend) = self.backend.as_mut() {
            let target_rebuilt = unsafe {
                // SAFETY: backend 由本窗口创建，hwnd 仍有效；w/h 来自当前客户区。
                backend.resize(w, h)
            };
            tree.layout(Size::new(w as f32, h as f32), backend);
            // **尺寸变了 = 自持 RT 被重建 = 内容全部丢失**。
            // 必须整窗重绘：否则只有脏区被清画，其余保持未初始化
            // → blit 出一片**纯黑**（alpha=0）。这是"代码明明画了，屏幕却黑"的典型来源。
            if target_rebuilt {
                tree.invalidate_all();
            }
        }
    }

    /// 测试专用：把客户区改成指定尺寸并重排（不走 WM_SIZE）。
    #[doc(hidden)]
    pub fn resize_to_for_test(&mut self, tree: &mut UiTree, w: u32, h: u32) {
        self.viewport = (w.max(1), h.max(1));
        self.resize(tree);
    }

    /// 设置系统光标。
    pub fn set_cursor(&self, cursor: Cursor) {
        let id = match cursor {
            Cursor::Text => IDC_IBEAM,
            Cursor::Pointer => IDC_HAND,
            Cursor::ResizeHorizontal => IDC_SIZEWE,
            Cursor::ResizeVertical => IDC_SIZENS,
            Cursor::Grab => IDC_SIZEALL,
            Cursor::Default => IDC_ARROW,
        };
    // SAFETY：`LoadCursorW` 的两个参数分别是 `null` 实例句柄与系统预定义光标 ID，
    // 都不涉及调用方内存；返回的句柄由系统持有，不需释放。
    // 违反后果：无（失败返回空句柄，下面已判空）；但若传自定义 ID 会得到空指针，光标变成默认箭头。
        unsafe {
            let handle = LoadCursorW(std::ptr::null_mut(), id);
            if !handle.is_null() {
                SetCursor(handle);
            }
        }
    }

    /// IME 候选窗定位。
    pub fn set_ime_position(&self, x: f32, y: f32) {
    // SAFETY：`set_composition_position` 只使用 `hwnd` 与两个坐标值，不持有指针；
    // `hwnd` 由本 `Window` 保证在 `self` 存活期间有效。
    // 违反后果：对已销毁的 HWND 设置 IME 位置会静默失败 —— 候选窗口出现在屏幕角落。
        unsafe {
            crate::ime::set_composition_position(self.hwnd, x, y);
        }
    }

    /// 请求关闭窗口（投递退出消息）。为什么不是析构：关闭要经过消息循环，直接销毁会让正在处理的这条消息访问悬垂句柄。
    pub fn close(&self) {
    // SAFETY：`DestroyWindow` 只接受本线程创建的窗口句柄；本 `Window` 独占该句柄，
    // 不会再被第二次销毁（`close` 之后调用方不应继续 `present`/`pump`）。
    // 违反后果：重复销毁会破坏窗口类状态（静默失败）；跨线程销毁会失败并让窗口残留。
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        self.backend = None;
    }
}

#[cfg(test)]
mod key_filter_tests {
    use super::{control_char_for_vk, is_duplicate_control_char};

    /// Enter：WM_KEYDOWN(VK_RETURN) 紧接着的 WM_CHAR(0x0D) 必须被丢弃。
    #[test]
    fn enter_key_does_not_double_fire() {
        assert!(
            is_duplicate_control_char(0x0D, '\r'),
            "Enter 的 WM_CHAR(\\r) 是重复投递，必须丢弃"
        );
    }

    /// Tab：同上（0x09）。
    #[test]
    fn tab_key_does_not_double_fire() {
        assert!(is_duplicate_control_char(0x09, '\t'));
    }

    /// Backspace / Escape 同样只保留 WM_KEYDOWN 那一次。
    #[test]
    fn backspace_and_escape_do_not_double_fire() {
        assert!(is_duplicate_control_char(0x08, '\u{8}'));
        assert!(is_duplicate_control_char(0x1B, '\u{1b}'));
    }

    /// 可打印字符不受影响：字母键的 WM_CHAR 必须照常投递。
    #[test]
    fn printable_char_still_fires() {
        // 'A' = VK 0x41，WM_CHAR 给 'a'
        assert!(
            !is_duplicate_control_char(0x41, 'a'),
            "字母键的 WM_CHAR 不能被丢弃"
        );
        // 没有前置 WM_KEYDOWN 时，任何字符都照常投递
        assert!(!is_duplicate_control_char(0, '\r'));
        // 键码与字符不匹配时不丢弃（例如按 Enter 但 WM_CHAR 是别的字符）
        assert!(!is_duplicate_control_char(0x0D, 'x'));
    }

    /// 映射表只覆盖四个键，其余返回 None。
    #[test]
    fn only_four_control_keys_are_mapped() {
        assert_eq!(control_char_for_vk(0x0D), Some('\r'));
        assert_eq!(control_char_for_vk(0x09), Some('\t'));
        assert_eq!(control_char_for_vk(0x20), None, "空格不是控制键");
        assert_eq!(control_char_for_vk(0x41), None);
    }
}

#[cfg(test)]
mod char_filter_tests {
    use super::CharFilter;

    /// Enter：KEYDOWN(VK_RETURN) 后紧跟的 CHAR('\r') 必须被丢弃，只投递一次。
    #[test]
    fn enter_key_does_not_double_fire() {
        let mut f = CharFilter::default();
        f.on_key_down(0x0D);
        assert_eq!(f.on_char('\r'), None, "Enter 的 WM_CHAR 是重复投递");
    }

    /// Tab：同上（0x09）。终端里按 Tab 应只收到一次。
    #[test]
    fn tab_key_does_not_double_fire() {
        let mut f = CharFilter::default();
        f.on_key_down(0x09);
        assert_eq!(f.on_char('\t'), None);
    }

    /// Backspace / Escape 同样只保留 WM_KEYDOWN 那一次。
    #[test]
    fn backspace_and_escape_do_not_double_fire() {
        let mut f = CharFilter::default();
        f.on_key_down(0x08);
        assert_eq!(f.on_char('\u{8}'), None);

        let mut f = CharFilter::default();
        f.on_key_down(0x1B);
        assert_eq!(f.on_char('\u{1b}'), None);
    }

    /// 可打印字符不受影响。
    #[test]
    fn printable_char_still_fires() {
        let mut f = CharFilter::default();
        f.on_key_down(0x41); // 'A'
        assert_eq!(f.on_char('a'), Some('a'), "字母键的 WM_CHAR 必须投递");

        // 没有前置 KEYDOWN（例如 IME 直接送字符）
        let mut f = CharFilter::default();
        assert_eq!(f.on_char('中'), Some('中'));
    }

    /// 键码只对紧随其后的那一个字符生效（不能影响后续输入）。
    #[test]
    fn key_state_is_consumed_by_exactly_one_char() {
        let mut f = CharFilter::default();
        f.on_key_down(0x0D);
        assert_eq!(f.on_char('\r'), None);
        // 下一个字符与上一次按键无关，必须投递
        assert_eq!(f.on_char('\r'), Some('\r'));
    }

    /// 键码与字符不匹配时不丢弃。
    #[test]
    fn mismatched_key_and_char_still_delivers() {
        let mut f = CharFilter::default();
        f.on_key_down(0x0D); // 用户按的是 Enter
        assert_eq!(f.on_char('x'), Some('x'));
    }
}

#[cfg(test)]
mod wndproc_sequence_tests {
    use super::{handle_wm_msg, CharFilter, RawEvent, WM_CHAR, WM_KEYDOWN};
    use y901_ui::Modifiers;

    /// Enter：KEYDOWN(VK_RETURN) 产出 KeyDown，紧随的 CHAR(0x0D) 被丢掉。
    /// 这是 P0-3 的端到端断言（走与 wndproc 相同的纯函数）。
    #[test]
    fn enter_key_produces_exactly_one_event() {
        let mut f = CharFilter::default();
        let first = handle_wm_msg(WM_KEYDOWN, 0x0D, 0, Modifiers::default(), &mut f);
        let second = handle_wm_msg(WM_CHAR, 0x0D, 0, Modifiers::default(), &mut f);

        assert!(
            matches!(first, Some(RawEvent::KeyDown { .. })),
            "第一次应产出 KeyDown，实际 {first:?}"
        );
        assert!(
            second.is_none(),
            "紧随的 WM_CHAR(\\r) 必须被丢弃，实际 {second:?}"
        );
    }

    /// Tab：同上（0x09）。
    #[test]
    fn tab_key_produces_exactly_one_event() {
        let mut f = CharFilter::default();
        assert!(matches!(
            handle_wm_msg(WM_KEYDOWN, 0x09, 0, Modifiers::default(), &mut f),
            Some(RawEvent::KeyDown { .. })
        ));
        assert!(handle_wm_msg(WM_CHAR, 0x09, 0, Modifiers::default(), &mut f).is_none());
    }

    /// 可打印字符：KeyDown + Char 都投递（组件靠 Char 拿到实际字符）。
    #[test]
    fn printable_key_produces_keydown_and_char() {
        let mut f = CharFilter::default();
        assert!(matches!(
            handle_wm_msg(WM_KEYDOWN, 0x41, 0, Modifiers::default(), &mut f),
            Some(RawEvent::KeyDown { .. })
        ));
        assert!(matches!(
            handle_wm_msg(WM_CHAR, 0x61, 0, Modifiers::default(), &mut f),
            Some(RawEvent::Char { c: 'a' })
        ));
    }

    /// 与键盘无关的消息不产出事件。
    #[test]
    fn unrelated_message_yields_nothing() {
        let mut f = CharFilter::default();
        assert!(handle_wm_msg(0x000F, 0, 0, Modifiers::default(), &mut f).is_none());
        // WM_PAINT
    }

    /// 非法码点（代理区）不产出事件、也不 panic。
    #[test]
    fn invalid_codepoint_is_dropped() {
        let mut f = CharFilter::default();
        assert!(handle_wm_msg(WM_CHAR, 0xD800, 0, Modifiers::default(), &mut f).is_none());
    }
}

#[cfg(test)]
mod dirty_plan_tests {
    use super::MAX_DIRTY_RECTS;
    use y901_ui::{DirtyRegion, Rect};

    /// 脏区碎片超过上限时必须退化成整窗重绘（返回 `None`）。
    #[test]
    fn too_many_dirty_rects_falls_back_to_full() {
        let mut d = DirtyRegion::default();
        for i in 0..(MAX_DIRTY_RECTS + 1) {
            // 彼此不相交、不包含（否则会被合并）
            d.add(Rect::new(
                (i as f32) * 40.0,
                (i as f32) * 40.0,
                10.0,
                10.0,
            ));
        }
        // 上限已前移到 `DirtyRegion::add`：超过 16 块时直接 `full = true`。
        // 这比"先攒着、渲染时再退化"更彻底：攒的过程本身就是 O(n²)。
        assert!(
            d.is_full(),
            "碎片超过上限时应立即标记为整窗"
        );
        assert!(
            super::plan_dirty_rects(&d).is_none(),
            "整窗脏区必须走整窗重绘路径"
        );
    }

    /// 等于上限时仍然走逐块路径（边界条件）。
    #[test]
    fn exactly_at_limit_still_partial() {
        let mut d = DirtyRegion::default();
        for i in 0..MAX_DIRTY_RECTS {
            d.add(Rect::new(
                (i as f32) * 40.0,
                (i as f32) * 40.0,
                10.0,
                10.0,
            ));
        }
        let plan = super::plan_dirty_rects(&d).expect("等于上限时应走逐块");
        assert_eq!(plan.len(), MAX_DIRTY_RECTS);
    }

    /// `full` 状态返回 `None`（整窗）。
    #[test]
    fn full_region_returns_none() {
        let d = DirtyRegion::full();
        assert!(super::plan_dirty_rects(&d).is_none());
    }

    /// 空脏区返回 `None`（调用方会在 `Window::render` 里提前 return）。
    #[test]
    fn empty_region_returns_none() {
        let d = DirtyRegion::default();
        assert!(super::plan_dirty_rects(&d).is_none());
    }
}

// ============================ S1：四原语的 Win32 实现 ============================

impl y901_runtime::EventSource for Window {
    fn poll(&mut self, timeout_ms: u32) -> Vec<y901_runtime::RawEvent> {
        if timeout_ms > 0 {
            let _ = self.wait_message(timeout_ms);
        }
        if !self.pump() {
            return vec![y901_runtime::RawEvent::Close];
        }
        self.drain_events()
    }

    fn waker(&self) -> std::sync::Arc<dyn y901_runtime::Waker> {
        struct W(crate::window::WakeHandle);
        impl y901_runtime::Waker for W {
            fn wake(&self) {
                self.0.wake();
            }
        }
        std::sync::Arc::new(W(self.wake_handle()))
    }
}

impl y901_runtime::Presenter for Window {
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.viewport = (width.max(1), height.max(1));
        if let Some(soft) = self.soft.as_mut() {
            if soft.resize(width, height) {
                // 尺寸变了 = 帧缓冲被重建 → 整窗重绘（模块级函数）
                crate::window::request_full_redraw();
            }
        }
        Ok(())
    }

    fn present(&mut self, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
        let Some(soft) = self.soft.as_mut() else {
            return Err(
                "当前为 GPU（D3D11）后端：Presenter 只接受纯 CPU 像素（请用 --force-soft）"
                    .to_string(),
            );
        };
        let expect = (width as usize) * (height as usize) * 4;
        if pixels.len() != expect {
            return Err(format!(
                "像素长度 {} 与 {width}x{height}x4 = {expect} 不符",
                pixels.len()
            ));
        }
        soft.blit(pixels, width, height);
        gdi_present(self.hwnd, soft.pixels(), soft.width(), soft.height());
        Ok(())
    }
}

/// 用 GDI `StretchDIBits` 把一帧 BGRA 像素贴到窗口客户区。
///
/// 为什么留在这里而不在 `y901-render`：这是 **Windows 的上屏手段**，
/// 渲染层只负责画像素（`SoftBackend::pixels()`），上屏归各平台。
fn gdi_present(hwnd: HWND, pixels: &[u8], width: u32, height: u32) {
    use windows_sys::Win32::Graphics::Gdi::{
        GetDC, ReleaseDC, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    };
    // SAFETY: 在拥有窗口的线程上调用，hwnd 有效；DC 获取后立即释放。
    unsafe {
        let hdc = GetDC(hwnd);
        if hdc.is_null() {
            return;
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            // 负高度 = 从上到下的 DIB（与我们的行主序一致）
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let _ = StretchDIBits(
            hdc,
            0,
            0,
            width as i32,
            height as i32,
            0,
            0,
            width as i32,
            height as i32,
            pixels.as_ptr() as *const core::ffi::c_void,
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        ReleaseDC(hwnd, hdc);
    }
}
