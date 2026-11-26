//! 与 X server 的**连接与握手**（S2 第二步）。
//!
//! 只做到"能连、��能握手、能发请求、能收事件/错误"。
//! 不做键盘映射、不做 IME、不做 SHM——那些在后面的步骤里。
//!
//! ## 一个必须记住的约束
//!
//! X11 是**异步协议**：`write` 成功不代表服务端接受了请求。
//! 错误（`BadLength` / `BadValue` / `BadWindow`）是**作为事件回来的**。
//! 想确认"刚刚那批请求都没错"，必须发一个**带响应的请求**（如 `GetInputFocus`）
//! 并读到它的响应——响应之前出现的任何错误包都属于那批请求。

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::decode::{decode_event, X11Event};
use crate::encode::{encode_get_input_focus, encode_put_image, encode_setup_request, PutImageArgs};
use crate::randr::{OutputDpi, EXTENSION_NAME, REQUIRED_MAJOR, REQUIRED_MINOR};

/// `PutImage` 单包数据上限（字节）：`length` 是 u16，单位 4 字节，去掉 6 个单位的头。
pub const PUT_IMAGE_MAX_BYTES: usize = (u16::MAX as usize - 6) * 4;

/// 一个错误包（X11 error，32 字节，字节 0 = 0）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X11Error {
    /// 错误码（`1=BadRequest, 2=BadValue, 3=BadWindow, 16=BadLength`）
    pub code: u8,
    /// 出错的序列号
    pub sequence: u16,
    /// 附加数据（请求特定）
    pub bad_value: u32,
    /// 小有效位
    pub minor_opcode: u16,
    /// 大有效位
    pub major_opcode: u8,
}

impl X11Error {
    /// 错误码的人类可读名称。
    pub fn name(&self) -> &'static str {
        match self.code {
            1 => "BadRequest",
            2 => "BadValue",
            3 => "BadWindow",
            4 => "BadPixmap",
            5 => "BadAtom",
            8 => "BadMatch",
            9 => "BadDrawable",
            10 => "BadAccess",
            11 => "BadAlloc",
            12 => "BadColormap",
            13 => "BadGContext",
            14 => "BadIDChoice",
            16 => "BadLength",
            17 => "BadImplementation",
            _ => "Unknown",
        }
    }
}

impl std::fmt::Display for X11Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}({}) seq={} major={} minor={} bad_value={:#x}",
            self.name(),
            self.code,
            self.sequence,
            self.major_opcode,
            self.minor_opcode,
            self.bad_value
        )
    }
}

/// 服务端握手后告诉我们的信息。
#[derive(Clone, Debug, Default)]
pub struct ServerInfo {
    /// 服务端发行号
    pub release: u32,
    /// 资源 ID 基域
    pub resource_id_base: u32,
    /// 资源 ID 掩码
    pub resource_id_mask: u32,
    /// 服务端名称（如 "The X.Org Foundation"）
    pub vendor: String,
    /// 根窗口
    pub root: u32,
    /// 屏幕宽（像素）
    pub width: u16,
    /// 屏幕高（像素）
    pub height: u16,
    /// 屏幕宽（毫米）—— 服务器按 EDID/配置报，用于推 DPI；0 = 未提供。
    pub width_mm: u16,
    /// 屏幕高（毫米）
    pub height_mm: u16,
    /// 默认可视化类型（画窗口时要用）
    pub root_visual: u32,
    /// 根窗口深度（通常 24）
    pub root_depth: u8,
    /// 最小键码
    pub min_keycode: u8,
    /// 最大键码
    pub max_keycode: u8,
}

/// 一个已连接的 X11 客户端。
pub struct Connection {
    stream: UnixStream,
    seq: u16,
    info: ServerInfo,
    /// 读到但尚未交付的事件（键盘之前可能混着 Expose）
    pending: Vec<X11Event>,
    /// **尚未凑满 32 字节的半个包**。
    ///
    /// 为什么必须有它：X11 事件包固定 32 字节，但 `read(2)` 只保证"至少读到 1 字节"。
    /// 旧实现在读到 `n < 32` 时直接报错并**丢弃这 n 字节** —— 之后每个包都从错位处
    /// 解析（表现为"界面卡死/乱事件"），而且因为读的是同一个流，错误会永久保留。
    ///
    /// 现在的约定：**任何读取路径都先往 `rx` 里补字节，凑满 32 才消费**；
    /// 非阻塞/超时只表示"暂时没有更多字节"，`rx` 里的部分数据必须留着等下次。
    rx: [u8; 32],
    /// `rx` 里已经填了多少字节（`0..=32`；等于 32 表示"有一个完整包待取"）。
    rx_filled: usize,
    /// **未读完的 reply 附加数据**（`length × 4` 字节）。
    ///
    /// 为什么不能像旧实现那样读进局部 `Vec`：`read_exact` 超时/`WouldBlock` 时
    /// 已消费的字节会随局部变量一起丢弃 —— 而它们**已经从 socket 读走了**，
    /// 流从此永久错位。所以附加数据也必须有跨调用的归属。
    tail: Vec<u8>,
    /// 本段附加数据的目标字节数（`0` = 当前没有待读的附加数据）。
    tail_needed: usize,
    /// 已经读到、但附加数据还没跟上来的包头（重试时接着用，不重读）。
    pending_head: Option<[u8; 32]>,
    /// 阻塞读取路径（reply / 附加数据）的总等待上限。
    ///
    /// 做成字段而不是常量：测试需要把 5s 缩短到几十毫秒来验证"超时后仍可续读"。
    reply_timeout: Duration,
}

impl Connection {
    /// **仅测试用**：用一个现成的 socket 构造 `Connection`，不做握手。
    ///
    /// 用途：用 `UnixStream::pair()` 灌入构造好的包，
    /// 验证读包对齐逻辑（drain 附加数据、不丢事件）——
    /// 这些逻辑在真实 X 会话里难以稳定制造。
    #[cfg(test)]
    pub(crate) fn from_stream(stream: std::os::unix::net::UnixStream) -> Self {
        Self {
            stream,
            seq: 0,
            info: ServerInfo {
                release: 0,
                resource_id_base: 0,
                resource_id_mask: 0,
                vendor: String::new(),
                root: 0,
                width_mm: 0,
                height_mm: 0,
                width: 0,
                height: 0,
                root_visual: 0,
                root_depth: 24,
                min_keycode: 0,
                max_keycode: 0,
            },
            pending: Vec::new(),
            rx: [0u8; 32],
            rx_filled: 0,
            tail: Vec::new(),
            tail_needed: 0,
            pending_head: None,
            reply_timeout: DEFAULT_REPLY_TIMEOUT,
        }
    }
    /// 连接到本地 X server 并完成握手。
    ///
    /// 输入：`display`（如 `":99"` 或 `":0"`；也接受 `"环境变量"`的空值→读 `DISPLAY`）。
    /// 返回：`Err` 包含具体原因（socket 不存在 / 握手被拒 / 版本不匹配）。
    pub fn connect(display: &str) -> Result<Self, String> {
        let d = if display.is_empty() {
            std::env::var("DISPLAY").map_err(|_| "DISPLAY 未设置".to_string())?
        } else {
            display.to_string()
        };
        let (host, number) = parse_display(&d)?;
        if !host.is_empty() && host != "unix" {
            return Err(format!(
                "只支持本地 unix socket 连接，当前 DISPLAY={d}（host={host}）"
            ));
        }
        let path = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
        let stream = connect_socket(&path).map_err(|e| {
            format!(
                "连接 {} 失败: {e}（已尝试文件系统路径与 Linux 抽象命名空间）",
                path.display()
            )
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

        let mut c = Self {
            stream,
            seq: 0,
            info: ServerInfo::default(),
            pending: Vec::new(),
            rx: [0u8; 32],
            rx_filled: 0,
            tail: Vec::new(),
            tail_needed: 0,
            pending_head: None,
            reply_timeout: DEFAULT_REPLY_TIMEOUT,
        };
        c.handshake(number)?;
        Ok(c)
    }

    /// 服务端信息（握手后可用）。
    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// 当前已发出的请求序列号。
    pub fn sequence(&self) -> u16 {
        self.seq
    }

    fn handshake(&mut self, number: u32) -> Result<(), String> {
        // 1) 取认证数据（Xvfb 通常没有认证文件，那就发空）
        let hostname = crate::xauth::local_hostname();
        let cookie = crate::xauth::load_cookie_from_home(number, &hostname);
        let (auth_name, auth_data): (&[u8], Vec<u8>) = match cookie {
            Some(data) => (b"MIT-MAGIC-COOKIE-1", data),
            None => (b"", Vec::new()),
        };

        // 2) 发 Setup
        let pkt = encode_setup_request(auth_name, &auth_data);
        self.stream
            .write_all(&pkt)
            .map_err(|e| format!("发送 Setup 失败: {e}"))?;
        self.stream.flush().map_err(|e| format!("flush: {e}"))?;

        // 3) 读响应头（8 字节）
        let mut head = [0u8; 8];
        self.read_exact(&mut head)?;
        let status = head[0];
        let extra_units = u16::from_le_bytes([head[6], head[7]]) as usize;
        let mut body = vec![0u8; extra_units * 4];
        if !body.is_empty() {
            self.read_exact(&mut body)?;
        }
        match status {
            1 => {}
            0 => {
                let reason = String::from_utf8_lossy(&body).to_string();
                return Err(format!("服务端拒绝连接: {reason}"));
            }
            2 => return Err("服务端要求进一步认证（本客户端不支持）".to_string()),
            other => return Err(format!("未知的 Setup 状态码 {other}")),
        }

        // 4) 解析 Setup 响应（字段偏移见 X11 规范 "Connection Setup" 一节）
        if body.len() < 32 {
            return Err(format!("Setup 响应太短：{} 字节", body.len()));
        }
        let u32at = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let u16at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]);

        let release = u32at(&body, 0);
        let resource_id_base = u32at(&body, 4);
        let resource_id_mask = u32at(&body, 8);
        let vendor_len = u16at(&body, 16) as usize;
        let n_screens = body[20] as usize;
        let min_keycode = body[26];
        let max_keycode = body[27];

        let vendor_start = 32;
        let vendor = body
            .get(vendor_start..vendor_start + vendor_len)
            .map(|b| String::from_utf8_lossy(b).to_string())
            .unwrap_or_default();
        // vendor 名字后有补齐，但不是补到 4 字节：规范说响应体本身已经 4 字节对齐，
        // 而 vendor 名字的补齐是否存在取决于服务端实现 → 不能假设，
        // 改用"从尾尾往前推"不可靠，故取两种候选中能解析出合法屏幕的那个。
        // 屏幕表在 vendor 之后：vendor 补齐到 4 字节 → pixmap-formats（每个 8 字节）→ screens。
        let n_formats = body[21] as usize;
        let pad = (4 - vendor_len % 4) % 4;
        let screens_start = 32 + vendor_len + pad + n_formats * 8;
        if screens_start + 40 > body.len() {
            return Err(format!(
                "屏幕表越界：screens_start={screens_start}，响应 {} 字节（vendor_len={vendor_len}, n_formats={n_formats}）",
                body.len()
            ));
        }
        let off = screens_start;
        // Screen 字段偏移：root 0, colormap 4, white 8, black 12, input-masks 16,
        // width 20, height 22, width-mm 24, height-mm 26, min-maps 28, max-maps 30,
        // root-visual 32, backing-stores 36, save-unders 37, root-depth 38, n-depths 39
        let chosen: Option<(u32, u16, u16, u16, u16, u32, u8)> = {
            let root = u32at(&body, off);
            let width = u16at(&body, off + 20);
            let height = u16at(&body, off + 22);
            // width-mm / height-mm：X11 协议里是 u16；Xvfb 也照报（默认按 100 DPI 折算）。
            let width_mm = u16at(&body, off + 24);
            let height_mm = u16at(&body, off + 26);
            let root_visual = u32at(&body, off + 32);
            let root_depth = body[off + 38];
            if root != 0 && width > 0 && height > 0 && root_depth > 0 {
                Some((
                    root,
                    width,
                    height,
                    width_mm,
                    height_mm,
                    root_visual,
                    root_depth,
                ))
            } else {
                None
            }
        };
        let Some((root, width, height, width_mm, height_mm, root_visual, root_depth)) = chosen
        else {
            return Err(format!(
                "无法从 Setup 响应中定位屏幕表（响应 {} 字节，n_screens={n_screens}）",
                body.len()
            ));
        };

        self.info = ServerInfo {
            release,
            resource_id_base,
            resource_id_mask,
            vendor,
            root,
            width,
            height,
            width_mm,
            height_mm,
            root_visual,
            root_depth,
            min_keycode,
            max_keycode,
        };
        Ok(())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), String> {
        // 等应答必须有上界：协议错位时对端不会再回应答。
        //
        // **WouldBlock/EAGAIN 不应当成读取失败**：
        // 它只是“本次 set_read_timeout 内没数据”，看上去像错误（实测报
        // `Resource temporarily unavailable`）。应在总时间上限内**继续等**，
        // 直到取满整个 `buf`，超时才报“读超时”。
        const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
        let deadline = Instant::now() + REPLY_TIMEOUT;
        self.stream
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .map_err(|e| format!("设置读超时失败: {e}"))?;

        let mut filled = 0usize;
        let mut result = Ok(());
        while filled < buf.len() {
            match self.stream.read(&mut buf[filled..]) {
                Ok(0) => {
                    result = Err("对端关闭连接".to_string());
                    break;
                }
                Ok(n) => filled += n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if Instant::now() >= deadline {
                        result = Err("读超时（5s 内未收到完整包）".to_string());
                        break;
                    }
                    // 短睡避免空转；总时间仍受 deadline 限制。
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => {
                    result = Err(format!("读取失败：{e}"));
                    break;
                }
            }
        }
        let _ = self.stream.set_read_timeout(None);
        result
    }

    /// 发送一个已编码的请求（不 flush）。
    pub fn send(&mut self, request: &[u8]) -> Result<(), String> {
        self.stream
            .write_all(request)
            .map_err(|e| format!("发送请求失败: {e}"))?;
        self.seq = self.seq.wrapping_add(1);
        Ok(())
    }

    /// 刷出缓冲区。
    pub fn flush(&mut self) -> Result<(), String> {
        self.stream.flush().map_err(|e| format!("flush: {e}"))
    }

    /// 同步：发 `GetInputFocus` 并读到它的响应。
    ///
    /// 返回：在响应之前收到的**所有错误包**（空 = 之前所有请求被接受）。
    /// 为什么要这么做：X11 的错误是异步回来的，不发一个带响应的请求根本知道不到"刚才那批请求到底成没成"。
    pub fn sync(&mut self) -> Result<Vec<X11Error>, String> {
        self.send(&encode_get_input_focus())?;
        self.flush()?;
        let want = self.seq;
        let mut errors = Vec::new();
        let mut events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                return Err("等待 GetInputFocus 响应超时".to_string());
            }
            // 读包与 drain 由 `read_packet` 负责；本函数只做**序列号匹配**与分派。
            let (head, _extra) = self.read_packet()?;
            match head[0] {
                0 => errors.push(parse_error(&head)),
                1 => {
                    // 响应：序列号在字节 2-3
                    let seq = u16::from_le_bytes([head[2], head[3]]);
                    if seq == want {
                        self.pending.extend(events);
                        return Ok(errors);
                    }
                    // 旧响应（如 InternAtom 的 reply）：丢弃即可。
                    // 附加数据已在 `read_packet` 里 drain，不会污染流。
                }
                _ => {
                    events.push(decode_event(&head).unwrap_or(X11Event::Unknown { code: head[0] }));
                }
            }
        }
    }

    /// 读一个**原始 32 字节包**（事件或错误都不解析）。
    ///
    /// 为什么需要它：剪贴板要处理 `SelectionRequest`/`SelectionNotify`
    /// 这类上层不关心的事件，而 `poll_events` 会把未知事件丢掉。
    pub fn next_packet(&mut self, timeout_ms: u32) -> Result<[u8; 32], String> {
        match self.try_next_packet(timeout_ms)? {
            Some(pkt) => Ok(pkt),
            None => Err(format!(
                "读包超时（{timeout_ms}ms 内未收到完整的 32 字节包；已读部分会保留，下次接着读）"
            )),
        }
    }

    /// 尝试读一个 32 字节包；**没有完整包时返回 `Ok(None)`**，不报错。
    ///
    /// 语义：
    /// - `wait_ms == 0`：**立即返回**（用 `set_nonblocking`，不是 `set_read_timeout(0)` ——
    ///   后者在 Linux 上是 `EINVAL`，旧实现因此让"0 超时轮询"永远失败）；
    /// - `wait_ms > 0`：最多等 `wait_ms`，超时返回 `Ok(None)`；
    /// - 无论哪种情况，**已读到的半包都留在 `rx` 里**，下次调用接着补 —— 不会丢字节，
    ///   也不会把半包当完整包交给解析器。
    ///
    /// 为什么保留 `next_packet`：它已有调用点（剪贴板），改签名是公开 API 破坏；
    /// 需要"无数据"语义的新代码请用本函数。
    pub fn try_next_packet(&mut self, wait_ms: u32) -> Result<Option<[u8; 32]>, String> {
        if wait_ms == 0 {
            self.stream
                .set_nonblocking(true)
                .map_err(|e| format!("set_nonblocking(true): {e}"))?;
        } else {
            self.stream
                .set_nonblocking(false)
                .map_err(|e| format!("set_nonblocking(false): {e}"))?;
            self.stream
                .set_read_timeout(Some(Duration::from_millis(wait_ms as u64)))
                .map_err(|e| format!("set_read_timeout: {e}"))?;
        }
        let result = self.fill_one_packet(wait_ms);
        if wait_ms > 0 {
            // 恢复"无超时的阻塞模式"：后续 `read_packet` 会自己按需设置轮询切片。
            let _ = self.stream.set_read_timeout(None);
        }
        if wait_ms == 0 {
            // 恢复阻塞模式（忽略错误：即使失败也不该盖掉真正的读取结果，
            // 但要留日志 —— 否则后续阻塞读会异常）。
            if let Err(e) = self.stream.set_nonblocking(false) {
                eprintln!("[y901] 恢复阻塞模式失败：{e}（后续读可能异常）");
            }
        }
        result
    }

    /// 把 `rx` 补满 32 字节并返回该包；超时/无数据时返回 `Ok(None)`（**保留已读字节**）。
    fn fill_one_packet(&mut self, wait_ms: u32) -> Result<Option<[u8; 32]>, String> {
        let deadline = Instant::now() + Duration::from_millis(wait_ms as u64);
        loop {
            // ① 先把未读完的 reply / GenericEvent 附加数据消费掉（保持字节顺序）。
            //    **与包头共享同一个 deadline**。
            if self.tail_needed > 0 {
                if !self.pump_tail_until(Some(deadline))? {
                    // 没读完：进度保留在 `tail`，本次返回"暂无完整包"。
                    return Ok(None);
                }
                self.pending_head = None;
                self.take_tail();
            }
            if self.rx_filled == 32 {
                let pkt = self.rx;
                self.rx_filled = 0;
                // Reply / GenericEvent 可能带附加数据：登记给**下一次调用**消费，
                // 这样调用方拿到的是完整包头，而流不会因为附加数据而错位。
                match packet_extra_units(&pkt) {
                    Some(units) if units > 0 => {
                        if units > MAX_REPLY_UNITS {
                            return Err(format!(
                                "包声明 {units} 单位附加数据，超过 sanity 上限（流已不可信）"
                            ));
                        }
                        self.pending_head = Some(pkt);
                        self.tail.clear();
                        self.tail_needed = units * 4;
                    }
                    _ => {
                        self.pending_head = None;
                        self.tail_needed = 0;
                    }
                }
                return Ok(Some(pkt));
            }
            // ② **总时限在每次尝试读取之前都检查**。
            //
            // 旧实现只在 `WouldBlock` 分支里比较 deadline：如果对端持续每 <200ms 送
            // 几个字节（每次都读到一点），`read` 每次都成功，循环就永远不会命中
            // 那个分支 → `wait_ms=50` 可以被拖成任意长。这里把判定提到循环顶部，
            // 使"总等待 ≤ wait_ms（+ 一次 read 的系统调用时长）"成为硬上界。
            if wait_ms == 0 {
                // 非阻塞：只尝试一次，读不到就返回（残留字节留在 rx 里）。
                match self.stream.read(&mut self.rx[self.rx_filled..]) {
                    Ok(0) => return Err("服务端断开连接".to_string()),
                    Ok(n) => {
                        self.rx_filled += n;
                        continue;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        return Ok(None)
                    }
                    Err(e) => return Err(format!("读包失败: {e}")),
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            // **每次读取前把 socket 超时收紧到"剩余时间"**。
            // 否则最后一次 `read` 可以用完整的 wait_ms 越过 deadline 才返回，
            // 于是"总等待 ≤ wait_ms"被破坏（实测：wait=100ms 时 120ms 才凑满并返回）。
            let left = deadline
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            let _ = self.stream.set_read_timeout(Some(left));
            match self.stream.read(&mut self.rx[self.rx_filled..]) {
                Ok(0) => return Err("服务端断开连接".to_string()),
                Ok(n) => self.rx_filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // 短睡避免忙等；上界取"剩余时间"与 2ms 的较小者。
                    let left = deadline.saturating_duration_since(Instant::now());
                    std::thread::sleep(left.min(Duration::from_millis(2)));
                }
                Err(e) => return Err(format!("读包失败: {e}")),
            }
        }
    }

    /// 诊断/测试用：`rx` 里尚未成包的字节数。
    pub fn partial_packet_bytes(&self) -> usize {
        self.rx_filled
    }

    /// 诊断/测试用：尚未读完的 reply 附加数据字节数。
    pub fn pending_reply_extra_bytes(&self) -> usize {
        self.tail_needed - self.tail.len()
    }

    /// **仅测试**：把阻塞读取的总超时从 5s 缩短，便于验证"超时后仍可续读"。
    #[cfg(test)]
    pub(crate) fn set_reply_timeout_for_test(&mut self, d: Duration) {
        self.reply_timeout = d;
    }

    /// 读属性（`GetProperty`）。
    ///
    /// 返回：`(format, data)`。属性不存在或为空时 `data` 为空。
    /// 读**一页**属性。
    ///
    /// **注意**：如果属性比 `long_length*4` 大，本函数返回 `Err`（而不是旧实现的
    /// "静默返回半截"）。要读全部请用 [`Connection::get_property_all`]。
    pub fn get_property(
        &mut self,
        window: u32,
        property: u32,
        type_atom: u32,
        long_offset: u32,
        long_length: u32,
        delete_after: bool,
    ) -> Result<(u8, Vec<u8>), String> {
        let (format, data, bytes_after, _type) = self.get_property_page(
            window,
            property,
            type_atom,
            long_offset,
            long_length,
            delete_after,
        )?;
        if bytes_after > 0 {
            // **不许静默截断**：调用方要么用 `get_property_all`，要么显式接受这份
            // 不完整数据（那就该自己分页）。返回半截数据会让上层解析出垃圾。
            return Err(format!(
                "GetProperty 只读到 {} 字节，属性还有 {bytes_after} 字节未读\
                 （long_length={long_length} 太小；请用 get_property_all）",
                data.len()
            ));
        }
        Ok((format, data))
    }

    /// GetProperty 的**单页**读取。
    ///
    /// 返回：`(format, data, bytes_after, type_atom)`。
    ///
    /// `bytes_after` 是 X11 在响应里给的"属性里还剩多少字节没返回"——
    /// 旧实现**完全忽略**它，于是超过 `long_length*4` 的属主数据被静默截断
    ///（典型：大段剪贴板、XSettings 被截一半，解析出垃圾或直接失败）。
    ///
    /// `long_offset` / `long_length` 的单位是**4 字节**（X11 协议规定），
    /// 与 `format` 无关；返回的 `data` 已按 `n_items × format/8` 截掉尾部填充。
    fn get_property_page(
        &mut self,
        window: u32,
        property: u32,
        type_atom: u32,
        long_offset: u32,
        long_length: u32,
        delete_after: bool,
    ) -> Result<(u8, Vec<u8>, u32, u32), String> {
        self.send(&crate::encode::encode_get_property(
            window,
            property,
            type_atom,
            long_offset,
            long_length,
            delete_after,
        ))?;
        self.flush()?;
        let (head, mut data) = self.read_reply_packet()?;
        let format = head[1];
        let type_atom = u32::from_le_bytes([head[8], head[9], head[10], head[11]]);
        let bytes_after = u32::from_le_bytes([head[12], head[13], head[14], head[15]]);
        // `n_items`（值长度，单位是**项**，不是字节）在字节 16..20。
        let n_items = u32::from_le_bytes([head[16], head[17], head[18], head[19]]) as usize;
        let bytes = match format {
            8 => n_items,
            16 => n_items.saturating_mul(2),
            32 => n_items.saturating_mul(4),
            // format=0 是**协议里的"属性不存在"**（type=None、长度为 0）。
            // 旧实现会算出 0 字节并返回空；新实现也要保持这个语义，不能报错，
            // 否则"读一个可能不存在的属性"会从"返回空"变成"启动失败"。
            0 => 0,
            _ => {
                return Err(format!(
                    "GetProperty 返回了非法 format={format}（只允许 0/8/16/32）"
                ))
            }
        };
        // **一致性校验**：reply 的 `length`（附加数据）必须装得下
        // `n_items × format/8`。少了就是协议/服务器异常 —— 旧实现直接
        // `truncate(bytes.min(data.len()))`，会**静默**返回不足的数据。
        if data.len() < bytes {
            return Err(format!(
                "GetProperty 声明 {bytes} 字节（items={n_items}, format={format}），\
                 实际只收到 {} 字节",
                data.len()
            ));
        }
        data.truncate(bytes);
        Ok((format, data, bytes_after, type_atom))
    }

    /// 读**完整**属性（自动处理 `bytes_after`），超限时**显式失败**。
    ///
    /// 为什么需要新函数而不是改 `get_property` 的语义：`get_property` 的
    /// `long_length` 由调用方决定，它无法知道"我要的是全部还是前 N 字节"。
    /// 本函数明确表达"我要全部"，因此可以安全地分页与限流。
    ///
    /// 实现（两阶段，避免"边读边删"把没读到的一半删掉）：
    /// 1. **探长度**：`GetProperty(offset=0, length=0, delete=false)` → `bytes_after` 即总字节数；
    /// 2. **一次读满**：`length = ceil(total/4)`（单位是 4 字节），最后一页才带 `delete`；
    ///    若服务器仍少给（`bytes_after > 0`），按页续读（**续读页一律不删**）。
    ///
    /// `max_bytes`：硬上限。超限返回 `Err` 且**不删除**属性（调用方还可以换策略重读）。
    pub fn get_property_all(
        &mut self,
        window: u32,
        property: u32,
        type_atom: u32,
        delete_after: bool,
        max_bytes: usize,
    ) -> Result<(u8, Vec<u8>), String> {
        // 逻辑在 `collect_property_all`（可被测试替身驱动，覆盖续页分支）。
        let mut src = ConnPropertySource {
            conn: self,
            window,
            property,
            type_atom,
        };
        collect_property_all(&mut src, type_atom, delete_after, max_bytes)
    }
    /// 查询某个 selection 的 owner 窗口（`GetSelectionOwner`，opcode 23）。
    ///
    /// 返回 `0` 表示当前没有任何客户端拥有它（例如裸 Xvfb 上没有 XSettings manager）。
    /// 这是**正常结果**，不是错误。
    pub fn get_selection_owner(&mut self, selection: u32) -> Result<u32, String> {
        self.send(&crate::encode::encode_get_selection_owner(selection))?;
        self.flush()?;
        self.read_reply_u32()
    }

    /// 探测扩展：返回它的**主 opcode**（不支持则 `None`）。
    ///
    /// 为什么要问而不是硬编码：X11 扩展的 opcode 是**服务器分配的**，
    /// 不同服务器/不同加载顺序都可能不同。硬编码就是猜。
    pub fn query_extension(&mut self, name: &[u8]) -> Result<Option<u8>, String> {
        self.send(&crate::encode::encode_query_extension(name))?;
        self.flush()?;
        let (head, _extra) = self.read_reply_packet()?;
        // QueryExtension 响应布局（X11 规范）：
        //   byte8 = present，byte9 = major-opcode，byte10 = first-event，byte11 = first-error
        Ok(if head[8] != 0 { Some(head[9]) } else { None })
    }

    /// Discover physical geometry and DPI for currently active XRandR outputs.
    ///
    /// This is an opt-in query only: callers must not use the result to change
    /// global scaling without knowing where a window will be placed. Use
    /// [`crate::randr::dpi_at`] to resolve the output containing a window's
    /// position. Missing XRandR or an older server returns an empty list;
    /// malformed protocol replies are returned as errors. Outputs with
    /// implausible physical sizes or inactive CRTCs are conservatively omitted.
    pub fn active_output_dpis(&mut self) -> Result<Vec<OutputDpi>, String> {
        let Some(opcode) = self.query_extension(EXTENSION_NAME)? else {
            return Ok(Vec::new());
        };
        self.send(&crate::randr::encode_query_version(opcode))?;
        self.flush()?;
        let (head, _) = self.read_reply_packet()?;
        let version = crate::randr::parse_version_reply(&head)?;
        if (version.major, version.minor) < (REQUIRED_MAJOR, REQUIRED_MINOR) {
            return Ok(Vec::new());
        }

        self.send(&crate::randr::encode_get_screen_resources_current(
            opcode,
            self.info.root,
        ))?;
        self.flush()?;
        let (head, extra) = self.read_reply_packet()?;
        let resources = crate::randr::parse_screen_resources_reply(&head, &extra)?;
        let mut values = Vec::new();
        for output in resources.outputs {
            self.send(&crate::randr::encode_get_output_info(
                opcode,
                output,
                resources.config_timestamp,
            ))?;
            self.flush()?;
            let (head, _) = self.read_reply_packet()?;
            let info = match crate::randr::parse_output_info_reply(&head) {
                Ok(info) => info,
                // Resource changes can race enumeration; skip only that output.
                Err(_) => continue,
            };
            if !info.connected || info.crtc == 0 || info.width_mm == 0 || info.height_mm == 0 {
                continue;
            }
            self.send(&crate::randr::encode_get_crtc_info(
                opcode,
                info.crtc,
                resources.config_timestamp,
            ))?;
            self.flush()?;
            let (head, extra) = self.read_reply_packet()?;
            let crtc = match crate::randr::parse_crtc_info_reply(&head, &extra) {
                Ok(crtc) => crtc,
                Err(_) => continue,
            };
            if crtc.width == 0 || crtc.height == 0 || !crtc.outputs.contains(&output) {
                continue;
            }
            let dpi_x = crtc.width as f32 * 25.4 / info.width_mm as f32;
            let dpi_y = crtc.height as f32 * 25.4 / info.height_mm as f32;
            if (20.0..=1000.0).contains(&dpi_x) && (20.0..=1000.0).contains(&dpi_y) {
                values.push(OutputDpi {
                    output,
                    x: crtc.x,
                    y: crtc.y,
                    width: crtc.width,
                    height: crtc.height,
                    dpi_x,
                    dpi_y,
                });
            }
        }
        Ok(values)
    }

    /// `MIT-SHM` 的 `QueryVersion`：返回 `(major, minor)`。
    ///
    /// 失败：`Err` 当扩展不可用或响应异常。
    pub fn shm_query_version(&mut self, major_opcode: u8) -> Result<(u16, u16), String> {
        self.send(&crate::encode::encode_shm_query_version(major_opcode))?;
        self.flush()?;
        let (head, _extra) = self.read_reply_packet()?;
        // byte8 = shared_pixmaps；bytes 10-11 = major；12-13 = minor
        let major = u16::from_le_bytes([head[10], head[11]]);
        let minor = u16::from_le_bytes([head[12], head[13]]);
        Ok((major, minor))
    }

    /// 读一个"必须是响应"的包，中途插入的事件收进 `pending`。
    ///
    /// 返回：`(32 字节响应头, 附加数据)`。附加数据已按
    /// `head[4..8]`（单位：4 字节）**drain 干净**，调用方不用再读。
    ///
    /// 为何要统一：X11 允许事件与响应任意交错。有 WM 时
    /// metacity 的 `ReparentNotify`（字节 0 = 21）会插在响应前面，
    /// 旧实现直接报“未得到响应”，导致启动失败。
    ///
    /// 读**一个完整的 X11 包**（32 字节头 + 附加数据）。
    ///
    /// 只做"读"，不做任何业务判断（不存 `pending`、不收集错误、
    /// 不匹配序列号）——那些留给调用方。
    ///
    /// **只有 reply（byte0=1）和 GenericEvent（byte0=35）带附加数据**：
    /// Error（byte0=0）的 byte4-5 是 `bad_value`，Event（byte0>=2 且 !=35）的 byte4 起是事件体，
    /// 都**不是长度**。把它们当长度 drain 会多读字节、流错位（已踩过坑）。
    ///
    /// 读取原语：socket 上的字节只有两种归属 —— `rx` 里的包头、`tail` 里的附加数据；
    /// 两者都能跨调用保留（超时不丢字节）。
    ///
    /// `rx` / `tail` 的不变量见结构体字段注释。
    ///
    /// 读一次 socket 追加到 `tail`（最多补到 `tail_needed`）。
    ///
    /// 返回：`Ok(true)` = 有进展；`Ok(false)` = **暂时**没有数据（`WouldBlock`/超时，
    /// 已读字节保留）；`Err` = EOF / 不可恢复错误（此时连接不可再用）。
    ///
    /// 关键点：缓冲**只增长到 `tail_needed`**，且单次最多扩 `TAIL_CHUNK` —— 既不会
    /// 因为 `length` 字段损坏而一次分配几个 GB，也不会无界增长。
    fn pump_tail_once(&mut self) -> Result<bool, String> {
        if self.tail.len() >= self.tail_needed {
            return Ok(true);
        }
        let old = self.tail.len();
        let want = self.tail_needed.min(old + TAIL_CHUNK);
        self.tail.resize(want, 0);
        loop {
            match self.stream.read(&mut self.tail[old..]) {
                Ok(0) => {
                    self.tail.truncate(old);
                    return Err("对端关闭连接".to_string());
                }
                Ok(n) => {
                    self.tail.truncate(old + n);
                    return Ok(true);
                }
                // 信号中断不是错误：立刻重试（不睡眠，避免放大时延）。
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    self.tail.truncate(old);
                    return Ok(false);
                }
                Err(e) => {
                    self.tail.truncate(old);
                    return Err(format!("读取失败：{e}"));
                }
            }
        }
    }

    /// 把 `tail` 补到 `tail_needed`（阻塞语义，受 `reply_timeout` 限制）。
    ///
    /// 超时返回 `Err`，但**已读的字节留在 `tail` 里**、`tail_needed` 也不变 ——
    /// 下一次调用会从这里接着读，不会错位。
    fn pump_tail_blocking(&mut self) -> Result<(), String> {
        // 保证 `read` 会周期性返回，让 deadline 生效（详见 `POLL_SLICE`）；
        // 切片不超过总超时，避免"切片 > deadline"时整段等待被拉长。
        let slice = POLL_SLICE
            .min(self.reply_timeout)
            .max(Duration::from_millis(1));
        let _ = self.stream.set_read_timeout(Some(slice));
        let deadline = Instant::now() + self.reply_timeout;
        let out = self.pump_tail_blocking_inner(deadline);
        let _ = self.stream.set_read_timeout(None);
        out
    }

    /// `pump_tail_blocking` 的实体（调用方已设好轮询切片）。
    /// `pump_tail_blocking` 的实体：**总 deadline 覆盖持续成功的小片段读取**。
    ///
    /// 与 `read_head_blocking_inner` 同一套规则（旧实现只在 `WouldBlock` 时比较
    /// deadline，慢速分片会让循环无限延长）：每轮读取前检查 deadline；每次 `read`
    /// 的等待不超过剩余预算；超时保留已读数据。
    fn pump_tail_blocking_inner(&mut self, deadline: Instant) -> Result<(), String> {
        while self.tail.len() < self.tail_needed {
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "读超时：reply 附加数据还差 {} 字节（已读部分保留，可重试）",
                    self.tail_needed - self.tail.len()
                ));
            }
            let left = deadline.saturating_duration_since(now);
            let _ = self
                .stream
                .set_read_timeout(Some(POLL_SLICE.min(left).max(Duration::from_millis(1))));
            // `pump_tail_once` 内部已处理 `Interrupted`（原地重试，不重置 deadline）。
            if !self.pump_tail_once()? {
                continue;
            }
        }
        Ok(())
    }

    /// 把 `tail` 补满；`deadline == None` 表示**纯非阻塞**（读不到就返回）。
    ///
    /// `Some(deadline)` 用于 `try_next_packet(wait_ms)`：**Reply / GenericEvent 的
    /// 附加数据与包头共享同一个总 deadline**。旧实现在这里调用无 deadline 的
    /// `pump_tail_poll`，附加数据缓慢到达时会被无限读取，`wait_ms` 形同虚设。
    fn pump_tail_until(&mut self, deadline: Option<Instant>) -> Result<bool, String> {
        while self.tail.len() < self.tail_needed {
            if let Some(dl) = deadline {
                let now = Instant::now();
                if now >= dl {
                    return Ok(false);
                }
                let left = dl.saturating_duration_since(now);
                let _ = self
                    .stream
                    .set_read_timeout(Some(POLL_SLICE.min(left).max(Duration::from_millis(1))));
            }
            if !self.pump_tail_once()? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// 纯非阻塞地把 `tail` 补满：`Ok(false)` = 暂时没读完（进度保留）。
    fn pump_tail_poll(&mut self) -> Result<bool, String> {
        self.pump_tail_until(None)
    }

    /// 取走已读满的附加数据并复位状态。
    fn take_tail(&mut self) -> Vec<u8> {
        self.tail_needed = 0;
        std::mem::take(&mut self.tail)
    }

    /// 读满一个 32 字节包头（阻塞语义，**先把 `rx` 里已有的半个包接上**）。
    ///
    /// 超时返回 `Err`，但 `rx`/`rx_filled` **原样保留** —— 旧实现先把 `rx` 清空再
    /// 读剩下的部分，一旦超时，已经消费的字节就随局部变量一起丢了。
    fn read_head_blocking(&mut self) -> Result<[u8; 32], String> {
        // 保证 `read` 会周期性返回，让 deadline 生效（详见 `POLL_SLICE`）；
        // 切片不超过总超时，避免"切片 > deadline"时整段等待被拉长。
        let slice = POLL_SLICE
            .min(self.reply_timeout)
            .max(Duration::from_millis(1));
        let _ = self.stream.set_read_timeout(Some(slice));
        let deadline = Instant::now() + self.reply_timeout;
        let out = self.read_head_blocking_inner(deadline);
        let _ = self.stream.set_read_timeout(None);
        out
    }

    /// `read_head_blocking` 的实体（调用方已设好初始轮询切片）。
    ///
    /// **总 deadline 覆盖"持续成功的小片段读取"**：旧实现只在 `WouldBlock` 分支
    /// 比较 deadline，对端只要每次都送几个字节、每次 `read` 都成功，循环就永远
    /// 不会命中那个分支 —— 于是 `reply_timeout=5s` 可以被拖成任意长。现在：
    /// 1. **每轮读取前**都检查 deadline；
    /// 2. 每次 `read` 之前把 socket 超时收紧到 `min(POLL_SLICE, 剩余预算)`，
    ///    保证单次 `read` 不会越过 deadline（`Interrupted` 重试也不重置预算）；
    /// 3. 超时返回 `Err`，但 `rx`/`rx_filled` **原样保留**，下一次调用接着读。
    fn read_head_blocking_inner(&mut self, deadline: Instant) -> Result<[u8; 32], String> {
        loop {
            if self.rx_filled == 32 {
                let head = self.rx;
                self.rx_filled = 0;
                return Ok(head);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "读超时：包头还差 {} 字节（已读部分保留，可重试）",
                    32 - self.rx_filled
                ));
            }
            let left = deadline.saturating_duration_since(now);
            let _ = self
                .stream
                .set_read_timeout(Some(POLL_SLICE.min(left).max(Duration::from_millis(1))));
            match self.stream.read(&mut self.rx[self.rx_filled..]) {
                Ok(0) => return Err("对端关闭连接".to_string()),
                Ok(n) => self.rx_filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // 回到循环顶部重新判定 deadline（不在这里自行 sleep）。
                    continue;
                }
                Err(e) => return Err(format!("读取失败：{e}")),
            }
        }
    }

    /// 非阻塞地读一个包头：`Ok(None)` = 暂时没有完整包（`rx` 里保留已读字节）。
    fn read_head_poll(&mut self) -> Result<Option<[u8; 32]>, String> {
        loop {
            if self.rx_filled == 32 {
                let head = self.rx;
                self.rx_filled = 0;
                return Ok(Some(head));
            }
            match self.stream.read(&mut self.rx[self.rx_filled..]) {
                Ok(0) => return Err("对端关闭连接".to_string()),
                Ok(n) => self.rx_filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Ok(None)
                }
                Err(e) => return Err(format!("读取失败：{e}")),
            }
        }
    }

    /// 读一个完整包（头 + 附加数据），**阻塞语义**。
    ///
    /// 恢复语义：超时返回 `Err` 时 `pending_head` / `tail` 都保留，下一次调用
    /// 会先补完附加数据再取包 —— 不会把半包交给解析器，也不会重复消费包头。
    fn read_packet(&mut self) -> Result<([u8; 32], Vec<u8>), String> {
        let head = match self.pending_head {
            Some(h) => h,
            None => {
                let h = self.read_head_blocking()?;
                // **reply length 是 CARD32**（Reply 头：1 type | 1 data | 2 seq | 4 length）。
                // 旧实现只读低 16 位：任何附加数据超过 65 535 单位（262 140 字节）的回复
                // （典型：`GetImage` 读回 1080p、大段剪贴板 `GetProperty`）只会被读掉一部分，
                // 剩下的字节留在流里 → 之后每个包都从错位处解析，界面卡死/花屏。
                // Reply 与 GenericEvent 都带 `length`（都在字节 4..8）；
                // 把它们当事件体解析会让后续每一包错位。
                let units = packet_extra_units(&h).unwrap_or(0);
                // 防御：损坏的长度不应让我们尝试分配几个 GB
                if units > MAX_REPLY_UNITS {
                    return Err(format!("reply length {units} units exceeds sanity limit"));
                }
                // 新包：开始一段新的附加数据
                self.tail.clear();
                self.tail_needed = units * 4;
                h
            }
        };
        self.pending_head = Some(head);
        if self.tail_needed > 0 {
            self.pump_tail_blocking()?;
        }
        self.pending_head = None;
        let extra = self.take_tail();
        Ok((head, extra))
    }

    /// 注意：事件包一般 `units = 0`，但协议没保证，所以同样 drain。
    fn read_reply_packet(&mut self) -> Result<([u8; 32], Vec<u8>), String> {
        loop {
            let (head, extra) = self.read_packet()?;
            match head[0] {
                // 响应：直接交给调用方（附加数据已在 `read_packet` 里 drain）
                1 => return Ok((head, extra)),
                // 错误包：不吞掉，交给上层看真实原因
                0 => return Err(format!("X11 错误包（字节 0 = 0，错误码 = {}）", head[1])),
                // 事件包：解码后存 `pending`（不丢），继续等响应
                _ => {
                    if let Ok(ev) = decode_event(&head) {
                        self.pending.push(ev);
                    }
                }
            }
        }
    }

    /// 读一个 32 位回复（`InternAtom` 等）；遇到事件包先解码存入 `pending`。
    ///
    /// 为何单独写：`sync()` 会丢弃"不属于当前序列"的响应，
    /// 而 `InternAtom` 的响应正好需要拿回来用，不能丢。
    ///
    /// 实现委托给 `read_reply_packet`（它负责跳事件与 drain 附加数据）。
    pub fn read_reply_u32(&mut self) -> Result<u32, String> {
        // **事件与响应可任意交错**：X11 协议不保证"下一个包就是响应"。
        // 实例：metacity 接管窗口时会发 `ReparentNotify`（字节 0 = 21），
        // 而旧版直接报 "期待响应，收到字节 0 = 21" 导致启动失败。
        //
        // 修法：遇到事件包就**解码后存进 `pending`**（与 `poll_events` 共用同一队列），
        // 然后继续读，直到真正的响应或错误包。
        let (head, _extra) = self.read_reply_packet()?;
        Ok(u32::from_le_bytes([head[8], head[9], head[10], head[11]]))
    }

    /// 回读一块像素（`GetImage`）。
    ///
    /// 返回：`(depth, pixels)`，`pixels` 是 ZPixmap 原始字节（小端序，24bpp 时每像素 4 字节 BGRX）。
    /// 用途：验证 `PutImage` 真的把像素写进去了。
    pub fn get_image(
        &mut self,
        drawable: u32,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
    ) -> Result<(u8, Vec<u8>), String> {
        self.send(&crate::encode::encode_get_image(
            drawable,
            x,
            y,
            width,
            height,
            0xFFFF_FFFF,
        ))?;
        self.flush()?;
        // 读 32 字节响应头
        let (head, data) = self.read_reply_packet()?;
        let depth = head[1];
        Ok((depth, data))
    }

    /// 拿到 keyboard mapping 表（keycode → keysym）。
    ///
    /// 读起来复杂的地方：响应的 `length` 字段是**4 字节单位的数量**，
    /// 而 keysym 正好 4 字节 → 所以 `n_keysyms == length`。
    pub fn get_keyboard_mapping(
        &mut self,
        first_keycode: u8,
        count: u8,
    ) -> Result<crate::keys::Keymap, String> {
        self.send(&crate::encode::encode_get_keyboard_mapping(
            first_keycode,
            count,
        ))?;
        self.flush()?;
        let (head, raw) = self.read_reply_packet()?;
        let per = head[1];
        let keysyms = raw
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Ok(crate::keys::Keymap {
            first_keycode,
            keysyms_per_keycode: per,
            keysyms,
        })
    }

    /// 非阻塞地读一批事件（`timeout_ms = 0` 表示不等）。
    ///
    /// 返回：事件列表与错误列表（错误不吞掉，交给调用方决定）。
    pub fn poll_events(
        &mut self,
        timeout_ms: u32,
    ) -> Result<(Vec<X11Event>, Vec<X11Error>), String> {
        // **`timeout_ms == 0` 必须用 `set_nonblocking`，不能用 `set_read_timeout(0)`**。
        //
        // 为何：Linux 下 `set_read_timeout(Some(0ms))` 会返回 `InvalidInput`（EINVAL）——
        // 而 `run_loop` 在“上一帧有工作”时正好传 0。于是**"有工作时"的路径从来没跑通过**，
        // 而且不是“慢”，是**每次都 Err**（旧代码把 Err 当空事件 → **196% CPU 空转**）。
        //
        // 恢复：函数结尾把非阻塞关掉（同样覆盖 `?` 提前返回的路径，故用内层函数 + 外层恢复）。
        if timeout_ms == 0 {
            self.stream
                .set_nonblocking(true)
                .map_err(|e| format!("set_nonblocking(true): {e}"))?;
        } else {
            self.stream
                .set_nonblocking(false)
                .map_err(|e| format!("set_nonblocking(false): {e}"))?;
            self.stream
                .set_read_timeout(Some(Duration::from_millis(timeout_ms as u64)))
                .map_err(|e| format!("set_read_timeout: {e}"))?;
        }
        let result = self.poll_events_inner();
        if timeout_ms == 0 {
            // 恢复阻塞模式（忽略错误：即使失败也不应盖掉真正的读取结果）
            // 恢复失败不能静默：那意味着**后续阻塞读会异常**
            // （可能返回 WouldBlock 而被当成空事件，又回到空转）。
            if let Err(e) = self.stream.set_nonblocking(false) {
                eprintln!("[y901] 恢复阻塞模式失败：{e}（后续读可能异常）");
            }
        }
        result
    }

    /// `poll_events` 的实体（已按 `timeout_ms` 设好阻塞模式）。
    fn poll_events_inner(&mut self) -> Result<(Vec<X11Event>, Vec<X11Error>), String> {
        let mut events = std::mem::take(&mut self.pending);
        let mut errors = Vec::new();
        loop {
            // ① **先把上一次没读完的 reply 附加数据吞掉**：字节顺序是硬约束，
            //    附加数据没消费完之前绝不能去解析下一个包头。
            if self.tail_needed > 0 {
                if !self.pump_tail_poll()? {
                    // 暂时没读完：进度保存在 `tail` 里，下次调用接着读。
                    return Ok((events, errors));
                }
                self.pending_head = None;
                self.take_tail(); // poll_events 不需要 reply 载荷，但必须消费掉
                continue;
            }
            // ② 读包头：`rx` 里可能已有上次的半个包；读不满就带着已读字节返回。
            let Some(buf) = self.read_head_poll()? else {
                return Ok((events, errors));
            };
            match buf[0] {
                0 => errors.push(parse_error(&buf)),
                // 字节 0 == 1 是**响应**（reply），不是事件。
                // 不区分的话，InternAtom/GetInputFocus 的响应会被当成 Unknown 事件丢给上层。
                //
                // 字节 0 == 35 是 **GenericEvent**：本轮后端**不支持**它（`decode.rs`
                // 明确写了"超过 32 字节的事件本轮不支持"），但它的头里**带 `length`**。
                // 旧实现把它当普通事件：附加数据留在流里 → 之后每一包都从错位处解析。
                // 现在的策略：与 reply 一样**先把附加数据 drain 干净再丢弃事件**。
                1 | 35 => {
                    // **防御性 drain**：正常情况下主连接的 reply 都在
                    // `read_reply_packet` / `sync` 里已读完，不会到这里；
                    // 但若将来加运行期请求（如实时查 `_NET_WM_STATE`），
                    // 它的 reply 可能先被 `poll_events` 摸到。
                    // 若不 drain，剩下的 `units*4` 字节会把后续每一包都错位
                    // （与 64c6088 修的是同一类灾难）。
                    //
                    // 选**drain 而不是报错**：poll_events 的调用方只想要事件，
                    // reply 对它没用；报错会让一个"无害的并发"升级成启动失败。
                    let units = packet_extra_units(&buf).unwrap_or(0);
                    if units > MAX_REPLY_UNITS {
                        return Err(format!(
                            "poll_events: 包声明附加数据 {units} 单位，超过 sanity 上限（流已不可信）"
                        ));
                    }
                    // 不在**这里**读附加数据：改为登记到 `tail_needed`，由循环顶部
                    // 统一 `pump_tail_poll`。旧实现直接 `std::io::Read::read_exact`
                    // 到局部 `Vec`：非阻塞模式下 `WouldBlock` 会立刻返回 Err 并**丢掉
                    // 已读字节**，整条流从此错位。
                    self.pending_head = Some(buf);
                    self.tail.clear();
                    self.tail_needed = units * 4;
                    if self.tail_needed == 0 {
                        self.pending_head = None;
                    }
                }
                _ => {
                    if let Ok(ev) = decode_event(&buf) {
                        events.push(ev);
                    }
                }
            }
        }
    }

    /// **分块**发送一帧 BGRA 像素。
    ///
    /// 为什么要分块：`PutImage` 的 `length` 是 u16 → 单包不能超 262,116 字节。
    /// 切片按**整行**进行（不按字节），否则会把一行像素拦腰断开。
    pub fn put_image_bgra(
        &mut self,
        drawable: u32,
        gc: u32,
        width: u16,
        height: u16,
        pixels: &[u8],
        depth: u8,
    ) -> Result<usize, String> {
        let stride = width as usize * 4;
        let need = stride * height as usize;
        if pixels.len() < need {
            return Err(format!(
                "像素缓冲太小：需 {need} 字节，实际 {}",
                pixels.len()
            ));
        }
        let rows_per_chunk = (PUT_IMAGE_MAX_BYTES / stride).max(1);
        let mut y = 0usize;
        let mut chunks = 0usize;
        while y < height as usize {
            let rows = rows_per_chunk.min(height as usize - y);
            let start = y * stride;
            let end = start + rows * stride;
            let pkt = encode_put_image(&PutImageArgs {
                drawable,
                gc,
                width,
                height: rows as u16,
                dst_x: 0,
                dst_y: y as i16,
                depth,
                data: &pixels[start..end],
            });
            self.send(&pkt)?;
            y += rows;
            chunks += 1;
        }
        self.flush()?;
        Ok(chunks)
    }
}

/// 连接 X server socket：**先试文件路径，再试 Linux 抽象命名空间**。
///
/// 为什么需要抽象名空间：实测发现 WSL 里的 Xvfb 在 `:99` 上
/// **只创建抽象 socket（`@/tmp/.X11-unix/X99`），不创建文件**，
/// 而 `xdpyinfo` 能连上（Xlib 两种都试）。只试文件路径就会 `ENOENT`。
fn connect_socket(path: &std::path::Path) -> std::io::Result<UnixStream> {
    match UnixStream::connect(path) {
        Ok(s) => Ok(s),
        Err(file_err) => {
            #[cfg(target_os = "linux")]
            {
                use std::os::linux::net::SocketAddrExt;
                let name = path.as_os_str().as_encoded_bytes();
                let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
                UnixStream::connect_addr(&addr).map_err(|abs_err| {
                    std::io::Error::new(
                        abs_err.kind(),
                        format!("文件路径失败（{file_err}），抽象命名空间也失败（{abs_err}）"),
                    )
                })
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(file_err)
            }
        }
    }
}

/// 解析 `":99"` / `":0.0"` / `"unix:0"` 为 `(host, number)`。
fn parse_display(d: &str) -> Result<(&str, u32), String> {
    let d = d.trim();
    let Some(idx) = d.rfind(':') else {
        return Err(format!("DISPLAY 格式不对：{d}"));
    };
    let host = &d[..idx];
    let rest = &d[idx + 1..];
    let num = rest.split('.').next().unwrap_or(rest);
    let number: u32 = num
        .parse()
        .map_err(|_| format!("DISPLAY 中的屏幕号不是数字：{d}"))?;
    Ok((host, number))
}

/// 从 32 字节错误包解析字段。
fn parse_error(buf: &[u8]) -> X11Error {
    let u16at = |o: usize| u16::from_le_bytes([buf[o], buf[o + 1]]);
    let u32at = |o: usize| u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
    X11Error {
        // 错误包：字节 0 = 0（包类型），**字节 1 = 错误码**、字节 2-3 = 序列号、
        // 字节 4-7 = bad value、字节 8-9 = minor、字节 10 = major。
        // 第一版把字节 0 当成错误码，结果所有错误都打成了"code 0"。
        code: buf[1],
        sequence: u16at(2),
        bad_value: u32at(4),
        minor_opcode: u16at(8),
        major_opcode: buf[10],
    }
}

/// 回复附加数据的**合理性上限**（单位：4 字节）。16 MiB 足够覆盖 4K 截图；
/// 超出说明长度字段损坏，不能拿它去 `vec![0u8; n]`。
const MAX_REPLY_UNITS: usize = 4 * 1024 * 1024;

/// 阻塞读取路径的默认总等待上限（reply / 附加数据）。
const DEFAULT_REPLY_TIMEOUT: Duration = Duration::from_secs(5);

/// 每次 `read(2)` 最多把缓冲扩多少字节（**有界增长**：避免一次分配几个 GB）。
const TAIL_CHUNK: usize = 64 * 1024;

/// 阻塞读取时给 socket 设的**轮询切片**。
///
/// 为什么必须有：socket 若处于"无超时的阻塞模式"，`read(2)` 会一直挂着，
/// 循环里的 deadline 检查根本没机会执行 —— 表现为"超时参数不生效、直接卡死"。
/// 设一个短超时后，`read` 会周期性返回 `WouldBlock`，deadline 才真正可执行。
const POLL_SLICE: Duration = Duration::from_millis(50);

/// `get_property_all` 单次读取的最大页数（防御服务器异常：永远报 `bytes_after > 0`
/// 却只给极少量数据 —— 没有这个上界就是**无界循环**）。
const MAX_PROPERTY_PAGES: usize = 64;

/// 该包头声明的**附加数据单位数**（4 字节为单位）；`None` = 这个包不带附加数据。
///
/// X11 里只有 **Reply**（byte0 = 1）和 **GenericEvent**（byte0 = 35）的字节 4..8
/// 是 `length`。Error（byte0 = 0）的字节 4..8 是 `bad_value`，普通事件的字节 4 起
/// 是事件体 —— 把它们当长度去 drain 会多读字节、整条流从此错位（本项目踩过两次）。
///
/// 所有读取路径（`read_packet` / `fill_one_packet` / `poll_events_inner`）都必须
/// 用它来判断，**不允许各自硬编码 `head[0] == 1`**。
fn packet_extra_units(head: &[u8; 32]) -> Option<usize> {
    if head[0] == 1 || head[0] == 35 {
        Some(u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize)
    } else {
        None
    }
}
/// 一页 `GetProperty` 的结果（`format` + 实际类型 + 剩余字节 + 本页数据）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PropertyPage {
    /// 8 / 16 / 32；`0` = 属性不存在。
    pub(crate) format: u8,
    /// 服务器回报的**实际**属性类型 atom。
    pub(crate) type_atom: u32,
    /// 本页之后还剩多少字节（X11 的 `bytes_after`）。
    pub(crate) bytes_after: u32,
    /// 本页数据（已按 `items × format/8` 截掉 padding）。
    pub(crate) data: Vec<u8>,
}

/// `collect_property_all` 的数据源。
///
/// 为什么抽这一层：Xvfb 上 `GetProperty` **一次就返回整个属性**，
/// 续页分支（`bytes_after > 0`）永远走不到 —— 而它正是最容易出错的逻辑
/// （offset 单位、跨页一致性、delete 语义）。有了这个 trait，测试可以用
/// 可控替身精确制造"首页只给一半""续页 type/format 变了""长度与探测不符"
/// 等场景，而不是靠代码审查。
pub(crate) trait PropertySource {
    /// 读一页。`offset_units` / `length_units` 的单位是 **4 字节**（X11 规范）。
    fn page(
        &mut self,
        offset_units: u32,
        length_units: u32,
        delete: bool,
    ) -> Result<PropertyPage, String>;
}

/// 生产实现：把请求转发给 `Connection::get_property_page`。
struct ConnPropertySource<'a> {
    conn: &'a mut Connection,
    window: u32,
    property: u32,
    type_atom: u32,
}

impl PropertySource for ConnPropertySource<'_> {
    fn page(
        &mut self,
        offset_units: u32,
        length_units: u32,
        delete: bool,
    ) -> Result<PropertyPage, String> {
        let (format, data, bytes_after, type_atom) = self.conn.get_property_page(
            self.window,
            self.property,
            self.type_atom,
            offset_units,
            length_units,
            delete,
        )?;
        Ok(PropertyPage {
            format,
            type_atom,
            bytes_after,
            data,
        })
    }
}

/// 校验一页与"探测得到的总量"是否自洽。
///
/// `consumed_before` = 本页之前已经累计的字节数。四项检查：
/// 1. `format` 跨页一致；
/// 2. `type_atom` 跨页一致（且与调用方请求的具体类型一致）；
/// 3. 累计长度不超过探测总量；
/// 4. `bytes_after` 必须恰好等于"总量 − 已累计 − 本页"，否则说明属性在读取
///    期间被其他客户端改过（或服务器异常）。
fn check_page(
    page: &PropertyPage,
    format: u8,
    expect_type: u32,
    total: usize,
    consumed_before: usize,
) -> Result<(), String> {
    if page.format != format {
        return Err(format!(
            "GetProperty 分页 format 不一致：首读 {format}、当前 {}",
            page.format
        ));
    }
    // `expect_type` 是**第一页实际返回的 type**（`AnyPropertyType` 时由首片确定），
    // 或调用方显式请求的 type。任何一页与之不同都必须失败：
    // 否则会把"两种不同类型的数据"拼成一个"成功"结果。
    if page.type_atom != expect_type {
        return Err(format!(
            "GetProperty 分页 type 不一致：基准 atom {expect_type}、当前 {}（属性可能在读取期间被修改）",
            page.type_atom
        ));
    }
    let consumed_after = consumed_before + page.data.len();
    if consumed_after > total {
        return Err(format!(
            "GetProperty 读到的数据超过探测长度：已累计 {consumed_after} > 总量 {total}\
             （属性可能在读取期间被修改）"
        ));
    }
    let expect_after = total - consumed_after;
    if page.bytes_after as usize != expect_after {
        return Err(format!(
            "GetProperty 的 bytes_after 与累计长度不一致：服务器说还剩 {}、\
             按总量推算应为 {expect_after}（属性可能在读取期间被修改）",
            page.bytes_after
        ));
    }
    Ok(())
}

/// 完整读取一个属性（分页 + 一致性校验）。`max_bytes` 是硬上限。
///
/// **不是原子快照**：X11 没有"读取期间禁止修改属性"的机制。本函数的策略是
/// "探测长度 + 逐页交叉校验"，一旦发现类型/格式/长度/`bytes_after` 不自洽就
/// **显式失败**，绝不把混合数据伪装成完整结果。
fn collect_property_all(
    src: &mut impl PropertySource,
    type_atom: u32,
    delete_after: bool,
    max_bytes: usize,
) -> Result<(u8, Vec<u8>), String> {
    // 1) 探长度（**不删除**：探测请求一律 delete=false）
    let probe = src.page(0, 0, false)?;
    let format = probe.format;
    // 类型校验：`probe.type_atom == 0` 是 X11 的 **None**，表示**属性不存在** ——
    // 那不是"类型不匹配"，调用方读一个可能不存在的属性应当得到"空"。
    if type_atom != 0 && probe.type_atom != 0 && probe.type_atom != type_atom {
        return Err(format!(
            "GetProperty 类型不匹配：请求 atom {type_atom}、实际 {}",
            probe.type_atom
        ));
    }
    // **分页的类型基准**：调用方指定了类型就用它；`AnyPropertyType`（0）时用
    // **第一页实际返回的 type** 作为基准，后续每一页必须与之相同。
    let expect_type = if type_atom != 0 {
        type_atom
    } else {
        probe.type_atom
    };

    let total = probe.bytes_after as usize;
    if total > max_bytes {
        return Err(format!(
            "属性 {total} 字节超过上限 {max_bytes}：已停止读取（属性未被删除）"
        ));
    }

    // 2) 读全部数据（**带 delete 的请求只在最后单独发一次**，见第 4 步）。
    //
    // 为什么不把 delete 放在数据页上：按协议，只有 `bytes_after == 0` 的读取才会
    // 真正删除属性；若服务器只返回一部分却带着 delete，我们既拿不到剩余数据、
    // 也无法确定属性是否还在。把删除推迟到"数据已完整拿到"之后单独发一次，
    // 既不会丢数据，也能明确报告删除是否生效。
    let mut data = Vec::with_capacity(total.min(max_bytes));
    let mut bytes_after = total as u32;
    let mut pages = 0usize;
    while bytes_after > 0 {
        pages += 1;
        if pages > MAX_PROPERTY_PAGES {
            return Err(format!(
                "GetProperty 分页超过 {MAX_PROPERTY_PAGES} 页仍未读完（{bytes_after} 字节剩余）"
            ));
        }
        // 下一页 offset 的单位是 4 字节：已读长度必须 4 字节对齐，否则无法表达。
        if data.len() % 4 != 0 {
            return Err(format!(
                "GetProperty 分页边界不是 4 字节对齐（已读 {} 字节）",
                data.len()
            ));
        }
        let offset_units = (data.len() / 4) as u32;
        let remaining = total - data.len();
        let want = remaining.div_ceil(4) as u32;
        let page = src.page(offset_units, want, false)?;
        check_page(&page, format, expect_type, total, data.len())?;
        if page.data.is_empty() && page.bytes_after > 0 {
            return Err(format!(
                "GetProperty 分页返回 0 字节但仍有 {} 字节剩余（协议异常）",
                page.bytes_after
            ));
        }
        data.extend_from_slice(&page.data);
        bytes_after = page.bytes_after;
        if data.len() > max_bytes {
            return Err(format!(
                "GetProperty 读取超过上限 {max_bytes} 字节：已停止（属性未被删除）"
            ));
        }
    }

    // 3) 终检：读到的长度必须与探测一致（防止"属性在读取期间被修改"混入）。
    if data.len() != total {
        return Err(format!(
            "GetProperty 读到的长度 {} 与探测的 {total} 不一致（属性可能在读取期间被修改；\
             属性未被删除）",
            data.len()
        ));
    }

    // 4) 删除（仅当调用方要求，且**数据已经完整拿到**）。
    //
    // 为什么要**再读一遍**而不是"在某一页上带 delete"：
    // - 协议规定只有 `bytes_after == 0` 的读取才会删除属性。若把 delete 挂在可能
    //   只返回一部分的数据页上，服务器若按"读完才删"的语义会让我们拿不到剩余数据；
    //   若按"请求即删"的语义则会直接丢数据（两种实现都存在）。
    // - 把一个 `delete=true` 的请求放在**覆盖全部数据**的那一次读取上，才是无歧义的。
    //   因为分页时我们不知道哪一页会是最后一页（服务器可能少给），所以先不带 delete
    //   读全，再单独发一次"整段 + delete"的请求：这次请求一定 `bytes_after == 0`。
    //
    // 代价是调用方要求删除时会多读一遍（剪贴板读取属于这种情况）。换来的是：
    // 既不会丢数据，也能**明确区分**"服务器确实删了"与"只是我们收到了错误"。
    // 同时把这次重读的内容与第一次读到的做比对：不一致说明属性在读期间被改过，
    // 此时删除结果未知，必须报错而不是宣称成功。
    if delete_after {
        let units = total.div_ceil(4) as u32;
        let del = src.page(0, units, true)?;
        if del.type_atom != expect_type {
            return Err(format!(
                "GetProperty 删除请求返回的 type（{}）与预期（{expect_type}）不一致：\
                 属性可能在读取期间被替换；删除结果未知",
                del.type_atom
            ));
        }
        if del.format != format {
            return Err(format!(
                "GetProperty 删除请求返回的 format（{}）与预期（{format}）不一致：\
                 属性可能在读取期间被替换；删除结果未知",
                del.format
            ));
        }
        if del.bytes_after != 0 {
            return Err(format!(
                "GetProperty 删除未生效：属性仍剩 {} 字节（协议上只有 bytes_after==0 的读取\
                 才会删除）；数据已完整读出，但**属性仍然存在**",
                del.bytes_after
            ));
        }
        if del.data != data {
            return Err(format!(
                "GetProperty 删除请求读到的内容（{} 字节）与首次读取（{} 字节）不一致：\
                 属性在读取期间被修改，删除结果未知",
                del.data.len(),
                data.len()
            ));
        }
    }

    Ok((format, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    /// poll_events 必须 **drain reply 的附加数据**，否则后续包会被当成
    /// "不完整包"——旧版（reply 分支为空时）会在这里直接 `Err`。
    #[test]
    fn poll_events_drains_reply_extra_data() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");

        // 1) reply：byte0=1，byte4-5 = units = 1（4 字节附加数据）
        let mut reply = [0u8; 32];
        reply[0] = 1;
        reply[4] = 1;
        theirs.write_all(&reply).expect("write reply");
        theirs
            .write_all(&[0xAA, 0xBB, 0xCC, 0xDD])
            .expect("write extra");

        // 2) 紧跟一个 32 字节事件包（byte0=2 = KeyPress，无附加数据）
        let mut event = [0u8; 32];
        event[0] = 2;
        theirs.write_all(&event).expect("write event");
        theirs.flush().ok();

        let mut conn = Connection::from_stream(ours);
        let first = conn.poll_events(100);
        assert!(
            first.is_ok(),
            "收到 reply 后必须能 drain 附加数据；否则会报不完整包：{first:?}"
        );
        let (events, errors) = first.unwrap();
        assert!(errors.is_empty(), "不应有错误包：{errors:?}");
        // 事件包被正确解析（KeyPress；字段全 0，decode 可能成功或失败，
        // 但**不应影响流对齐**）。这里只断言“没有被 reply 带偏”。
        assert!(events.len() <= 1, "最多一个事件，实测 {events:?}");

        // 3) 再 poll 一次：流应该已清空（无少读/多读）
        let second = conn.poll_events(10).expect("second poll");
        assert!(
            second.0.is_empty(),
            "第二次 poll 不应再有事件（说明流已对齐）：{second:?}"
        );
    }

    /// 对照：reply 的 units=0 时不应多读（保证 drain 不会误伤正常响应）。
    #[test]
    fn poll_events_keeps_stream_aligned_for_plain_reply() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut reply = [0u8; 32];
        reply[0] = 1; // units = 0
        theirs.write_all(&reply).expect("write reply");
        let mut event = [0u8; 32];
        event[0] = 2;
        theirs.write_all(&event).expect("write event");
        theirs.flush().ok();

        let mut conn = Connection::from_stream(ours);
        let first = conn.poll_events(100);
        assert!(first.is_ok(), "units=0 的 reply 不应导致错误：{first:?}");
    }
}

/// **包重组状态机**的测试（`UnixStream::pair()` 灌字节，不需要 X server）。
///
/// 覆盖的正是旧实现会丢字节的路径：`read(2)` 只给半个包。
#[cfg(test)]
mod read_path_tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    /// 一个"可识别"的 32 字节包：首字节 = 事件码，其余按序号填充。
    fn packet(tag: u8) -> [u8; 32] {
        let mut p = [0u8; 32];
        p[0] = tag;
        for (i, b) in p.iter_mut().enumerate() {
            *b ^= (i as u8).wrapping_mul(7).wrapping_add(tag);
        }
        p[0] = tag;
        p
    }

    // ---- next_packet / try_next_packet ----

    #[test]
    fn try_next_packet_returns_none_without_error_when_no_data() {
        let (ours, _theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        // 0 超时 + 没数据 = **正常**的"暂无数据"，不是错误。
        let r = c.try_next_packet(0).expect("无数据不应报错");
        assert!(r.is_none(), "无数据时必须返回 None");
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    #[test]
    fn next_packet_zero_timeout_is_not_an_einval() {
        let (ours, _theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        // 旧实现在这里必定 Err("set_read_timeout: ...")（Linux 的 EINVAL）。
        let err = c.next_packet(0).expect_err("无数据时 next_packet 应报超时");
        assert!(
            !err.contains("set_read_timeout") && !err.contains("InvalidInput"),
            "0 超时不得走 set_read_timeout(0)（Linux EINVAL）：{err}"
        );
        assert!(err.contains("超时"), "错误应说明是超时：{err}");
    }

    #[test]
    fn partial_packet_is_preserved_and_completed_later() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        let want = packet(2); // KeyPress 之类的事件码

        // 只发前 10 字节
        theirs.write_all(&want[..10]).expect("write head");
        theirs.flush().ok();
        assert!(
            c.try_next_packet(0).expect("部分包不应报错").is_none(),
            "10/32 字节不能算一个完整包"
        );
        assert_eq!(
            c.partial_packet_bytes(),
            10,
            "已读的 10 字节必须留着，不能丢"
        );

        // 补上剩下的 22 字节
        theirs.write_all(&want[10..]).expect("write tail");
        theirs.flush().ok();
        let got = c
            .try_next_packet(0)
            .expect("合并后应成功")
            .expect("应有完整包");
        assert_eq!(got, want, "拼出来的包必须与写入的逐字节相同");
        assert_eq!(c.partial_packet_bytes(), 0, "交付后缓冲区必须清空");
    }

    #[test]
    fn next_packet_timeout_keeps_partial_bytes() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        let want = packet(3);
        theirs.write_all(&want[..5]).expect("write");
        theirs.flush().ok();

        let err = c.next_packet(20).expect_err("只有 5 字节，应超时");
        assert!(err.contains("超时"), "{err}");
        assert_eq!(
            c.partial_packet_bytes(),
            5,
            "超时也不能丢掉已读的 5 字节（否则流永久错位）"
        );

        theirs.write_all(&want[5..]).expect("write rest");
        theirs.flush().ok();
        let got = c.next_packet(100).expect("补满后应成功");
        assert_eq!(got, want);
    }

    #[test]
    fn back_to_back_packets_are_delivered_in_order_without_duplicates() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        let a = packet(2);
        let b = packet(3);
        let d = packet(4);
        let mut all = Vec::new();
        all.extend_from_slice(&a);
        all.extend_from_slice(&b);
        all.extend_from_slice(&d);
        theirs.write_all(&all).expect("write 3 packets");
        theirs.flush().ok();

        assert_eq!(c.try_next_packet(0).unwrap(), Some(a));
        assert_eq!(c.try_next_packet(0).unwrap(), Some(b));
        assert_eq!(c.try_next_packet(0).unwrap(), Some(d));
        assert_eq!(
            c.try_next_packet(0).unwrap(),
            None,
            "三个包消费完后不得再产出（不重复消费）"
        );
    }

    #[test]
    fn half_packet_then_rest_plus_next_full_packet() {
        // 场景：a 的前 13 字节先到；随后"a 的剩余 19 字节 + 完整的 b"一起到达。
        // 这是最容易错位的形态：补完 a 之后必须**从正确偏移**继续解析 b。
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        let a = packet(2);
        let b = packet(3);

        theirs.write_all(&a[..13]).expect("write head of a");
        theirs.flush().ok();
        assert!(
            c.try_next_packet(0).unwrap().is_none(),
            "13/32 字节不能算完整包"
        );
        assert_eq!(c.partial_packet_bytes(), 13);

        let mut rest = Vec::new();
        rest.extend_from_slice(&a[13..]);
        rest.extend_from_slice(&b);
        theirs.write_all(&rest).expect("write rest of a + b");
        theirs.flush().ok();

        assert_eq!(c.try_next_packet(0).unwrap(), Some(a), "a 必须完整拼出");
        assert_eq!(c.try_next_packet(0).unwrap(), Some(b), "b 不得错位");
        assert_eq!(c.try_next_packet(0).unwrap(), None);
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    #[test]
    fn packet_split_across_three_writes() {
        // 分三次到达（每次都不足 32 字节）：必须累积到第三个才交付。
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        let a = packet(5);

        for (i, chunk) in [&a[..5], &a[5..17], &a[17..]].into_iter().enumerate() {
            theirs.write_all(chunk).expect("write chunk");
            theirs.flush().ok();
            let got = c.try_next_packet(0).expect("分片读取不应报错");
            if i < 2 {
                assert!(got.is_none(), "第 {i} 次分片不应交付完整包");
                assert_eq!(c.partial_packet_bytes(), [5, 17][i]);
            } else {
                assert_eq!(got, Some(a), "第三次分片后必须拼出完整包");
            }
        }
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    // ---- 与阻塞式读取共享缓冲 ----

    #[test]
    fn blocking_reply_read_consumes_buffered_partial_head() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        // 一个 units=0 的 reply：byte0=1，字节 8..12 是返回值
        let mut reply = [0u8; 32];
        reply[0] = 1;
        reply[8..12].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());

        // 先只给 7 字节，让 `rx` 里留下 7 字节
        theirs.write_all(&reply[..7]).expect("write head");
        theirs.flush().ok();
        let mut c = Connection::from_stream(ours);
        assert!(c.try_next_packet(0).expect("不应报错").is_none());
        assert_eq!(c.partial_packet_bytes(), 7);

        // 再给剩下的 25 字节；阻塞式 `read_reply_u32` 必须**接上**这 7 字节
        theirs.write_all(&reply[7..]).expect("write tail");
        theirs.flush().ok();
        let v = c.read_reply_u32().expect("应能读到 reply");
        assert_eq!(v, 0xDEAD_BEEF, "缓冲里的 7 字节必须被当作包头的一部分");
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    #[test]
    fn poll_events_tolerates_partial_packet_without_desync() {
        let (ours, mut theirs) = UnixStream::pair().expect("pair");
        let mut c = Connection::from_stream(ours);
        let ev = packet(2); // 字节 0 = 2（KeyPress 类事件码）

        // 半个包 → poll_events 必须**正常返回**（不是 Err），并保留已读字节
        theirs.write_all(&ev[..12]).expect("write head");
        theirs.flush().ok();
        let first = c.poll_events(0);
        assert!(
            first.is_ok(),
            "半个包不该让 poll_events 报错（旧实现会 Err 并丢字节）：{first:?}"
        );
        assert_eq!(c.partial_packet_bytes(), 12);

        // 补齐剩余 20 字节 → 下一次 poll 必须把它当成**一个**包消费掉
        theirs.write_all(&ev[12..]).expect("write tail");
        theirs.flush().ok();
        let second = c.poll_events(0).expect("补齐后应成功");
        assert!(
            second.0.len() <= 1,
            "补满后最多解出一个事件，实测 {:?}",
            second.0
        );
        assert_eq!(
            c.partial_packet_bytes(),
            0,
            "消费完必须清空缓冲（不能重复消费）"
        );
        // 再 poll 一次不应有残留
        let third = c.poll_events(0).expect("第三次 poll");
        assert!(third.0.is_empty(), "不应残留事件：{:?}", third.0);
    }
}

/// **阻塞路径超时后的续读**与**Reply 附加数据的分片到达**（任务 A / B）。
///
/// 这些用例针对旧实现的真实缺陷：
/// - `read_head` 先把 `rx` 清空再去读剩余部分 → 超时后已消费字节随局部变量丢失；
/// - `read_packet` / `poll_events` 把附加数据读进**局部 `Vec`** → 超时同样丢字节；
/// - `poll_events` 在非阻塞模式下用 `std::io::Read::read_exact` → `WouldBlock` 立刻报错。
#[cfg(test)]
mod blocking_resume_tests {
    use super::*;
    use std::io::Write;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    fn packet(tag: u8) -> [u8; 32] {
        let mut p = [0u8; 32];
        for (i, b) in p.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(11).wrapping_add(tag);
        }
        p[0] = tag;
        p
    }

    /// 构造一个 `type=1`、`units` 个 4 字节附加数据的 reply 头。
    fn reply_head(units: u32) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = 1;
        h[4..8].copy_from_slice(&units.to_le_bytes());
        h
    }

    fn conn_from(pair: (UnixStream, UnixStream)) -> (Connection, UnixStream) {
        let (ours, theirs) = pair;
        (Connection::from_stream(ours), theirs)
    }

    // ---- 任务 A：阻塞读取超时不得丢字节 ----

    #[test]
    fn blocking_timeout_keeps_bytes_already_taken_from_rx() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        c.set_reply_timeout_for_test(Duration::from_millis(60));
        let want = packet(2);

        // 先让 poll_events 把 20 字节读进 rx（非阻塞）
        peer.write_all(&want[..20]).expect("write 20");
        peer.flush().ok();
        assert!(c.try_next_packet(0).expect("try").is_none());
        assert_eq!(c.partial_packet_bytes(), 20);

        // 再给 5 字节（共 25，仍不足 32）→ 阻塞读必须超时
        peer.write_all(&want[20..25]).expect("write 5");
        peer.flush().ok();
        let err = c.read_packet().expect_err("25/32 字节应超时");
        assert!(err.contains("读超时"), "错误应说明超时：{err}");
        assert_eq!(
            c.partial_packet_bytes(),
            25,
            "**超时不能丢掉已经从 socket 消费的字节**（含 rx 里原有的 20 字节）"
        );

        // 补上剩余 7 字节 → 必须拼出**原始**包
        peer.write_all(&want[25..]).expect("write rest");
        peer.flush().ok();
        let (head, extra) = c.read_packet().expect("续读应成功");
        assert_eq!(head, want, "续读必须得到原始完整包");
        assert!(extra.is_empty());

        // 紧跟的第二个包不能重复/丢失
        let second = packet(3);
        peer.write_all(&second).expect("write second");
        peer.flush().ok();
        let (head2, _) = c.read_packet().expect("第二个包");
        assert_eq!(head2, second, "第二个包不得错位或重复");
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    #[test]
    fn blocking_timeout_then_eof_is_a_distinct_error() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        c.set_reply_timeout_for_test(Duration::from_millis(40));
        let want = packet(4);
        peer.write_all(&want[..8]).expect("write");
        peer.flush().ok();
        let err = c.read_packet().expect_err("应超时");
        assert!(err.contains("读超时"), "{err}");

        // 对端关闭：下一次读必须是**明确的 EOF**，而不是伪造成"完整包"或继续超时
        peer.shutdown(Shutdown::Both).ok();
        drop(peer);
        let err = c.read_packet().expect_err("EOF 必须报错");
        assert!(
            err.contains("对端关闭连接"),
            "EOF 应有明确语义（不要把半包当完整包）：{err}"
        );
    }

    // ---- 任务 B：Reply 附加数据分片 ----

    #[test]
    fn reply_extra_split_across_arrivals_resumes_correctly() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        c.set_reply_timeout_for_test(Duration::from_millis(60));

        let head = reply_head(2); // 8 字节附加数据
        let extra = [0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44];
        peer.write_all(&head).expect("write head");
        peer.write_all(&extra[..4]).expect("write half extra");
        peer.flush().ok();

        let err = c.read_packet().expect_err("附加数据只有一半，应超时");
        assert!(err.contains("读超时"), "{err}");
        assert_eq!(
            c.pending_reply_extra_bytes(),
            4,
            "已读的 4 字节附加数据必须保留（不能随局部 Vec 丢弃）"
        );

        peer.write_all(&extra[4..]).expect("write rest extra");
        peer.flush().ok();
        let (h2, got) = c.read_packet().expect("续读应成功");
        assert_eq!(h2, head, "包头不得重复消费");
        assert_eq!(got, extra, "附加数据必须逐字节相同");
        assert_eq!(c.pending_reply_extra_bytes(), 0);

        // 紧跟一个普通事件包：不能被 reply 的附加数据误消费
        let ev = packet(2);
        peer.write_all(&ev).expect("write event");
        peer.flush().ok();
        let (h3, e3) = c.read_packet().expect("事件包");
        assert_eq!(h3, ev);
        assert!(e3.is_empty());
    }

    #[test]
    fn poll_events_finishes_partial_reply_extra_before_parsing_next_packet() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let head = reply_head(2);
        let extra = [1u8, 2, 3, 4, 5, 6, 7, 8];
        peer.write_all(&head).expect("head");
        peer.write_all(&extra[..4]).expect("half extra");
        peer.flush().ok();

        // 第一轮：reply 附加数据不完整 → 返回 Ok，不报错，进度保留
        let (events, errors) = c.poll_events(0).expect("非阻塞 poll 不应报错");
        assert!(events.is_empty(), "reply 不该产生事件：{events:?}");
        assert!(errors.is_empty());
        assert_eq!(c.pending_reply_extra_bytes(), 4, "进度必须保留");

        // 补齐附加数据 + 一个事件包
        peer.write_all(&extra[4..]).expect("rest extra");
        let ev = packet(2);
        peer.write_all(&ev).expect("event");
        peer.flush().ok();

        let (events, errors) = c.poll_events(0).expect("第二轮 poll");
        assert!(errors.is_empty());
        assert_eq!(
            events.len(),
            1,
            "**必须先消费完 reply 附加数据再解析事件**，实测 {events:?}"
        );
        assert_eq!(c.pending_reply_extra_bytes(), 0);
        // 事件没有被附加数据污染：再 poll 一次应为空
        let (events, _) = c.poll_events(0).expect("第三轮 poll");
        assert!(events.is_empty(), "不得残留/重复事件：{events:?}");
    }

    #[test]
    fn poll_events_eof_during_reply_extra_is_an_error() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let head = reply_head(2);
        peer.write_all(&head).expect("head");
        peer.write_all(&[9u8, 8, 7]).expect("partial extra");
        peer.flush().ok();
        peer.shutdown(Shutdown::Both).ok();
        drop(peer);

        // 附加数据读不全 + EOF：必须是 Err，**不能**伪造一个"完整 reply"
        let r = c.poll_events(50);
        assert!(r.is_err(), "EOF 时必须报错，而不是假装成功：{r:?}");
    }

    #[test]
    fn reply_event_reply_boundaries_are_respected() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let r1 = reply_head(1);
        let e1 = packet(2);
        let r2 = reply_head(1);
        let mut buf = Vec::new();
        buf.extend_from_slice(&r1);
        buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        buf.extend_from_slice(&e1);
        buf.extend_from_slice(&r2);
        buf.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
        peer.write_all(&buf).expect("write");
        peer.flush().ok();

        // ① reply1 + 它的附加数据
        let (h1, x1) = c.read_packet().expect("reply1");
        assert_eq!(h1, r1);
        assert_eq!(x1, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        // ② 事件夹在两条 reply 之间：同步读 reply 的路径必须**跳过事件并留住它**，
        //    然后拿到 reply2 的完整数据（这是 `read_reply_packet` 的既有契约）。
        let (h2, x2) = c.read_reply_packet().expect("reply2");
        assert_eq!(h2, r2);
        assert_eq!(x2, vec![0x01, 0x02, 0x03, 0x04]);

        // ③ 被跳过的事件此时必须在 `pending` 里能取到（没被吞、没错位）。
        let (events, errors) = c.poll_events(0).expect("poll");
        assert!(errors.is_empty());
        assert_eq!(
            events.len(),
            1,
            "夹在两条 reply 之间的事件必须被保留下来：{events:?}"
        );

        // ④ 流已空：再读必须超时（而不是读到残留字节）
        c.set_reply_timeout_for_test(Duration::from_millis(30));
        let err = c.read_packet().expect_err("流已空应超时");
        assert!(err.contains("读超时"), "{err}");
    }

    // ---- 任务 D：try_next_packet 的总时限 ----

    #[test]
    fn try_next_packet_is_bounded_under_slow_drip() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        // 对端每 10ms 送 1 字节（远小于 32），持续 40 字节 = 400ms。
        let writer = std::thread::spawn(move || {
            let want = packet(7);
            // 32 字节分 4 次送完（每次 8 字节，间隔 10ms），总时长远超 wait_ms。
            for (i, b) in want.iter().enumerate() {
                if peer.write_all(&[*b]).is_err() {
                    return;
                }
                peer.flush().ok();
                // 每 8 字节停 40ms：单次 read 拿不到完整包头，总发送时长 ≈120ms > wait_ms=100ms。
                if i % 8 == 7 {
                    std::thread::sleep(Duration::from_millis(40));
                }
            }
        });

        let t0 = Instant::now();
        let got = c.try_next_packet(100).expect("不应报错");
        let elapsed = t0.elapsed();
        assert!(
            got.is_none(),
            "100ms 内只到了部分字节，不该返回完整包：{got:?}"
        );
        // 两侧都有界：不能提前返回（证明真的在等），也不能超过 wait_ms（证明总时限生效）。
        assert!(
            elapsed >= Duration::from_millis(80),
            "应当等到接近 wait_ms 才返回，实际 {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "**总等待必须受 wait_ms 约束**（旧实现会被持续成功的小读无限延长）：实际 {elapsed:?}"
        );
        assert!(
            c.partial_packet_bytes() > 0,
            "超时返回 None 时，已读字节必须保留"
        );

        // 等对端把剩余的字节发完，再读：必须拼出完整包
        writer.join().ok();
        let got = c.try_next_packet(2000).expect("等待剩余字节");
        assert!(got.is_some(), "补齐后必须得到完整包");
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    #[test]
    fn try_next_packet_zero_wait_returns_immediately() {
        let (mut c, _peer) = conn_from(UnixStream::pair().expect("pair"));
        let t0 = Instant::now();
        assert!(c.try_next_packet(0).expect("try").is_none());
        assert!(
            t0.elapsed() < Duration::from_millis(50),
            "0 等待必须立即返回，实际 {:?}",
            t0.elapsed()
        );
    }
}

/// **GetProperty 分页一致性**的可控测试（任务 C）。
///
/// 为什么必须用测试替身：Xvfb 上 `GetProperty` 一次就返回整个属性，
/// 续页分支（`bytes_after > 0`）**永远走不到**。上一轮"600 KiB 属性读回"
/// 只证明了单页路径，不能证明分页逻辑正确。这里用脚本化的 `PropertySource`
/// 精确制造每一种异常。
#[cfg(test)]
mod property_paging_tests {
    use super::*;
    use std::collections::VecDeque;

    /// 按脚本返回预设页面，并记录每次请求的 `(offset, length, delete)`。
    #[derive(Default)]
    struct Scripted {
        calls: Vec<(u32, u32, bool)>,
        replies: VecDeque<Result<PropertyPage, String>>,
    }

    impl Scripted {
        fn new(pages: Vec<PropertyPage>) -> Self {
            Self {
                calls: Vec::new(),
                replies: pages.into_iter().map(Ok).collect(),
            }
        }
    }

    impl PropertySource for Scripted {
        fn page(
            &mut self,
            offset_units: u32,
            length_units: u32,
            delete: bool,
        ) -> Result<PropertyPage, String> {
            self.calls.push((offset_units, length_units, delete));
            self.replies
                .pop_front()
                .unwrap_or_else(|| Err("脚本用尽：出现了未预期的额外分页请求".to_string()))
        }
    }

    fn page(format: u8, type_atom: u32, bytes_after: u32, data: Vec<u8>) -> PropertyPage {
        PropertyPage {
            format,
            type_atom,
            bytes_after,
            data,
        }
    }

    fn bytes(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    // ---- 正常路径 ----

    #[test]
    fn two_pages_are_joined_and_offset_is_in_4_byte_units() {
        const TY: u32 = 7;
        let all = bytes(100);
        let mut src = Scripted::new(vec![
            page(8, TY, 100, Vec::new()),        // 探测：总长 100
            page(8, TY, 60, all[..40].to_vec()), // 第一页 40 字节，还剩 60
            page(8, TY, 0, all[40..].to_vec()),  // 第二页 60 字节，读完
        ]);
        let (format, data) = collect_property_all(&mut src, TY, false, 1 << 20).expect("应成功");
        assert_eq!(format, 8);
        assert_eq!(data.len(), 100, "两页必须拼接成完整长度");
        assert!(data == bytes(100), "内容必须逐字节相同");
        // 探测页 (0,0)；首页 (0, ceil(100/4)=25)；续页 offset = 40/4 = 10、length = ceil(60/4)=15
        assert_eq!(src.calls[0], (0, 0, false));
        assert_eq!(src.calls[1], (0, 25, false));
        assert_eq!(
            src.calls[2],
            (10, 15, false),
            "续页 offset 必须按 4 字节单位计算（40 字节 → 10）"
        );
    }

    #[test]
    fn empty_property_without_delete_makes_one_call() {
        let mut src = Scripted::new(vec![page(0, 0, 0, Vec::new())]);
        let (format, data) = collect_property_all(&mut src, 7, false, 1 << 20).expect("空属性");
        assert_eq!(format, 0);
        assert!(data.is_empty());
        assert_eq!(src.calls.len(), 1, "空属性不应触发数据页");
    }

    #[test]
    fn empty_property_with_delete_issues_a_delete_page() {
        let mut src = Scripted::new(vec![page(0, 0, 0, Vec::new()), page(0, 0, 0, Vec::new())]);
        collect_property_all(&mut src, 0, true, 1 << 20).expect("空属性 + delete");
        assert_eq!(src.calls[0], (0, 0, false), "探测页不得带 delete");
        assert_eq!(src.calls[1], (0, 0, true), "空属性也要按要求删除");
    }

    /// **delete 语义**：数据页一律 `delete=false`，读完后再单独发一次删除请求
    ///（`offset = ceil(total/4)`、`length = 0`、`delete = true`）。
    #[test]
    fn delete_is_issued_once_after_all_data_is_read() {
        let mut src = Scripted::new(vec![
            page(8, 7, 4, Vec::new()), // 探测
            page(8, 7, 0, bytes(4)),   // 数据页（不带 delete）
            page(8, 7, 0, bytes(4)),   // 删除请求：整段 + delete，内容与首次一致
        ]);
        let (_, data) = collect_property_all(&mut src, 7, true, 1 << 20).expect("读满 + delete");
        assert_eq!(data, bytes(4));
        assert_eq!(
            src.calls[1],
            (0, 1, false),
            "数据页不得带 delete（服务器可能只返回一部分）"
        );
        assert_eq!(
            src.calls[2],
            (0, 1, true),
            "删除请求必须在数据读完后单独发：offset=0、length=ceil(4/4)=1、delete=true"
        );
    }

    /// 部分页 + `delete=true`：属性此时不会被删，数据仍要读完，随后删除成功。
    #[test]
    fn partial_pages_with_delete_still_read_everything_then_delete() {
        let all = bytes(100);
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            page(8, 7, 60, all[..40].to_vec()),
            page(8, 7, 0, all[40..].to_vec()),
            page(8, 7, 0, all.clone()), // 删除请求：整段重读 + delete
        ]);
        let (_, data) = collect_property_all(&mut src, 7, true, 1 << 20).expect("应成功");
        assert_eq!(data, all, "部分页不得阻止读全");
        assert!(
            src.calls[1..3].iter().all(|(_, _, del)| !*del),
            "所有数据页都不得带 delete：{:?}",
            &src.calls[1..3]
        );
        assert_eq!(src.calls[3], (0, 25, true), "最后单独删除（整段重读）");
    }

    /// 删除请求读到的内容与首次不一致：属性在读期间被改过，删除结果未知 → 必须报错。
    #[test]
    fn delete_readback_mismatch_is_reported() {
        let mut src = Scripted::new(vec![
            page(8, 7, 4, Vec::new()),
            page(8, 7, 0, bytes(4)),
            page(8, 7, 0, vec![9u8, 9, 9, 9]), // 内容变了
        ]);
        let err = collect_property_all(&mut src, 7, true, 1 << 20).expect_err("必须报错");
        assert!(
            err.contains("不一致") && err.contains("删除结果未知"),
            "{err}"
        );
    }

    /// 删除请求返回 `bytes_after != 0`：按协议**没有删除**，必须如实报告。
    #[test]
    fn delete_not_effective_is_reported_honestly() {
        let mut src = Scripted::new(vec![
            page(8, 7, 4, Vec::new()),
            page(8, 7, 0, bytes(4)),
            page(8, 7, 4, Vec::new()), // 服务器说还剩 4 字节 → 没删
        ]);
        let err = collect_property_all(&mut src, 7, true, 1 << 20).expect_err("必须报告未删除");
        assert!(err.contains("删除未生效"), "{err}");
        assert!(err.contains("属性仍然存在"), "必须说明属性是否还在：{err}");
    }

    /// 删除页 type 变了（属性在读取期间被替换）：删除结果未知，必须报错。
    #[test]
    fn delete_page_type_change_is_reported() {
        let mut src = Scripted::new(vec![
            page(8, 7, 4, Vec::new()),
            page(8, 7, 0, bytes(4)),
            page(8, 8, 0, bytes(4)), // 删除请求返回的 type 变了
        ]);
        let err = collect_property_all(&mut src, 7, true, 1 << 20).expect_err("必须报错");
        assert!(err.contains("删除") && err.contains("type"), "{err}");
    }

    // ---- 类型 / 格式一致性 ----

    #[test]
    fn probe_type_mismatch_is_rejected() {
        let mut src = Scripted::new(vec![page(8, 99, 100, Vec::new())]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("类型不符必须失败");
        assert!(err.contains("类型不匹配"), "{err}");
        assert_eq!(src.calls.len(), 1, "类型不符时不应继续读数据");
    }

    /// `AnyPropertyType`：第一页实际 type 成为基准，后续页必须同 type（一致 → 成功）。
    #[test]
    fn any_property_type_accepts_consistent_pages() {
        let all = bytes(100);
        let mut src = Scripted::new(vec![
            page(8, 99, 100, Vec::new()),
            page(8, 99, 60, all[..40].to_vec()),
            page(8, 99, 0, all[40..].to_vec()),
        ]);
        let (_, data) = collect_property_all(&mut src, 0, false, 1 << 20).expect("Any 跨页一致");
        assert_eq!(data, all);
    }

    /// `AnyPropertyType` 下第二页 type 变了：**必须失败**（不能把两类数据拼起来）。
    #[test]
    fn any_property_type_rejects_type_change_on_later_page() {
        let mut src = Scripted::new(vec![
            page(8, 99, 100, Vec::new()),
            page(8, 99, 60, bytes(40)),
            page(8, 100, 0, bytes(60)), // 第二页类型变了
        ]);
        let err = collect_property_all(&mut src, 0, false, 1 << 20)
            .expect_err("Any 模式下 type 变化也必须失败");
        assert!(err.contains("type 不一致"), "{err}");
        assert!(
            err.contains("99") && err.contains("100"),
            "应指出两边的 type：{err}"
        );
    }

    #[test]
    fn type_change_on_later_page_is_rejected() {
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            page(8, 7, 60, bytes(40)),
            page(8, 8, 0, bytes(60)), // 类型变了
        ]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("类型变化必须失败");
        assert!(err.contains("type 不一致"), "{err}");
    }

    #[test]
    fn format_change_on_later_page_is_rejected() {
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            page(8, 7, 60, bytes(40)),
            page(16, 7, 0, bytes(60)),
        ]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("格式变化必须失败");
        assert!(err.contains("format 不一致"), "{err}");
    }

    // ---- 长度 / bytes_after 一致性 ----

    #[test]
    fn bytes_after_inconsistent_with_accumulated_length_is_rejected() {
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            // 40 字节已读，服务器却说还剩 50（应为 60）→ 属性被改过或协议异常
            page(8, 7, 50, bytes(40)),
        ]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("不一致必须失败");
        assert!(
            err.contains("bytes_after") && err.contains("不一致"),
            "{err}"
        );
    }

    #[test]
    fn data_longer_than_probe_length_is_rejected() {
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            page(8, 7, 0, bytes(120)), // 比探测到的 100 还长
        ]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("超长必须失败");
        assert!(err.contains("超过探测长度"), "{err}");
    }

    #[test]
    fn short_total_is_rejected_by_final_length_check() {
        // 两页拼起来只有 80 字节，但探测说 100：必须失败，不能返回短数据。
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            page(8, 7, 60, bytes(40)),
            // 服务器提前说"读完了"：60 剩余却报 after=0 → bytes_after 先不一致
            page(8, 7, 0, bytes(40)),
        ]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("短读必须失败");
        assert!(
            err.contains("bytes_after") || err.contains("不一致"),
            "必须报告不一致：{err}"
        );
    }

    #[test]
    fn empty_page_while_bytes_remain_is_rejected() {
        let mut src = Scripted::new(vec![
            page(8, 7, 100, Vec::new()),
            page(8, 7, 60, bytes(0)), // 0 字节但还有 60
        ]);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("空页必须失败");
        // check_page 会先命中 bytes_after 不一致（total-0=100 ≠ 60）
        assert!(err.contains("不一致"), "{err}");
    }

    // ---- 上限与页数保护 ----

    #[test]
    fn oversized_property_is_rejected_before_reading_data() {
        let mut src = Scripted::new(vec![page(8, 7, 5000, Vec::new())]);
        let err = collect_property_all(&mut src, 7, false, 1024).expect_err("超限必须失败");
        assert!(err.contains("上限") && err.contains("5000"), "{err}");
        assert_eq!(src.calls.len(), 1, "超限时不得读数据页（属性也未被删除）");
    }

    #[test]
    fn page_cap_is_enforced() {
        // 每页只给 4 字节，永远读不完 → 必须在上限页数处失败（**不能无界循环**）。
        let total: u32 = 4 * (MAX_PROPERTY_PAGES as u32 + 10);
        let mut pages = vec![page(8, 7, total, Vec::new())];
        let mut consumed = 0u32;
        for _ in 0..(MAX_PROPERTY_PAGES as u32 + 5) {
            consumed += 4;
            pages.push(page(8, 7, total - consumed, bytes(4)));
        }
        let mut src = Scripted::new(pages);
        let err = collect_property_all(&mut src, 7, false, 1 << 20).expect_err("页数必须受限");
        assert!(err.contains("分页超过"), "{err}");
        assert!(
            src.calls.len() <= MAX_PROPERTY_PAGES + 2,
            "请求数必须被页数上限约束，实际 {}",
            src.calls.len()
        );
    }

    // ---- delete 语义 ----
}

/// **慢速分片下的总 deadline**（任务 A / B）与 **GenericEvent 附加数据**（任务 D）。
#[cfg(test)]
mod deadline_and_generic_event_tests {
    use super::*;
    use std::io::Write;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    fn packet(tag: u8) -> [u8; 32] {
        let mut p = [0u8; 32];
        for (i, b) in p.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(13).wrapping_add(tag);
        }
        p[0] = tag;
        p
    }

    /// `kind` 字节 + `units` 个 4 字节附加数据（Reply=1 / GenericEvent=35）。
    fn head_with_units(kind: u8, units: u32) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = kind;
        h[4..8].copy_from_slice(&units.to_le_bytes());
        h
    }

    fn conn_from(pair: (UnixStream, UnixStream)) -> (Connection, UnixStream) {
        let (ours, theirs) = pair;
        (Connection::from_stream(ours), theirs)
    }

    // ---- A1：阻塞读取包头的慢速分片 ----

    #[test]
    fn blocking_head_read_respects_total_deadline_under_slow_drip() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        // 预算 120ms；对端每 60ms 送 8 字节 → 送完 32 字节需 180ms，必然超时。
        c.set_reply_timeout_for_test(Duration::from_millis(120));
        let want = packet(2);
        let writer = std::thread::spawn(move || {
            for chunk in want.chunks(8) {
                if peer.write_all(chunk).is_err() {
                    return;
                }
                peer.flush().ok();
                std::thread::sleep(Duration::from_millis(60));
            }
        });

        let t0 = Instant::now();
        let err = c.read_packet().expect_err("慢速分片必须按总预算超时");
        let elapsed = t0.elapsed();
        assert!(err.contains("读超时"), "错误应说明超时：{err}");
        assert!(
            elapsed >= Duration::from_millis(100),
            "应当接近预算才返回（证明真的在等），实际 {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(600),
            "**总 deadline 必须覆盖持续成功的分片读取**：实际 {elapsed:?}"
        );
        let kept = c.partial_packet_bytes();
        assert!(
            kept > 0 && kept < 32,
            "超时前读到的字节必须保留（实际 {kept}）"
        );

        writer.join().ok();
        let (head, extra) = c.read_packet().expect("补齐后应成功");
        assert_eq!(head, want, "续读必须得到原始包");
        assert!(extra.is_empty());
        assert_eq!(c.partial_packet_bytes(), 0);
    }

    // ---- A2：阻塞读取 Reply 附加数据的慢速分片 ----

    #[test]
    fn blocking_tail_read_respects_total_deadline_under_slow_drip() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        c.set_reply_timeout_for_test(Duration::from_millis(120));
        let head = head_with_units(1, 4); // 16 字节附加数据
        let extra: Vec<u8> = (0..16).map(|i| 0xA0u8.wrapping_add(i)).collect();

        peer.write_all(&head).expect("head");
        peer.flush().ok();
        let for_writer = extra.clone();
        let writer = std::thread::spawn(move || {
            for chunk in for_writer.chunks(4) {
                if peer.write_all(chunk).is_err() {
                    return;
                }
                peer.flush().ok();
                std::thread::sleep(Duration::from_millis(60));
            }
        });

        let t0 = Instant::now();
        let err = c.read_packet().expect_err("附加数据慢速到达必须超时");
        let elapsed = t0.elapsed();
        assert!(err.contains("读超时"), "{err}");
        assert!(err.contains("附加数据"), "错误应指明是附加数据：{err}");
        assert!(
            elapsed < Duration::from_millis(600),
            "总 deadline 必须覆盖附加数据的分片读取：实际 {elapsed:?}"
        );
        let missing = c.pending_reply_extra_bytes();
        assert!(
            missing > 0 && missing < 16,
            "已读附加数据必须保留（还差 {missing}）"
        );

        writer.join().ok();
        let (h2, got) = c.read_packet().expect("补齐后应恢复完整 Reply");
        assert_eq!(h2, head, "包头不得重复消费");
        assert_eq!(got, extra, "附加数据必须逐字节恢复");
        assert_eq!(c.pending_reply_extra_bytes(), 0);
    }

    // ---- B1 / B2：try_next_packet 的总时限覆盖附加数据 ----

    #[test]
    fn try_next_packet_deadline_covers_reply_extra_and_keeps_event_once() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let head = head_with_units(1, 5); // 20 字节附加数据
        let extra: Vec<u8> = (0..20).map(|i| i as u8).collect();
        let ev = packet(2);

        peer.write_all(&head).expect("head");
        peer.write_all(&extra[..4]).expect("first extra chunk");
        peer.flush().ok();

        // ① 第一次：拿到完整包头（附加数据登记为"待读"）
        let got = c.try_next_packet(0).expect("第一次").expect("应拿到包头");
        assert_eq!(got, head);
        assert_eq!(
            c.pending_reply_extra_bytes(),
            20,
            "包头返回时附加数据尚未读取（20 字节待读）"
        );

        // ② 持续但很慢地送到（每 40ms 4 字节）：
        //    - 分片间隔 **小于** 单次 socket 超时（60ms）→ 每次 read 都会成功；
        //    - 若有共享 deadline，则 ~60ms 就返回；
        //    - 若无 deadline（旧实现），会一路读到 160ms 才返回。
        //    断言 `<120ms` 才能真正区分这两种实现。
        let mut peer_writer = peer.try_clone().expect("clone stream");
        let writer = std::thread::spawn(move || {
            for chunk in extra[4..].chunks(4) {
                std::thread::sleep(Duration::from_millis(40));
                if peer_writer.write_all(chunk).is_err() {
                    return;
                }
                peer_writer.flush().ok();
            }
        });

        let t0 = Instant::now();
        let got = c.try_next_packet(60).expect("第二次不应报错");
        let elapsed = t0.elapsed();
        assert!(
            got.is_none(),
            "附加数据未读完时不能返回下一个包（更不能把附加数据当包头）：{got:?}"
        );
        assert!(
            elapsed < Duration::from_millis(120),
            "**附加数据读取必须共享 wait_ms 预算**（旧实现会被持续成功的小读拖到 160ms+）：实际 {elapsed:?}"
        );
        let pending = c.pending_reply_extra_bytes();
        assert!(
            pending > 0 && pending < 20,
            "必须在预算内读到一部分并保留（还差 {pending}）"
        );

        // ③ 写完剩余附加数据 + 一个普通事件；再读必须恰好得到那个事件
        writer.join().expect("writer");
        peer.write_all(&ev).expect("event");
        peer.flush().ok();
        let got = c
            .try_next_packet(1000)
            .expect("第三次")
            .expect("应拿到事件");
        assert_eq!(got, ev, "事件必须恰好出现一次，且未被附加数据污染");
        assert_eq!(c.pending_reply_extra_bytes(), 0);
        assert!(c.try_next_packet(0).expect("第四次").is_none());
    }

    // ---- D：GenericEvent ----

    #[test]
    fn generic_event_without_extra_is_readable() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let ge = head_with_units(35, 0);
        peer.write_all(&ge).expect("write");
        peer.flush().ok();
        assert_eq!(c.try_next_packet(50).expect("try").expect("应有包"), ge);
        assert!(c.try_next_packet(0).expect("再读").is_none());
    }

    #[test]
    fn generic_event_extra_is_drained_before_next_event() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let ge = head_with_units(35, 2); // 8 字节附加数据
        let extra = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let ev = packet(2);
        let mut buf = Vec::new();
        buf.extend_from_slice(&ge);
        buf.extend_from_slice(&extra);
        buf.extend_from_slice(&ev);
        peer.write_all(&buf).expect("write");
        peer.flush().ok();

        // `poll_events`：GenericEvent 本轮不支持（会被丢弃），但**必须先 drain 附加数据**，
        // 否则后面的普通事件会从错位处解析。
        let (events, errors) = c.poll_events(0).expect("poll");
        assert!(errors.is_empty());
        assert_eq!(
            events.len(),
            1,
            "GenericEvent 之后的事件必须被正确解析（附加数据不得当成事件头）：{events:?}"
        );
        let (again, _) = c.poll_events(0).expect("poll2");
        assert!(again.is_empty(), "不得重复解析：{again:?}");
    }

    #[test]
    fn generic_event_extra_split_does_not_leak_into_event() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let ge = head_with_units(35, 2);
        let extra = [9u8, 8, 7, 6, 5, 4, 3, 2];
        let ev = packet(3);
        peer.write_all(&ge).expect("head");
        peer.write_all(&extra[..3]).expect("partial extra");
        peer.flush().ok();

        let (events, errors) = c.poll_events(0).expect("poll 1");
        assert!(errors.is_empty());
        assert!(
            events.is_empty(),
            "附加数据未完整时不得产生事件：{events:?}"
        );
        assert!(c.pending_reply_extra_bytes() > 0, "进度必须保留");

        peer.write_all(&extra[3..]).expect("rest extra");
        peer.write_all(&ev).expect("event");
        peer.flush().ok();
        let (events, _) = c.poll_events(0).expect("poll 2");
        assert_eq!(events.len(), 1, "收齐后事件必须恰好出现一次：{events:?}");
    }

    #[test]
    fn generic_event_eof_during_extra_is_an_error() {
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let ge = head_with_units(35, 2);
        peer.write_all(&ge).expect("head");
        peer.write_all(&[1u8, 2]).expect("partial extra");
        peer.flush().ok();
        peer.shutdown(Shutdown::Both).ok();
        drop(peer);
        let r = c.poll_events(100);
        assert!(r.is_err(), "EOF 时必须报错，不能伪造完整事件：{r:?}");
    }

    #[test]
    fn read_packet_returns_generic_event_payload_for_future_support() {
        // 阻塞路径把 GenericEvent 的附加数据**完整读出并交给调用方**，
        // 将来支持 GenericEvent 时不需要改读取层。
        let (mut c, mut peer) = conn_from(UnixStream::pair().expect("pair"));
        let ge = head_with_units(35, 1);
        let extra = [0xABu8, 0xCD, 0xEF, 0x01];
        peer.write_all(&ge).expect("head");
        peer.write_all(&extra).expect("extra");
        peer.flush().ok();
        let (head, payload) = c.read_packet().expect("read");
        assert_eq!(head, ge);
        assert_eq!(payload, extra, "GenericEvent 附加数据必须完整返回");
    }
}
