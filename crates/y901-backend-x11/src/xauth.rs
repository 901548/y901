//! `~/.Xauthority` 解析（**只做读取与匹配**，不写文件）。
//!
//! 格式（没有任何文件头，一个接一个条目直接拼起来）：
//!
//! ```text
//! family        : CARD16  —— **大端序**（这是 Xauth 的历史遗留，与 X11 连接层小端序相反）
//! address       : CARD16 长度 + 字节
//! number        : CARD16 长度 + 字节（屏幕号，如 "0"）
//! name          : CARD16 长度 + 字节（如 "MIT-MAGIC-COOKIE-1"）
//! data          : CARD16 长度 + 字节
//! ```
//!
//! **很容易错的点**：
//! 1. 长度字段是大端序（X11 请求里全是小端序，两者不同）；
//! 2. 条目之间**没有**对齐填充（和协议包相反）。

/// @Xauthority 中的一个认证条目。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XauthEntry {
    /// 家族代码（`FamilyLocal` = 256，`FamilyWild` = 65535）。
    pub family: u16,
    /// 地址（主机名，没有时为空）。
    pub address: Vec<u8>,
    /// 屏幕号（ASCII 字符串，如 `b"0"`）。
    pub number: Vec<u8>,
    /// 认证协议名（如 `b"MIT-MAGIC-COOKIE-1"`）。
    pub name: Vec<u8>,
    /// 认证数据（cookie，通常 16 字节）。
    pub data: Vec<u8>,
}

/// 家族代码：本机（与主机名绑定）。
pub const FAMILY_LOCAL: u16 = 256;
/// 家族代码：万用（匹配任何主机）。
pub const FAMILY_WILD: u16 = 0xFFFF;

/// 从字节串解析所有条目。
///
/// 返回：解析出的条目（遇到截断的尾部就停止，不报错——
/// 真实的 Xauthority 文件确实会因为并发写入而出现半条记录）。
/// 副作用：无。
pub fn parse(bytes: &[u8]) -> Vec<XauthEntry> {
    let mut out = Vec::new();
    let mut i = 0usize;
    // 读一个 CARD16（大端序），越界返回 None
    let rd16 = |b: &[u8], off: usize| -> Option<u16> {
        b.get(off..off + 2).map(|s| u16::from_be_bytes([s[0], s[1]]))
    };
    // 读一个 "长度 + 字节"字段
    let rd_field = |b: &[u8], off: &mut usize| -> Option<Vec<u8>> {
        let n = rd16(b, *off)? as usize;
        *off += 2;
        let v = b.get(*off..*off + n)?.to_vec();
        *off += n;
        Some(v)
    };

    while i < bytes.len() {
        let Some(family) = rd16(bytes, i) else { break };
        i += 2;
        let (Some(address), Some(number), Some(name), Some(data)) = (
            rd_field(bytes, &mut i),
            rd_field(bytes, &mut i),
            rd_field(bytes, &mut i),
            rd_field(bytes, &mut i),
        ) else {
            break; // 尾部截断：忽略
        };
        out.push(XauthEntry {
            family,
            address,
            number,
            name,
            data,
        });
    }
    out
}

/// 从条目中选出适合指定屏幕号的 `MIT-MAGIC-COOKIE-1`。
///
/// 输入：`entries`、`display_number`（如 0）、`hostname`（用于匹配 FamilyLocal）。
/// 返回：`Some(cookie)` 或 `None`（无认证需求——正常情况）。
pub fn find_cookie(entries: &[XauthEntry], display_number: u32, hostname: &str) -> Option<Vec<u8>> {
    let want_number = display_number.to_string();
    let host = hostname.as_bytes();
    for e in entries {
        if e.name != b"MIT-MAGIC-COOKIE-1" {
            continue;
        }
        if e.number != want_number.as_bytes() {
            continue;
        }
        let addr_matches = e.family == FAMILY_WILD
            || (e.family == FAMILY_LOCAL && e.address.as_slice() == host)
            || e.address.as_slice() == host;
        if addr_matches {
            return Some(e.data.clone());
        }
    }
    None
}

/// 从 `$XAUTHORITY`（没有就 `~/.Xauthority`）读取并选出对应屏幕的 cookie。
///
/// 输入：`display_number`（如 99）、`hostname`。
/// 返回：`Some(cookie)` 或 `None`（文件不存在 / 没匹配条目）——
/// 这不是错误：Xvfb 默认不启用认证，空认证就能连。
/// 副作用：读文件（不写）。
pub fn load_cookie_from_home(display_number: u32, hostname: &str) -> Option<Vec<u8>> {
    let path = std::env::var_os("XAUTHORITY")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".Xauthority")))?;
    let bytes = std::fs::read(&path).ok()?;
    let entries = parse(&bytes);
    find_cookie(&entries, display_number, hostname)
}
