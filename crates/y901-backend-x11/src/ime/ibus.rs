//! **IBus 客户端**（纯 Rust：`rustbus`，无 C 依赖）。
//!
//! ## 为什么不用 zbus（实测记录）
//!
//! IBus 发出的信号**违反 D-Bus 规范**：`sender` 字段填的是**知名名** `org.freedesktop.IBus`，
//! 而规范要求必须是唯一名（`:1.x`）。libdbus（Python）宽容，zbus 严格：
//! zbus 5.19 与 4.4 都在 `message/fields.rs` 里 `expect("Invalid field reconstruction")`
//! **直接 panic**，把连接执行器线程打死 ⇒ 一条信号也收不到。
//! `rustbus` 不校验该字段，实测能完整解出 `CommitText` / `UpdatePreeditText`（含 55 万字符的候选表）。
//!
//! ## 线程模型（`RpcConn` 不能跨线程共享）
//!
//! ```text
//! 主线程                       连接线程
//!   filter_key   ── Cmd::Key ──►   ProcessKeyEvent → 回 SyncSender<bool>
//!   set_cursor   ── Cmd::Cursor ─► SetCursorLocation
//!   take_events  ◄─ ImeEvent ────  CommitText / UpdatePreeditText（mpsc）
//! ```
//! 所有对主线程的接口都**非阻塞**：`filter_key` 用 `recv_timeout`（超时按 `Forward` 处理，
//! 宁可英文可用也不吞键），`take_events` 用 `try_recv`。
//!
//! ## 载荷格式
//!
//! 信号里的文本是 IBus 序列化对象：
//! `Variant(Struct(['IBusText', {}, '你好', Variant(Struct(['IBusAttrList', {}, []]))]))`
//! ⇒ 取其中**第三个字段**（`Base::String`），并跳过 `IBus*` 类名与空串。

use std::sync::mpsc::{channel, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustbus::connection::rpc_conn::RpcConn;
use rustbus::connection::Timeout;
use rustbus::message_builder::{MarshalledMessage, MessageBuilder};
use rustbus::params::{Base, Container, Param};

use super::{drain, ImeClient, ImeEvent, KeyOutcome};

/// IBus 服务名 / 根对象 / 上下文接口。
const SERVICE: &str = "org.freedesktop.IBus";
const ROOT: &str = "/org/freedesktop/IBus";
const IFACE: &str = "org.freedesktop.IBus.InputContext";
/// 能力位：`PREEDIT_TEXT = 1<<0`、`FOCUS = 1<<1`。必须先设，否则 `FocusIn` 报 not support focus。
const CAPS: u32 = 1 | 2;

/// 一次 IBus 连接调用的超时（信号轮询也用这个粒度，保证主循环不被卡住）。
const POLL_MS: u64 = 10;

/// 主线程 → 连接线程的命令。
enum Cmd {
    /// 把按键交给 IBus；`reply` 回 `true` = 输入法已处理（调用方不得再发 `KeyDown`）。
    Key {
        keyval: u32,
        keycode: u32,
        state: u32,
        reply: SyncSender<bool>,
    },
    /// 候选窗位置（输入框光标所在的屏幕坐标）。
    Cursor { x: i32, y: i32, w: i32, h: i32 },
    /// 输入框获得焦点。
    FocusIn,
    /// 输入框失去焦点。
    FocusOut,
}

/// IBus 输入法客户端。
pub struct IbusIme {
    tx: Sender<Cmd>,
    rx: Receiver<ImeEvent>,
    ctx_path: String,
    _thread: JoinHandle<()>,
}

impl IbusIme {
    /// 连接 IBus 并创建输入上下文。
    ///
    /// `engine` 为空串表示沿用 IBus 当前默认引擎；建议传 `"libpinyin"`。
    /// 返回 `Err` 表示 IBus 不可用（未运行 / 无 `ibus` 命令 / 报文异常），调用方应降级为 `NoIme`。
    pub fn connect(engine: &str) -> Result<Self, String> {
        let addr = ibus_address()?;
        // **连接与上下文都建立成功后才起线程**：失败路径一律不产生半死不活的状态。
        let mut conn = connect_ibus_bus(&addr)?;
        let ctx_path = create_context(&mut conn, engine)?;
        let ctx_for_thread = ctx_path.clone();
        let (tx, cmd_rx) = channel::<Cmd>();
        let (evt_tx, rx) = channel::<ImeEvent>();
        let thread = std::thread::Builder::new()
            .name("y901-ibus".to_string())
            .spawn(move || conn_loop(conn, ctx_for_thread, cmd_rx, evt_tx))
            .map_err(|e| format!("IBus 线程启动失败：{e}"))?;
        Ok(Self {
            tx,
            rx,
            ctx_path,
            _thread: thread,
        })
    }

    /// 上下文对象路径（日志用）。
    pub fn context_path(&self) -> &str {
        &self.ctx_path
    }

    fn send(&self, cmd: Cmd) {
        // 线程已退出时静默丢弃：主循环绝不能因此 panic。
        let _ = self.tx.send(cmd);
    }
}

impl ImeClient for IbusIme {
    fn available(&self) -> bool {
        true
    }

    fn filter_key(&mut self, keyval: u32, keycode: u32, state: u32) -> KeyOutcome {
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<bool>(1);
        self.send(Cmd::Key {
            keyval,
            keycode,
            state,
            reply: reply_tx,
        });
        // 20ms 足够本地 D-Bus 往返；超时按 Forward 处理（不吞键、不卡主循环）。
        match reply_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(true) => KeyOutcome::Consumed,
            _ => KeyOutcome::Forward,
        }
    }

    fn set_cursor_location(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.send(Cmd::Cursor { x, y, w, h });
    }

    fn focus_in(&mut self) {
        self.send(Cmd::FocusIn);
    }

    fn focus_out(&mut self) {
        self.send(Cmd::FocusOut);
    }

    fn take_events(&mut self) -> Vec<ImeEvent> {
        drain(&self.rx)
    }
}

// ---------------------------------------------------------------------------
// 连接与线程
// ---------------------------------------------------------------------------

/// 连接 IBus 的私有总线（地址形如 `unix:abstract=/home/x/.cache/ibus/dbus-xxx`）。
///
/// 注意：`get_session_bus_path()` 只读 `DBUS_SESSION_BUS_ADDRESS`，因此先临时设环境变量再还原。
/// 这一步是 rustbus 的公开入口里唯一能解析 `unix:abstract=` 的办法（其内部函数未导出）。
fn connect_ibus_bus(addr: &str) -> Result<RpcConn, String> {
    let saved = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok();
    unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", addr) };
    let unix = rustbus::connection::get_session_bus_path();
    match saved {
        Some(v) => unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", v) },
        None => unsafe { std::env::remove_var("DBUS_SESSION_BUS_ADDRESS") },
    }
    let unix = unix.map_err(|e| format!("IBus 地址解析失败（{addr}）：{e}"))?;
    let mut conn = RpcConn::connect_to_path(unix, Timeout::Duration(Duration::from_secs(2)))
        .map_err(|e| format!("IBus 连接失败：{e}"))?;
    // 私有总线同样需要 Hello 握手，否则 IBus 不认我们的唯一名。
    call_simple(
        &mut conn,
        "Hello",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "org.freedesktop.DBus",
        |_| {},
    )
    .map_err(|e| format!("IBus Hello 失败：{e}"))?;
    // 让连接线程的 refill 不阻塞太久。
    Ok(conn)
}

/// 取 IBus 私有总线地址：`IBUS_ADDRESS` → bus 目录文件 → `ibus address` 命令。
fn ibus_address() -> Result<String, String> {
    if let Ok(a) = std::env::var("IBUS_ADDRESS") {
        let a = a.trim();
        if !a.is_empty() {
            return Ok(a.to_string());
        }
    }
    if let Some(dir) = home_config_ibus_dir() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with("-unix-0") {
                    if let Ok(text) = std::fs::read_to_string(e.path()) {
                        if let Some(line) =
                            text.lines().find_map(|l| l.strip_prefix("IBUS_ADDRESS="))
                        {
                            let line = line.trim();
                            if !line.is_empty() {
                                return Ok(line.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let out = std::process::Command::new("ibus")
        .arg("address")
        .output()
        .map_err(|e| format!("无法执行 `ibus address`：{e}"))?;
    if !out.status.success() {
        return Err(format!("`ibus address` 失败（{}）", out.status));
    }
    let addr = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if addr.is_empty() {
        return Err("`ibus address` 输出为空".into());
    }
    Ok(addr)
}

fn home_config_ibus_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::Path::new(&home).join(".config/ibus/bus"))
}

/// 发一个"无参数/可带参数"的方法调用并等回复（错误转字符串）。
fn call_simple(
    conn: &mut RpcConn,
    member: &str,
    path: &str,
    iface: &str,
    dest: &str,
    build: impl FnOnce(&mut MarshalledMessage),
) -> Result<Vec<Param<'static, 'static>>, String> {
    let mut msg = MessageBuilder::new()
        .call(member)
        .on(path)
        .with_interface(iface)
        .at(dest)
        .build();
    build(&mut msg);
    let serial = conn
        .send_message(&mut msg)
        .map_err(|e| format!("{member} 发送失败：{e}"))?
        .write_all()
        .map_err(|(_, e)| format!("{member} 写入失败（无法 Display 已丢弃 context）：{e}"))?;
    let reply = conn
        .wait_response(serial, Timeout::Duration(Duration::from_secs(2)))
        .map_err(|e| format!("{member} 无回复：{e}"))?;
    let parsed = reply
        .unmarshall_all()
        .map_err(|e| format!("{member} 回复解析失败：{e}"))?;
    Ok(parsed.params)
}

/// 创建输入上下文并完成能力/引擎/焦点初始化，返回上下文路径。
fn create_context(conn: &mut RpcConn, engine: &str) -> Result<String, String> {
    let params = call_simple(
        conn,
        "CreateInputContext",
        ROOT,
        "org.freedesktop.IBus",
        SERVICE,
        |m| {
            m.body.push_param("y901").ok();
        },
    )?;
    let ctx = params
        .iter()
        .find_map(|p| match p {
            Param::Base(Base::ObjectPath(s)) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| "CreateInputContext 未返回对象路径".to_string())?;

    let _ = call_simple(conn, "SetCapabilities", &ctx, IFACE, SERVICE, |m| {
        m.body.push_param(CAPS).ok();
    });
    if !engine.is_empty() {
        let _ = call_simple(conn, "SetEngine", &ctx, IFACE, SERVICE, |m| {
            m.body.push_param(engine).ok();
        });
    }
    // 部分 IBus 版本会回 "does not support focus"；不致命，ProcessKeyEvent 仍可用。
    let _ = call_simple(conn, "FocusIn", &ctx, IFACE, SERVICE, |_| {});
    Ok(ctx)
}

/// 连接线程主循环：处理命令（按键/焦点/候选位置）并把信号转成事件。
fn conn_loop(mut conn: RpcConn, ctx_path: String, cmd_rx: Receiver<Cmd>, evt_tx: Sender<ImeEvent>) {
    loop {
        // 1) 先处理主线程命令（非阻塞）
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Cmd::Key {
                    keyval,
                    keycode,
                    state,
                    reply,
                } => {
                    let handled = call_simple(
                        &mut conn,
                        "ProcessKeyEvent",
                        &ctx_path,
                        IFACE,
                        SERVICE,
                        |m| {
                            m.body.push_param(keyval).ok();
                            m.body.push_param(keycode).ok();
                            m.body.push_param(state).ok();
                        },
                    )
                    .ok()
                    .and_then(|params| {
                        params.iter().find_map(|p| match p {
                            Param::Base(Base::Boolean(b)) => Some(*b),
                            _ => None,
                        })
                    })
                    .unwrap_or(false);
                    let _ = reply.send(handled);
                }
                Cmd::Cursor { x, y, w, h } => {
                    let _ = call_simple(
                        &mut conn,
                        "SetCursorLocation",
                        &ctx_path,
                        IFACE,
                        SERVICE,
                        |m| {
                            m.body.push_param(x).ok();
                            m.body.push_param(y).ok();
                            m.body.push_param(w).ok();
                            m.body.push_param(h).ok();
                        },
                    );
                }
                Cmd::FocusIn => {
                    let _ = call_simple(&mut conn, "FocusIn", &ctx_path, IFACE, SERVICE, |_| {});
                }
                Cmd::FocusOut => {
                    let _ = call_simple(&mut conn, "FocusOut", &ctx_path, IFACE, SERVICE, |_| {});
                }
            }
        }

        // 2) 收信号：短超时轮询，保证主循环节奏（refill 会阻塞最多 timeout）
        let deadline = Instant::now() + Duration::from_millis(POLL_MS);
        // 单次非阻塞轮询（不循环：读完即进第 3 步消费队列）
        let _ = conn.refill_once(Timeout::Duration(Duration::from_millis(2)));

        // 3) 取出已到达的信号（refill_once 内部已入队，这里逐条消费）
        while let Some(sig) = conn.try_get_signal() {
            let member = sig.dynheader.member.clone().unwrap_or_default();
            let parsed = match sig.unmarshall_all() {
                Ok(p) => p,
                Err(_) => continue,
            };
            match member.as_str() {
                "CommitText" => {
                    if let Some(text) = text_of(&parsed.params) {
                        if !text.is_empty() && evt_tx.send(ImeEvent::Commit(text)).is_err() {
                            return;
                        }
                    }
                }
                "UpdatePreeditText" => {
                    if let Some(text) = text_of(&parsed.params) {
                        if evt_tx.send(ImeEvent::Preedit(text)).is_err() {
                            return;
                        }
                    }
                }
                "HidePreeditText" => {
                    if evt_tx.send(ImeEvent::Preedit(String::new())).is_err() {
                        return;
                    }
                }
                _ => {}
            }
        }

        if Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

// ---------------------------------------------------------------------------
// 文本提取
// ---------------------------------------------------------------------------

/// 从信号参数里取 IBus 文本对象的正文。
///
/// 结构：`Variant(Struct(['IBusText', {..}, '你好', Variant(Struct(['IBusAttrList', ..]))]))`。
/// 规则与 M1 一致：**跳过 `IBus*` 类名与空串**，取第一个真正的字符串。
fn text_of(params: &[Param<'_, '_>]) -> Option<String> {
    params.iter().find_map(find_text)
}

fn find_text(p: &Param<'_, '_>) -> Option<String> {
    match p {
        Param::Base(Base::String(s)) => {
            if s.is_empty() || s.starts_with("IBus") {
                None
            } else {
                Some(s.clone())
            }
        }
        Param::Base(_) => None,
        Param::Container(c) => find_text_in_container(c),
    }
}

fn find_text_in_container(c: &Container<'_, '_>) -> Option<String> {
    match c {
        Container::Variant(v) => find_text(&v.value),
        Container::Struct(fields) => fields.iter().find_map(find_text),
        Container::StructRef(fields) => fields.iter().find_map(find_text),
        Container::Array(arr) => arr.values.iter().find_map(find_text),
        Container::ArrayRef(arr) => arr.values.iter().find_map(find_text),
        Container::Dict(d) => d.map.values().find_map(find_text),
        Container::DictRef(d) => d.map.values().find_map(find_text),
    }
}

#[cfg(test)]
mod tests {
    use super::find_text;
    use rustbus::params::{Base, Container, Param, Variant};
    use rustbus::signature::Type;

    fn boxed_variant(value: Param<'static, 'static>) -> Param<'static, 'static> {
        Param::Container(Container::Variant(Box::new(Variant {
            sig: Type::parse_description("v")
                .expect("sig")
                .into_iter()
                .next()
                .expect("one type"),
            value,
        })))
    }

    /// IBus 文本对象：`('IBusText', {}, '你好', Variant(...))`
    /// 提取规则：跳过 `IBus*` 类名与空串，取第一个真字符串。
    #[test]
    fn extracts_text_from_ibus_object() {
        let attrs = boxed_variant(Param::Base(Base::String("IBusAttrList".into())));
        let text_obj = Param::Container(Container::Struct(vec![
            Param::Base(Base::String("IBusText".into())),
            Param::Base(Base::String(String::new())),
            Param::Base(Base::String("NIHAO_PLACEHOLDER".into())),
            attrs,
        ]));
        let params = [boxed_variant(text_obj)];
        assert_eq!(find_text(&params[0]), Some("NIHAO_PLACEHOLDER".to_string()));
    }

    /// 空文本（组字取消 / HidePreeditText）必须返回 None，否则会误提交空串。
    #[test]
    fn empty_text_is_none() {
        let obj = Param::Container(Container::Struct(vec![
            Param::Base(Base::String("IBusText".into())),
            Param::Base(Base::String(String::new())),
        ]));
        assert_eq!(find_text(&obj), None);
    }
}
