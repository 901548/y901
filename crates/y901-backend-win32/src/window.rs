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
    SetCursor, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTCLIENT, IDC_ARROW,
    IDC_HAND, IDC_IBEAM, IDC_SIZENS, IDC_SIZEWE, IDC_SIZEALL, MSG, PM_REMOVE, SW_SHOW,
    WM_CHAR, WM_DESTROY, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NCDESTROY, WM_PAINT, WM_QUIT, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SETCURSOR, WM_SIZE, WM_SYSKEYDOWN, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
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
        0x41..=0x5A => char::from_u32(vk + 0x20).map(Key::Char).unwrap_or(Key::Unknown),
        0x30..=0x39 => char::from_u32(vk).map(Key::Char).unwrap_or(Key::Unknown),
        _ => vk_to_key(vk),
    }
}

unsafe fn client_pos(hwnd: HWND) -> Pos {
    let mut pt = POINT { x: 0, y: 0 };
    let _ = GetCursorPos(&mut pt);
    let _ = ScreenToClient(hwnd, &mut pt);
    Pos::new(pt.x as f32, pt.y as f32)
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
        WM_KEYUP => Some(RawEvent::KeyUp {
            key: vk_to_key(wparam as u32),
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
    RawEvent::Wheel {
        x,
        y,
        delta,
        mods,
    }
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
                if let Some(ev) = handle_wm_msg(msg, wparam as u64, lparam as i64, modifiers(), filter) {
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
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_CHAR => {
            // 键盘消息统一走纯函数 `handle_wm_msg`（可单测）：它负责去重，
            // wndproc 只做"取状态 + 投递"。
            if let Ok(mut slot) = CHAR_FILTER.lock() {
                let filter = slot.get_or_insert_with(CharFilter::default);
                if let Some(ev) = handle_wm_msg(msg, wparam as u64, lparam as i64, modifiers(), filter) {
                    push(ev);
                }
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

        let theme = options.theme.unwrap_or(y901_ui::theme::Theme::DARK);
        let font = options.font.clone().unwrap_or_default();
        let backend = D3dBackend::new(hwnd, viewport.0, viewport.1, theme, &font)
            .map_err(|e| format!("D3D11 init failed: {e}"))?;

        ShowWindow(hwnd, SW_SHOW);

        Ok(Self {
            hwnd,
            backend: Some(backend),
            viewport,
        })
    }

    /// 运行时切换主题（转发到后端）。
    ///
    /// 返回：`Ok(())` 已切换；`Err` 表示后端尚未就绪。
    /// 副作用：主题字段更新。**全量重绘需要调用方做**
    /// （`UiTree::invalidate_all`）——窗口不认识 UI 树。
    pub fn set_theme(&mut self, theme: y901_ui::theme::Theme) -> Result<(), String> {
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
                // SAFETY: backend 由本窗口创建，hwnd 仍有效；w/h 来自当前客户区。
                backend.resize(w, h);
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
        assert!(handle_wm_msg(0x000F, 0, 0, Modifiers::default(), &mut f).is_none()); // WM_PAINT
    }

    /// 非法码点（代理区）不产出事件、也不 panic。
    #[test]
    fn invalid_codepoint_is_dropped() {
        let mut f = CharFilter::default();
        assert!(handle_wm_msg(WM_CHAR, 0xD800, 0, Modifiers::default(), &mut f).is_none());
    }
}