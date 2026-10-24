//! Agent-Native RPC：本地 TCP + 行分隔 JSON。
//!
//! 线程模型：
//!   RPC 线程只做「收字节 → 解析 → 转交 → 回写」，绝不碰 UI 状态。
//!   UI 线程在自己的循环里 `drain` 请求，用同一份语义树处理后回包。
//!
//! 这样框架依旧单线程独占 UiTree，RPC 只是又一个输入源。

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use y901_ui::Json;

fn button_name(b: y901_ui::MouseButton) -> &'static str {
    match b {
        y901_ui::MouseButton::Left => "left",
        y901_ui::MouseButton::Right => "right",
        y901_ui::MouseButton::Middle => "middle",
    }
}

pub const DEFAULT_ADDR: &str = "127.0.0.1:9410";

/// 解析后的请求（已去掉传输层细节）。
#[derive(Clone, Debug, PartialEq)]
pub enum RpcRequest {
    /// 返回整棵语义树 + 每个节点的矩形。
    GetTree,
    /// 向目标节点投递一条事件（事件类型由协议描述，框架不理解语义）。
    Dispatch { node: String, event: RpcEvent },
    /// 向目标节点投递一个动作（等价于一条 `UiEvent::Action`）。
    Invoke {
        node: String,
        action: String,
        args: Json,
    },
    /// 按查询语言筛选节点（与 MCP 的 ui_query 完全一致）。
    Query { q: String },
    /// 切换焦点。
    Focus { node: String },
    /// 订阅某个节点的状态变化（服务端主动推送 `state_changed`）。
    Subscribe { node: String, interval_ms: u64 },
    /// 取消订阅。
    Unsubscribe { node: String },
    /// 健康检查。
    Ping,
    Unknown { method: String },
}

/// RPC 可以构造的事件。刻意保持极简：只覆盖"外部真的会发"的那几种。
#[derive(Clone, Debug, PartialEq)]
pub enum RpcEvent {
    Text(String),
    Key(y901_ui::Key),
    Compose(String),
    MouseDown { pos: y901_ui::Pos, button: y901_ui::MouseButton },
    MouseUp { pos: y901_ui::Pos, button: y901_ui::MouseButton },
    MouseMove { pos: y901_ui::Pos },
    Wheel { pos: y901_ui::Pos, delta: f32 },
}

impl RpcEvent {
    /// 还原成协议 JSON（RPC 转 gateway 时用，保证与 MCP 走同一套解析）。
    pub fn to_json(&self) -> Json {
        use y901_ui::UiEvent;
        let _: Option<UiEvent> = None;
        match self {
            RpcEvent::Text(t) => Json::obj().set("type", "text").set("text", t.clone()),
            RpcEvent::Key(k) => Json::obj()
                .set("type", "key")
                .set("key", format!("{k:?}")),
            RpcEvent::Compose(t) => Json::obj().set("type", "compose").set("text", t.clone()),
            RpcEvent::MouseDown { pos, button } => Json::obj()
                .set("type", "mouse_down")
                .set("x", pos.x)
                .set("y", pos.y)
                .set("button", button_name(*button)),
            RpcEvent::MouseUp { pos, button } => Json::obj()
                .set("type", "mouse_up")
                .set("x", pos.x)
                .set("y", pos.y)
                .set("button", button_name(*button)),
            RpcEvent::MouseMove { pos } => Json::obj()
                .set("type", "mouse_move")
                .set("x", pos.x)
                .set("y", pos.y),
            RpcEvent::Wheel { pos, delta } => Json::obj()
                .set("type", "wheel")
                .set("x", pos.x)
                .set("y", pos.y)
                .set("delta", *delta),
        }
    }

    /// 转成框架内部的 `UiEvent`。
    pub fn to_ui_event(&self) -> y901_ui::UiEvent {
        use y901_ui::UiEvent;
        match self {
            RpcEvent::Text(t) => UiEvent::Text(t.clone()),
            RpcEvent::Key(k) => UiEvent::Key {
                key: *k,
                modifiers: Default::default(),
            },
            RpcEvent::Compose(t) => UiEvent::Compose(t.clone()),
            RpcEvent::MouseDown { pos, button } => UiEvent::MouseDown {
                pos: *pos,
                button: *button,
            },
            RpcEvent::MouseUp { pos, button } => UiEvent::MouseUp {
                pos: *pos,
                button: *button,
            },
            RpcEvent::MouseMove { pos } => UiEvent::MouseMove { pos: *pos },
            RpcEvent::Wheel { pos, delta } => UiEvent::Wheel {
                delta: *delta,
                pos: *pos,
            },
        }
    }
}
impl RpcEvent {
    /// 解析 `{"type":"text","text":"ls\r"}` 这类描述。
    pub fn from_json(value: &Json) -> Result<Self, String> {
        let kind = value
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or("event.type is required")?;
        match kind {
            "text" => Ok(RpcEvent::Text(
                value
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("text event requires text")?
                    .to_string(),
            )),
            "compose" => Ok(RpcEvent::Compose(
                value
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            )),
            "mouse_down" | "mouse_up" | "mouse_move" => {
                let pos = y901_ui::Pos::new(
                    value.get("x").and_then(|v| v.as_f64()).ok_or("mouse event requires x")? as f32,
                    value.get("y").and_then(|v| v.as_f64()).ok_or("mouse event requires y")? as f32,
                );
                let button = match value.get("button").and_then(|v| v.as_str()) {
                    Some("right") => y901_ui::MouseButton::Right,
                    Some("middle") => y901_ui::MouseButton::Middle,
                    _ => y901_ui::MouseButton::Left,
                };
                match kind {
                    "mouse_down" => Ok(RpcEvent::MouseDown { pos, button }),
                    "mouse_up" => Ok(RpcEvent::MouseUp { pos, button }),
                    _ => Ok(RpcEvent::MouseMove { pos }),
                }
            }
            "wheel" => {
                let pos = y901_ui::Pos::new(
                    value.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                    value.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                );
                let delta = value.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                Ok(RpcEvent::Wheel { pos, delta })
            }
            "key" => {
                let name = value
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or("key event requires key")?;
                let key = match name {
                    "enter" => y901_ui::Key::Enter,
                    "tab" => y901_ui::Key::Tab,
                    "backspace" => y901_ui::Key::Backspace,
                    "escape" => y901_ui::Key::Escape,
                    "up" => y901_ui::Key::Up,
                    "down" => y901_ui::Key::Down,
                    "left" => y901_ui::Key::Left,
                    "right" => y901_ui::Key::Right,
                    other => return Err(format!("unknown key: {other}")),
                };
                Ok(RpcEvent::Key(key))
            }
            other => Err(format!("unknown event type: {other}")),
        }
    }
}

/// 一条待处理请求：UI 线程处理完把 JSON 塞进 `reply`。
pub struct Pending {
    pub request: RpcRequest,
    pub id: Json,
    pub reply: Sender<Result<Json, String>>,
}

/// 启动 RPC 服务。
///
/// 返回实际监听地址（端口传 0 时可拿到系统分配端口，测试用）。
/// 推送总线：**UI 线程只投递字符串，绝不碰 socket**。
///
/// - 有界队列（`std::sync::mpsc::sync_channel`），`send` 用 `try_send`：
///   队列满时直接丢弃并计数，UI 线程永远不会被慢客户端拖住。
/// - 后台线程独占连接池，负责 `writeln!` + `flush`，写失败的连接自动剔��。
#[derive(Clone)]
pub struct Broadcaster {
    tx: std::sync::mpsc::SyncSender<String>,
    conns: Arc<Mutex<Vec<Arc<Mutex<TcpStream>>>>>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl Broadcaster {
    /// 建一个总线（不启动线程）。返回接收端，方便测试验证"投递不阻塞"。
    pub fn new(capacity: usize) -> (Self, std::sync::mpsc::Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(capacity);
        (
            Self {
                tx,
                conns: Arc::new(Mutex::new(Vec::new())),
                dropped: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            },
            rx,
        )
    }

    /// 投递一行 JSON。**永不阻塞**：满了就丢，返回是否入队。
    pub fn send(&self, line: &str) -> bool {
        use std::sync::mpsc::TrySendError;
        match self.tx.try_send(line.to_string()) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.dropped
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                false
            }
        }
    }

    /// 因队列满而丢弃的消息数（诊断）。
    pub fn dropped(&self) -> u64 {
        self.dropped.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn connection_count(&self) -> usize {
        self.conns.lock().map(|c| c.len()).unwrap_or(0)
    }

    fn register(&self, stream: Arc<Mutex<TcpStream>>) {
        if let Ok(mut conns) = self.conns.lock() {
            conns.push(stream);
        }
    }

    /// 后台写线程：从队列取数据，串行写给所有连接。
    fn spawn_writer(self, rx: std::sync::mpsc::Receiver<String>) {
        thread::Builder::new()
            .name("y901-rpc-push".to_string())
            .spawn(move || {
                while let Ok(line) = rx.recv() {
                    let mut conns = match self.conns.lock() {
                        Ok(c) => c,
                        Err(_) => break,
                    };
                    conns.retain(|conn| {
                        let mut stream = match conn.lock() {
                            Ok(s) => s,
                            Err(_) => return false,
                        };
                        writeln!(stream, "{line}").is_ok() && stream.flush().is_ok()
                    });
                }
            })
            .ok();
    }
}

/// 启动 RPC 服务。
///
/// 返回 `(监听地址, 推送总线)`：端口传 0 可拿系统分配端口（测试用）。
pub fn spawn(addr: &str, tx: Sender<Pending>) -> std::io::Result<(SocketAddr, Broadcaster)> {
    let token = std::env::var("Y901_TOKEN").unwrap_or_default();
    spawn_with_token(addr, tx, token)
}

/// 显式指定 Token（测试用，避免依赖全局 env）。
pub fn spawn_with_token(
    addr: &str,
    tx: Sender<Pending>,
    token: String,
) -> std::io::Result<(SocketAddr, Broadcaster)> {
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;

    let (broadcaster, push_rx) = Broadcaster::new(1024);
    broadcaster.clone().spawn_writer(push_rx);

    let hub = broadcaster.clone();
    let token_for_conn = token;
    thread::Builder::new()
        .name("y901-rpc".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        let hub = hub.clone();
                        let token = token_for_conn.clone();
                        thread::Builder::new()
                            .name("y901-rpc-conn".to_string())
                            .spawn(move || serve(stream, tx, hub, token))
                            .ok();
                    }
                    Err(err) => {
                        eprintln!("[rpc] accept failed: {err}");
                        break;
                    }
                }
            }
        })
        .ok();

    Ok((local, broadcaster))
}

fn serve(stream: TcpStream, tx: Sender<Pending>, hub: Broadcaster, expected: String) {
    let reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };
    // 写端共享：既能回包，也能被推送线程使用
    let writer = Arc::new(Mutex::new(stream));
    hub.register(writer.clone());

    // 每请求鉴权：Token 非空时，所有请求必须携带 params.token

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let value = match Json::parse(&line) {
            Ok(v) => v,
            Err(err) => {
                write_line(&writer, &error_response(Json::Null, &format!("parse error: {err}")));
                continue;
            }
        };

        if !expected.is_empty() {
            let presented = value
                .get("params")
                .and_then(|p| p.get("token"))
                .and_then(|t| t.as_str());
            if presented != Some(expected.as_str()) {
                let id = value.get("id").cloned().unwrap_or(Json::Null);
                let mut body = error_response(id, "unauthorized: invalid or missing token");
                body = body.set("status", 401u16).set("code", "unauthorized");
                write_line(&writer, &body);
                break; // 鉴权失败直接断开，不给穷举机会
            }
        }

        let response = handle(value, &tx);
        if !write_line(&writer, &response) {
            break;
        }
    }
}

/// 写一行并 flush；返回是否成功。
fn write_line(writer: &Arc<Mutex<TcpStream>>, response: &Json) -> bool {
    let Ok(mut w) = writer.lock() else {
        return false;
    };
    writeln!(w, "{response}").is_ok() && w.flush().is_ok()
}

fn handle(value: Json, tx: &Sender<Pending>) -> Json {
    let id = value.get("id").cloned().unwrap_or(Json::Null);
    let method = match value.get("method").and_then(|m| m.as_str()) {
        Some(m) => m.to_string(),
        None => return error_response(id, "missing method"),
    };

    let params = value.get("params").cloned().unwrap_or_else(Json::obj);

    let request = match method.as_str() {
        "get_tree" => RpcRequest::GetTree,
        "ping" => RpcRequest::Ping,
        "dispatch" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "dispatch requires id"),
            };
            let event = match params.get("event").map(RpcEvent::from_json) {
                Some(Ok(ev)) => ev,
                Some(Err(err)) => return error_response(id, &err),
                None => return error_response(id, "dispatch requires event"),
            };
            RpcRequest::Dispatch { node, event }
        }
        "invoke" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "invoke requires id"),
            };
            let action = match params.get("action").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "invoke requires action"),
            };
            let args = params.get("args").cloned().unwrap_or_else(Json::obj);
            RpcRequest::Invoke {
                node,
                action,
                args,
            }
        }
        "query" => {
            let q = match params.get("q").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "query requires q"),
            };
            RpcRequest::Query { q }
        }
        "subscribe" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "subscribe requires id"),
            };
            let interval_ms = params
                .get("interval_ms")
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0) as u64;
            RpcRequest::Subscribe { node, interval_ms }
        }
        "unsubscribe" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "unsubscribe requires id"),
            };
            RpcRequest::Unsubscribe { node }
        }
        "focus" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return error_response(id, "focus requires id"),
            };
            RpcRequest::Focus { node }
        }
        other => RpcRequest::Unknown {
            method: other.to_string(),
        },
    };

    let (reply_tx, reply_rx) = channel();
    let pending = Pending {
        request,
        id: id.clone(),
        reply: reply_tx,
    };
    if tx.send(pending).is_err() {
        return error_response(id, "ui thread is gone");
    }

    match reply_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(result)) => Json::obj()
            .set("id", id)
            .set("ok", true)
            .set("result", result),
        Ok(Err(err)) => error_response(id, &err),
        Err(_) => error_response(id, "timeout waiting for ui thread"),
    }
}

fn error_response(id: Json, message: &str) -> Json {
    Json::obj()
        .set("id", id)
        .set("ok", false)
        .set("error", message)
}

/// UI 线程侧：把请求处理结果写回。
pub fn reply_to(pending: Pending, result: Result<Json, String>) {
    let _ = pending.reply.send(result);
}

/// 便捷类型：UI 线程持有的接收端。
pub type RpcReceiver = Receiver<Pending>;
