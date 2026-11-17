//! 一个可用的 X11 窗口：把 [`Connection`] 包成 `EventSource` / `Presenter`。
//!
//! 它是 S3 “接入框架”的一半：事件与像素呈现都对外就只剩
//! `y901_runtime` 的两个 trait。另一半（`PlatformRuntime`）还需要一个**平台无关的软渲染器**，
//! 而它目前还在 `y901-backend-win32` 里（见报告的阻塞项）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_render::soft::SoftBackend;
use y901_runtime::{EventSource, FontConfig, Presenter, RawEvent, Waker};
use y901_ui::theme::Theme;
use y901_ui::{Key, Modifiers, MouseButton, Size, UiTree};

use crate::conn::Connection;
use crate::encode::{
    encode_create_gc, encode_create_window, encode_map_window, CreateWindowArgs, GcMask, ValueMask,
    WindowClass,
};

use crate::keys::{keysym_to_key, modifiers_from_state, Keymap};
use crate::X11Event;

/// 一个已创建并显示的 X11 窗口。
pub struct X11Window {
    conn: Connection,
    /// 窗口 ID
    pub wid: u32,
    /// GC ID
    gc: u32,
    width: u16,
    height: u16,
    depth: u8,
    keymap: Keymap,
    /// 被 `wake()` 置位的标志（`poll` 的短轮询会检查它）
    woken: Arc<AtomicBool>,
    /// MIT-SHM：有则走共享内存，没有则回退到分块 PutImage
    shm: Option<ShmState>,
    /// 三种光标形状（`None` = 服务器没有 cursor 字体，保持默认）
    cursors: Option<Cursors>,
    /// 当前光标形状（避免每帧重发请求）
    cursor_now: CursorShape,
    /// CPU 光栅器：清屏 + 整树绘制 + 上屏三段都在 [`X11Window::render_tree`] 里。
    soft: SoftBackend,
    /// 上屏用的像素暂存（`Present` 要 `&mut self`，不能同时借用 `soft`）。
    frame: Vec<u8>,
    /// 当前是否处于 fullscreen（状态机，避免重复发 EWMH）。
    fullscreen: bool,
    /// **输入法客户端**（IBus 或降级的 `NoIme`）。
    ///
    /// M1 阶段只持有连接（保证上下文不会随 D-Bus 连接销毁），
    /// 按键转发在 M2 接入 `wait_events`。
    ime: Box<dyn crate::ime::ImeClient>,
    /// 是否已经向 IME 声明过焦点（避免每个按键都发 `FocusIn`）。
    ime_focused: bool,
    /// 全屏下按 Esc：`translate`（`&self`）置位，`wait_events`（`&mut self`）消费。
    ///
    /// **为何不用进程级 `static`**：`static` 它会让两个窗口共享同一个标志 ——
    /// A 窗口的 Esc 可能被 B 窗口的 `wait_events` 消费掉，A 反而退不出全屏。
    want_exit_fs: std::cell::Cell<bool>,
}

/// 对外可用的光标形状（与 `y901_ui::Cursor` 一一对应）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// 箭头（XC_left_ptr = 68）
    Default,
    /// I 型（XC_xterm = 152）—— 文本输入区
    Text,
    /// 手型（XC_hand2 = 60）—— 可点击元素
    Pointer,
}

/// 预先创建好的三个光标资源。
struct Cursors {
    /// cursor 字体 ID。
    ///
    /// 为什么暂不 CloseFont：字体在**连接关闭时由服务端自动回收**，
    /// 而光标在窗口关闭前一直要用它（提前关会让光标变成 X 默认）。
    /// 保留字段供将来"字体切换"时复用。
    // 诊断 API：字体 id 由 `XSetFont` 设置后保留，供 `TextItem` 排字使用
    #[allow(dead_code)]
    font: u32,
    /// 三个光标 ID，顺序：Default / Text / Pointer
    ids: [u32; 3],
}

/// 共享内存上屏所需的全部状态。
struct ShmState {
    /// 扩展主 opcode（服务器分配）
    major: u8,
    /// 客户端自己分配的 shmseg ID
    seg_id: u32,
    /// 内存段（Drop 时自动 shmdt + shmctl）
    seg: crate::shm::ShmSegment,
    /// 段对应的像素尺寸（不匹配时需要重建）
    w: u16,
    h: u16,
}

/// [`X11Window::create`] 的参数。
///
/// **为何用结构体而非位置参数**：加字段时所有调用点都会**编译失败**，
/// 不会漏改。之前用 6 个位置参数时，加 `fullscreen` 就漏改了
/// `tests/real_window.rs` 的一个调用点（而它在 `#[cfg(unix)]` 下，
/// Windows 编译机看不到）。
///
/// 生命周期：`display` 与 `title` 是借用的字符串，其余均为值（`soft` 为所有权）。
pub struct CreateArgs<'a> {
    /// 上屏用的 X display（如 `":99"`；空字符串 = 读 `DISPLAY`）。
    pub display: &'a str,
    /// 窗口宽度（像素）。
    pub width: u16,
    /// 窗口高度（像素）。
    pub height: u16,
    /// 窗口标题。
    pub title: &'a str,
    /// 光栅器（由调用方构造；字体/主题是应用级选择）。
    pub soft: SoftBackend,
    /// 是否以 borderless fullscreen 创建。
    pub fullscreen: bool,
}

impl X11Window {
    /// 连接、建窗、建 GC、显示。
    ///
    /// `soft` 由调用方准备好（字体/主题是应用级选择，窗口不替它决定）。
    /// 失败：任一步服务器报错都会把错误原文带回来（而不是吞掉）。
    /// # 调用点（改动签名时同步更新）
    ///
    /// - `crates/y901-backend-x11/src/runtime.rs`
    /// - `crates/y901-backend-x11/tests/real_window.rs`
    ///
    /// 为何写在这里：本函数在 `#[cfg(unix)]` 下，Windows 编译机**看不到**调用点，
    /// 改签名时极容易漏改测试里的那一处（已发生一次）。
    pub fn create(args: CreateArgs<'_>) -> Result<Self, String> {
        let CreateArgs {
            display,
            width,
            height,
            title,
            soft,
            fullscreen,
        } = args;
        let mut conn = Connection::connect(display)?;
        let info = conn.info().clone();

        // 标题只用 ASCII 最简形式：WM_NAME + UTF8_STRING（中文标题需要 _NET_WM_NAME，本轮不做）
        let name_atom = intern(&mut conn, b"WM_NAME")?;
        let utf8_atom = intern(&mut conn, b"UTF8_STRING")?;
        let base = info.resource_id_base;
        let wid = base | 0x01;
        let gc = base | 0x02;

        const VALUES: &[(ValueMask, u32)] = &[
            (ValueMask::BackPixel, 0x0000_0000),
            // ExposureMask(1<<15=0x8000) | KeyPress(1) | KeyRelease(2) | ButtonPress(4)
            // | ButtonRelease(8) | PointerMotion(64) | StructureNotify(1<<17=0x20000)
            (ValueMask::EventMask, 0x0002_804F),
        ];
        conn.send(&encode_create_window(&CreateWindowArgs {
            wid,
            parent: info.root,
            x: 0,
            y: 0,
            width,
            height,
            border_width: 0,
            class: WindowClass::InputOutput,
            visual: info.root_visual,
            values: VALUES,
        }))?;
        conn.send(&encode_change_title(wid, name_atom, utf8_atom, title))?;
        // **注册 `WM_PROTOCOLS = [WM_DELETE_WINDOW]`**。
        //
        // 为何必需：不注册时，用户点窗口的 × 之后，WM（如 metacity）
        // 不会发 `WM_DELETE_WINDOW` ClientMessage，而是直接 **`XKillClient`** 把客户端杀掉 ——
        // 于是我们只能从 socket EOF 得知，走"兜底退出"而不是干净路径。
        // 注册之后才是 WM 发 ClientMessage → `RawEvent::Close`（见 `map_event` 的 ClientMessage 分支）。
        //
        // **必须在 `MapWindow` 之前设置**：WM 在管理（reparent）窗口时就会读取 `WM_PROTOCOLS`，
        // 映射后才设就晚了 —— 那时 WM 仍会走 `XKillClient`。
        {
            let wm_protocols = intern(&mut conn, b"WM_PROTOCOLS")?;
            let wm_delete_window = intern(&mut conn, b"WM_DELETE_WINDOW")?;
            // 预定义原子 `ATOM = 4`（X11 协议附录 B）：`WM_PROTOCOLS` 的类型就是 ATOM 数组。
            const ATOM_ATOM: u32 = 4;
            conn.send(&crate::encode_change_property(
                wid,
                wm_protocols,
                ATOM_ATOM,
                32,
                crate::encode::PropMode::Replace,
                &wm_delete_window.to_le_bytes(),
            ))?;
            conn.flush()?;
        }

        conn.send(&encode_map_window(wid))?;

        // SPIKE: `Y901_FULLSCREEN=1` 时走 EWMH 全屏。
        // 必须在 Map 之后发：窗口还未映射时部分 WM 会忽略该消息。
        let fullscreen = fullscreen || std::env::var_os("Y901_FULLSCREEN").is_some();
        if fullscreen {
            // best-effort：全屏请求失败（无 WM、属性不受支持、协议序列异常）
            // 不应阻止窗口启动 —— 用户至少能看到一个普通窗口。
            if let Err(e) = fullscreen_atoms(&mut conn, wid, info.root, true) {
                eprintln!("[y901] EWMH fullscreen 请求失败（无 WM 或协议时序问题）：{e}");
            }
        }
        conn.flush()?;
        let errs = conn.sync()?;
        if !errs.is_empty() {
            return Err(format!("建窗失败：{errs:?}"));
        }

        // GC：GXcopy + plane-mask 全 1 + 关掉 GraphicsExpose
        const GC_VALUES: &[(GcMask, u32)] = &[
            (GcMask::Function, 3),
            (GcMask::PlaneMask, 0xFFFF_FFFF),
            (GcMask::GraphicsExposures, 0),
        ];
        conn.send(&encode_create_gc(gc, wid, GC_VALUES))?;
        conn.flush()?;
        let errs = conn.sync()?;
        if !errs.is_empty() {
            return Err(format!("CreateGC 失败：{errs:?}"));
        }

        // 键盘映射表（一次拿全，后面就不用再问服务器）
        let count = info
            .max_keycode
            .saturating_sub(info.min_keycode)
            .saturating_add(1);
        let keymap = conn.get_keyboard_mapping(info.min_keycode, count)?;

        // MIT-SHM 探测：支持就用，不支持（或失败）静默回退
        let shm = match try_enable_shm(&mut conn, base | 0x10, width, height, info.root_depth) {
            Ok(Some(state)) => {
                eprintln!(
                    "[y901] MIT-SHM 已启用（{}x{}，共享内存 {}字节）",
                    width,
                    height,
                    state.seg.size()
                );
                Some(state)
            }
            Ok(None) => {
                eprintln!("[y901] 服务器不支持 MIT-SHM，回退到分块 PutImage");
                None
            }
            Err(e) => {
                eprintln!("[y901] MIT-SHM 启用失败（{e}），回退到分块 PutImage");
                None
            }
        };

        // 光标：用服务器内置 "cursor" 字体的字形创建（零图片资源）
        let cursors = match create_cursors(&mut conn, base) {
            Ok(c) => {
                eprintln!("[y901] 光标形状已就绪：箭头 / I 型 / 手型");
                Some(c)
            }
            Err(e) => {
                eprintln!("[y901] 光标创建失败（{e}），使用默认箭头");
                None
            }
        };

        Ok(Self {
            conn,
            wid,
            gc,
            width,
            height,
            depth: info.root_depth,
            keymap,
            woken: Arc::new(AtomicBool::new(false)),
            shm,
            cursors,
            cursor_now: CursorShape::Default,
            soft,
            frame: Vec::new(),
            fullscreen,
            want_exit_fs: std::cell::Cell::new(false),
            ime: Box::new(crate::ime::NoIme),
            ime_focused: false,
        })
    }

    /// 光栅器尺寸与窗口客户区对齐（启动后由 [`WindowBackend::relayout`] 调用）。
    /// **装上输入法客户端**（IBus 连接成功时由 runtime 调用）。
    ///
    /// 传 `NoIme` 即关闭 IME，与没有 IME 的行为完全一致。
    pub fn set_ime(&mut self, ime: Box<dyn crate::ime::ImeClient>) {
        self.ime = ime;
        self.ime_focused = false;
    }

    /// 当前输入法客户端是否真的可用。
    pub fn ime_available(&self) -> bool {
        self.ime.available()
    }

    /// 只读光栅器（命令行工具 / 测试用）。
    pub fn soft(&self) -> &SoftBackend {
        &self.soft
    }

    /// 可变光栅器（启动时按真实字体/主题建树后 `tree.layout` 需要它）。
    pub fn soft_mut(&mut self) -> &mut SoftBackend {
        &mut self.soft
    }

    /// 渲染一帧并提交 —— 先清屏，再画树，最后推送。
    ///
    /// 为何还要自己查**脏区**：`run_loop` 不再查 `tree.dirty()`，
    /// 它只看返回的 `RenderOutcome`；而"该不该跳过"的判定留在这个函数里。
    /// 两条渲染路径都必须有这个守卫，否则空帧也会推一帧 PutImage。
    pub fn render_tree(&mut self, tree: &mut UiTree) -> Result<(), String> {
        if tree.dirty().is_empty() {
            return Ok(());
        }
        // 纯 CPU 路径：整窗重绘（与 Windows 的 --force-soft 一致）
        tree.invalidate_all();
        self.soft.clear();
        tree.render(&mut self.soft);
        // Present 需要 `&mut self`，而像素在 `self.soft` 里：先拷到暂存再上屏。
        let (w, h) = (self.soft.width(), self.soft.height());
        self.frame.clear();
        self.frame.extend_from_slice(self.soft.pixels());
        let frame = std::mem::take(&mut self.frame);
        let result = self.present(&frame, w, h);
        self.frame = frame;
        result
    }

    /// 窗口尺寸（逻辑像素）。
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// 键盘映射表（验收用）。
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// 把 X11 事件翻译成平台无关事件；不关心的返回 `None`。
    fn map_event(&self, ev: &X11Event) -> Option<RawEvent> {
        Some(match ev {
            // Expose 只告诉你"哪块需要重绘"，不是尺寸变化。
            // 对外没有"局部重绘"事件，所以这里发一个当前窗口尺寸的 Resize 作为"该画一帧"的信号。
            X11Event::Expose { .. } => RawEvent::Resize {
                w: self.width as u32,
                h: self.height as u32,
            },
            X11Event::ConfigureNotify { width, height, .. } => RawEvent::Resize {
                w: *width as u32,
                h: *height as u32,
            },
            X11Event::Key {
                pressed,
                keycode,
                state,
                ..
            } => {
                let (plain, shifted) = self.keymap.plain_and_shift(*keycode);
                let shift = state & 1 != 0;
                let ks = if shift { shifted.or(plain) } else { plain }?;
                let key = keysym_to_key(ks)?;
                if *pressed {
                    // borderless fullscreen 没有标题栏与关闭按钮，必须给用户一条出路。
                    // 与 Win32 一致：**只在全屏下**生效；普通窗口的 Esc 仍作为普通按键上报。
                    if self.fullscreen && key == Key::Escape {
                        // 全屏下 Esc = **退出全屏**（标题栏由 WM 恢复），不再关窗。
                        // `translate` 是 `&self`，所以用全局标志，由 `wait_events` 消费。
                        self.want_exit_fs.set(true);
                        return None;
                    }
                    RawEvent::KeyDown {
                        key,
                        mods: modifiers_from_state(*state),
                    }
                } else {
                    // 对外只需 KeyDown；释放事件目前不用
                    return None;
                }
            }
            X11Event::Button {
                pressed,
                button,
                event_x,
                event_y,
                state,
                ..
            } => {
                let (x, y) = (*event_x as f32, *event_y as f32);
                match (*button, *pressed) {
                    (4, true) | (5, true) => RawEvent::Wheel {
                        x,
                        y,
                        delta: if *button == 4 { 1.0 } else { -1.0 },
                        mods: modifiers_from_state(*state),
                    },
                    (b, true) => RawEvent::MouseDown {
                        x,
                        y,
                        button: match b {
                            1 => MouseButton::Left,
                            2 => MouseButton::Middle,
                            3 => MouseButton::Right,
                            _ => MouseButton::Left,
                        },
                    },
                    (b, false) => RawEvent::MouseUp {
                        x,
                        y,
                        button: match b {
                            1 => MouseButton::Left,
                            2 => MouseButton::Middle,
                            3 => MouseButton::Right,
                            _ => MouseButton::Left,
                        },
                    },
                }
            }
            X11Event::Motion {
                event_x, event_y, ..
            } => RawEvent::MouseMove {
                x: *event_x as f32,
                y: *event_y as f32,
            },
            X11Event::Destroy { .. } => RawEvent::Close,
            X11Event::ClientMessage {
                message_type, data, ..
            } => {
                // WM_DELETE_WINDOW 的第一个 data 项是那个原子名，
                // 本轮只要收到 ClientMessage 就当关窗处理（WM_PROTOCOLS 协商已完成）
                if *message_type != 0 && data[0] != 0 {
                    RawEvent::Close
                } else {
                    return None;
                }
            }
            _ => return None,
        })
    }
}

impl X11Window {
    /// 切换窗口光标形状。
    ///
    /// 输入：形状枚举。副作用：向服务器发一个 `ChangeWindowAttributes`。
    /// 失败：`Err` 当服务器不接受（如无 cursor 字体）。
    ///
    /// 为什么要幂等：`poll` 每帧都会告知"鼠标在哪"，如果每帧都发一次请求，
    /// 就是白白浪费带宽与服务端 CPU。
    pub fn set_cursor(&mut self, shape: CursorShape) -> Result<(), String> {
        if self.cursor_now == shape {
            return Ok(());
        }
        let Some(cursors) = self.cursors.as_ref() else {
            self.cursor_now = shape;
            return Ok(());
        };
        let idx = match shape {
            CursorShape::Default => 0,
            CursorShape::Text => 1,
            CursorShape::Pointer => 2,
        };
        let cid = cursors.ids[idx];
        // ChangeWindowAttributes(window, value-mask=Cursor(1<<14), value-list=cid)
        let mut out = Vec::with_capacity(16);
        out.push(crate::opcode::CHANGE_WINDOW_ATTRIBUTES);
        out.push(0);
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&self.wid.to_le_bytes());
        out.extend_from_slice(&(1u32 << 14).to_le_bytes());
        out.extend_from_slice(&cid.to_le_bytes());
        self.conn.send(&out)?;
        self.conn.flush()?;
        self.cursor_now = shape;
        Ok(())
    }
}

impl EventSource for X11Window {
    fn poll(&mut self, timeout_ms: u32) -> Vec<RawEvent> {
        // 为什么用"短轮询 + 标志：没有 `poll(2)` 就无法同时等 socket 与自管管道；
        // 这里用 8ms 超时反复读，CPU 代价极小（空循环 125 次/秒）。
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
        let mut out = Vec::new();
        loop {
            let slice = if timeout_ms == 0 { 0 } else { 8 };
            match self.conn.poll_events(slice) {
                Ok((events, _errors)) => {
                    for e in &events {
                        // **输入法优先**：组字中的字母、空格选词都应被 IBus 吃掉，
                        // 不能变成 `KeyDown` 去触发快捷键或插入英文。
                        if let X11Event::Key {
                            pressed: true,
                            keycode,
                            state,
                            ..
                        } = e
                        {
                            // 取 keysym：与 map_event 同一套（Shift 时用第 1 列）
                            let (plain, shifted) = self.keymap.plain_and_shift(*keycode);
                            let shift = state & 1 != 0;
                            if let Some(ks) = if shift { shifted.or(plain) } else { plain } {
                                // 首次收到按键时告知 IME “我们有焦点”（只发一次）
                                if !self.ime_focused {
                                    self.ime.focus_in();
                                    self.ime_focused = true;
                                }
                                let outcome = self.ime.filter_key(
                                    crate::keys::ibus_keyval(ks),
                                    *keycode as u32,
                                    crate::keys::ibus_state(*state),
                                );
                                if outcome == crate::ime::KeyOutcome::Consumed {
                                    continue;
                                }
                            }
                        }
                        if let Some(raw) = self.map_event(e) {
                            out.push(raw);
                        }
                    }
                    // 把 IME 侧已到达的事件（非阻塞 try_recv）并入本轮事件流
                    push_ime_events(&mut self.ime, &mut out);
                    if !out.is_empty() || timeout_ms == 0 || std::time::Instant::now() >= deadline {
                        return out;
                    }
                    if self.woken.swap(false, Ordering::SeqCst) {
                        push_ime_events(&mut self.ime, &mut out);
                        return out;
                    }
                }
                // **错误不能当“没有事件”，也不能一律当“断线”。**
                //
                // `poll_events` 内部已过滤 `WouldBlock`/`TimedOut`（返回空事件）
                // 并对 `Interrupted` 重试，所以能到这里的 `Err` 分两类：
                //
                // 1. **连接已不可用**（EOF / 真实读错 / 不完整包）→ 推 `Close` 让主循环退出；
                // 2. **配置失败**（`set_nonblocking` / `set_read_timeout` 报错，如 EINVAL）→ **不是断线**，
                //    它是我们自己写错的信号；把它当断线会让所有窗口在首帧后集体关闭（已发生）。
                Err(e) => {
                    let msg = e.to_string();
                    if msg.starts_with("set_") {
                        // 配置错误：只报一次，不退出。
                        // 留在这里是为了让 bug 可见（否则又回到"静默空转"）。
                        eprintln!("[y901] X11 轮询配置错误（非断线）：{msg}");
                    } else {
                        eprintln!("[y901] X11 连接已不可用：{msg}（发 Close 让主循环退出）");
                        out.push(RawEvent::Close);
                    }
                    return out;
                }
            }
        }
    }

    fn waker(&self) -> Arc<dyn Waker> {
        struct W(Arc<AtomicBool>);
        impl Waker for W {
            fn wake(&self) {
                // 只置位，不打断系统调用：`poll` 最多 8ms 后就会看到它
                self.0.store(true, Ordering::SeqCst);
            }
        }
        Arc::new(W(self.woken.clone()))
    }
}

/// 把 IME 客户端里已到达的事件转成 `RawEvent`（**非阻塞**）。
///
/// - `Preedit` → `RawEvent::Ime { composition: true }`（只画不插入）
/// - `Commit`  → `RawEvent::Ime { composition: false }`（正式插入）
///
/// 为何放在这里而不在 `map_event`：IME 事件不来自 X11 事件序列，
/// 而来自 IBus 信号线程，它们只能在“每轮轮询结尾”统一并入。
fn push_ime_events(ime: &mut Box<dyn crate::ime::ImeClient>, out: &mut Vec<RawEvent>) {
    for ev in ime.take_events() {
        let (text, composition) = match ev {
            crate::ime::ImeEvent::Preedit(t) => (t, true),
            crate::ime::ImeEvent::Commit(t) => (t, false),
        };
        out.push(RawEvent::Ime { text, composition });
    }
}

impl Presenter for X11Window {
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.width = width.max(1).min(u16::MAX as u32) as u16;
        self.height = height.max(1).min(u16::MAX as u32) as u16;
        Ok(())
    }

    fn present(&mut self, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
        let w = width.min(self.width as u32) as u16;
        let h = height.min(self.height as u32) as u16;

        // 优先走共享内存：一次 memcpy + 一个 40 字节请求
        if let Some(state) = self.shm.as_mut() {
            if state.w == w && state.h == h {
                let need = w as usize * h as usize * 4;
                if pixels.len() >= need {
                    state.seg.write(&pixels[..need])?;
                    self.conn.send(&crate::encode::encode_shm_put_image(
                        state.major,
                        &crate::encode::ShmPutImageArgs {
                            drawable: self.wid,
                            gc: self.gc,
                            total_width: w,
                            total_height: h,
                            src_x: 0,
                            src_y: 0,
                            src_width: w,
                            src_height: h,
                            dst_x: 0,
                            dst_y: 0,
                            depth: self.depth,
                            // false：不要求完成事件（我们不做阻塞等待，
                            // 下一帧的 ShmPutImage 会自然排在后面）
                            send_event: false,
                            shmseg: state.seg_id,
                            offset: 0,
                        },
                    ))?;
                    self.conn.flush()?;
                    return Ok(());
                }
            }
        }

        self.conn
            .put_image_bgra(self.wid, self.gc, w, h, pixels, self.depth)?;
        Ok(())
    }
}

impl WindowBackend for X11Window {
    /// 语义：可阻塞至多 `wait_ms`；关闭（`RawEvent::Close`）由 `run_loop` 判定。
    fn wait_events(&mut self, wait_ms: u32) -> Option<Vec<RawEvent>> {
        // 全屏下的 Esc：`translate`（`&self`）只能置标志，在这里（`&mut self`）执行切换。
        if self.want_exit_fs.replace(false) && self.fullscreen {
            if let Err(e) = self.set_fullscreen(false) {
                eprintln!("[y901] 退出全屏失败：{e}");
            }
        }
        Some(self.poll(wait_ms))
    }

    fn viewport(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    /// 语义：先把光栅器尺寸对齐窗口，再按新视口整树重排并全量失效。
    fn relayout(&mut self, tree: &mut UiTree) {
        let (w, h) = self.size();
        self.soft.resize(w as u32, h as u32);
        tree.layout(Size::new(w as f32, h as f32), &mut self.soft);
        tree.invalidate_all();
    }

    /// 脏区判定在 `render_tree` 内部（本层是唯一判定点）。
    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String> {
        // 这里只生成 outcome；实际 skip 与 Present 在 `render_tree` 内。
        let had_dirty = !tree.dirty().is_empty();
        self.render_tree(tree)?;
        Ok(if had_dirty {
            RenderOutcome::Presented
        } else {
            RenderOutcome::Skipped
        })
    }

    /// 运行时切换全屏（EWMH）。
    ///
    /// 只发 ClientMessage，边框/几何由 WM 恢复。无 WM 时消息被忽略（不报错）。
    fn set_fullscreen(&mut self, on: bool) -> Result<(), String> {
        if self.fullscreen == on {
            return Ok(()); // 状态机：不重复发
        }
        let root = self.conn.info().root;
        fullscreen_atoms(&mut self.conn, self.wid, root, on)?;
        self.fullscreen = on;
        Ok(())
    }

    fn set_theme(&mut self, theme: Theme) -> Result<(), String> {
        self.soft.set_theme(theme)
    }

    /// X11 后端启动时固定字体（与 Win32 的软后端一致：重建引擎需换字体路径，
    /// 本轮不做运行时切换）。保留默认实现即可，这里显式说明能力缺省。
    fn set_font(&mut self, _font: &FontConfig) -> Result<(), String> {
        Err("X11 后端不支持运行时切换字体（启动时固定）".to_string())
    }
}

/// 内部原子化（取回原子 ID）。
fn intern(conn: &mut Connection, name: &[u8]) -> Result<u32, String> {
    conn.send(&crate::encode_intern_atom(name, false))?;
    conn.flush()?;
    // InternAtom 的响应必须单独读：`sync()` 会丢弃非当前序列的响应
    conn.read_reply_u32()
}

/// 编码 `ChangeProperty`（WM_NAME，UTF8）。
fn encode_change_title(wid: u32, name_atom: u32, type_atom: u32, title: &str) -> Vec<u8> {
    crate::encode_change_property(
        wid,
        name_atom,
        type_atom,
        8,
        crate::PropMode::Replace,
        title.as_bytes(),
    )
}

/// `Key` 的调试名（报告与测试用）。
pub fn key_name(k: Key) -> String {
    match k {
        Key::Char(c) => format!("Char({c:?})"),
        Key::Enter => "Enter".into(),
        Key::Escape => "Escape".into(),
        Key::Backspace => "Backspace".into(),
        Key::Tab => "Tab".into(),
        Key::Delete => "Delete".into(),
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Function(n) => format!("F{n}"),
        Key::Unknown => "Unknown".into(),
    }
}

/// 修饰键展示（测试用）。
pub fn mods_name(m: &Modifiers) -> String {
    format!(
        "{}{}{}",
        if m.shift { "Shift+" } else { "" },
        if m.ctrl { "Ctrl+" } else { "" },
        if m.alt { "Alt+" } else { "" }
    )
}

/// 尝试启用 MIT-SHM：探测扩展 → 查版本 → 建段 → 挂载。
///
/// 返回：`Ok(None)` = 服务器不支持（正常情况，调用方应回退）；
/// `Err` = 支持但启用过程出错（同样回退，但要打日志）。
fn try_enable_shm(
    conn: &mut Connection,
    seg_id: u32,
    width: u16,
    height: u16,
    _depth: u8,
) -> Result<Option<ShmState>, String> {
    // 强制回退开关：用于验证"没有 SHM 时画面也对"以及性能对比
    if std::env::var_os("Y901_NO_SHM").is_some() {
        return Ok(None);
    }
    let Some(major) = conn.query_extension(b"MIT-SHM")? else {
        return Ok(None);
    };
    let (vmajor, vminor) = conn.shm_query_version(major)?;
    if vmajor == 0 && vminor == 0 {
        return Ok(None);
    }
    let bytes = width as usize * height as usize * 4;
    let seg = crate::shm::ShmSegment::new(bytes)?;
    let shmid = seg.id() as u32;
    conn.send(&crate::encode::encode_shm_attach(
        major, seg_id, shmid, false,
    ))?;
    conn.flush()?;
    let errs = conn.sync()?;
    if !errs.is_empty() {
        return Err(format!("ShmAttach 报错：{errs:?}"));
    }
    Ok(Some(ShmState {
        major,
        seg_id,
        seg,
        w: width,
        h: height,
    }))
}

/// 用服务器内置 `"cursor"` 字体的字形创建三个光标。
///
/// 字形号来自 X11 的 `cursorfont.h`：`XC_left_ptr` = 68、`XC_xterm` = 152、`XC_hand2` = 60。
fn create_cursors(conn: &mut Connection, base: u32) -> Result<Cursors, String> {
    let font = base | 0x20;
    conn.send(&crate::encode::encode_open_font(font, b"cursor"))?;
    conn.flush()?;
    let errs = conn.sync()?;
    if !errs.is_empty() {
        return Err(format!("OpenFont(cursor) 报错：{errs:?}"));
    }
    const SHAPES: [(u16, u32); 3] = [(68, 0x21), (152, 0x22), (60, 0x23)];
    let mut ids = [0u32; 3];
    for (i, (ch, off)) in SHAPES.iter().enumerate() {
        let cid = base | off;
        conn.send(&crate::encode::encode_create_glyph_cursor(
            cid,
            font,
            *ch,
            font,
            *ch,
            (0, 0, 0),
            (0xFFFF, 0xFFFF, 0xFFFF),
        ))?;
        ids[i] = cid;
    }
    conn.flush()?;
    let errs = conn.sync()?;
    if !errs.is_empty() {
        return Err(format!("CreateGlyphCursor 报错：{errs:?}"));
    }
    Ok(Cursors { font, ids })
}

/// 发 EWMH 全屏消息（`_NET_WM_STATE` + `_NET_WM_STATE_FULLSCREEN`）。
///
/// 依据 freedesktop EWMH §7.6：发一个 `ClientMessage` 到 root，
/// `data[0] = 1`（`_NET_WM_STATE_ADD`）、`data[1]` 为要新增的属性 atom。
/// 事件掩码用 `SubstructureRedirect | SubstructureNotify`（EWMH 规定的发送对象是 root）。
///
/// **注意**：若没有 WM（如 Xvfb），本消息会被忽略，窗口不会真的全屏 ——
/// 这是 EWMH 的设计（全屏由 WM 实施），不是实现缺陷。
fn fullscreen_atoms(conn: &mut Connection, wid: u32, root: u32, add: bool) -> Result<(), String> {
    let state_atom = intern(conn, b"_NET_WM_STATE")?;
    let fs_atom = intern(conn, b"_NET_WM_STATE_FULLSCREEN")?;
    // data[0]: 1 = _NET_WM_STATE_ADD，0 = _NET_WM_STATE_REMOVE（EWMH §7.6）
    let action = if add { 1 } else { 0 };
    let ev = crate::encode::client_message_event(wid, state_atom, [action, fs_atom, 0, 0, 0]);
    // SubstructureRedirectMask = 1<<20, SubstructureNotifyMask = 1<<19
    conn.send(&crate::encode::encode_send_event(
        root,
        (1 << 20) | (1 << 19),
        &ev,
    ))?;
    Ok(())
}
