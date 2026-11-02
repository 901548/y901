//! 纯逻辑文本缓冲区：UTF-8 存储 + 光标 + 选区 + 坐标换算。
//!
//! 这一层**不涉及任何渲染**：它只知道自己有哪些字符、光标在哪个字节、
//! 以及"第 N 个字节对应多宽的 X"。字号、字形、DPI 全部由 `TextMetrics` 提供。
//!
//! 三条不变量：
//!
//! 1. `cursor` 与 `anchor` **永远落在 char 边界**上（否则 `String::insert` 会 panic）；
//! 2. `anchor == cursor` 表示"没有选区"；
//! 3. 所有编辑操作都先用选区替换，再落笔（与用户预期一致）。
//!
//! 已知限制（刻意的）：不做 grapheme cluster 合并（emoji 组合、变音符号会被
//! 当成多个字符逐个删除），不做 bidi，不折行。这些等 `TextInputWidget` 需要时再补。

use crate::color::TextStyle;
use crate::widget::TextMetrics;

/// 文本长度上限（字节）：16 MiB。
///
/// 为什么需要：粘贴、IME 提交、RPC 注入都汇到 `insert_str`——
/// 没有上限时一次 10 MB 粘贴会让测量、布局、字形图集全部超线性变慢。
pub const MAX_TEXT_LEN: usize = 16 * 1024 * 1024;

/// 可编辑文本。
#[derive(Clone, Debug, Default)]
pub struct TextBuffer {
    text: String,
    /// 光标：字节偏移，始终在 char 边界。
    cursor: usize,
    /// 选区锚点：字节偏移。`anchor == cursor` 表示无选区。
    anchor: usize,
}

impl TextBuffer {
    /// 建立空文本编辑缓冲。
    pub fn new() -> Self {
        Self::default()
    }

    /// 用初始文本构造，光标落在末尾。
    pub fn from_text(text: impl Into<String>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            cursor: end,
            anchor: end,
        }
    }

    // ---------- 只读 ----------

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 字节长度（UTF-8）；分配缓冲时用它。
    pub fn len_bytes(&self) -> usize {
        self.text.len()
    }

    /// 字符数（不是字节数）；光标移动与选区都按字符计。
    pub fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// 光标字节偏移。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 选区锚点字节偏移。
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// 规范化选区（start <= end）；无选区返回 `None`。
    pub fn selection(&self) -> Option<(usize, usize)> {
        if self.anchor == self.cursor {
            None
        } else if self.anchor < self.cursor {
            Some((self.anchor, self.cursor))
        } else {
            Some((self.cursor, self.anchor))
        }
    }

    /// 是否存在选区；Ctrl+C 据此决定\「复制还是把信号透给 shell\「。
    pub fn has_selection(&self) -> bool {
        self.anchor != self.cursor
    }

    /// 选中文本；无选区返回 `None`。
    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|(a, b)| &self.text[a..b])
    }

    // ---------- 光标与选区 ----------

    /// 把任意字节偏移吸附到最近的合法 char 边界（向下取整）。
    fn snap(&self, byte: usize) -> usize {
        let mut pos = byte.min(self.text.len());
        while pos > 0 && !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        pos
    }

    /// 设置光标；越界或落在字符中间会自动吸附。返回是否真的移动了。
    pub fn set_cursor(&mut self, byte: usize) -> bool {
        let pos = self.snap(byte);
        let moved = pos != self.cursor;
        self.cursor = pos;
        self.anchor = pos;
        moved
    }

    /// 设置选区（锚点 + 光标），两者都会被吸附到 char 边界。
    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.anchor = self.snap(anchor);
        self.cursor = self.snap(cursor);
    }

    /// 全选。
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.cursor = self.text.len();
    }

    /// 清除选区（保留文本）。
    pub fn clear_selection(&mut self) {
        self.anchor = self.cursor;
    }

    /// 上一个 char 的起始偏移。
    fn prev_boundary(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let mut p = pos.min(self.text.len());
        // 跳过当前位置（如果它本身不是边界）
        while p > 0 && !self.text.is_char_boundary(p) {
            p -= 1;
        }
        if p == 0 {
            return 0;
        }
        // 再往前一个字符
        match self.text[..p].chars().next_back() {
            Some(ch) => p - ch.len_utf8(),
            None => 0,
        }
    }

    /// 下一个 char 的结束偏移。
    fn next_boundary(&self, pos: usize) -> usize {
        let p = self.snap(pos);
        match self.text[p..].chars().next() {
            Some(ch) => p + ch.len_utf8(),
            None => self.text.len(),
        }
    }

    /// 左移一个字符。`extend = true` 时扩展选区（Shift+←）。
    pub fn move_left(&mut self, extend: bool) -> bool {
        let target = self.prev_boundary(self.cursor);
        if target == self.cursor {
            return false;
        }
        self.cursor = target;
        if !extend {
            self.anchor = target;
        }
        true
    }

    /// 右移一个字符。
    pub fn move_right(&mut self, extend: bool) -> bool {
        let target = self.next_boundary(self.cursor);
        if target == self.cursor {
            return false;
        }
        self.cursor = target;
        if !extend {
            self.anchor = target;
        }
        true
    }

    /// 行首（Home）。
    pub fn move_line_start(&mut self, extend: bool) -> bool {
        let target = self.line_start(self.cursor);
        if target == self.cursor {
            return false;
        }
        self.cursor = target;
        if !extend {
            self.anchor = target;
        }
        true
    }

    /// 行尾（End）。
    pub fn move_line_end(&mut self, extend: bool) -> bool {
        let target = self.line_end(self.cursor);
        if target == self.cursor {
            return false;
        }
        self.cursor = target;
        if !extend {
            self.anchor = target;
        }
        true
    }

    // ---------- 编辑 ----------

    /// 删除选区；没有选区返回 `false`。
    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        self.text.replace_range(start..end, "");
        self.cursor = start;
        self.anchor = start;
        true
    }

    /// 在光标处插入字符串（先删选区）。
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.delete_selection();
        // 超限时截断到剩余容量（按字符边界切），**不 panic、不静默丢整段**。
        let room = MAX_TEXT_LEN.saturating_sub(self.text.len());
        let mut take = s.len().min(room);
        while take > 0 && !s.is_char_boundary(take) {
            take -= 1;
        }
        if take == 0 {
            return;
        }
        self.text.insert_str(self.cursor, &s[..take]);
        self.cursor += take;
        self.anchor = self.cursor;
    }

    /// 插入单个字符。
    pub fn insert_char(&mut self, ch: char) {
        let mut buf = [0u8; 4];
        self.insert_str(ch.encode_utf8(&mut buf));
    }

    /// 退格：删光标前一个字符（或整个选区）。
    pub fn backspace(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection();
        }
        if self.cursor == 0 {
            return false;
        }
        let start = self.prev_boundary(self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
        self.anchor = start;
        true
    }

    /// 向前删除：删光标后一个字符（或整个选区）。
    pub fn delete_forward(&mut self) -> bool {
        if self.has_selection() {
            return self.delete_selection();
        }
        if self.cursor >= self.text.len() {
            return false;
        }
        let end = self.next_boundary(self.cursor);
        self.text.replace_range(self.cursor..end, "");
        // 光标不动
        self.anchor = self.cursor;
        true
    }

    // ---------- 行 ----------

    /// 包含 `offset` 的行的起始字节。
    pub fn line_start(&self, offset: usize) -> usize {
        let offset = self.snap(offset);
        match self.text[..offset].rfind('\n') {
            Some(idx) => idx + 1,
            None => 0,
        }
    }

    /// 包含 `offset` 的行的结束字节（不含换行符）。
    pub fn line_end(&self, offset: usize) -> usize {
        let offset = self.snap(offset);
        match self.text[offset..].find('\n') {
            Some(idx) => offset + idx,
            None => self.text.len(),
        }
    }

    /// 光标所在行的 (start, end)。
    pub fn current_line(&self) -> (usize, usize) {
        (self.line_start(self.cursor), self.line_end(self.cursor))
    }

    /// 光标所在行的文本（不含换行符）。
    pub fn current_line_text(&self) -> &str {
        let (start, end) = self.current_line();
        &self.text[start..end]
    }

    // ---------- 度量 ----------

    /// 单个字符的物理宽度。
    ///
    /// 用 `encode_utf8` 到栈上缓冲，避免每个字符一次堆分配。
    pub fn char_width(ch: char, metrics: &dyn TextMetrics, style: &TextStyle) -> f32 {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        metrics.measure_text(s, style).w
    }

    /// 从行首到 `offset` 的物理宽度（行内 X 坐标）。
    ///
    /// `offset` 会被吸附到 char 边界并限制在当前行内。
    pub fn x_for_offset(&self, offset: usize, metrics: &dyn TextMetrics, style: &TextStyle) -> f32 {
        let start = self.line_start(offset);
        let end = self.line_end(offset);
        let offset = self.snap(offset).clamp(start, end);

        let mut x = 0.0;
        for ch in self.text[start..offset].chars() {
            x += Self::char_width(ch, metrics, style);
        }
        x
    }

    /// 光标当前所在的 X 坐标（相对行首）。
    pub fn cursor_x(&self, metrics: &dyn TextMetrics, style: &TextStyle) -> f32 {
        self.x_for_offset(self.cursor, metrics, style)
    }

    /// 根据点击的 X 坐标反推最近的插入点（字节偏移）。
    ///
    /// 以"字符边界"为候选，取物理距离最近的那个：
    /// 点在字符左半 → 落到它前面；点过中点 → 落到它后面。
    pub fn offset_for_x(&self, x: f32, metrics: &dyn TextMetrics, style: &TextStyle) -> usize {
        let (start, end) = self.current_line();

        let mut best = start;
        let mut best_dist = (x - 0.0).abs();
        let mut pen = 0.0f32;

        for (idx, ch) in self.text[start..end].char_indices() {
            pen += Self::char_width(ch, metrics, style);
            let boundary = start + idx + ch.len_utf8();
            let dist = (x - pen).abs();
            if dist < best_dist {
                best = boundary;
                best_dist = dist;
            }
        }
        best
    }

    /// 同上，但结果会同时把光标移过去（鼠标点击的完整动作）。
    pub fn place_cursor_at_x(
        &mut self,
        x: f32,
        metrics: &dyn TextMetrics,
        style: &TextStyle,
    ) -> usize {
        let offset = self.offset_for_x(x, metrics, style);
        self.set_cursor(offset);
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Size;

    /// 等宽度量：ASCII 8px，CJK 16px（两格）。
    struct Mono;

    impl TextMetrics for Mono {
        fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
            let mut w = 0.0;
            for ch in text.chars() {
                w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
            }
            Size::new(w, 16.0)
        }
        fn line_height(&self, _style: &TextStyle) -> f32 {
            16.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    fn style() -> TextStyle {
        TextStyle::default()
    }

    // ---------- 插入 ----------

    #[test]
    fn insert_mixed_cjk_and_ascii() {
        let mut buf = TextBuffer::new();
        buf.insert_str("Hello");
        buf.insert_char('中');
        buf.insert_str(" Y901");

        assert_eq!(buf.text(), "Hello中 Y901");
        // 5 + 3(中) + 5 = 13 字节
        assert_eq!(buf.len_bytes(), 13);
        assert_eq!(buf.cursor(), 13);
        assert_eq!(buf.len_chars(), 11);
    }

    #[test]
    fn insert_replaces_selection() {
        let mut buf = TextBuffer::from_text("abcdef");
        buf.set_selection(1, 4); // "bcd"
        assert_eq!(buf.selected_text(), Some("bcd"));
        buf.insert_str("X");
        assert_eq!(buf.text(), "aXef");
        assert_eq!(buf.cursor(), 2);
        assert!(!buf.has_selection());
    }

    // ---------- 退格 ----------

    #[test]
    fn backspace_across_cjk_never_splits_utf8() {
        let mut buf = TextBuffer::from_text("a中b");
        // 光标在末尾：a=1, 中=3, b=1 → 5
        assert_eq!(buf.cursor(), 5);

        assert!(buf.backspace()); // 删 b
        assert_eq!(buf.text(), "a中");
        assert_eq!(buf.cursor(), 4);

        assert!(buf.backspace()); // 删整个"中"（3 字节）
        assert_eq!(buf.text(), "a");
        assert_eq!(buf.cursor(), 1);

        assert!(buf.backspace()); // 删 a
        assert_eq!(buf.text(), "");
        assert_eq!(buf.cursor(), 0);

        assert!(!buf.backspace(), "空缓冲区退格应返回 false");
        assert!(buf.text().is_char_boundary(0));
    }

    #[test]
    fn backspace_with_selection_deletes_selection() {
        let mut buf = TextBuffer::from_text("你好世界");
        buf.set_selection(3, 9); // "好世"
        assert!(buf.backspace());
        assert_eq!(buf.text(), "你界");
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn backspace_on_emoji_removes_whole_codepoint() {
        let mut buf = TextBuffer::from_text("a😀");
        assert_eq!(buf.len_bytes(), 5); // 1 + 4
        buf.backspace();
        assert_eq!(buf.text(), "a");
    }

    #[test]
    fn delete_forward_removes_next_char() {
        let mut buf = TextBuffer::from_text("中a");
        buf.set_cursor(0);
        assert!(buf.delete_forward());
        assert_eq!(buf.text(), "a");
        assert_eq!(buf.cursor(), 0);
    }

    // ---------- 光标吸附 ----------

    #[test]
    fn cursor_snaps_to_char_boundary() {
        let mut buf = TextBuffer::from_text("中");
        // 1、2 都落在"中"的字节中间
        buf.set_cursor(1);
        assert_eq!(buf.cursor(), 0);
        buf.set_cursor(2);
        assert_eq!(buf.cursor(), 0);
        buf.set_cursor(3);
        assert_eq!(buf.cursor(), 3);
        // 越界
        buf.set_cursor(999);
        assert_eq!(buf.cursor(), 3);
    }

    #[test]
    fn move_left_right_by_char() {
        let mut buf = TextBuffer::from_text("a中b");
        buf.set_cursor(0);
        assert!(buf.move_right(false));
        assert_eq!(buf.cursor(), 1); // 过了 'a'
        assert!(buf.move_right(false));
        assert_eq!(buf.cursor(), 4); // 过了 '中'
        assert!(buf.move_left(false));
        assert_eq!(buf.cursor(), 1); // 回到 '中' 前
        assert!(!buf.move_left(false) || buf.cursor() == 0);
    }

    #[test]
    fn shift_move_extends_selection() {
        let mut buf = TextBuffer::from_text("abc");
        buf.set_cursor(0);
        buf.move_right(true);
        buf.move_right(true);
        assert_eq!(buf.selection(), Some((0, 2)));
        assert_eq!(buf.selected_text(), Some("ab"));
        // 反向移动会缩短选区
        buf.move_left(true);
        assert_eq!(buf.selection(), Some((0, 1)));
    }

    // ---------- 坐标换算 ----------

    #[test]
    fn x_for_offset_counts_cjk_as_two_cells() {
        let buf = TextBuffer::from_text("a中b");
        let s = style();
        // 边界：0 → 0px, 1 → 8px, 4 → 24px, 5 → 32px
        assert_eq!(buf.x_for_offset(0, &Mono, &s), 0.0);
        assert_eq!(buf.x_for_offset(1, &Mono, &s), 8.0);
        assert_eq!(buf.x_for_offset(4, &Mono, &s), 24.0);
        assert_eq!(buf.x_for_offset(5, &Mono, &s), 32.0);
    }

    #[test]
    fn offset_for_x_picks_nearest_boundary() {
        let mut buf = TextBuffer::from_text("a中b");
        buf.set_cursor(0);
        let s = style();
        // 边界 0/8/24/32
        assert_eq!(buf.offset_for_x(-5.0, &Mono, &s), 0);
        assert_eq!(buf.offset_for_x(3.0, &Mono, &s), 0); // 更靠近 0
        assert_eq!(buf.offset_for_x(9.0, &Mono, &s), 1); // 更靠近 8
        assert_eq!(buf.offset_for_x(15.0, &Mono, &s), 1); // 中点 16 之前
        assert_eq!(buf.offset_for_x(17.0, &Mono, &s), 4); // 中点之后 → '中' 后面
        assert_eq!(buf.offset_for_x(30.0, &Mono, &s), 5); // 更靠近 32
        assert_eq!(buf.offset_for_x(999.0, &Mono, &s), 5); // 超出 → 行尾
    }

    #[test]
    fn offset_roundtrip_through_x() {
        let text = "Hello 你好 Y901";
        let buf = TextBuffer::from_text(text);
        let s = style();
        for (byte, _ch) in text.char_indices() {
            let x = buf.x_for_offset(byte, &Mono, &s);
            let back = buf.offset_for_x(x, &Mono, &s);
            assert_eq!(back, byte, "byte {byte} 经 X 往返后变成 {back}");
        }
        // 末尾
        let end = text.len();
        let x = buf.x_for_offset(end, &Mono, &s);
        assert_eq!(buf.offset_for_x(x, &Mono, &s), end);
    }

    #[test]
    fn click_places_cursor() {
        let mut buf = TextBuffer::from_text("ab中");
        // 行宽 = 8 + 8 + 16 = 32
        buf.place_cursor_at_x(20.0, &Mono, &style());
        assert_eq!(buf.cursor(), 2); // 落在 "ab" 之后、"中" 之前
        assert!(!buf.has_selection());
    }

    // ---------- 行 ----------

    #[test]
    fn line_bounds_and_measure_are_per_line() {
        let mut buf = TextBuffer::from_text("abc\ndefg");
        assert_eq!(buf.line_start(0), 0);
        assert_eq!(buf.line_end(0), 3);

        // 光标放到第二行中间
        buf.set_cursor(6); // "de|fg"
        assert_eq!(buf.current_line(), (4, 8));
        assert_eq!(buf.current_line_text(), "defg");

        // 行内 X 从行首算起
        let s = style();
        assert_eq!(buf.x_for_offset(4, &Mono, &s), 0.0);
        assert_eq!(buf.x_for_offset(6, &Mono, &s), 16.0);

        // 反向：点 x=17 应落到第二行第 2 个字符之后
        assert_eq!(buf.offset_for_x(17.0, &Mono, &s), 6);
    }

    #[test]
    fn empty_buffer_measurements_are_zero() {
        let buf = TextBuffer::new();
        let s = style();
        assert_eq!(buf.x_for_offset(0, &Mono, &s), 0.0);
        assert_eq!(buf.offset_for_x(100.0, &Mono, &s), 0);
        assert_eq!(buf.current_line(), (0, 0));
    }
}
