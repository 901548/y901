//! Agent-Native RPC：本地 TCP + 行分隔 JSON。
//!
//! 线程模型：
//!   RPC 线程只做「收字节 → 解析 → 转交 → 回写」，绝不碰 UI 状态。
//!   UI 线程在自己的循环里 `drain` 请求，用同一份语义树处理后回包。
//!
//! 这样框架依旧单线程独占 UiTree，RPC 只是又一个输入源。

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender};
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

/// 默认监听地址。只绑回环：这是本机 Agent 通道，不应暴露到网络。
pub const DEFAULT_ADDR: &str = "127.0.0.1:9410";

/// 解析后的请求（已去掉传输层细节）。
#[derive(Clone, Debug, PartialEq)]
pub enum RpcRequest {
    /// 返回整棵语义树 + 每个节点的矩形。
    GetTree,
    /// 向目标节点投递一条事件（事件类型由协议描述，框架不理解语义）。
    Dispatch {
        /// 焦点所在节点 id。
        node: String,
        /// 具体事件。
        event: RpcEvent,
    },
    /// 向目标节点投递一个动作（等价于一条 `UiEvent::Action`）。
    Invoke {
        /// 触发动作的节点 id。
        node: String,
        /// 动作名（组件自定义）。
        action: String,
        /// 动作参数。
        args: Json,
    },
    /// 按查询语言筛选节点（与 MCP 的 ui_query 完全一致）。
    Query {
        /// 检索串。
        q: String,
    },
    /// 切换焦点。
    Focus {
        /// 订阅的根节点 id。
        node: String,
    },
    /// 订阅某个节点的状态变化（服务端主动推送 `state_changed`）。
    Subscribe {
        /// 要退订的节点 id；与订阅时一致。
        node: String,
        /// 最小推送间隔（毫秒）。为什么要节流：语义树变化可能每帧都发生，不节流会把订阅端淹掉。
        interval_ms: u64,
    },
    /// 取消订阅。
    Unsubscribe {
        /// 目标节点 id。
        node: String,
    },
    /// 健康检查。
    Ping,
    /// 无法识别的方法名。单独一个变体而不是直接报错：这样服务端能回标准的 "method not found"，而不是连接断开。
    Unknown {
        /// 原始的未知方法名（回给调用方，便于排查）。
        method: String,
    },
}

/// RPC 可以构造的事件。刻意保持极简：只覆盖"外部真的会发"的那几种。
#[derive(Clone, Debug, PartialEq)]
pub enum RpcEvent {
    /// 输入一段文本（等价于用户打字）：走 `UiEvent::Text`。
    Text(String),
    /// 按一个键（无修饰键）：走 `UiEvent::Key`。
    Key(y901_ui::Key),
    /// 输入法预编辑串：只走覆盖层，**不进网格**——提交与否由后续的 `Text` 决定。
    Compose(String),
    /// 在指定位置按下鼠标。坐标是客户区逻辑像素。
    MouseDown {
        /// 客户区坐标。
        pos: y901_ui::Pos,
        /// 哪个键被按下。
        button: y901_ui::MouseButton,
    },
    /// 抬起鼠标。
    MouseUp {
        /// 客户区坐标。
        pos: y901_ui::Pos,
        /// 哪个键被抬起。
        button: y901_ui::MouseButton,
    },
    /// 移动鼠标（会驱动 hover 与拖拽）。
    MouseMove {
        /// 客户区坐标。
        pos: y901_ui::Pos,
    },
    /// 滚轮。
    Wheel {
        /// 滚轮发生的位置（决定滚哪个容器）。
        pos: y901_ui::Pos,
        /// 滚动量；正数向上。
        delta: f32,
    },
}

/// `Key` -> 线上字符串（与 `from_json` 的解析表一一对应）。
///
/// 这是**唯一的序列化入口**：`to_json` 只能用它，避免两处各写一份映射而漂移。
pub fn key_to_wire(key: y901_ui::Key) -> &'static str {
    use y901_ui::Key;
    match key {
        Key::Enter => "enter",
        Key::Tab => "tab",
        Key::Backspace => "backspace",
        Key::Escape => "escape",
        Key::Up => "up",
        Key::Down => "down",
        Key::Left => "left",
        Key::Right => "right",
        Key::Delete => "delete",
        Key::Home => "home",
        Key::End => "end",
        Key::PageUp => "page_up",
        Key::PageDown => "page_down",
        // 以下三类无法用名称表达（依赖具体字符/编号），序列化后 `from_json` 会拒绝：
        // 这是有意的单向映射——RPC 的键盘注入只支持具名键。
        // 不可达：`to_json` 对 `Key::Char` 走独立的 `type: "char"` 分支，不会调用本函数。
        // 保留 arm 只为满足穷尽性；若真的走到这里说明有调用方漏判。
        Key::Char(_) => {
            debug_assert!(false, "Key::Char 不应走 key_to_wire，请走 type=char 分支");
            "char"
        }
        Key::Function(_) => "function",
        Key::Unknown => "unknown",
    }
}
impl RpcEvent {
    /// 还原成协议 JSON（RPC 转 gateway 时用，保证与 MCP 走同一套解析）。
    pub fn to_json(&self) -> Json {
        use y901_ui::UiEvent;
        let _: Option<UiEvent> = None;
        match self {
            RpcEvent::Text(t) => Json::obj().set("type", "text").set("text", t.clone()),
            RpcEvent::Key(k) => match k {
                // 字符键用码点表示：`{"type":"char","code":20013}`。
                // 与 `Text`（输入一串文本、走 IME）语义不同，故独立 type。
                y901_ui::Key::Char(ch) => Json::obj().set("type", "char").set("code", *ch as u32),
                other => Json::obj()
                    .set("type", "key")
                    // P0-4：必须与 from_json 的解析表一致（小写），
                    // 否则 to_json -> from_json 回环失败，RPC 键盘注入实际不可用。
                    .set("key", key_to_wire(*other)),
            },
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
                mods: y901_ui::Modifiers::default(),
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
                    value
                        .get("x")
                        .and_then(|v| v.as_f64())
                        .ok_or("mouse event requires x")? as f32,
                    value
                        .get("y")
                        .and_then(|v| v.as_f64())
                        .ok_or("mouse event requires y")? as f32,
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
            // 字符键：`{"type":"char","code":<码点>}`。只接受 code，拒绝 text 字段
            // （text 与 RpcEvent::Text 语义重叠，会造成"按字符键"与"输入文本"混淆）。
            "char" => {
                let code = value.get("code").ok_or("char event requires code")?;
                let v = match code {
                    Json::Num(v) => *v,
                    other => {
                        return Err(format!("code must be a number, got {other:?}"));
                    }
                };
                // 严格校验：有限、非负、整数、落在 u32 范围
                if !v.is_finite() || v < 0.0 || v.fract() != 0.0 || v > u32::MAX as f64 {
                    return Err(format!(
                        "code {v} 不是合法字符（代理区 0xD800-0xDFFF 或 > 0x10FFFF）"
                    ));
                }
                let ch = char::from_u32(v as u32).ok_or_else(|| {
                    format!("code {v} 不是合法字符（代理区 0xD800-0xDFFF 或 > 0x10FFFF）")
                })?;
                Ok(RpcEvent::Key(y901_ui::Key::Char(ch)))
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
                    "delete" => y901_ui::Key::Delete,
                    "home" => y901_ui::Key::Home,
                    "end" => y901_ui::Key::End,
                    "page_up" => y901_ui::Key::PageUp,
                    "page_down" => y901_ui::Key::PageDown,
                    other => return Err(format!("unknown key: {other}")),
                };
                Ok(RpcEvent::Key(key))
            }
            other => Err(format!("unknown event type: {other}")),
        }
    }
}

/// 一条待处理请求。
///
/// **`writer_tx` 是该连接的唯一写路径**：回包与推送都进同一个有界 FIFO 队列，
/// 由该连接的写线程按入队顺序写出。UI 线程只做 `send`/`send_timeout`，永不碰 socket。
/// 早期实现里回包与推送由两个线程分别写 socket，先后顺序不受控，
/// 曾导致客户端先收到 `state_changed`、后收到 `subscribe` 的 ack。
pub struct Pending {
    /// 已解析的请求。
    pub request: RpcRequest,
    /// JSON-RPC 请求 id，原样回写响应，用于把响应与请求配对。
    pub id: Json,
    /// 该连接的写队列。
    pub writer_tx: std::sync::mpsc::SyncSender<WriteJob>,
    /// 该连接是否已被标记为"废"（写队列溢出）。
    ///
    /// `reply_to` 开头先看它：为真直接返回，**连 `try_send` 都不做**，
    /// 保证 UI 线程零阻塞。设置的唯一时机是"第一次 `try_send` 失败"。
    pub dead: Arc<AtomicBool>,
}

/// 写线程的任务：回包与推送共用一条队列，保存入队顺序。
pub enum WriteJob {
    /// 回包：**不允许丢**（用阻塞 `send` + 超时）。
    Reply(String),
    /// 推送：可丢（用 `try_send`，满了丢弃并计数）。
    Push(String),
}

/// 每条连接的写队列容量。
///
/// 取 1024：足够吸收一次突发（一帧多条 state_changed），
/// 又能在客户端不读时尽快让 `try_send` 失败、推送被丢弃而不是堆积内存。
const WRITE_QUEUE: usize = 1024;

/// UI 线程向连接写队列投递回包时的最长等待。
///
/// B+ 之后 UI 线程**不再等待**：`try_send` 失败即标记 `dead` 并返回。
/// 保留这个常量只作为写线程的写超时（同值），语义已从"UI 等待"变为"写线程周期"。
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// 一条连接的注册项：写队列 + 判死标志。
type ConnSlot = (std::sync::mpsc::SyncSender<WriteJob>, Arc<AtomicBool>);

/// 推送总线：**UI 线程只投递字符串，绝不碰 socket**。
///
/// 每个连接一条有界队列；推送走 `try_send`（满了丢 + 计数），绝不阻塞 UI 线程。
#[derive(Clone)]
pub struct Broadcaster {
    senders: Arc<Mutex<Vec<ConnSlot>>>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
}

impl Broadcaster {
    /// 建一个总线（不启动线程）。
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(Vec::new())),
            dropped: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// 投递一行 JSON 到所有连接。**永不阻塞**：满了就丢，返回是否全部入队。
    pub fn send(&self, line: &str) -> bool {
        use std::sync::mpsc::TrySendError;
        let mut all = true;
        let mut senders = match self.senders.lock() {
            Ok(s) => s,
            Err(_) => return false,
        };
        senders.retain(
            |(tx, dead)| match tx.try_send(WriteJob::Push(line.to_string())) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    // 推送可丢：**不**因此踢连接（丢推送是正常背压行为）
                    self.dropped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    all = false;
                    true
                }
                Err(TrySendError::Disconnected(_)) => {
                    dead.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.dropped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    all = false;
                    false // 写线程已退出，注销该连接
                }
            },
        );
        all
    }

    /// 因队列满/断开而丢弃的消息数（诊断）。
    pub fn dropped(&self) -> u64 {
        self.dropped.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 当前在册连接数（诊断用）。
    ///
    /// 返回：`senders` 表长度。无副作用。
    /// 注意：清理是**惰性**的——只有下一次 `send()` 发现 `Disconnected` 才会移除，
    /// 因此长时间没有推送时读数会偏高（不影响正确性）。
    pub fn connection_count(&self) -> usize {
        self.senders.lock().map(|s| s.len()).unwrap_or(0)
    }

    /// 连接数告警阈值：超过时打一行日志（不做硬上限，避免误拒真用户）。
    const WARN_CONNECTIONS: usize = 64;

    fn register(&self, tx: std::sync::mpsc::SyncSender<WriteJob>, dead: Arc<AtomicBool>) {
        if let Ok(mut senders) = self.senders.lock() {
            senders.push((tx, dead));
            if senders.len() > Self::WARN_CONNECTIONS {
                eprintln!(
                    "[y901] RPC 连接数 {} 超过告警阈值 {}",
                    senders.len(),
                    Self::WARN_CONNECTIONS
                );
            }
        }
    }
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// 给连接设置写超时。
///
/// 单独抽出来是为了**可测**：测试可以建一对 socket、调用它、
/// 再用 `TcpStream::write_timeout()` 读回实际值（行为断言，不靠 grep 源码）。
///
/// 失败：仅当系统调用失败时返回 `io::Error`（极少见）。
pub fn configure_stream(stream: &TcpStream) -> std::io::Result<()> {
    stream.set_write_timeout(Some(WRITE_TIMEOUT))
}

/// 写线程主体：从队列取任务，按 FIFO 写 socket。
///
/// **写超时由 `setup_connection` 在此之前设置**：没有它，客户端不读时
/// `writeln!` 永久阻塞，线程既写不出这一条、也永远检查不到 `dead`，
/// 整个 B+ 设计会退化成"静默丢包"。
///
/// 退出条件：
/// 1. `recv` 返回 Err（所有 Sender 被 drop）；
/// 2. 写失败（超时 / EPIPE）；
/// 3. 读完一条后发现 `dead` 为真 → `shutdown` 并退出。
fn writer_loop(
    mut stream: TcpStream,
    rx: std::sync::mpsc::Receiver<WriteJob>,
    dead: Arc<AtomicBool>,
) {
    loop {
        if dead.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            break;
        }
        let Ok(job) = rx.recv() else { break };
        let line = match job {
            WriteJob::Reply(s) | WriteJob::Push(s) => s,
        };
        if writeln!(stream, "{line}").is_err() || stream.flush().is_err() {
            dead.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = stream.shutdown(std::net::Shutdown::Both);
            break;
        }
    }
}

/// 为一条连接建立写队列与写线程。
///
/// 输入：已 accept 的 `TcpStream`（**消耗所有权**）。
/// 返回：`(写队列, 判死标志, 线程句柄)`。
///
/// 一条连接的写端三件套：队列、判死标志、写线程句柄。
type ConnectionWriter = (
    std::sync::mpsc::SyncSender<WriteJob>,
    Arc<AtomicBool>,
    std::thread::JoinHandle<()>,
);

/// 失败：`set_write_timeout` 失败，或写线程创建失败（都极罕见）。
/// **失败时调用方必须关闭连接、且不要注册到 Broadcaster**——
/// 否则会出现"连接在册但没有写线程"的僵尸：队列慢慢满、推送与回包全丢、
/// 而连接永远不断开（B+ 的"快速失败"保证失效）。
fn setup_connection(stream: TcpStream) -> Result<ConnectionWriter, String> {
    configure_stream(&stream).map_err(|e| format!("set_write_timeout 失败: {e}"))?;
    let (tx, rx) = std::sync::mpsc::sync_channel::<WriteJob>(WRITE_QUEUE);
    let dead = Arc::new(AtomicBool::new(false));
    let dead_for_thread = dead.clone();
    let handle = thread::Builder::new()
        .name("y901-rpc-conn-writer".to_string())
        .spawn(move || writer_loop(stream, rx, dead_for_thread))
        .map_err(|e| format!("写线程创建失败: {e}"))?;
    Ok((tx, dead, handle))
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

    let broadcaster = Broadcaster::new();
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
    // 读端先克隆（写端所有权要交给 setup_connection）
    let reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(_) => return,
    };
    // 每连接一个写线程：回包与推送共用同一条有界 FIFO 队列。
    // UI 线程只 send，永不碰 socket —— 客户端不读时阻塞的是写线程，不是 UI。
    //
    // 失败时**必须**关闭连接且不注册：否则会留下"在册但没有写线程"的僵尸连接。
    // `stream` 已被 setup_connection 消耗并在失败分支 drop → 内核发 FIN，连接自然关闭。
    let (job_tx, dead, _writer_handle) = match setup_connection(stream) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("[y901] 连接初始化失败，已关闭: {err}");
            return;
        }
    };
    hub.register(job_tx.clone(), dead.clone());

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
                let _ = job_tx.send(WriteJob::Reply(
                    error_response(Json::Null, &format!("parse error: {err}")).to_string(),
                ));
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
                let body = error_response(id, "unauthorized: invalid or missing token")
                    .set("status", 401u16)
                    .set("code", "unauthorized");
                let _ = job_tx.send(WriteJob::Reply(body.to_string()));
                break; // 鉴权失败直接断开，不给穷举机会
            }
        }

        let response = handle(value, &tx, &job_tx, &dead);
        // `Deferred` 表示请求已转交 UI 线程，回包由 `reply_to` 走同一队列写出
        if let Handled::Sync(response) = response {
            let _ = job_tx.send(WriteJob::Reply(response.to_string()));
        }
    }
}

/// `handle` 的去向：同步回包，或已转交 UI 线程（回包稍后由 `reply_to` 写）。
///
/// 用枚举替代早期的 `Json::Null` 哨兵：类型系统强制调用方区分两种情况，
/// 忘记处理会编译失败而不是静默丢回包。
enum Handled {
    Sync(Json),
    Deferred,
}

/// 同步错误的简写，保持 `handle` 内的 `return sync_err(..)` 与原签名同形。
fn sync_err(id: Json, message: &str) -> Handled {
    Handled::Sync(error_response(id, message))
}

fn handle(
    value: Json,
    tx: &Sender<Pending>,
    writer_tx: &std::sync::mpsc::SyncSender<WriteJob>,
    dead: &Arc<AtomicBool>,
) -> Handled {
    let id = value.get("id").cloned().unwrap_or(Json::Null);
    let method = match value.get("method").and_then(|m| m.as_str()) {
        Some(m) => m.to_string(),
        None => return sync_err(id, "missing method"),
    };

    let params = value.get("params").cloned().unwrap_or_else(Json::obj);

    let request = match method.as_str() {
        "get_tree" => RpcRequest::GetTree,
        "ping" => RpcRequest::Ping,
        "dispatch" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return sync_err(id, "dispatch requires id"),
            };
            let event = match params.get("event").map(RpcEvent::from_json) {
                Some(Ok(ev)) => ev,
                Some(Err(err)) => return sync_err(id, &err),
                None => return sync_err(id, "dispatch requires event"),
            };
            RpcRequest::Dispatch { node, event }
        }
        "invoke" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return sync_err(id, "invoke requires id"),
            };
            let action = match params.get("action").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return sync_err(id, "invoke requires action"),
            };
            let args = params.get("args").cloned().unwrap_or_else(Json::obj);
            RpcRequest::Invoke { node, action, args }
        }
        "query" => {
            let q = match params.get("q").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return sync_err(id, "query requires q"),
            };
            RpcRequest::Query { q }
        }
        "subscribe" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return sync_err(id, "subscribe requires id"),
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
                None => return sync_err(id, "unsubscribe requires id"),
            };
            RpcRequest::Unsubscribe { node }
        }
        "focus" => {
            let node = match params.get("id").and_then(|v| v.as_str()) {
                Some(v) => v.to_string(),
                None => return sync_err(id, "focus requires id"),
            };
            RpcRequest::Focus { node }
        }
        other => RpcRequest::Unknown {
            method: other.to_string(),
        },
    };

    let pending = Pending {
        request,
        id: id.clone(),
        writer_tx: writer_tx.clone(),
        dead: dead.clone(),
    };
    if tx.send(pending).is_err() {
        return sync_err(id, "ui thread is gone");
    }
    // 已转交 UI 线程：回包由 reply_to 投递到同一条写队列
    Handled::Deferred
}

fn error_response(id: Json, message: &str) -> Json {
    Json::obj()
        .set("id", id)
        .set("ok", false)
        .set("error", message)
}

/// UI 线程侧：把请求处理结果投递到该连接的写队列。
///
/// **零阻塞**（B+）：开头先看 `dead`，为真直接返回；否则 `try_send` 一次，
/// 失败即把连接标记为 `dead` 并返回，不再重试、不再睡眠。
/// 写线程醒来看到 `dead` 会 `shutdown`，客户端立刻收到 EOF —— 快速失败。
///
/// 顺序由队列保证：本函数入队后，调用方才会 `hub.send` 推送，
/// 所以客户端一定先收到 ack、再收到该请求触发的推送。
pub fn reply_to(pending: Pending, result: Result<Json, String>) -> Result<(), String> {
    if pending.dead.load(std::sync::atomic::Ordering::Relaxed) {
        return Err("连接已被标记断开".to_string());
    }
    let body = match result {
        Ok(result) => Json::obj()
            .set("id", pending.id.clone())
            .set("ok", true)
            .set("result", result),
        Err(err) => error_response(pending.id.clone(), &err),
    };
    match pending
        .writer_tx
        .try_send(WriteJob::Reply(body.to_string()))
    {
        Ok(()) => Ok(()),
        Err(_) => {
            // 第一次失败就判死：队列深度 1024，能填满说明写线程卡住或客户端不读，
            // 这个连接已经废了。"连续 N 次才判死"只会延长"部分成功部分丢失"的窗口。
            pending
                .dead
                .store(true, std::sync::atomic::Ordering::Relaxed);
            eprintln!("[y901] RPC 回包入队失败，标记连接断开");
            Err("写队列已满，连接已标记断开".to_string())
        }
    }
}

/// 便捷类型：UI 线程持有的接收端。
pub type RpcReceiver = Receiver<Pending>;

#[cfg(test)]
mod connection_setup_tests {
    /// 结构断言：`setup_connection` 里的 `configure_stream` 必须用 `?` 传播，
    /// 不能退回 `let _ =`（后者会让连接在没有写超时的情况下运行）。
    #[test]
    fn setup_connection_propagates_write_timeout_failure() {
        let src = include_str!("rpc.rs");
        let start = src
            .find("fn setup_connection")
            .expect("setup_connection 存在");
        let rest = &src[start..];
        let end = rest[1..].find("\n}").map(|i| i + 1).unwrap_or(rest.len());
        let body = &rest[..end];
        assert!(
            body.contains("configure_stream(&stream).map_err") && body.contains("?"),
            "写超时失败必须用 `?` 传播（不能 let _ 忽略），body:\n{body}"
        );
        assert!(
            !body.contains(&format!("let {} = configure_stream", "_")),
            "不得忽略 configure_stream 的失败"
        );
    }

    /// 结构断言：写线程创建失败必须用 `?` 传播，不能 `.ok()` / `.is_err()` 忽略。
    #[test]
    fn setup_connection_propagates_spawn_failure() {
        let src = include_str!("rpc.rs");
        let start = src
            .find("fn setup_connection")
            .expect("setup_connection 存在");
        let rest = &src[start..];
        let end = rest[1..].find("\n}").map(|i| i + 1).unwrap_or(rest.len());
        let body = &rest[..end];
        assert!(
            body.contains(".spawn(") && body.contains(".map_err(|e| format!(\"写线程创建失败"),
            "写线程创建失败必须用 `?` 传播，body:\n{body}"
        );
        assert!(
            !body.contains(&format!("let {} = thread::Builder", "_")),
            "不得忽略线程创建失败"
        );
    }
}
