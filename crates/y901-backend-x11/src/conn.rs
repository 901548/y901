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
                width: 0,
                height: 0,
                root_visual: 0,
                root_depth: 24,
                min_keycode: 0,
                max_keycode: 0,
            },
            pending: Vec::new(),
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
        let hostname = std::env::var("HOSTNAME").unwrap_or_default();
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
        let chosen: Option<(u32, u16, u16, u32, u8)> = {
            let root = u32at(&body, off);
            let width = u16at(&body, off + 20);
            let height = u16at(&body, off + 22);
            let root_visual = u32at(&body, off + 32);
            let root_depth = body[off + 38];
            if root != 0 && width > 0 && height > 0 && root_depth > 0 {
                Some((root, width, height, root_visual, root_depth))
            } else {
                None
            }
        };
        let Some((root, width, height, root_visual, root_depth)) = chosen else {
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
        self.stream
            .set_read_timeout(Some(Duration::from_millis(timeout_ms as u64)))
            .map_err(|e| format!("set_read_timeout: {e}"))?;
        let mut buf = [0u8; 32];
        self.stream
            .read_exact(&mut buf)
            .map_err(|e| format!("读包失败: {e}"))?;
        Ok(buf)
    }

    /// 读属性（`GetProperty`）。
    ///
    /// 返回：`(format, data)`。属性不存在或为空时 `data` 为空。
    pub fn get_property(
        &mut self,
        window: u32,
        property: u32,
        type_atom: u32,
        long_offset: u32,
        long_length: u32,
        delete_after: bool,
    ) -> Result<(u8, Vec<u8>), String> {
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
        let n_items = u32::from_le_bytes([head[16], head[17], head[18], head[19]]) as usize;
        // format=8 时每项 1 字节；32 时每项 4 字节
        let bytes = match format {
            8 => n_items,
            16 => n_items * 2,
            32 => n_items * 4,
            _ => 0,
        };
        data.truncate(bytes.min(data.len()));
        Ok((format, data))
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
    /// 超时：由 `read_exact` 提供（200ms 轮询 + 5s 总限）；本函数**不另设 deadline**，
    /// 否则会与调用方（如 `sync`）的 deadline 重叠。
    fn read_packet(&mut self) -> Result<([u8; 32], Vec<u8>), String> {
        let mut head = [0u8; 32];
        self.read_exact(&mut head)?;
        let units = if head[0] == 1 || head[0] == 35 {
            u16::from_le_bytes([head[4], head[5]]) as usize
        } else {
            0
        };
        let mut extra = vec![0u8; units * 4];
        if !extra.is_empty() {
            self.read_exact(&mut extra)?;
        }
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
        let mut buf = [0u8; 32];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Err("服务端断开连接".to_string()),
                Ok(n) if n == 32 => {
                    match buf[0] {
                        0 => errors.push(parse_error(&buf)),
                        // 字节 0 == 1 是**响应**（reply），不是事件。
                        // 不区分的话，InternAtom/GetInputFocus 的响应会被当成 Unknown 事件丢给上层。
                        1 => {
                            // **防御性 drain**：正常情况下主连接的 reply 都在
                            // `read_reply_packet` / `sync` 里已读完，不会到这里；
                            // 但若将来加运行期请求（如实时查 `_NET_WM_STATE`），
                            // 它的 reply 可能先被 `poll_events` 摸到。
                            // 若不 drain，剩下的 `units*4` 字节会把后续每一包都错位
                            // （与 64c6088 修的是同一类灾难）。
                            //
                            // 选**drain 而不是报错**：poll_events 的调用方只想要事件，
                            // reply 对它没用；报错会让一个"无害的并发"升级成启动失败。
                            let units = u16::from_le_bytes([buf[4], buf[5]]) as usize;
                            let mut extra = vec![0u8; units * 4];
                            if !extra.is_empty() {
                                self.stream.read_exact(&mut extra).map_err(|e| {
                                    format!("poll_events 收到 reply，drain 附加数据失败: {e}")
                                })?;
                            }
                        }
                        _ => {
                            if let Ok(ev) = decode_event(&buf) {
                                events.push(ev);
                            }
                        }
                    }
                }
                Ok(n) => return Err(format!("收到 {n} 字节的不完整包（X11 事件固定 32 字节）")),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    return Ok((events, errors))
                }
                // 信号中断不是错误：重试即可（否则会被当成“读错”升级成断线）。
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(format!("读事件失败: {e}")),
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
                return UnixStream::connect_addr(&addr).map_err(|abs_err| {
                    std::io::Error::new(
                        abs_err.kind(),
                        format!("文件路径失败（{file_err}），抽象命名空间也失败（{abs_err}）"),
                    )
                });
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
