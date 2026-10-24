//! Y901 应用壳（终局形态）。
//!
//! 三层在这里汇合：
//!   `y901-ui`      布局 / 事件路由 / 语义树 / RPC
//!   `y901-widgets` 终端组件
//!   本 crate        Win32 窗口 + D3D11 后端（`RenderCtx` + `TextMetrics`）
//!
//! 渲染完全由框架驱动：`tree.render(&mut backend)` 走一遍树，
//! 后端把 `fill_rect` / `draw_text` 累积成实例数组，**一次 DrawInstanced** 提交。




use std::any::Any;
use std::ffi::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr::{null, null_mut};
use std::sync::mpsc::Receiver;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
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
    WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WM_SETCURSOR, HTCLIENT, PM_REMOVE, WM_QUIT, IDC_HAND, IDC_IBEAM, IDC_SIZENS, IDC_SIZEWE, IDC_SIZEALL,
};

use y901_agent::{Pending, RpcRequest};
use y901_ui::{
    Color, Command, Edges, Flex, FlexItem, Json, Key, Modifiers, MouseButton, Pos,
    Size, UiEvent, UiTree,
};
use y901_widgets::TextLabel;

use y901_backend_win32::backend::D3dBackend;
use y901_backend_win32::ime::{ImeAction, ImeState};

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
    /// --mcp 时的协议服务端与请求队列（None 表示未启用）。
    mcp: Option<(y901_agent::McpServer, std::sync::mpsc::Receiver<y901_mcp::bridge::Job>)>,
    hwnd: Option<Hwnd>,
    tree: UiTree,
    /// 当前 Demo 编号：1 = 终端，2 = 组件联动。

    /// 窗口创建后才有 HWND，后端只能那时初始化。
    backend: Option<D3dBackend>,
    rpc_rx: Receiver<Pending>,
    broadcaster: y901_agent::rpc::Broadcaster,
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

/// 最小启动树：一张标签，没有任何业务组件。
///
/// Demo 已移除——框架优先，示例留给用户自己写。
fn build_shell() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(24.0))
        .push(Box::new(
            TextLabel::new("title", "Y901")
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(28.0)),
        ));

    let mut tree = UiTree::new(Box::new(root));
    tree.set_focus(Some("title".to_string()));
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


/// 取当前鼠标位置的客户区坐标（WM_SETCURSOR 不带坐标）。
unsafe fn client_pos(hwnd: HWND) -> Pos {
    let mut pt = POINT { x: 0, y: 0 };
    let _ = GetCursorPos(&mut pt);
    let _ = ScreenToClient(hwnd, &mut pt);
    Pos::new(pt.x as f32, pt.y as f32)
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

fn handle_rpc(app: &mut App, pending: Pending) {
    let result = match &pending.request {
        RpcRequest::GetTree => Ok(app.tree.snapshot_json()),
        RpcRequest::Query { q } => {
            // 与 MCP 的 ui_query 走同一个入口
            let policy = y901_agent::Policy {
                token: String::new(),
                redact: true,
            };
            let params = Json::obj().set("q", q.clone());
            y901_agent::dispatch_request(&mut app.tree, &policy, "query", &params)
                .map_err(|e| e.code().to_string())
        }
        RpcRequest::Ping => Ok(Json::obj().set("pong", true)),
        RpcRequest::Dispatch { node, event } => {
            let ev = event.to_ui_event();
            let handled = app.tree.dispatch_to(node, &ev).is_handled();

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
    y901_agent::rpc::reply_to(pending, result);
}

/// 空闲工作：RPC 请求、PTY 心跳、按需重绘。
///
/// 不用 SetTimer：WM_TIMER 与 WM_PAINT 都是"队列空时合成"，
/// 重绘一多就互相饿死。这里由应用自己驱动，行为可预测。
unsafe fn idle(app: &mut App) {
    while let Ok(pending) = app.rpc_rx.try_recv() {
        handle_rpc(app, pending);
    }

    // MCP：每帧最多 8 条，绝不阻塞渲染
    if let Some((server, jobs)) = app.mcp.as_mut() {
        y901_mcp::bridge::pump(server, &mut app.tree, jobs, 8);
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
            y901_backend_win32::ime::set_composition_position(hwnd.as_raw(), pos.x, pos.y);
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

        WM_SETCURSOR => {
            // lParam 低字是命中区域；只有客户区才由我们接管光标
            let hit = (lparam as u32) & 0xFFFF;
            if hit == HTCLIENT {
                let pos = client_pos(hwnd);
                apply_cursor(app, pos);
                1 // TRUE：阻止系统把光标重置成箭头
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
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

            if result.needs_redraw() {
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }

        // ---- IME ----
        y901_backend_win32::ime::WM_IME_STARTCOMPOSITION | y901_backend_win32::ime::WM_IME_COMPOSITION | y901_backend_win32::ime::WM_IME_ENDCOMPOSITION => {
            match y901_backend_win32::ime::handle_ime_message(hwnd, msg, lparam, &mut app.ime) {
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
            eprintln!("[y901] 退出");
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_mcp = args.iter().any(|a| a == "--mcp");
    let want_rpc = args.iter().any(|a| a == "--rpc");
    let headless = args.iter().any(|a| a == "--headless");

    // 无窗口 MCP：不建窗口、不进消息循环，直接跑协议（供 Claude Desktop / 测试）
    if want_mcp && headless {
        return run_mcp_headless();
    }

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        run(want_mcp, want_rpc)
    }
}

/// 无窗口环境的等宽度量（headless 也要能算出真实 rect）。
struct HeadlessMetrics;

impl y901_ui::TextMetrics for HeadlessMetrics {
    fn measure_text(&self, text: &str, _s: &y901_ui::TextStyle) -> y901_ui::Size {
        let mut w = 0.0;
        for ch in text.chars() {
            w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
        }
        y901_ui::Size::new(w, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// `y901-app --mcp --headless`：纯协议模式，stdout 只留给 MCP。
fn run_mcp_headless() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("Y901_TOKEN").unwrap_or_default();
    let policy = y901_agent::Policy { token, redact: true };

    let mut tree = build_shell();
    // 无窗口也要先布局，否则所有 rect 为 0，语义 API 无法定位
    tree.layout(y901_ui::Size::new(1200.0, 800.0), &HeadlessMetrics);

    let mut server = y901_agent::McpServer::new(policy);
    let jobs = y901_mcp::bridge::spawn_io(
        std::io::BufReader::new(std::io::stdin()),
        std::io::stdout(),
    );
    y901_mcp::bridge::run_blocking(&mut server, &mut tree, &jobs);
    Ok(())
}

unsafe fn run(want_mcp: bool, want_rpc: bool) -> Result<(), Box<dyn std::error::Error>> {
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

    // 裸奔 TCP 是漏洞：默认关闭，只有显式 --rpc 且配置了 Token 才开
    let (tx, rpc_rx) = std::sync::mpsc::channel();
    let (broadcaster, _rpc_addr) = if want_rpc {
        let token = std::env::var("Y901_TOKEN").unwrap_or_default();
        if token.is_empty() {
            eprintln!("[y901] --rpc 需要 Y901_TOKEN，已拒绝启动 TCP 通道");
            (y901_agent::Broadcaster::new(16).0, None)
        } else {
            let (addr, hub) = y901_agent::rpc::spawn(y901_agent::rpc::DEFAULT_ADDR, tx)?;
            eprintln!("[y901] RPC listening on {addr} (token required)");
            (hub, Some(addr))
        }
    } else {
        (y901_agent::Broadcaster::new(16).0, None)
    };

    let tree = build_shell();

    // --mcp：把正在运行的这棵树挂到 stdio 上
    let mcp = if want_mcp {
        let token = std::env::var("Y901_TOKEN").unwrap_or_default();
        let policy = y901_agent::Policy { token, redact: true };
        let jobs = y901_mcp::bridge::spawn_io(
            std::io::BufReader::new(std::io::stdin()),
            std::io::stdout(),
        );
        eprintln!("[y901] MCP bridge attached (stdio)");
        Some((y901_agent::McpServer::new(policy), jobs))
    } else {
        None
    };

    STATE_TAKEN.store(false, std::sync::atomic::Ordering::SeqCst);
    let app = Box::into_raw(Box::new(App {
        hwnd: None,
        tree,

        backend: None,
        rpc_rx,
        broadcaster,
        mcp,
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

    eprintln!("[y901] tree:\n{}", (*app).tree.snapshot_json().to_string());

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
