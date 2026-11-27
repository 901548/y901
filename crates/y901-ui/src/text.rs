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
//! 光标移动与删除**按字素簇**（`e` + 组合重音算一个整体，见 `crate::cluster`），
//! 不按 UTF-8 字节、也不按 `char`。
//!
//! 已知限制（刻意的）：不做 bidi，不折行（折行在控件层按可视宽度做）。

use crate::color::TextStyle;
use crate::widget::TextMetrics;
use std::collections::VecDeque;
use std::time::Instant;

/// 撤销历史里最多保留多少步。
///
/// 为什么是"条数 + 总字节"双上限：只按条数限，一份 4 MB 的文本按 200 步存
/// 就是 800 MB；只按字节限，短文本会攒下几千步。两个都要。
const HISTORY_MAX_STEPS: usize = 200;
/// 撤销历史最多占多少字节（所有快照的文本副本之和）。
const HISTORY_MAX_BYTES: usize = 4 * 1024 * 1024;
/// 连续打字合并为一步的时间窗。
///
/// 超过这个间隔再按键就另起一步：用户停顿一下再打字，通常希望 Ctrl+Z
/// 撤到停顿处，而不是把整段话一次撤掉。
const TYPING_COALESCE: std::time::Duration = std::time::Duration::from_millis(700);

/// 撤销栈里的一份快照：文本 + 光标 + 锚点。
#[derive(Clone, Debug)]
struct Snapshot {
    text: String,
    cursor: usize,
    anchor: usize,
}

/// 一次编辑的类别：决定能不能与上一步合并。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditKind {
    /// 连续输入（可合并）。
    Typing,
    /// 删除、粘贴、剪切等（不与任何东西合并）。
    Other,
}

/// 这个字符算不算"词"的一部分（Ctrl+←/→ 与双击选词的边界依据）。
///
/// 规则刻意简单、可预期：字母/数字/下划线成词，其余（空白、标点、emoji）成界。
/// 不做语言相关分词 —— 那需要词典，且猜错时用户更难纠正。
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

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
    /// 撤销栈（每项是"这次编辑**之前**"的状态）。
    undo: VecDeque<Snapshot>,
    /// 重做栈（撤销时压入，任何新编辑清空）。
    redo: Vec<Snapshot>,
    /// 撤销栈当前占用的文本字节数（用于 `HISTORY_MAX_BYTES` 上限）。
    undo_bytes: usize,
    /// 上一次编辑的时间与类别，用于把连续打字合并成一步。
    last_edit: Option<(Instant, EditKind)>,
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
            ..Self::default()
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
        // **吸附到字素簇边界**（不只是 char 边界）：否则光标会落在 `कि` 的辅音与
        // 元音符号之间、或 `e` 与它的组合重音之间，接着打字就会把字素拆成两个孤立字符。
        crate::cluster::snap(&self.text, byte)
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
        // **按集群簇退格**：`कि` 要一次删掉整个音节；阿拉伯语则一个字母一簇
        // （见 `crate::cluster` 的规则说明）。
        crate::cluster::prev_boundary(&self.text, pos).unwrap_or(0)
    }

    #[allow(dead_code)]
    fn prev_boundary_chars(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        let mut p = pos.min(self.text.len());
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
        // 与 `prev_boundary` 对称：同样按集群簇。
        crate::cluster::next_boundary(&self.text, pos).unwrap_or(self.text.len())
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

    // ---------- 撤销 / 重做 ----------

    /// 把"当前状态"存进撤销栈（**在改文本之前**调用）。
    ///
    /// 合并规则：`Typing` 与上一步同类且间隔小于 [`TYPING_COALESCE`] 时不重复入栈 ——
    /// 于是"连打 10 个字母"是**一步**，而"打字 → 停顿 → 打字"是两步。
    fn record(&mut self, kind: EditKind) {
        let now = Instant::now();
        if kind == EditKind::Typing {
            if let Some((last_at, EditKind::Typing)) = self.last_edit {
                if now.duration_since(last_at) < TYPING_COALESCE {
                    self.last_edit = Some((now, kind));
                    // 仍在同一段输入里：不新增快照（旧快照已覆盖这段之前的状态）。
                    return;
                }
            }
        }
        self.last_edit = Some((now, kind));
        self.undo_bytes += self.text.len();
        self.undo.push_back(Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        // 新编辑让"未来的撤销"失效。
        self.redo.clear();
        // 两道上限：条数与总字节。超了从**最旧**的开始丢。
        while self.undo.len() > HISTORY_MAX_STEPS
            || (self.undo_bytes > HISTORY_MAX_BYTES && self.undo.len() > 1)
        {
            if let Some(dropped) = self.undo.pop_front() {
                self.undo_bytes = self.undo_bytes.saturating_sub(dropped.text.len());
            } else {
                break;
            }
        }
    }

    /// 撤销栈的步数（诊断/测试用）。
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// 撤销栈占用的文本字节数（诊断/测试用；用于验证上限真的生效）。
    pub fn history_bytes(&self) -> usize {
        self.undo_bytes
    }

    /// 是否还有可撤销的编辑。
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// 是否还有可重做的编辑。
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// 撤销一步。返回 `false` 表示已经在最早的状态。
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop_back() else {
            return false;
        };
        self.undo_bytes = self.undo_bytes.saturating_sub(prev.text.len());
        self.redo.push(Snapshot {
            text: std::mem::take(&mut self.text),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        self.text = prev.text;
        self.cursor = prev.cursor.min(self.text.len());
        self.anchor = prev.anchor.min(self.text.len());
        // 撤销之后又是一段新输入的起点。
        self.last_edit = None;
        true
    }

    /// 重做一步。返回 `false` 表示没有可重做的编辑。
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo_bytes += self.text.len();
        self.undo.push_back(Snapshot {
            text: std::mem::take(&mut self.text),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        self.text = next.text;
        self.cursor = next.cursor.min(self.text.len());
        self.anchor = next.anchor.min(self.text.len());
        self.last_edit = None;
        true
    }

    /// 清空撤销/重做历史（例如 `set_text` 载入全新文档后）。
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.last_edit = None;
    }

    // ---------- 词移动 / 选词（Ctrl+←/→、双击）----------

    /// 词首（Ctrl+←）：先跳过紧邻的非单词字符，再跳过单词字符。
    ///
    /// 结果会吸附到字素簇边界，绝不把 `e` 与它的重音拆开。
    pub fn move_word_left(&mut self, extend: bool) -> bool {
        let target = self.word_left_target(self.cursor);
        if target == self.cursor {
            return false;
        }
        self.cursor = target;
        if !extend {
            self.anchor = target;
        }
        true
    }

    /// 词尾（Ctrl+→）：先跳过单词字符，再跳过紧邻的非单词字符（与上文对称）。
    pub fn move_word_right(&mut self, extend: bool) -> bool {
        let target = self.word_right_target(self.cursor);
        if target == self.cursor {
            return false;
        }
        self.cursor = target;
        if !extend {
            self.anchor = target;
        }
        true
    }

    fn word_left_target(&self, pos: usize) -> usize {
        let mut idx = pos;
        // 1) 跳过非单词字符
        while idx > 0 {
            let (i, ch) = self.text[..idx].char_indices().next_back().unwrap();
            if is_word_char(ch) {
                break;
            }
            idx = i;
        }
        // 2) 跳过单词字符
        while idx > 0 {
            let (i, ch) = self.text[..idx].char_indices().next_back().unwrap();
            if !is_word_char(ch) {
                break;
            }
            idx = i;
        }
        crate::cluster::snap(&self.text, idx)
    }

    fn word_right_target(&self, pos: usize) -> usize {
        let mut idx = pos;
        // 1) 跳过单词字符
        while idx < self.text.len() {
            let ch = self.text[idx..].chars().next().unwrap();
            if !is_word_char(ch) {
                break;
            }
            idx += ch.len_utf8();
        }
        // 2) 跳过非单词字符
        while idx < self.text.len() {
            let ch = self.text[idx..].chars().next().unwrap();
            if is_word_char(ch) {
                break;
            }
            idx += ch.len_utf8();
        }
        self.cluster_ceil(idx)
    }

    /// 把位置**向上**对齐到字素簇边界（不越过簇尾）。
    fn cluster_ceil(&self, pos: usize) -> usize {
        let pos = pos.min(self.text.len());
        if pos == 0 || pos >= self.text.len() || crate::cluster::snap(&self.text, pos) == pos {
            return pos;
        }
        crate::cluster::next_boundary(&self.text, pos).unwrap_or(self.text.len())
    }

    /// `byte` 所在"词"的区间（双击选中用）。
    ///
    /// 规则：落在单词字符上 → 选中该词；落在空白/标点上 → **优先选右边那个词**
    /// （与主流编辑器一致：双击行尾空格选最后一个词，否则什么也选不中）；
    /// 右边没有词就选左边那个。返回值一定落在字素簇边界上。
    pub fn word_range_at(&self, byte: usize) -> Option<(usize, usize)> {
        if self.text.is_empty() {
            return None;
        }
        let pos = crate::cluster::snap(&self.text, byte.min(self.text.len()));
        let at_word = self.text[pos..].chars().next().is_some_and(is_word_char);
        if at_word {
            return Some(self.word_bounds_around(pos, true));
        }
        // 非单词字符：找右边的词
        let mut p = pos;
        while p < self.text.len() {
            let ch = self.text[p..].chars().next().unwrap();
            if is_word_char(ch) {
                return Some(self.word_bounds_around(p, true));
            }
            p += ch.len_utf8();
        }
        // 右边没有词：选左边的词
        let mut p = pos;
        while p > 0 {
            let (i, ch) = self.text[..p].char_indices().next_back().unwrap();
            if is_word_char(ch) {
                return Some(self.word_bounds_around(i, false));
            }
            p = i;
        }
        // 整段没有任何单词字符（如全是标点）：选中这串标点
        let start = {
            let mut s = pos;
            while s > 0 {
                let (i, ch) = self.text[..s].char_indices().next_back().unwrap();
                if is_word_char(ch) {
                    break;
                }
                s = i;
            }
            crate::cluster::snap(&self.text, s)
        };
        let end = {
            let mut e = pos;
            while e < self.text.len() {
                let ch = self.text[e..].chars().next().unwrap();
                if is_word_char(ch) {
                    break;
                }
                e += ch.len_utf8();
            }
            self.cluster_ceil(e)
        };
        Some((start, end))
    }

    /// 从 `pos`（必须是指向单词字符的位置）向两侧扩展成一整段词。
    fn word_bounds_around(&self, pos: usize, _right_biased: bool) -> (usize, usize) {
        let mut start = pos;
        while start > 0 {
            let (i, ch) = self.text[..start].char_indices().next_back().unwrap();
            if !is_word_char(ch) {
                break;
            }
            start = i;
        }
        let mut end = pos;
        while end < self.text.len() {
            let ch = self.text[end..].chars().next().unwrap();
            if !is_word_char(ch) {
                break;
            }
            end += ch.len_utf8();
        }
        (
            crate::cluster::snap(&self.text, start),
            self.cluster_ceil(end),
        )
    }

    /// 选中 `byte` 所在的词（双击）。没有词时返回 `false`。
    pub fn select_word_at(&mut self, byte: usize) -> bool {
        let Some((start, end)) = self.word_range_at(byte) else {
            return false;
        };
        if start == end {
            return false;
        }
        self.set_selection(start, end);
        true
    }

    // ---------- 编辑 ----------

    /// 删除选区；没有选区返回 `false`。
    pub fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        self.record(EditKind::Other);
        self.delete_selection_raw();
        true
    }

    /// 删除选区（**不记撤销历史**）：供 `insert_str` 等"整体是一次编辑"的路径复用。
    fn delete_selection_raw(&mut self) -> bool {
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
        self.record(EditKind::Typing);
        self.delete_selection_raw();
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
        self.record(EditKind::Other);
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
        self.record(EditKind::Other);
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
    fn cursor_snaps_out_of_a_cluster_middle() {
        // "कि" = 辅音 + 元音符号：中间没有合法光标位。
        let mut b = TextBuffer::from_text("कि");
        for i in 0..=b.len_bytes() {
            b.set_cursor(i);
            let cur = b.cursor();
            assert!(
                cur == 0 || cur == b.len_bytes(),
                "光标落在集群中间：set_cursor({i}) -> {cur}"
            );
        }
    }

    #[test]
    fn backspace_deletes_a_whole_cluster_not_half_a_syllable() {
        // "कि" 后按一次 Backspace 应整簇删除（留下空串），而不是只删掉元音符号。
        let mut b = TextBuffer::from_text("कि");
        b.set_cursor(b.len_bytes());
        assert!(b.backspace());
        assert_eq!(b.text(), "", "整簇被删掉，实得 {:?}", b.text());
    }

    #[test]
    fn combining_accent_is_deleted_with_its_base_letter() {
        let text = "e\u{0301}";
        let mut b = TextBuffer::from_text(text);
        b.set_cursor(b.len_bytes());
        assert!(b.backspace());
        assert_eq!(b.text(), "", "重音应与基字母一起删除");
    }

    #[test]
    fn arabic_letters_delete_one_at_a_time() {
        // 与编辑器惯例一致：阿拉伯语每个字母是一簇。
        let text = "مرحبا";
        let mut b = TextBuffer::from_text(text);
        b.set_cursor(b.len_bytes());
        assert!(b.backspace());
        assert_eq!(
            b.text().chars().count(),
            text.chars().count() - 1,
            "只应删掉最后一个字母"
        );
    }
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

    // ---------- 撤销 / 重做 ----------

    #[test]
    fn undo_is_a_stack_and_redo_mirrors_it() {
        // 只用公开编辑接口，避免测试依赖 `record` 的内部调用时点：
        // 输入("a") → 删除 → 输入("b")，共三步。
        let mut buf = TextBuffer::new();
        buf.insert_str("a");
        assert!(buf.backspace());
        buf.insert_str("b");
        assert_eq!(buf.text(), "b");

        assert!(buf.undo(), "应有可撤销步骤");
        assert_eq!(buf.text(), "", "撤销插入 b → 空");
        assert!(buf.undo());
        assert_eq!(buf.text(), "a", "再撤销删除 → 回到 a");
        assert!(buf.undo());
        assert_eq!(buf.text(), "", "再撤销第一次插入 → 空");
        assert!(!buf.undo(), "到底之后必须返回 false（不是 panic）");

        assert!(buf.redo());
        assert_eq!(buf.text(), "a", "重做第一次插入");
        assert!(buf.redo());
        assert_eq!(buf.text(), "", "重做删除");
        assert!(buf.redo());
        assert_eq!(buf.text(), "b", "重做第二次插入");
        assert!(!buf.redo());
    }

    #[test]
    fn consecutive_typing_coalesces_but_delete_breaks_the_run() {
        let mut buf = TextBuffer::new();
        buf.insert_str("x");
        buf.insert_str("y");
        buf.insert_str("z");
        assert_eq!(buf.text(), "xyz");
        assert_eq!(buf.undo_depth(), 1, "连续输入应合并为一步");
        buf.undo();
        assert_eq!(buf.text(), "");

        let mut buf = TextBuffer::from_text("");
        buf.insert_str("a");
        buf.backspace();
        assert_eq!(buf.text(), "");
        assert_eq!(buf.undo_depth(), 2, "输入 + 删除是两步（删除不可合并）");
    }

    #[test]
    fn history_respects_the_step_limit() {
        let mut buf = TextBuffer::new();
        // 每步之间用删除打断合并，制造大量独立步骤。
        for _ in 0..(HISTORY_MAX_STEPS + 50) {
            buf.insert_str("a");
            buf.record(EditKind::Other);
        }
        assert!(
            buf.undo_depth() <= HISTORY_MAX_STEPS,
            "撤销步数必须有上限，实得 {}",
            buf.undo_depth()
        );
        // 一直撤到底不应 panic，且文本长度单调不增。
        let mut last = buf.text().len();
        while buf.undo() {
            let now = buf.text().len();
            assert!(now <= last, "撤销后文本变长了：{last} → {now}");
            last = now;
        }
    }

    #[test]
    fn history_respects_the_byte_budget() {
        let mut buf = TextBuffer::new();
        // 往同一个缓冲里灌 1 MiB，每次插入都记一步（用 Other 打断合并）。
        let chunk = "x".repeat(64 * 1024);
        for _ in 0..16 {
            buf.insert_str(&chunk);
            buf.record(EditKind::Other);
        }
        assert!(
            buf.history_bytes() <= HISTORY_MAX_BYTES,
            "快照总字节必须受 HISTORY_MAX_BYTES 约束，实得 {}",
            buf.history_bytes()
        );
    }

    // ---------- 按词移动 / 选词 ----------

    #[test]
    fn word_moves_land_on_word_edges() {
        let mut buf = TextBuffer::from_text("hello brave world");
        assert_eq!(buf.cursor(), 17);
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 12, "world 的词首");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 6, "brave 的词首");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 0, "hello 的词首");
        assert!(!buf.move_word_left(false), "已在开头，不再移动");
        // 约定（与 Ctrl+← 对称）：Ctrl+→ 永远落在**下一个词首**；
        // 最后一个词的下一站就是文本末尾。
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 6, "hello 之后的下一个词首是 brave");
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 12, "再下一站是 world 的词首");
        buf.move_word_right(false);
        assert_eq!(buf.cursor(), 17, "最后一个词的下一站是末尾");
        assert!(!buf.move_word_right(false), "已在末尾，不再移动");
    }

    #[test]
    fn word_moves_skip_punctuation_and_whitespace_runs() {
        // 连续标点/空白是一个"非词"区段，应一次跨过。
        let mut buf = TextBuffer::from_text("foo --- bar");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 8, "bar 的词首");
        buf.move_word_left(false);
        assert_eq!(buf.cursor(), 0, "一次跨过 ' --- ' 到 foo 的词首");
    }

    #[test]
    fn word_range_at_selects_the_word_or_the_neighbouring_one() {
        let text = "alpha beta gamma";
        let buf = TextBuffer::from_text(text);
        assert_eq!(buf.word_range_at(7), Some((6, 10)), "点在词内 → 该词");
        // 点在空格上：优先选右边的词（主流编辑器行为）
        assert_eq!(buf.word_range_at(5), Some((6, 10)));
        // 末尾空白右边没有词 → 选左边的词
        let buf = TextBuffer::from_text("alpha ");
        assert_eq!(buf.word_range_at(5), Some((0, 5)));
        // 全是标点：选这串标点本身
        let buf = TextBuffer::from_text("***");
        assert_eq!(buf.word_range_at(1), Some((0, 3)));
        // 空文本没有词
        assert_eq!(TextBuffer::new().word_range_at(0), None);
    }

    #[test]
    fn word_range_never_splits_a_grapheme_cluster() {
        // "e" + 组合重音 是一簇（3 字节），选词不能把它切开。
        let text = "cafe\u{301} bar";
        let buf = TextBuffer::from_text(text);
        let (start, end) = buf.word_range_at(6).expect("空格右侧有词");
        assert_eq!(&text[start..end], "bar");
        let (s2, e2) = buf.word_range_at(0).expect("左侧有词");
        assert_eq!(
            &text[s2..e2],
            "cafe\u{301}",
            "整个 cafe\u{301} 必须一次选中（e 与重音不拆开）"
        );
    }

    #[test]
    fn select_word_at_reports_whether_it_selected_anything() {
        let mut buf = TextBuffer::from_text("one two");
        assert!(buf.select_word_at(6));
        assert_eq!(buf.selection(), Some((4, 7)));
        let mut empty = TextBuffer::new();
        assert!(!empty.select_word_at(0), "空文本选不出词");
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
