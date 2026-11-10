//! X11 剪贴板（Selections 协议）—— **独立线程 + 自己的连接 + 隐藏窗口**。
//!
//! ## 为什么必须独立线程
//!
//! X11 剪贴板不是"读写共享内存"，而是**事件驱动的协议**：
//! 拥有 selection 的窗口必须在**它自己的事件循环里**实时应答 `SelectionRequest`。
//! 而业务代码会在任意时刻调 `get_text` / `set_text`。
//! 把它嵌进主循环会污染 runtime 契约（还会让"粘贴"延迟一帧）。
//!
//! 所以：专用线程拿一个**独立的 X 连接**，建一个 1×1 隐藏窗口当 owner，
//! 主线程通过 channel 与它通信。这是 alacritty / wezterm 的标准做法。
//!
//! ## 支持两个 selection
//!
//! - `CLIPBOARD`：Ctrl+C / Ctrl+V 的那个（跨应用复制粘贴）
//! - `PRIMARY`：选中即复制（Linux 惯例，中键粘贴用它）

use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use crate::conn::Connection;

/// 两个 selection 的标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// 常规剪贴板
    Clipboard,
    /// 选中即复制
    Primary,
}

/// 向剪贴板线程发的命令。
enum Cmd {
    /// 设置文本并抢占 selection（reply = 是否成功）
    Set {
        /// 哪个 selection
        which: Selection,
        /// 要写入的文本
        text: String,
        /// 回执通道
        reply: Sender<Result<(), String>>,
    },
    /// 读取文本
    Get {
        /// 哪个 selection
        which: Selection,
        /// 回执通道
        reply: Sender<Result<String, String>>,
    },
    /// 让线程退出（关窗口、关连接）
    Quit,
}

/// X11 剪贴板句柄：对外只暴露 `get_text` / `set_text`。
pub struct X11Clipboard {
    tx: Sender<Cmd>,
}

impl X11Clipboard {
    /// 启动剪贴板线程。
    ///
    /// 输入：`display`（如 `":99"`）。
    /// 失败：`Err` 当连接不上（这时剪贴板不可用，但不影响 UI）。
    pub fn start(display: &str) -> Result<Self, String> {
        // 先在主线程完成连接与建窗，这样失败能当场告诉调用方
        let mut conn = Connection::connect(display)?;
        let info = conn.info().clone();
        let wid = info.resource_id_base | 0x30;

        // 1x1 隐藏窗口（不 Map）：只用来持有 selection 与存放属性
        conn.send(&crate::encode::encode_create_window(
            &crate::encode::CreateWindowArgs {
                wid,
                parent: info.root,
                x: -10,
                y: -10,
                width: 1,
                height: 1,
                border_width: 0,
                class: crate::encode::WindowClass::InputOnly,
                visual: 0,
                values: &[(crate::encode::ValueMask::EventMask, 0x0)],
            },
        ))?;
        conn.flush()?;
        let errs = conn.sync()?;
        if !errs.is_empty() {
            return Err(format!("剪贴板隐藏窗口创建失败：{errs:?}"));
        }

        let (tx, rx) = channel();
        let handle = std::thread::Builder::new()
            .name("y901-clipboard".to_string())
            .spawn(move || clipboard_thread(conn, wid, rx))
            .map_err(|e| format!("剪贴板线程启动失败：{e}"))?;
        // 线程自己会退出（Quit 或连接断），不持有 JoinHandle 也不泄漏资源
        let _ = handle;
        Ok(Self { tx })
    }

    /// 读取（`None` = 线程已退出）。
    pub fn get(&self, which: Selection) -> Result<String, String> {
        let (tx, rx) = channel();
        self.tx
            .send(Cmd::Get { which, reply: tx })
            .map_err(|_| "剪贴板线程已退出".to_string())?;
        rx.recv_timeout(Duration::from_millis(500))
            .map_err(|_| "读剪贴板超时（owner 未响应）".to_string())?
    }

    /// 写入。
    pub fn set(&self, which: Selection, text: &str) -> Result<(), String> {
        let (tx, rx) = channel();
        self.tx
            .send(Cmd::Set {
                which,
                text: text.to_string(),
                reply: tx,
            })
            .map_err(|_| "剪贴板线程已退出".to_string())?;
        rx.recv_timeout(Duration::from_millis(500))
            .map_err(|_| "写剪贴板超时".to_string())?
    }
}

impl Drop for X11Clipboard {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Quit);
    }
}

/// 线程主体：自己的 X 连接，领导一个极小的事件循环。
fn clipboard_thread(mut conn: Connection, wid: u32, rx: Receiver<Cmd>) {
    let Some(atoms) = Atoms::intern(&mut conn) else {
        return;
    };
    // 本地保存我们拥有的文本（哪个 selection 归我，就存哪个）
    let mut owned_clip: Option<String> = None;
    let mut owned_prim: Option<String> = None;

    loop {
        // 先处理命令（不阻塞）
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Quit => return,
                Cmd::Set { which, text, reply } => {
                    let sel = match which {
                        Selection::Clipboard => atoms.clipboard,
                        Selection::Primary => atoms.primary,
                    };
                    match conn.send(&crate::encode::encode_set_selection_owner(wid, sel, 0)) {
                        Ok(()) => {
                            // 不静默：若没刷出去，服务器不会建立 owner，
                            // 后续 get_text 会读到空——而调用方看不出原因。
                            if let Err(e) = conn.flush() {
                                eprintln!("[y901] 剪贴板设置 owner 后 flush 失败：{e}");
                            }
                            match which {
                                Selection::Clipboard => owned_clip = Some(text),
                                Selection::Primary => owned_prim = Some(text),
                            }
                            let _ = reply.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = reply.send(Err(e));
                        }
                    }
                }
                Cmd::Get { which, reply } => {
                    let res = fetch_text(
                        &mut conn,
                        &atoms,
                        wid,
                        which,
                        owned_clip.as_deref(),
                        owned_prim.as_deref(),
                    );
                    let _ = reply.send(res);
                }
            }
        }

        // 再等事件（10ms 超时，保证命令能及时被看到）
        let Ok(pkt) = conn.next_packet(10) else {
            continue;
        };
        // **必须屏蔽最高位**：send_event 标志（我们自己用 SendEvent 发的 SelectionNotify 就是 159 而不是 31）
        match pkt[0] & 0x7F {
            crate::event_code::SELECTION_REQUEST => {
                serve_request(
                    &mut conn,
                    &atoms,
                    &pkt,
                    owned_clip.as_deref(),
                    owned_prim.as_deref(),
                );
            }
            crate::event_code::SELECTION_CLEAR => {
                // 被别人抢走了：清掉本地副本（否则会把旧内容当成"当前剪贴板"）
                let sel = le32(&pkt, 8);
                if sel == atoms.clipboard {
                    owned_clip = None;
                } else if sel == atoms.primary {
                    owned_prim = None;
                }
            }
            _ => {}
        }
    }
}

/// 应答一个 `SelectionRequest`。
fn serve_request(
    conn: &mut Connection,
    atoms: &Atoms,
    pkt: &[u8; 32],
    clip: Option<&str>,
    prim: Option<&str>,
) {
    let owner = le32(pkt, 8);
    let requestor = le32(pkt, 12);
    let selection = le32(pkt, 16);
    let target = le32(pkt, 20);
    let property = le32(pkt, 24);
    let _ = owner;
    let Some(text) = (if selection == atoms.clipboard {
        clip
    } else if selection == atoms.primary {
        prim
    } else {
        None
    }) else {
        send_notify(conn, requestor, selection, target, 0);
        return;
    };

    // TARGETS：告诉对方"我能提供哪些格式"
    if target == atoms.targets {
        let mut data = Vec::with_capacity(12);
        for a in [atoms.targets, atoms.utf8, atoms.text, 31u32] {
            data.extend_from_slice(&a.to_le_bytes());
        }
        // 不静默：写属性失败时对方拿不到数据，
        // **不能再发 SelectionNotify**（那会让对方去读一个不存在的属性）。
        if let Err(e) = conn.send(&crate::encode_change_property(
            requestor,
            property,
            33, // ATOM
            32,
            crate::encode::PropMode::Replace,
            &data,
        )) {
            eprintln!("[y901] 剪贴板 TARGETS 响应失败：{e}");
            return;
        }
        if let Err(e) = conn.flush() {
            eprintln!("[y901] 剪贴板 TARGETS 响应 flush 失败：{e}");
            return;
        }
        send_notify(conn, requestor, selection, target, property);
        return;
    }

    // 文本类型：UTF8_STRING / TEXT / STRING（都按 UTF-8 发，现代应用都能接）
    if target == atoms.utf8 || target == atoms.text || target == 31 {
        // 同 TARGETS：写属性或 flush 失败就不发 SelectionNotify。
        if let Err(e) = conn.send(&crate::encode_change_property(
            requestor,
            property,
            atoms.utf8,
            8,
            crate::encode::PropMode::Replace,
            text.as_bytes(),
        )) {
            eprintln!("[y901] 剪贴板文本响应失败：{e}");
            return;
        }
        if let Err(e) = conn.flush() {
            eprintln!("[y901] 剪贴板文本响应 flush 失败：{e}");
            return;
        }
        send_notify(conn, requestor, selection, target, property);
        return;
    }

    // 不支持的格式：property = 0 表示拒绝
    send_notify(conn, requestor, selection, target, 0);
}

/// 给请求方发 `SelectionNotify`。
fn send_notify(conn: &mut Connection, requestor: u32, selection: u32, target: u32, property: u32) {
    let mut ev = [0u8; 32];
    ev[0] = crate::event_code::SELECTION_NOTIFY;
    ev[4..8].copy_from_slice(&0u32.to_le_bytes()); // time
    ev[8..12].copy_from_slice(&requestor.to_le_bytes());
    ev[12..16].copy_from_slice(&selection.to_le_bytes());
    ev[16..20].copy_from_slice(&target.to_le_bytes());
    ev[20..24].copy_from_slice(&property.to_le_bytes());
    // 不静默：SelectionNotify 是"已备好，去读吧"的唯一信号，发不出就是请求失败。
    if let Err(e) = conn.send(&crate::encode::encode_send_event(requestor, 0, &ev)) {
        eprintln!("[y901] 剪贴板 SelectionNotify 发送失败：{e}");
        return;
    }
    if let Err(e) = conn.flush() {
        eprintln!("[y901] 剪贴板 SelectionNotify flush 失败：{e}");
    }
}

/// 请求并读取剪贴板文本（阻塞直到 `SelectionNotify` 或超时）。
fn fetch_text(
    conn: &mut Connection,
    atoms: &Atoms,
    wid: u32,
    which: Selection,
    clip: Option<&str>,
    prim: Option<&str>,
) -> Result<String, String> {
    let sel = match which {
        Selection::Clipboard => atoms.clipboard,
        Selection::Primary => atoms.primary,
    };
    if std::env::var_os("Y901_CLIP_TRACE").is_some() {
        eprintln!(
            "[x11] ConvertSelection sel={sel:#x} target={:#x} prop={:#x}",
            atoms.utf8, atoms.property
        );
    }
    conn.send(&crate::encode::encode_convert_selection(
        wid,
        sel,
        atoms.utf8,
        atoms.property,
        0,
    ))?;
    conn.flush()?;

    // 等 SelectionNotify（最多 2 秒：对方可能是一个慢应用）
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let Ok(pkt) = conn.next_packet(50) else {
            continue;
        };
        if std::env::var_os("Y901_CLIP_TRACE").is_some() {
            eprintln!(
                "[x11] 收到包 code={} sel={:#x} prop={:#x}",
                pkt[0],
                le32(&pkt, 16),
                le32(&pkt, 20)
            );
        }
        // **关键**：如果 selection 的 owner 就是我们自己，服务器会把
        // ConvertSelection 转成 SelectionRequest 发回给我们 ——
        // 而我们正在这里等 SelectionNotify。不先应答就是**自己等自己**。
        let code = pkt[0] & 0x7F;
        if code == crate::event_code::SELECTION_REQUEST {
            serve_request(conn, atoms, &pkt, clip, prim);
            continue;
        }
        if code != crate::event_code::SELECTION_NOTIFY {
            continue;
        }
        let property = le32(&pkt, 20);
        if property == 0 {
            return Err("剪贴板为空或 owner 拒绝提供 UTF8 文本".to_string());
        }
        let (_, data) = conn.get_property(wid, property, 0, 0, 1 << 20, true)?;
        return String::from_utf8(data).map_err(|e| format!("剪贴板内容不是合法 UTF-8：{e}"));
    }
    Err("等待 SelectionNotify 超时".to_string())
}

/// 常用原子（一次预先内部化，后面全用 ID）。
struct Atoms {
    /// `CLIPBOARD`
    clipboard: u32,
    /// `PRIMARY`
    primary: u32,
    /// `TARGETS`
    targets: u32,
    /// `UTF8_STRING`
    utf8: u32,
    /// `TEXT`
    text: u32,
    /// 我们用来存放数据的属性名
    property: u32,
}

impl Atoms {
    fn intern(conn: &mut Connection) -> Option<Self> {
        let get = |conn: &mut Connection, name: &[u8]| -> Option<u32> {
            conn.send(&crate::encode_intern_atom(name, false)).ok()?;
            conn.flush().ok()?;
            conn.read_reply_u32().ok()
        };
        Some(Self {
            clipboard: get(conn, b"CLIPBOARD")?,
            primary: get(conn, b"PRIMARY")?,
            targets: get(conn, b"TARGETS")?,
            utf8: get(conn, b"UTF8_STRING")?,
            text: get(conn, b"TEXT")?,
            property: get(conn, b"Y901_CLIPBOARD")?,
        })
    }
}

/// 小工具：从 32 字节包里读一个小端 u32。
fn le32(b: &[u8; 32], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
