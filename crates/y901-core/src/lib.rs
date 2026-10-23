//! Y901 纯逻辑终端内核。
//!
//! 内容零丢失的 reflow：行尾 padding 不计入逻辑流（用 `len` 区分），
//! 宽字符只会整字下推，光标用 `CursorAnchor` 的逻辑坐标跨重排存活。

use unicode_width::UnicodeWidthChar;

pub const STYLE_DEFAULT: u16 = 0;

/// 一个颜色槽的取值：默认 / 索引色 / 24 位真彩色。
///
/// Cell 只有 8 字节，塞不下 RGB；所以颜色与标志位统一存在 `StyleTable` 里，
/// Cell 只保留一个 u16 索引。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ColorSpec {
    #[default]
    Default,
    /// 0..=255：0 保留给默认，1..=16 是 ANSI，17..=255 是 256 色扩展。
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl ColorSpec {
    pub const fn is_default(self) -> bool {
        matches!(self, ColorSpec::Default)
    }
}

/// 一个格子的完整视觉属性。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct CellStyle {
    pub fg: ColorSpec,
    pub bg: ColorSpec,
    pub bold: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl CellStyle {
    pub const fn fg(mut self, color: ColorSpec) -> Self {
        self.fg = color;
        self
    }

    pub const fn bg(mut self, color: ColorSpec) -> Self {
        self.bg = color;
        self
    }

    /// 反显：交换前景/背景。
    pub const fn flipped(self) -> Self {
        Self {
            fg: self.bg,
            bg: self.fg,
            ..self
        }
    }

    pub fn is_plain(&self) -> bool {
        *self == CellStyle::default()
    }
}

/// 样式表：把 `CellStyle` 驻留成 u16 索引。
///
/// - 索引 0 永远是默认样式（`STYLE_DEFAULT`）。
/// - 表满（65535 项）后新样式退化为 0，颜色丢失但不 panic。
pub struct StyleTable {
    entries: Vec<CellStyle>,
    map: std::collections::HashMap<CellStyle, u16>,
}

impl Default for StyleTable {
    fn default() -> Self {
        Self {
            entries: vec![CellStyle::default()],
            map: std::collections::HashMap::new(),
        }
    }
}

impl StyleTable {
    pub fn intern(&mut self, style: CellStyle) -> u16 {
        if style.is_plain() {
            return STYLE_DEFAULT;
        }
        if let Some(&id) = self.map.get(&style) {
            return id;
        }
        if self.entries.len() >= u16::MAX as usize {
            return STYLE_DEFAULT;
        }
        let id = self.entries.len() as u16;
        self.entries.push(style);
        self.map.insert(style, id);
        id
    }

    pub fn get(&self, id: u16) -> CellStyle {
        self.entries.get(id as usize).copied().unwrap_or_default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.len() <= 1
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum CellKind {
    Narrow = 0,
    WideHead = 1,
    WideTail = 2,
}

/// 严格 8 字节：4 (char) + 2 (style) + 1 (kind) + 1 (pad)。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: u16,
    pub kind: CellKind,
    _pad: u8,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 8);

impl Cell {
    pub const fn blank(style: u16) -> Self {
        Self {
            ch: ' ',
            style,
            kind: CellKind::Narrow,
            _pad: 0,
        }
    }
    pub const fn narrow(ch: char, style: u16) -> Self {
        Self {
            ch,
            style,
            kind: CellKind::Narrow,
            _pad: 0,
        }
    }
    pub const fn wide_head(ch: char, style: u16) -> Self {
        Self {
            ch,
            style,
            kind: CellKind::WideHead,
            _pad: 0,
        }
    }
    pub const fn wide_tail(style: u16) -> Self {
        Self {
            ch: ' ',
            style,
            kind: CellKind::WideTail,
            _pad: 0,
        }
    }
}

/// 逻辑行的稳定身份：一次创建，终生不变（同一逻辑行被切成多行时共享）。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LineId(pub u32);

/// 光标锚点：逻辑坐标，重排后仍然有效。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CursorAnchor {
    pub line: LineId,
    /// 逻辑行内的 cell 偏移（含 WideTail 占位格，末端 = 行长）。
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct Row {
    /// 长度恒为 `cols`，尾部空白只是缓冲，不算内容。
    pub cells: Vec<Cell>,
    /// 有效 cell 数：逻辑流只取 `cells[..len]`。
    pub len: u16,
    /// true = 下一行首 cell 是本行的直接续写。
    pub wrapped: bool,
    pub line: LineId,
    /// 本行首 cell 在该逻辑行内的偏移。
    pub line_start: u32,
}

impl Row {
    pub fn new(cols: u16, line: LineId, line_start: u32) -> Self {
        Self {
            cells: vec![Cell::blank(STYLE_DEFAULT); cols as usize],
            len: 0,
            wrapped: false,
            line,
            line_start,
        }
    }

    /// 取文本时跳过 WideTail，否则每个中文会多出一个空格。
    pub fn text(&self) -> String {
        self.cells[..self.len as usize]
            .iter()
            .filter(|c| c.kind != CellKind::WideTail)
            .map(|c| c.ch)
            .collect()
    }
}

pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub screen: Vec<Row>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub pending_wrap: bool,
    pub anchor: CursorAnchor,
    next_line: u32,

    /// 滚出屏幕顶部的行（FIFO，容量 `history_limit`）。
    pub history: std::collections::VecDeque<Row>,
    pub history_limit: usize,
    /// 视口上滚的行数：0 = 贴底（实时），N = 往回看 N 行。
    pub scroll_offset: usize,
    /// 样式驻留表：Cell.style 是这里的索引。
    pub styles: StyleTable,

    /// 备用屏幕（vim / htop 用）：进入时把主屏整屏存这里。
    alt: Option<Box<AltScreen>>,
    /// DECTCEM：光标是否可见（`CSI ?25h/l`）。
    pub cursor_hidden: bool,
    /// 自动换行（`CSI ?7h/l`），vim 会关掉它。
    pub autowrap: bool,
}

/// 备用屏幕期间保存的主屏状态。
struct AltScreen {
    screen: Vec<Row>,
    cursor_row: usize,
    cursor_col: usize,
    pending_wrap: bool,
    anchor: CursorAnchor,
    scroll_offset: usize,
}

impl Grid {
    pub fn new(cols: u16, rows: u16) -> Self {
        assert!(cols > 0 && rows > 0, "grid must be non-empty");
        let screen = (0..rows)
            .map(|i| Row::new(cols, LineId(i as u32), 0))
            .collect();
        Self {
            cols,
            rows,
            screen,
            cursor_row: 0,
            cursor_col: 0,
            pending_wrap: false,
            anchor: CursorAnchor {
                line: LineId(0),
                offset: 0,
            },
            next_line: rows as u32,
            history: std::collections::VecDeque::new(),
            history_limit: 10_000,
            scroll_offset: 0,
            styles: StyleTable::default(),
            alt: None,
            cursor_hidden: false,
            autowrap: true,
        }
    }

    /// 是否处于备用屏幕。
    pub fn in_alt_screen(&self) -> bool {
        self.alt.is_some()
    }

    /// 进入备用屏幕：主屏整屏快照，然后清空备用屏。
    pub fn enter_alt_screen(&mut self) {
        if self.alt.is_some() {
            return;
        }
        let saved = AltScreen {
            screen: self.screen.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
            pending_wrap: self.pending_wrap,
            anchor: self.anchor,
            scroll_offset: self.scroll_offset,
        };
        self.alt = Some(Box::new(saved));

        for row in self.screen.iter_mut() {
            *row = Row::new(self.cols, row.line, 0);
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.pending_wrap = false;
        self.scroll_offset = 0;
    }

    /// 退出备用屏幕：恢复主屏快照。
    pub fn leave_alt_screen(&mut self) {
        let Some(saved) = self.alt.take() else {
            return;
        };
        self.screen = saved.screen;
        self.cursor_row = saved.cursor_row.min(self.rows as usize - 1);
        self.cursor_col = saved.cursor_col.min(self.cols as usize - 1);
        self.pending_wrap = saved.pending_wrap;
        self.anchor = saved.anchor;
        self.scroll_offset = saved.scroll_offset.min(self.history.len());
    }

    /// 驻留一个样式，返回可存进 Cell 的索引。
    pub fn intern_style(&mut self, style: CellStyle) -> u16 {
        self.styles.intern(style)
    }

    /// 查索引对应的样式（渲染时用）。
    pub fn style_at(&self, id: u16) -> CellStyle {
        self.styles.get(id)
    }

    // ---- 滚动历史 ----

    /// 往回看 `lines` 行（受历史长度限制）。
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = (self.scroll_offset + lines).min(self.history.len());
    }

    /// 往回 `lines` 行；到 0 即贴底。
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    /// 用户按键后应回到实时视图。
    pub fn reset_scroll_on_input(&mut self) {
        self.scroll_offset = 0;
    }

    /// 视口第 `view_row` 行对应的实际 Row。
    pub fn visible_row(&self, view_row: usize) -> Option<&Row> {
        let offset = self.scroll_offset.min(self.history.len());
        if view_row < offset {
            let idx = self.history.len() - offset + view_row;
            self.history.get(idx)
        } else {
            self.screen.get(view_row - offset)
        }
    }

    fn push_history(&mut self, row: Row) {
        if self.history_limit == 0 {
            return;
        }
        while self.history.len() >= self.history_limit {
            self.history.pop_front();
        }
        self.history.push_back(row);
        // 视口上滚时，新行入历史应保持当前画面不跳动
        if self.scroll_offset > 0 {
            self.scroll_offset = (self.scroll_offset + 1).min(self.history.len());
        }
    }

    fn width_of_char(ch: char) -> usize {
        match ch.width().unwrap_or(1) {
            0 => 0,
            1 => 1,
            _ => 2,
        }
    }

    /// 写入一个字符。宽字符在右边界放不下时整字下推，末列留空。
    pub fn place_cell(&mut self, ch: char, style: u16) {
        let mut w = Self::width_of_char(ch);
        if w == 0 {
            return; // 骨架阶段：组合字符暂不处理
        }
        if self.cols < 2 {
            w = 1; // 1 列网格永远装不下宽字，退化为窄字
        }
        let cols = self.cols as usize;

        // 1) 结算上一次写满留下的 pending_wrap
        if self.pending_wrap {
            self.pending_wrap = false;
            self.soft_wrap();
        }

        // 2) 宽字只剩 1 列：整字下推
        if w == 2 && self.cursor_col + 2 > cols {
            self.soft_wrap();
        }

        // 3) 先取出所有标量，再拿 &mut，避免借用重叠
        let (r, c) = (self.cursor_row, self.cursor_col);
        let row = &mut self.screen[r];

        clear_wide_pair(row, c);
        if w == 2 {
            clear_wide_pair(row, c + 1);
            row.cells[c] = Cell::wide_head(ch, style);
            row.cells[c + 1] = Cell::wide_tail(style);
        } else {
            row.cells[c] = Cell::narrow(ch, style);
        }
        row.len = row.len.max((c + w) as u16);

        // 4) 推进光标；写满则挂起，光标留在末列
        let next = c + w;
        if next >= cols {
            // DECAWM 关闭时（vim 画状态行会关），停在末列不换行
            self.pending_wrap = self.autowrap;
            self.cursor_col = cols - 1;
        } else {
            self.cursor_col = next;
        }
        self.sync_anchor();
    }

    /// 移动光标（骨架里只做模拟命中/定位）。落在 WideTail 上时归位到 WideHead。
    pub fn set_cursor(&mut self, row: usize, col: usize) {
        let row = row.min(self.rows as usize - 1);
        let col = col.min(self.cols as usize - 1);
        let (line, line_start, len, kind) = {
            let r = &self.screen[row];
            (r.line, r.line_start, r.len as usize, r.cells[col].kind)
        };
        let col = if kind == CellKind::WideTail && col > 0 {
            col - 1
        } else {
            col
        };
        self.anchor = CursorAnchor {
            line,
            offset: (line_start as usize + col.min(len)) as u32,
        };
        self.cursor_row = row;
        self.cursor_col = col;
        self.pending_wrap = false;
    }

    /// 按新列宽重排整屏：合并 wrapped 段 → 重新切片 → 重新投影光标。
    pub fn reflow(&mut self, new_cols: u16) {
        assert!(new_cols > 0, "cols must be positive");
        if new_cols == self.cols {
            return;
        }
        let cols = new_cols as usize;
        let old = std::mem::take(&mut self.screen); // 先摘走，避免 &self 与 &mut self.screen 打架

        // 视口底部未被写过的空行只是填充，不参与重排，否则会挤掉真实内容
        let rows = self.rows as usize;
        let last_nonempty = old.iter().rposition(|r| r.len > 0).map_or(0, |i| i + 1);
        let used = last_nonempty.max(self.cursor_row + 1).min(rows);

        // 历史与屏幕是一条连续的物理行序列：一起重排，否则跨边界的
        // 软换行会被拆成两段，滚回去就是断行。
        let mut all: Vec<Row> = Vec::with_capacity(self.history.len() + used);
        all.extend(self.history.drain(..));
        all.extend(old.into_iter().take(used));

        // 1) 按 wrapped 把物理行合并成逻辑行（尾随 padding 已被 len 排除）
        struct Logical {
            id: LineId,
            cells: Vec<Cell>,
        }
        let mut lines: Vec<Logical> = Vec::new();
        let mut prev_wrapped = false;
        for row in &all {
            let content = &row.cells[..row.len as usize];
            match lines.last_mut() {
                Some(last) if prev_wrapped => last.cells.extend_from_slice(content),
                _ => lines.push(Logical {
                    id: row.line,
                    cells: content.to_vec(),
                }),
            }
            prev_wrapped = row.wrapped;
        }
        if lines.is_empty() {
            lines.push(Logical {
                id: LineId(0),
                cells: Vec::new(),
            });
        }

        // 2) 按新宽度重新切片：宽字整字下推，永远不产生孤立 WideTail
        let mut new_rows: Vec<Row> = Vec::new();
        for line in &lines {
            let mut cells = vec![Cell::blank(STYLE_DEFAULT); cols];
            let mut len = 0usize;
            let mut start = 0u32;
            let mut i = 0usize;
            while i < line.cells.len() {
                let cell = line.cells[i];
                if cell.kind == CellKind::WideTail {
                    i += 1; // 尾格由 head 成对写入，单独出现一律丢弃
                    continue;
                }
                let mut w = if cell.kind == CellKind::WideHead {
                    2
                } else {
                    1
                };
                if w == 2 && cols < 2 {
                    w = 1;
                }
                if len + w > cols {
                    new_rows.push(Row {
                        cells,
                        len: len as u16,
                        wrapped: true,
                        line: line.id,
                        line_start: start,
                    });
                    start += len as u32;
                    cells = vec![Cell::blank(STYLE_DEFAULT); cols];
                    len = 0;
                    continue; // 同一 cell 在新行重试
                }
                if w == 2 {
                    cells[len] = Cell::wide_head(cell.ch, cell.style);
                    cells[len + 1] = Cell::wide_tail(cell.style);
                } else {
                    cells[len] = Cell::narrow(cell.ch, cell.style);
                }
                len += w;
                i += 1;
            }
            new_rows.push(Row {
                cells,
                len: len as u16,
                wrapped: false,
                line: line.id,
                line_start: start,
            });
        }

        // 3) 视口裁剪：变高时保留尾部（顶部滚出到历史），并修正该逻辑行的偏移基准
        if new_rows.len() > rows {
            let drop = new_rows.len() - rows;
            let base_line = new_rows[drop].line;
            let base_off = new_rows[drop].line_start;
            let overflow: Vec<Row> = new_rows.drain(..drop).collect();
            for row in overflow {
                if self.history_limit > 0 {
                    while self.history.len() >= self.history_limit {
                        self.history.pop_front();
                    }
                    self.history.push_back(row);
                }
            }
            if base_off > 0 {
                for r in new_rows.iter_mut() {
                    if r.line == base_line {
                        r.line_start -= base_off;
                    }
                }
                if self.anchor.line == base_line {
                    self.anchor.offset = self.anchor.offset.saturating_sub(base_off);
                }
            }
        }
        while new_rows.len() < rows {
            let id = LineId(self.next_line);
            self.next_line += 1;
            new_rows.push(Row::new(new_cols, id, 0));
        }

        // 4) 换上新屏，再把逻辑光标重新投影成 (row, col)
        self.screen = new_rows;
        self.cols = new_cols;
        self.scroll_offset = self.scroll_offset.min(self.history.len());
        let (r, c, pending) = self.project(self.anchor);
        self.cursor_row = r;
        self.cursor_col = c;
        self.pending_wrap = pending;
    }

    /// 把逻辑锚点投影回物理坐标。返回 (row, col, pending_wrap)。
    fn project(&self, anchor: CursorAnchor) -> (usize, usize, bool) {
        let cols = self.cols as usize;
        let Some(mut idx) = self.screen.iter().position(|r| r.line == anchor.line) else {
            return (0, 0, false); // 该逻辑行已滚出视口
        };
        let base = self.screen[idx].line_start as usize;
        let mut off = (anchor.offset as usize).saturating_sub(base);

        while idx < self.screen.len() {
            let row = &self.screen[idx];
            let len = row.len as usize;
            if off < len {
                let c = if row.cells[off].kind == CellKind::WideTail && off > 0 {
                    off - 1
                } else {
                    off
                };
                return (idx, c, false);
            }
            if off == len {
                if !row.wrapped {
                    return if len == cols {
                        (idx, cols - 1, true) // 正好写满末行：光标挂在末列
                    } else {
                        (idx, len, false)
                    };
                }
                off = 0;
                idx += 1;
                continue;
            }
            // off > len
            if row.wrapped {
                off -= len;
                idx += 1;
            } else {
                let c = len.min(cols - 1);
                return (idx, c, len == cols);
            }
        }
        (self.screen.len() - 1, cols - 1, true)
    }

    fn sync_anchor(&mut self) {
        let row = &self.screen[self.cursor_row];
        let off = row.line_start as usize
            + if self.pending_wrap {
                row.len as usize
            } else {
                self.cursor_col
            };
        self.anchor = CursorAnchor {
            line: row.line,
            offset: off as u32,
        };
    }

    /// 软换行：标脏当前行，下移一行（必要时滚屏），新行继承同一逻辑行。
    fn soft_wrap(&mut self) {
        let (line, next_start) = {
            let row = &mut self.screen[self.cursor_row];
            row.wrapped = true;
            (row.line, row.line_start + row.len as u32)
        };
        self.cursor_col = 0;
        if self.cursor_row + 1 < self.rows as usize {
            self.cursor_row += 1;
            let row = &mut self.screen[self.cursor_row];
            row.line = line;
            row.line_start = next_start;
        } else {
            let top = self.screen.remove(0);
            self.push_history(top);
            self.screen.push(Row::new(self.cols, line, next_start));
        }
    }

    /// 仅改变可见行数：追加空行或裁剪末尾行，绝不触发重排。
    pub fn set_rows(&mut self, new_rows: u16) {
        assert!(new_rows > 0, "rows must be positive");
        if new_rows == self.rows {
            return;
        }
        let target = new_rows as usize;
        self.rows = new_rows;

        if target < self.screen.len() {
            self.screen.truncate(target);
            // 被截断后末行不再续写，清掉悬空的 wrapped 标记
            if let Some(last) = self.screen.last_mut() {
                last.wrapped = false;
            }
            if self.cursor_row >= target {
                let col = self.cursor_col.min(self.screen[target - 1].len as usize);
                self.set_cursor(target - 1, col);
            }
        } else {
            while self.screen.len() < target {
                let id = LineId(self.next_line);
                self.next_line += 1;
                self.screen.push(Row::new(self.cols, id, 0));
            }
        }
    }

    /// 硬换行（LF）：结束当前逻辑行，下移一行，列不变；到底则滚屏。
    pub fn line_feed(&mut self) {
        self.pending_wrap = false;
        let src_line = self.screen[self.cursor_row].line;
        self.screen[self.cursor_row].wrapped = false;

        if self.cursor_row + 1 < self.rows as usize {
            self.cursor_row += 1;
        } else {
            let top = self.screen.remove(0);
            self.push_history(top);
            let id = LineId(self.next_line);
            self.next_line += 1;
            self.screen.push(Row::new(self.cols, id, 0));
        }

        // 落点原本是同一条逻辑行的续行时，硬断必须让它独立成行
        if self.screen[self.cursor_row].line == src_line {
            let id = LineId(self.next_line);
            self.next_line += 1;
            let row = &mut self.screen[self.cursor_row];
            row.line = id;
            row.line_start = 0;
        }
        self.sync_anchor();
    }

    /// 回车（CR）：回到行首，不换行。
    pub fn carriage_return(&mut self) {
        self.pending_wrap = false;
        self.cursor_col = 0;
        self.sync_anchor();
    }

    /// 退格：只移动光标，不擦除（TTY 的 "\b \b" 会自己擦）。
    pub fn backspace(&mut self) {
        self.pending_wrap = false;
        self.cursor_col = self.cursor_col.saturating_sub(1);
        self.sync_anchor();
    }

    /// 制表符：跳到下一个 8 列制表位。
    pub fn tab(&mut self) {
        self.pending_wrap = false;
        let next = ((self.cursor_col / 8) + 1) * 8;
        self.cursor_col = next.min(self.cols as usize - 1);
        self.sync_anchor();
    }

    /// 相对移动光标（CSI A/B/C/D）。
    pub fn move_cursor(&mut self, d_row: isize, d_col: isize) {
        self.pending_wrap = false;
        self.cursor_row =
            (self.cursor_row as isize + d_row).clamp(0, self.rows as isize - 1) as usize;
        self.cursor_col =
            (self.cursor_col as isize + d_col).clamp(0, self.cols as isize - 1) as usize;
        self.sync_anchor();
    }

    /// 擦除整行或行内片段（CSI K）：0=光标到行尾，1=行首到光标，2=整行。
    pub fn erase_in_line(&mut self, mode: u16) {
        let c = self.cursor_col.min(self.cols as usize - 1);
        let row = &mut self.screen[self.cursor_row];
        match mode {
            0 => blank_from(row, c),
            1 => blank_upto(row, c),
            _ => blank_from(row, 0),
        }
        recompute_len(row);
        self.sync_anchor();
    }

    /// 擦除屏幕（CSI J）：0=光标以下，1=光标以上，2=全部。
    pub fn erase_in_display(&mut self, mode: u16) {
        let cursor_row = self.cursor_row;
        let c = self.cursor_col.min(self.cols as usize - 1);
        match mode {
            0 => {
                blank_from(&mut self.screen[cursor_row], c);
                recompute_len(&mut self.screen[cursor_row]);
                for row in self.screen.iter_mut().skip(cursor_row + 1) {
                    blank_from(row, 0);
                    recompute_len(row);
                }
            }
            1 => {
                for row in self.screen.iter_mut().take(cursor_row) {
                    blank_from(row, 0);
                    recompute_len(row);
                }
                blank_upto(&mut self.screen[cursor_row], c);
                recompute_len(&mut self.screen[cursor_row]);
            }
            _ => {
                for row in self.screen.iter_mut() {
                    blank_from(row, 0);
                    recompute_len(row);
                }
            }
        }
        self.sync_anchor();
    }

    /// 取整条逻辑行的可见文本（跨软换行拼接，跳过 WideTail）。
    pub fn line_text(&self, row: usize) -> String {
        let mut first = row.min(self.screen.len() - 1);
        while first > 0 && self.screen[first - 1].wrapped {
            first -= 1;
        }
        let mut out = String::new();
        let mut i = first;
        loop {
            let r = &self.screen[i];
            out.extend(
                r.cells[..r.len as usize]
                    .iter()
                    .filter(|c| c.kind != CellKind::WideTail)
                    .map(|c| c.ch),
            );
            if !r.wrapped || i + 1 >= self.screen.len() {
                break;
            }
            i += 1;
        }
        out
    }
}

/// 清空 `row.cells[start..]`。
fn blank_from(row: &mut Row, start: usize) {
    for cell in &mut row.cells[start..] {
        *cell = Cell::blank(STYLE_DEFAULT);
    }
}

/// 清空 `row.cells[..=end]`。
fn blank_upto(row: &mut Row, end: usize) {
    for cell in &mut row.cells[..=end] {
        *cell = Cell::blank(STYLE_DEFAULT);
    }
}

/// 重新计算有效长度（最后一个非空白 cell 之后）。
fn recompute_len(row: &mut Row) {
    let mut last = 0;
    for (i, cell) in row.cells.iter().enumerate() {
        if cell.kind != CellKind::Narrow || cell.ch != ' ' {
            last = i + 1;
        }
    }
    row.len = last as u16;
}

/// 若目标列属于某个宽字（头或尾），把该宽字两格一起清空。
fn clear_wide_pair(row: &mut Row, col: usize) {
    let kind = row.cells[col].kind;
    let (lo, hi) = match kind {
        CellKind::WideHead => (col, col + 1),
        CellKind::WideTail if col > 0 => (col - 1, col),
        _ => return,
    };
    if hi >= row.cells.len() {
        return;
    }
    let style = row.cells[lo].style;
    for cell in &mut row.cells[lo..=hi] {
        *cell = Cell::blank(style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_is_eight_bytes() {
        assert_eq!(std::mem::size_of::<Cell>(), 8);
    }

    #[test]
    fn alt_screen_roundtrip() {
        let mut grid = Grid::new(10, 3);
        for (i, ch) in "MAIN".chars().enumerate() {
            grid.set_cursor(0, i);
            grid.place_cell(ch, 0);
        }

        grid.enter_alt_screen();
        assert!(grid.in_alt_screen());
        assert_eq!(grid.line_text(0), ""); // 备用屏是空的

        for (i, ch) in "ALT".chars().enumerate() {
            grid.set_cursor(0, i);
            grid.place_cell(ch, 0);
        }
        assert_eq!(grid.line_text(0), "ALT");

        grid.leave_alt_screen();
        assert!(!grid.in_alt_screen());
        assert_eq!(grid.line_text(0), "MAIN"); // 主屏原样恢复
    }

    #[test]
    fn scrollback_keeps_evicted_rows() {
        let mut grid = Grid::new(8, 2);
        grid.place_cell('a', 0);
        grid.carriage_return();
        grid.line_feed();
        grid.place_cell('b', 0);
        grid.carriage_return();
        grid.line_feed();
        grid.place_cell('c', 0);

        assert_eq!(grid.history.len(), 1);
        assert_eq!(grid.history[0].text(), "a");
        assert_eq!(grid.screen[0].text(), "b");
        assert_eq!(grid.screen[1].text(), "c");

        // 视口偏移
        grid.scroll_up(1);
        assert_eq!(grid.visible_row(0).unwrap().text(), "a");
        assert_eq!(grid.visible_row(1).unwrap().text(), "b");
        grid.scroll_bottom();
        assert_eq!(grid.visible_row(0).unwrap().text(), "b");
    }

    #[test]
    fn reflow_includes_history() {
        let mut grid = Grid::new(20, 2);
        // 写 3 行，第一行进入历史
        for word in ["1111", "2222", "3333"] {
            for ch in word.chars() {
                grid.place_cell(ch, 0);
            }
            grid.line_feed();
        }
        assert_eq!(grid.history.len(), 2);

        // 缩窄后历史行也应该重排到新宽度
        grid.reflow(4);
        assert_eq!(grid.cols, 4);
        assert_eq!(grid.history[0].text(), "1111");
        assert_eq!(grid.history[0].cells.len(), 4);
    }

    #[test]
    fn style_table_interns_and_dedups() {
        let mut grid = Grid::new(4, 2);
        let a = grid.intern_style(CellStyle {
            fg: ColorSpec::Rgb(255, 100, 0),
            ..CellStyle::default()
        });
        let b = grid.intern_style(CellStyle {
            fg: ColorSpec::Rgb(255, 100, 0),
            ..CellStyle::default()
        });
        assert_eq!(a, b, "同样的样式必须复用同一个索引");
        assert_eq!(grid.style_at(a).fg, ColorSpec::Rgb(255, 100, 0));

        let plain = grid.intern_style(CellStyle::default());
        assert_eq!(plain, STYLE_DEFAULT);
    }

    #[test]
    fn wide_char_pushes_down_instead_of_splitting() {
        let mut g = Grid::new(5, 4);
        for ch in "abcd".chars() {
            g.place_cell(ch, 0);
        }
        g.place_cell('中', 0);
        assert_eq!(g.screen[0].text(), "abcd");
        assert!(g.screen[0].wrapped);
        assert_eq!(g.screen[1].cells[0].kind, CellKind::WideHead);
        assert_eq!(g.screen[1].cells[1].kind, CellKind::WideTail);
        assert_eq!(g.line_text(0), "abcd中");
    }

    #[test]
    fn overwriting_tail_clears_whole_wide_char() {
        let mut g = Grid::new(4, 2);
        g.place_cell('中', 0);
        g.set_cursor(0, 1); // 归位到 head
        g.cursor_col = 1; // 手动落到 WideTail 上
        g.place_cell('x', 0);
        assert_eq!(g.screen[0].cells[0].kind, CellKind::Narrow);
        assert_eq!(g.screen[0].cells[1].ch, 'x');
    }

    #[test]
    fn pending_wrap_defers_until_next_char() {
        let mut g = Grid::new(3, 2);
        for ch in "abc".chars() {
            g.place_cell(ch, 0);
        }
        assert!(g.pending_wrap);
        assert_eq!(g.cursor_row, 0);
        g.place_cell('d', 0);
        assert_eq!(g.screen[0].text(), "abc");
        assert_eq!(g.screen[1].cells[0].ch, 'd');
        assert!(!g.pending_wrap);
    }

    /// 5 列缩到 2 列：中文整字下推，绝不劈开。
    #[test]
    fn narrowing_never_splits_wide_chars() {
        let mut g = Grid::new(5, 5);
        for ch in "ab中cd".chars() {
            g.place_cell(ch, 0);
        }
        assert_eq!(g.line_text(0), "ab中cd");

        g.reflow(2);

        for row in &g.screen {
            let len = row.len as usize;
            for (i, cell) in row.cells[..len].iter().enumerate() {
                match cell.kind {
                    CellKind::WideHead => {
                        assert!(i + 1 < len, "WideHead 在行尾被劈开");
                        assert_eq!(row.cells[i + 1].kind, CellKind::WideTail);
                    }
                    CellKind::WideTail => {
                        assert!(i > 0, "行首出现孤立 WideTail");
                        assert_eq!(row.cells[i - 1].kind, CellKind::WideHead);
                    }
                    CellKind::Narrow => {}
                }
            }
        }
        assert_eq!(g.line_text(0), "ab中cd");

        // 再放回 5 列，内容必须逐字还原
        g.reflow(5);
        assert_eq!(g.line_text(0), "ab中cd");
    }

    /// 光标锚点是逻辑坐标：重排后仍指向同一个字符。
    #[test]
    fn cursor_keeps_pointing_at_same_char() {
        let mut g = Grid::new(5, 4);
        for ch in "a中b".chars() {
            g.place_cell(ch, 0);
        }
        g.set_cursor(0, 2); // 落在 中 的 WideTail 上
        assert_eq!(g.anchor.offset, 1); // 已归位到 WideHead
        assert_eq!(g.screen[0].cells[1].kind, CellKind::WideHead);

        g.reflow(2);

        let (r, c) = (g.cursor_row, g.cursor_col);
        assert_eq!(g.screen[r].cells[c].ch, '中');
        assert_eq!(g.screen[r].cells[c].kind, CellKind::WideHead);
        assert_eq!(g.anchor.offset, 1);
        assert_eq!(g.line_text(r), "a中b");
    }

    /// 光标在行尾（pending_wrap）时重排，仍停在逻辑行末。
    #[test]
    fn pending_wrap_cursor_survives_reflow() {
        let mut g = Grid::new(3, 3);
        for ch in "abcdef".chars() {
            g.place_cell(ch, 0);
        }
        assert!(g.pending_wrap);
        g.reflow(2);
        assert_eq!(g.line_text(g.cursor_row), "abcdef");
        // 6 个窄字 2 列 = 3 整行，光标挂末行末列
        assert!(g.pending_wrap);
        assert_eq!(g.cursor_row, 2);
        assert_eq!(g.cursor_col, 1);
    }

    /// 硬换行与回车：Shell 输出流（CRLF）在重排后仍是两条独立逻辑行。
    #[test]
    fn shell_crlf_makes_hard_lines() {
        let mut g = Grid::new(10, 4);
        for ch in "abc".chars() {
            g.place_cell(ch, 0);
        }
        g.carriage_return();
        g.line_feed();
        for ch in "de".chars() {
            g.place_cell(ch, 0);
        }
        assert_eq!(g.screen[0].text(), "abc");
        assert_eq!(g.screen[1].text(), "de");
        assert!(!g.screen[0].wrapped);

        g.reflow(5);
        assert_eq!(g.screen[0].text(), "abc");
        assert_eq!(g.screen[1].text(), "de");
    }
}
