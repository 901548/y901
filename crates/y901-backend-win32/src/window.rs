//! Win32 窗口与消息泵。
//!
//! **纪律**：`wndproc` 只把平台事件翻译成 `RawEvent` 推进队列，
//! 绝不触碰 `UiTree` / `Host`。这样窗口层可以被任何宿主复用，
//! 也让"事件翻译"成为可单测的纯函数（见 `y901_runtime::translate`）。

use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
    LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassExW,
    SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTCLIENT, IDC_ARROW,
    IDC_HAND, IDC_IBEAM, IDC_SIZENS, IDC_SIZEWE, IDC_SIZEALL, MSG, PM_REMOVE, SW_SHOW,
    WM_CHAR, WM_DESTROY, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_QUIT, WM_SETCURSOR, WM_SIZE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

use y901_runtime::{Options, RawEvent};
use y901_ui::{Cursor, Key, Modifiers, MouseButton, Pos, Size, UiTree};

use crate::backend::D3dBackend;
use crate::ime::{handle_ime_message, ImeAction, ImeState};

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
        0x41..=0x5A if unsafe { GetKeyState(0x11) < 0 } => {
            char::from_u32(vk + 0x20).map(Key::Char).unwrap_or(Key::Unknown)
        }
        _ => Key::Unknown,
    }
}

unsafe fn client_pos(hwnd: HWND) -> Pos {
    let mut pt = POINT { x: 0, y: 0 };
    let _ = GetCursorPos(&mut pt);
    let _ = ScreenToClient(hwnd, &mut pt);
    Pos::new(pt.x as f32, pt.y as f32)
}

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
        WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let x = (lparam as u32 & 0xFFFF) as i16 as f32;
            let y = ((lparam as u32 >> 16) & 0xFFFF) as i16 as f32;
            let button = MouseButton::Left;
            if msg == WM_LBUTTONDOWN {
                push(RawEvent::MouseDown { x, y, button });
            } else {
                push(RawEvent::MouseUp { x, y, button });
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as u16 as i16) as f32;
            let pos = client_pos(hwnd);
            push(RawEvent::Wheel {
                x: pos.x,
                y: pos.y,
                delta,
            });
            0
        }
        WM_KEYDOWN => {
            push(RawEvent::KeyDown {
                key: vk_to_key(wparam as u32),
                mods: modifiers(),
            });
            0
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam as u32) {
                push(RawEvent::Char { c });
            }
            0
        }
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
    viewport: (u32, u32),
}

impl Window {
    pub fn create(options: &Options) -> Result<Self, String> {
        unsafe { Self::create_inner(options) }
    }

    unsafe fn create_inner(options: &Options) -> Result<Self, String> {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

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
            options.width as i32,
            options.height as i32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err("CreateWindowExW failed".into());
        }

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

        let backend = D3dBackend::new(hwnd, viewport.0, viewport.1)
            .map_err(|e| format!("D3D11 init failed: {e}"))?;

        ShowWindow(hwnd, SW_SHOW);

        Ok(Self {
            hwnd,
            backend: Some(backend),
            viewport,
        })
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    /// 泵一轮消息。返回 false 表示窗口已退出。
    pub fn pump(&mut self) -> bool {
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

    /// 取走本轮平台事件。
    pub fn drain_events(&mut self) -> Vec<RawEvent> {
        std::mem::take(&mut queue())
    }

    /// 把 UI 树画到交换链。
    pub fn render(&mut self, tree: &mut UiTree) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        backend.begin();
        tree.render(backend);
        let (w, h) = self.viewport;
        unsafe {
            let _ = backend.present(self.hwnd, w, h);
        }
    }

    /// 重新布局（尺寸变化后调用）。
    pub fn resize(&mut self, tree: &mut UiTree) {
        let (w, h) = self.viewport;
        if let Some(backend) = self.backend.as_mut() {
            unsafe {
                let _ = backend.resize(w, h);
            }
            tree.layout(Size::new(w as f32, h as f32), backend);
        }
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
        unsafe {
            let handle = LoadCursorW(std::ptr::null_mut(), id);
            if !handle.is_null() {
                SetCursor(handle);
            }
        }
    }

    /// IME 候选窗定位。
    pub fn set_ime_position(&self, x: f32, y: f32) {
        unsafe {
            crate::ime::set_composition_position(self.hwnd, x, y);
        }
    }

    pub fn close(&self) {
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

/// 让窗口可见（`SetWindowPos` 的常见用法）。
pub fn show(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            0x0001 | 0x0002,
        );
    }
}
