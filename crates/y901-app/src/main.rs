//! Y901 应用壳（终局形态）。
//!
//! 三层在这里汇合：
//!   `y901-ui`      布局 / 事件路由 / 语义树 / RPC
//!   `y901-widgets` 终端组件
//!   本 crate        Win32 窗口 + D3D11 后端（`RenderCtx` + `TextMetrics`）
//!
//! 渲染完全由框架驱动：`tree.render(&mut backend)` 走一遍树，
//! 后端把 `fill_rect` / `draw_text` 累积成实例数组，**一次 DrawInstanced** 提交。

mod backend;
mod ime;

use std::any::Any;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, UpdateWindow, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, PeekMessageW,
    SetCursor,
    GetWindowLongPtrW, LoadCursorW, PostQuitMessage, RegisterClassExW,
    SetWindowLongPtrW, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, IDC_ARROW, MSG, SW_SHOW, WM_CHAR, WM_DESTROY,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_SIZE,
    WNDCLASSEXW, WS_OVERLAPPEDWINDOW, PM_REMOVE, WM_QUIT, IDC_HAND, IDC_IBEAM, IDC_SIZENS, IDC_SIZEWE, IDC_SIZEALL,
};

use y901_ui::{
    Color, Command, Edges, Flex, FlexItem, Json, Key, Modifiers, MouseButton, Pending, Pos,
    RpcRequest,
    Size, UiEvent, UiTree,
};
use y901_widgets::{Button, TerminalWidget, TextLabel, VerticalList};

use backend::D3dBackend;
use ime::{ImeAction, ImeState};

/// 打包 HWND：`NonNull` 天然 `!Send + !Sync`。
#[derive(Clone, Copy, PartialEq, Eq)]
struct Hwnd(std::ptr::NonNull<c_void>);

impl Hwnd {
    fn from_raw(hwnd: HWND) -> Option<Self> {
        std::ptr::NonNull::new(hwnd).map(Self)
    }
    fn as_raw(self) -> HWND {
        self.0.as_ptr()
    }
}

struct App {
    hwnd: Option<Hwnd>,
    tree: UiTree,
    /// 当前 Demo 编号：1 = 终端，2 = 组件联动。
    demo: u8,
    /// 窗口创建后才有 HWND，后端只能那时初始化。
    backend: Option<D3dBackend>,
    rpc_rx: Receiver<Pending>,
    broadcaster: y901_ui::rpc::Broadcaster,
    ime: ImeState,
    viewport: (u32, u32),
}

/// panic 不能跨 FFI。
static PANIC_PAYLOAD: Mutex<Option<Box<dyn Any + Send + 'static>>> = Mutex::new(None);
static STATE_TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

// ============================ Demo 组件树 ============================
//
// 注意：这里**只有应用层**知道"终端""清屏""切换 Demo"这些业务概念。
// 框架（y901-ui）一个都不认识，组件（y901-widgets）也互不认识。

/// Demo 1：标题栏（两个按钮）+ 终端。
fn build_demo1() -> UiTree {
    let mut terminal = TerminalWidget::new("main_terminal", 120, 32).flex_item(FlexItem::fill());
    match terminal.attach_shell() {
        Ok(()) => println!("[y901] shell attached"),
        Err(err) => eprintln!("[y901] shell attach failed: {err}"),
    }

    let titlebar = Flex::row("titlebar")
        .background(Color::from_rgb8(24, 26, 32))
        .padding(Edges::symmetric(10.0, 6.0))
        .gap(8.0)
        .flex_item(FlexItem::height(34.0))
        // 按钮只发命令，不认识终端
        .push(Box::new(
            Button::new("btn_clear", "Clear")
                .on_click("terminal.clear", Json::obj().set("target", "main_terminal"))
                .flex_item(FlexItem::width(84.0)),
        ))
        .push(Box::new(
            Button::new("btn_demo2", "Demo 2 →")
                .on_click("demo.switch", Json::obj().set("to", 2))
                .flex_item(FlexItem::width(110.0)),
        ));

    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .push(Box::new(titlebar))
        .push(Box::new(terminal));

    let mut tree = UiTree::new(Box::new(root));
    tree.set_focus(Some("main_terminal".to_string()));
    tree
}

/// Demo 2：纯组件演示，展示「按钮 → 命令 → 应用层 → 标签」的联动。
fn build_demo2() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(14, 14, 20))
        .padding(Edges::all(20.0))
        .gap(14.0)
        .push(Box::new(
            TextLabel::new("title", "Demo 2 · Framework Composition")
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(24.0)),
        ))
        .push(Box::new(
            TextLabel::new("hint", "按钮不知道谁会响应，它只发一条命令。")
                .color(Color::rgba(0.6, 0.66, 0.8, 1.0))
                .flex_item(FlexItem::height(22.0)),
        ))
        .push(Box::new(
            Button::new("btn_hello", "Say Hi")
                .on_click("label.set", Json::obj().set("target", "status").set("text", "Hi from Button"))
                .flex_item(FlexItem::width(110.0)),
        ))
        .push(Box::new(
            Button::new("btn_back", "← Demo 1")
                .on_click("demo.switch", Json::obj().set("to", 1))
                .flex_item(FlexItem::width(110.0)),
        ))
        .push(Box::new(
            TextLabel::new("status", "(等待命令)")
                .color(Color::rgba(0.5, 0.9, 0.7, 1.0))
                .flex_item(FlexItem::height(24.0)),
        ))
        .push(Box::new(
            VerticalList::new("list")
                .gap(6.0)
                .flex_item(FlexItem::fill())
                .push(Box::new(
                    TextLabel::new("row1", "· VerticalList 第一行")
                        .color(Color::rgba(0.7, 0.75, 0.85, 1.0))
                        .flex_item(FlexItem::height(20.0)),
                ))
                .push(Box::new(
                    TextLabel::new("row2", "· 第二行")
                        .color(Color::rgba(0.7, 0.75, 0.85, 1.0))
                        .flex_item(FlexItem::height(20.0)),
                ))
                .push(Box::new(
                    TextLabel::new("row3", "· 容器自己决定子节点怎么摆")
                        .color(Color::rgba(0.7, 0.75, 0.85, 1.0))
                        .flex_item(FlexItem::height(20.0)),
                )),
        ));

    let mut tree = UiTree::new(Box::new(root));
    tree.set_focus(Some("btn_hello".to_string()));
    tree
}

// ============================ 事件翻译 ============================

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
        _ => Key::Unknown,
    }
}

/// Ctrl+字母：Windows 会把它变成控制字符走 WM_CHAR，组件收不到原字母。
/// 这里在按着 Ctrl 时把 A-Z 直接映射成 `Key::Char(小写)`，
/// 由组件自己决定是"全选"还是"复制"。
fn vk_to_ctrl_char(vk: u32) -> Option<Key> {
    match vk {
        0x41..=0x5A => char::from_u32(vk + 0x20).map(Key::Char),
        _ => None,
    }
}


/// 把框架声明的光标形状映射到系统光标。
unsafe fn apply_cursor(app: &App, pos: Pos) {
    use y901_ui::Cursor;
    let id = match app.tree.cursor_at(pos) {
        Cursor::Text => IDC_IBEAM,
        Cursor::Pointer => IDC_HAND,
        Cursor::ResizeHorizontal => IDC_SIZEWE,
        Cursor::ResizeVertical => IDC_SIZENS,
        Cursor::Grab => IDC_SIZEALL,
        Cursor::Default => IDC_ARROW,
    };
    let handle = LoadCursorW(null_mut(), id);
    if !handle.is_null() {
        SetCursor(handle);
    }
}
// ============================ 渲染 / 布局 ============================

unsafe fn paint(app: &mut App) {
    if app.backend.is_none() {
        return;
    }
    let (w, h) = app.viewport;
    let hwnd = app.hwnd.unwrap().as_raw();

    app.backend.as_mut().unwrap().begin();

    let App { tree, backend, .. } = app;
    let backend = backend.as_mut().unwrap();
    tree.render(backend);

    let keep_going = app.backend.as_mut().unwrap().present(hwnd, w, h);
    if !keep_going {
    }
}

fn relayout(app: &mut App, w: u32, h: u32) {
    app.viewport = (w, h);
    if app.backend.is_none() {
        return;
    }
    unsafe {
        app.backend.as_mut().unwrap().resize(w, h);
    }
    let App { tree, backend, .. } = app;
    let backend = backend.as_ref().unwrap();
    tree.layout(Size::new(w as f32, h as f32), backend);
}

// ============================ RPC ============================

/// 应用层：解释 `Command::App` 的语义。
///
/// **这里是唯一知道"terminal.clear"是什么意思的地方。**
/// 框架只是把命令搬过来，组件只是把命令发出去。
fn process_commands(app: &mut App) {
    let commands = app.tree.drain_commands();
    for cmd in commands {
        let Command::App { name, args } = cmd else {
            continue; // Node 级命令框架已经自己路由完了
        };

        match name.as_str() {
            // 业务语义：清空终端
            "terminal.clear" => {
                let target = args
                    .get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main_terminal")
                    .to_string();
                app.tree.dispatch_to(
                    &target,
                    &UiEvent::Action {
                        name: "clear".to_string(),
                        args: Json::obj(),
                    },
                );
                println!("[app] terminal.clear -> {target}");
            }
            // 业务语义：改标签文字（示例：跨组件联动）
            "label.set" => {
                let target = args
                    .get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                app.tree.dispatch_to(
                    &target,
                    &UiEvent::Action {
                        name: "set_text".to_string(),
                        args: Json::obj().set("text", text),
                    },
                );
                println!("[app] label.set -> {target}");
            }
            // 业务语义：切换 Demo
            "demo.switch" => {
                let to = args.get("to").and_then(|v| v.as_f64()).unwrap_or(1.0) as u8;
                switch_demo(app, to);
            }
            other => {
                println!("[app] unhandled command: {other}");
            }
        }
    }
}

fn switch_demo(app: &mut App, to: u8) {
    if to == app.demo {
        return;
    }
    app.demo = to;
    app.tree = match to {
        2 => build_demo2(),
        _ => build_demo1(),
    };
    let (w, h) = app.viewport;
    relayout(app, w, h);
    println!("[app] switched to demo {to}");
    if let Some(hwnd) = app.hwnd {
        unsafe {
            InvalidateRect(hwnd.as_raw(), null(), 0);
        }
    }
}

fn handle_rpc(app: &mut App, pending: Pending) {
    let result = match &pending.request {
        RpcRequest::GetTree => Ok(app.tree.snapshot_json()),
        RpcRequest::Ping => Ok(Json::obj().set("pong", true)),
        RpcRequest::Dispatch { node, event } => {
            let ev = event.to_ui_event();
            let handled = app.tree.dispatch_to(node, &ev).is_handled();
            process_commands(app);
            if handled {
                Ok(Json::obj().set("ok", true))
            } else {
                Err(format!("node ignored event: {node}"))
            }
        }
        RpcRequest::Invoke {
            node,
            action,
            args,
        } => {
            let handled = app
                .tree
                .dispatch_to(
                    node,
                    &UiEvent::Action {
                        name: action.clone(),
                        args: args.clone(),
                    },
                )
                .is_handled();
            process_commands(app);
            if handled {
                Ok(Json::obj().set("ok", true))
            } else {
                Err(format!("node ignored action: {action} on {node}"))
            }
        }
        RpcRequest::Focus { node } => {
            if app.tree.set_focus(Some(node.clone())) {
                Ok(Json::obj().set("focused", node.clone()))
            } else {
                Err(format!("unknown node: {node}"))
            }
        }
        RpcRequest::Subscribe { node, interval_ms } => {
            let ok = app
                .tree
                .subscribe(node, std::time::Duration::from_millis(*interval_ms));
            if ok {
                Ok(Json::obj().set("subscribed", node.clone()))
            } else {
                Err(format!("unknown node: {node}"))
            }
        }
        RpcRequest::Unsubscribe { node } => {
            app.tree.unsubscribe(node);
            Ok(Json::obj().set("unsubscribed", node.clone()))
        }
        RpcRequest::Unknown { method } => Err(format!("unknown method: {method}")),
    };
    y901_ui::rpc::reply_to(pending, result);
}

/// 空闲工作：RPC 请求、PTY 心跳、按需重绘。
///
/// 不用 SetTimer：WM_TIMER 与 WM_PAINT 都是"队列空时合成"，
/// 重绘一多就互相饿死。这里由应用自己驱动，行为可预测。
unsafe fn idle(app: &mut App) {
    while let Ok(pending) = app.rpc_rx.try_recv() {
        handle_rpc(app, pending);
    }

    let mut dirty = false;
    for _ in 0..8 {
        if app.tree.dispatch(&UiEvent::Tick).is_handled() {
            dirty = true;
        } else {
            break;
        }
    }

    // IME：组件请求把候选窗钉到光标处（TextInput 在组合态发出）
    if let Some(pos) = app.tree.take_pending_ime() {
        if let Some(hwnd) = app.hwnd {
            ime::set_composition_position(hwnd.as_raw(), pos.x, pos.y);
        }
    }

    // 语义订阅：到期且有变化就推给所有连接的 Agent（差分 + 节流在框架里做）
    let events = app.tree.poll_subscriptions(std::time::Instant::now());
    for event in events {
        app.broadcaster.send(&event.to_string());
    }

    if dirty {
        if let Some(hwnd) = app.hwnd {
            InvalidateRect(hwnd.as_raw(), null(), 0);
        }
    }
}

// ============================ 窗口过程 ============================

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = lparam as *const CREATESTRUCTW;
        if create.is_null() {
            return 0;
        }
        let state = (*create).lpCreateParams as *mut App;
        if state.is_null() {
            return 0;
        }
        (*state).hwnd = Hwnd::from_raw(hwnd);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        STATE_TAKEN.store(true, std::sync::atomic::Ordering::SeqCst);
        return 1;
    }

    if msg == WM_NCDESTROY {
        let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        if !state.is_null() {
            drop(Box::from_raw(state));
        }
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }

    match catch_unwind(AssertUnwindSafe(|| handle(&mut *state, msg, wparam, lparam))) {
        Ok(result) => result,
        Err(payload) => {
            *PANIC_PAYLOAD.lock().unwrap() = Some(payload);
            PostQuitMessage(1);
            0
        }
    }
}

unsafe fn handle(app: &mut App, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let hwnd = app.hwnd.unwrap().as_raw();

    match msg {
        WM_SIZE => {
            let w = (lparam as u32) & 0xFFFF;
            let h = ((lparam as u32) >> 16) & 0xFFFF;
            if w > 0 && h > 0 {
                relayout(app, w, h);
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            BeginPaint(hwnd, &mut ps);
            EndPaint(hwnd, &ps);

            paint(app);
            // 不要在这里无条件 InvalidateRect：
            // WM_PAINT 与 WM_TIMER 都是"队列空时合成"，而 WM_PAINT 优先。
            // 持续重绘会把定时器饿死，PTY 数据就再也 pump 不进 Grid。
            0
        }

        WM_CHAR => {
            if app.ime.active {
                return 0;
            }
            let ch = wparam as u32;
            if let Some(c) = char::from_u32(ch) {
                if !c.is_control() {
                    app.tree.dispatch(&UiEvent::Text(c.to_string()));
                    process_commands(app);
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            0
        }

        WM_KEYDOWN => {
            let mods = modifiers();
            let vk = wparam as u32;
            // Ctrl+字母优先：WM_CHAR 会把它们变成控制字符，组件收不到原字母
            let key = if mods.ctrl {
                vk_to_ctrl_char(vk).unwrap_or(Key::Unknown)
            } else {
                vk_to_key(vk)
            };
            if key == Key::Unknown {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let result = app.tree.dispatch(&UiEvent::Key {
                key,
                modifiers: mods,
            });
            process_commands(app);
            if result.needs_redraw() {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let pos = Pos::new(
                (lparam as u32 & 0xFFFF) as f32,
                ((lparam as u32 >> 16) & 0xFFFF) as f32,
            );
            let ev = match msg {
                WM_LBUTTONDOWN => UiEvent::MouseDown {
                    pos,
                    button: MouseButton::Left,
                },
                WM_LBUTTONUP => UiEvent::MouseUp {
                    pos,
                    button: MouseButton::Left,
                },
                _ => UiEvent::MouseMove { pos },
            };
            // 光标形状：分栏分割线 → 左右箭头，输入框 → I 形
            apply_cursor(app, pos);
            let result = app.tree.dispatch(&ev);
            process_commands(app);
            if result.needs_redraw() {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16;
            let pos = Pos::new(
                (lparam as u32 & 0xFFFF) as f32,
                ((lparam as u32 >> 16) & 0xFFFF) as f32,
            );
            let result = app.tree.dispatch(&UiEvent::Wheel {
                delta: delta as f32,
                pos,
            });
            process_commands(app);
            if result.needs_redraw() {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        // ---- IME ----
        ime::WM_IME_STARTCOMPOSITION | ime::WM_IME_COMPOSITION | ime::WM_IME_ENDCOMPOSITION => {
            match ime::handle_ime_message(hwnd, msg, lparam, &mut app.ime) {
                ImeAction::Unhandled => DefWindowProcW(hwnd, msg, wparam, lparam),
                ImeAction::Redraw => {
                    let text = app.ime.composing_text();
                    app.tree.dispatch(&UiEvent::Compose(text));
                    InvalidateRect(hwnd, null(), 0);
                    0
                }
                ImeAction::Commit(text) => {
                    app.tree.dispatch(&UiEvent::Compose(String::new()));
                    app.tree.dispatch(&UiEvent::Text(text));
                    InvalidateRect(hwnd, null(), 0);
                    0
                }
            }
        }

        WM_DESTROY => {
            println!("[y901] 退出");
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ============================ 入口 ============================

fn client_size(hwnd: HWND) -> (u32, u32) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        (
            (rect.right - rect.left).max(1) as u32,
            (rect.bottom - rect.top).max(1) as u32,
        )
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        run()
    }
}

unsafe fn run() -> Result<(), Box<dyn std::error::Error>> {
    let instance = GetModuleHandleW(null());
    if instance.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }

    let class_name = wide("Y901App");
    let window_title = wide("Y901 · Agent-Native UI");

    let window_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: null_mut(),
    };

    if RegisterClassExW(&window_class) == 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    let (tx, rpc_rx) = std::sync::mpsc::channel();
    let (rpc_addr, broadcaster) = y901_ui::rpc::spawn(y901_ui::rpc::DEFAULT_ADDR, tx)?;
    println!("[y901] RPC listening on {rpc_addr}");

    let tree = build_demo1();

    STATE_TAKEN.store(false, std::sync::atomic::Ordering::SeqCst);
    let app = Box::into_raw(Box::new(App {
        hwnd: None,
        tree,
        demo: 1,
        backend: None,
        rpc_rx,
        broadcaster,
        ime: ImeState::default(),
        viewport: (1, 1),
    }));

    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        window_title.as_ptr(),
        WS_OVERLAPPEDWINDOW,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        1200,
        760,
        null_mut(),
        null_mut(),
        instance,
        app as *const c_void,
    );

    if hwnd.is_null() {
        if !STATE_TAKEN.swap(false, std::sync::atomic::Ordering::SeqCst) {
            drop(Box::from_raw(app));
        }
        return Err(std::io::Error::last_os_error().into());
    }

    ShowWindow(hwnd, SW_SHOW);

    let (w, h) = client_size(hwnd);
    (*app).viewport = (w, h);
    match D3dBackend::new(hwnd, w, h) {
        Ok(backend) => (*app).backend = Some(backend),
        Err(err) => {
            eprintln!("[y901] backend init failed: {err}");
            DestroyWindow(hwnd);
            return Err(err.into());
        }
    }

    relayout(&mut *app, w, h);

    InvalidateRect(hwnd, null(), 0);
    UpdateWindow(hwnd);

    println!("[y901] tree:\n{}", (*app).tree.snapshot_json().to_string());

    let mut msg: MSG = std::mem::zeroed();
    'outer: loop {
        // 先抽干消息队列（非阻塞）。
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                break 'outer;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // 队列空闲：RPC + 心跳 + 按需重绘。
        idle(&mut *app);

        // 不忙等：4ms ≈ 250Hz，足够终端流畅，又不烧 CPU。
        std::thread::sleep(std::time::Duration::from_millis(4));
    }

    if let Some(payload) = PANIC_PAYLOAD.lock().unwrap().take() {
        std::panic::resume_unwind(payload);
    }
    Ok(())
}
