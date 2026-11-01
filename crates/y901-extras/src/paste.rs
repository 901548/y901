//! 粘贴处理：**控制字符过滤** + **bracketed paste**。
//!
//! ## 为什么这不是"可选优化"，而是安全红线
//!
//! 终端里的粘贴会直接写进 PTY，而 PTY 另一端是 **shell**。
//! 如果粘贴的文本里含 `\n`，没有 bracketed paste 时 shell 会把它当成"按了回车"——
//! 于是一段多行脚本会**逐行立即执行**（包含 `rm -rf` 的剪贴就是事故）。
//! bracketed paste（`ESC[200~` … `ESC[201~`）告诉 shell："这是一段粘贴，不要当输入执行"。
//!
//! ## 另一个坑：控制字符
//!
//! 从剪贴板来的文本可能含 `\x1b`（ESC）——直接写进 PTY 等于**伪造终端转义序列**，
//! 可以把你的屏幕刷成乱码甚至执行命令。所以必须过滤掉 `\x00..=\x1f`（保留 `\r`、`\t`）。

/// 清洗粘贴文本：统一换行为 `\r`，丢掉危险控制字符。
///
/// 输入：任意 UTF-8 文本。输出：可安全写进 PTY 的文本。
/// 副作用：无。
///
/// 为什么 `\n` → `\r`：真正的终端就是发 `\r`（CR），
/// 发 `\n` 在部分原始模式程序里不会触发"回车"。
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                // CRLF 归一：吃掉紧跟的 LF
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\r');
            }
            '\n' => out.push('\r'),
            '\t' => out.push('\t'),
            c if (c as u32) < 0x20 => {} // 其余控制字符一律丢弃
            '' => {} // DEL
            c => out.push(c),
        }
    }
    out
}

/// 粘贴开始标记（bracketed paste start）。
pub const PASTE_START: &[u8] = b"\x1b[200~";
/// 粘贴结束标记（bracketed paste end）。
pub const PASTE_END: &[u8] = b"\x1b[201~";

/// 把已清洗的文本打包成"要写进 PTY 的字节"。
///
/// 输入：`text`、`bracketed`（shell 是否开了 bracketed paste）。
/// 返回：带（或不带）`ESC[200~`…`ESC[201~` 包裹的字节。
pub fn wrap(text: &str, bracketed: bool) -> Vec<u8> {
    let body = sanitize(text);
    if !bracketed {
        return body.into_bytes();
    }
    let mut out = Vec::with_capacity(body.len() + PASTE_START.len() + PASTE_END.len());
    out.extend_from_slice(PASTE_START);
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(PASTE_END);
    out
}

/// 从 PTY 输出里识别 `ESC[?2004h` / `ESC[?2004l`，追踪 bracketed paste 是否开启。
///
/// 为什么用"扫描字节流"而不是改 VT 状态机：
/// 我们只需要**一个布尔值**，而改 CSI 解析器会动到正在稳定工作的核心路径。
/// 带一个 8 字节的尾部缓冲，处理"序列被拆在两个 chunk 里"的情况。
#[derive(Default)]
pub struct BracketedPasteDetector {
    /// 上一次剩下的尾部（可能是不完整的序列）
    tail: Vec<u8>,
    /// 当前是否处于 bracketed paste 模式
    enabled: bool,
}

impl BracketedPasteDetector {
    /// 喂入一段 PTY 输出，更新内部状态。
    pub fn feed(&mut self, chunk: &[u8]) {
        const ON: &[u8] = b"\x1b[?2004h";
        const OFF: &[u8] = b"\x1b[?2004l";
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(chunk);
        // 按序扫描：后出现的覆盖先出现的
        let mut i = 0usize;
        while i + ON.len() <= buf.len() {
            if &buf[i..i + ON.len()] == ON {
                self.enabled = true;
                i += ON.len();
            } else if &buf[i..i + OFF.len()] == OFF {
                self.enabled = false;
                i += OFF.len();
            } else {
                i += 1;
            }
        }
        // 保留尾部（最多 ON.len()-1 字节），下次拼上
        let keep = buf.len().saturating_sub(ON.len() - 1).max(i);
        self.tail = buf[keep.min(buf.len())..].to_vec();
    }

    /// 当前是否开启了 bracketed paste。
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_is_normalized_to_cr() {
        assert_eq!(sanitize("a\r\nb\nc\rd"), "a\rb\rc\rd");
    }

    #[test]
    fn control_characters_are_dropped_except_cr_and_tab() {
        // ESC 必须被丢掉：不然粘贴可以伪造终端转义序列
        assert_eq!(sanitize("a\x1b[31mred\x07b\x00c"), "a[31mredbc");
        assert_eq!(sanitize("x\ty"), "x\ty", "制表符要保留");
    }

    #[test]
    fn bracketed_wrap_uses_esc_200_and_201() {
        let b = wrap("ls\npwd", true);
        assert!(b.starts_with(b"\x1b[200~"));
        assert!(b.ends_with(b"\x1b[201~"));
        assert_eq!(&b[6..b.len() - 6], b"ls\rpwd", "内部已经归一为 CR");
    }

    #[test]
    fn non_bracketed_wrap_is_plain_sanitized_bytes() {
        assert_eq!(wrap("a\nb", false), b"a\rb");
    }

    #[test]
    fn detector_sees_enable_and_disable() {
        let mut d = BracketedPasteDetector::default();
        assert!(!d.enabled());
        d.feed(b"\x1b[?2004h");
        assert!(d.enabled(), "应识别开启");
        d.feed(b"output\x1b[?2004l");
        assert!(!d.enabled(), "应识别关闭");
    }

    #[test]
    fn detector_handles_sequence_split_across_chunks() {
        let mut d = BracketedPasteDetector::default();
        d.feed(b"hello\x1b[?20");
        assert!(!d.enabled(), "序列还没完整");
        d.feed(b"04h world");
        assert!(d.enabled(), "跨 chunk 也要能识别");
    }

    #[test]
    fn detector_last_occurrence_wins() {
        let mut d = BracketedPasteDetector::default();
        d.feed(b"\x1b[?2004h\x1b[?2004l");
        assert!(!d.enabled(), "后面的 OFF 应覆盖前面的 ON");
    }
}
