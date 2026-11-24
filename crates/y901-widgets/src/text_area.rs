//! 多行文本编辑框（`TextArea`）。
//!
//! 与 [`TextInput`](crate::TextInput) 的分工：那个是**单行**输入（宽度锁死、水平滚动），
//! 这个是**多行**编辑（软换行、垂直滚动、跨行选区、回车换行）。
//!
//! 复用关系是刻意的：
//!
//! - 文本与光标/选区的**唯一真相**仍在 [`TextBuffer`]（`y901_ui::text`），
//!   本文件只负责"字节偏移 ↔ 屏幕坐标"的映射；
//! - 剪贴板抽象、光标闪烁周期、DPI 内边距缩放、主题取色全部沿用 `TextInput` 的口径
//!   （重复实现会让两个控件的键盘行为慢慢分叉）。
//!
//! ## 为什么排版表放在控件里
//!
//! 事件阶段拿不到 `TextMetrics`（框架约束：事件不携带字体度量），
//! 所以软换行的结果（每行字节范围）在 `measure`/`arrange` 时算好缓存，
//! 点击命中和上下移动直接查表。渲染时再按同一份表逐行画，
//! **保证"画在哪"与"点在哪"用的是同一套坐标**。

use std::time::Duration;

use web_time::Instant;

use y901_ui::text::TextBuffer;
use y901_ui::{
    Color, Constraints, Dimension, Edges, EventCtx, EventResult, FlexItem, Key, Modifiers, Pos,
    Rect, RenderCtx, Role, Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

use crate::text_input::{Clipboard, NullClipboard};

/// 光标闪烁周期（与 `TextInput` 一致）。
const BLINK: Duration = Duration::from_millis(530);
/// 光标线宽。
const CARET_W: f32 = 1.5;
/// 没有父级高度约束时的默认可视行数。
const DEFAULT_ROWS: usize = 4;

/// 软换行后的一条**可视行**：缓冲区里的字节区间（不含行尾的 `\n`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VisLine {
    start: usize,
    end: usize,
}

/// 多行文本编辑框。
///
/// 用法：
///
/// ```no_run
/// # use y901_widgets::TextArea;
/// let area = TextArea::new("editor").with_text("第一行\n第二行");
/// ```
pub struct TextArea {
    id: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
    buffer: TextBuffer,
    style: TextStyle,
    selection_color: Color,
    caret_color: Color,
    padding: Edges,
    /// 最近一次布局用到的 DPI 缩放（事件/渲染阶段拿不到 metrics）。
    ui_scale: f32,
    item: FlexItem,
    rect: Rect,
    /// 软换行结果：每次 `relayout` 重建。
    lines: Vec<VisLine>,
    /// 行高（物理像素），`relayout` 时从 metrics 取。
    line_h: f32,
    /// 半角字符宽度（物理像素），`relayout` 时实测。
    ///
    /// 为什么不在事件里临时算：事件阶段拿不到 `TextMetrics`。旧实现用
    /// `line_h * 0.5` 近似，在**奇数逻辑字号**下与真实格宽不等
    /// （13px 时近似 6.5、真实 `round(6.5)=7`），14 个字符就偏差一个整格 ——
    /// 光标与点击会落到相邻字符上。这里直接存度量结果。
    cell_w: f32,
    /// 全角字符宽度（物理像素），同上。
    wide_w: f32,
    /// 可视区尺寸（已扣掉内边距）。
    inner_w: f32,
    inner_h: f32,
    /// 垂直滚动偏移（物理像素）。
    scroll_y: f32,
    focused: bool,
    disabled: bool,
    caret_visible: bool,
    last_blink: Instant,
    preedit: String,
    clipboard: Box<dyn Clipboard>,
    version: u64,
    dragging: bool,
    /// 上下移动时保持的**目标 x**（像素）：连续按 ↑↓ 不应左右漂移。
    goal_x: Option<f32>,
    placeholder: Option<String>,
    /// 期望可视行数（只影响 `measure` 的默认高度）。
    rows: usize,
    /// 文本/宽度变了、需要重新软换行（`render` 首步消费）。
    ///
    /// 为什么不在每次编辑里直接重排：编辑阶段没有 `TextMetrics`（框架约束），
    /// 而 `render` 的 `RenderCtx` 同时实现了 `TextMetrics`，是唯一能拿到度量的时机。
    needs_relayout: bool,
    /// 上次 `render` 用到的可视宽度（宽度变化也要重排）。
    inner_w_last: f32,
}

impl TextArea {
    /// 建一个空的编辑框。
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            id,
            key,
            buffer: TextBuffer::new(),
            style: TextStyle::fg(Color::WHITE).with_bg(Color::rgba(0.10, 0.11, 0.14, 1.0)),
            selection_color: Color::rgba(0.24, 0.45, 0.75, 0.55),
            caret_color: Color::rgba(0.85, 0.90, 1.0, 1.0),
            padding: Edges::symmetric(8.0, 4.0),
            ui_scale: 1.0,
            item: FlexItem::default(),
            rect: Rect::ZERO,
            lines: vec![VisLine { start: 0, end: 0 }],
            line_h: 16.0,
            cell_w: 8.0,
            wide_w: 16.0,
            inner_w: 0.0,
            inner_h: 0.0,
            scroll_y: 0.0,
            focused: false,
            disabled: false,
            caret_visible: true,
            last_blink: Instant::now(),
            preedit: String::new(),
            clipboard: Box::new(NullClipboard),
            version: 0,
            dragging: false,
            goal_x: None,
            placeholder: None,
            rows: DEFAULT_ROWS,
            needs_relayout: true,
            inner_w_last: 0.0,
        }
    }

    /// 设置初始文本，光标落在末尾。
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.buffer = TextBuffer::from_text(text);
        self.version += 1;
        self
    }

    /// 覆盖文本样式（前景/字号等）。
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// 内边距（**单位是 DIP**，实际值乘 `metrics.scale()`，与 `TextInput`/`Flex` 同口径）。
    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    /// flex 伸缩参数。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 禁用：不参与焦点、不接收事件、渲染成 `State::Disabled`。
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 占位提示（不进缓冲区、不进语义，只在完全空时画）。
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// 注入剪贴板实现（默认 `NullClipboard`：Ctrl+C/V 无效果）。
    pub fn with_clipboard(mut self, clipboard: Box<dyn Clipboard>) -> Self {
        self.clipboard = clipboard;
        self
    }

    /// 默认可视行数（只影响 `measure` 在无高度约束时的高度）。
    pub fn rows(mut self, rows: usize) -> Self {
        self.rows = rows.max(1);
        self
    }

    /// 设文本（光标移到末尾）。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.buffer = TextBuffer::from_text(text);
        self.touch();
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.buffer = TextBuffer::new();
        self.touch();
    }

    /// 当前文本（含换行符）。
    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// 底部编辑缓冲（只读）：光标、锚点、选区都在这里。
    pub fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    /// 光标字节偏移。
    pub fn cursor(&self) -> usize {
        self.buffer.cursor()
    }

    /// 选区（规范化后的 `(start, end)`）。
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.buffer.selection()
    }

    /// 是否聚焦。
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// 当前**可视行**数（软换行之后）。诊断/测试用。
    pub fn visual_line_count(&self) -> usize {
        self.lines.len()
    }

    /// 垂直滚动偏移。
    pub fn scroll_y(&self) -> f32 {
        self.scroll_y
    }

    /// 当前 IME 预编辑串（不进缓冲区）。
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// 每次编辑 +1（供增量语义推送感知变化）。
    fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.caret_visible = true;
        self.last_blink = Instant::now();
        self.goal_x = None;
        self.needs_relayout = true;
    }

    /// 实际内边距（DIP → 物理像素）。
    fn padding_scaled(&self, scale: f32) -> Edges {
        Edges {
            left: self.padding.left * scale,
            top: self.padding.top * scale,
            right: self.padding.right * scale,
            bottom: self.padding.bottom * scale,
        }
    }

    // ---------- 排版 ----------

    /// 按当前 `inner_w` 做软换行，重建 `lines`。
    ///
    /// 规则：
    /// - 先按 `\n` 切**逻辑行**，再在逻辑行内按宽度切**可视行**；
    /// - 断行优先取最近一个空格之后（词边界），没有空格就硬断（与主流编辑器一致）；
    /// - 空逻辑行也占一条可视行（否则空行无法点进去）。
    fn relayout(&mut self, metrics: &dyn TextMetrics) {
        self.line_h = metrics.line_height(&self.style).max(1.0);
        self.cell_w = metrics.measure_text("M", &self.style).w.max(1.0);
        self.wide_w = metrics.measure_text("中", &self.style).w.max(self.cell_w);
        let width = self.inner_w.max(1.0);
        let text = self.buffer.text().to_string();
        let mut lines: Vec<VisLine> = Vec::new();

        let mut logical_start = 0usize;
        loop {
            let logical_end = match text[logical_start..].find('\n') {
                Some(i) => logical_start + i,
                None => text.len(),
            };
            // 逻辑行内软换行
            let mut seg_start = logical_start;
            let mut w = 0.0f32;
            let mut last_break: Option<usize> = None;
            for (off, ch) in text[logical_start..logical_end].char_indices() {
                let abs = logical_start + off;
                let cw = TextBuffer::char_width(ch, metrics, &self.style);
                if w + cw > width && abs > seg_start {
                    let cut = match last_break {
                        Some(b) if b > seg_start && b <= abs => b,
                        _ => abs,
                    };
                    lines.push(VisLine {
                        start: seg_start,
                        end: cut,
                    });
                    seg_start = cut;
                    // 重新累计 cut..abs 的宽度（跨行后从 0 开始）
                    w = 0.0;
                    for c in text[cut..abs].chars() {
                        w += TextBuffer::char_width(c, metrics, &self.style);
                    }
                }
                if ch == ' ' {
                    last_break = Some(abs + ch.len_utf8());
                }
                w += cw;
            }
            lines.push(VisLine {
                start: seg_start,
                end: logical_end,
            });
            if logical_end >= text.len() {
                break;
            }
            logical_start = logical_end + 1; // 跳过 '\n'
        }
        if lines.is_empty() {
            lines.push(VisLine { start: 0, end: 0 });
        }
        self.lines = lines;
        // **滚动偏移必须跟着内容收缩**：文本从 20 行变回 1 行时，旧的 `scroll_y`
        // 会把第一条可视行推到窗口外，`first` 直接等于 `lines.len()` ——
        // 结果是整块空白（看起来像 set_text 没生效），而且要等重新聚焦才恢复。
        let max_scroll = (self.lines.len() as f32 * self.line_h - self.inner_h).max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);
    }

    /// 光标所在的可视行下标。
    fn line_index_of_byte(&self, byte: usize) -> usize {
        for (i, line) in self.lines.iter().enumerate() {
            if byte <= line.end && byte >= line.start {
                return i;
            }
        }
        // 落在换行符上（== 上一行 end）或越界：归到最后一行
        self.lines.len().saturating_sub(1)
    }

    /// 切片的**安全区间**：`lines` 可能比当前文本"旧"（编辑只返回 `Redraw`，
    /// 重排要等到下一次 `render` 才有度量），直接拿 `line.start..line.end` 去切字符串
    /// 会在文本变短后越界 panic（"删字后同一帧内点一下"就复现）。
    ///
    /// 这里只做**钳制**，不试图修正映射：旧行边界至多让指针落点偏一个字符，
    /// 而 panic 会让整个进程崩溃 —— 两害相权。
    /// 可视行的字节区间（**两端都对齐到 UTF-8 边界**）。
    ///
    /// 为什么必须对齐：`VisLine` 是**上一次** `relayout` 的结果，而编辑只返回 `Redraw`
    /// （重排发生在下一次 `render`）。同一批事件里点击会拿旧表去切**新**文本 ——
    /// 把起点 clamp 到 `len` 并不够：`len` 可能落在某个多字节字符中间，
    /// `&text[start..end]` 直接 panic（"byte index N is not a char boundary"）。
    fn line_range(&self, line: &VisLine) -> std::ops::Range<usize> {
        let text = self.buffer.text();
        let len = text.len();
        let mut start = line.start.min(len);
        let mut end = line.end.min(len).max(start);
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        start..end
    }

    /// 字节偏移在本行内的 x 偏移（物理像素）。
    fn x_in_line(&self, line: &VisLine, byte: usize) -> f32 {
        let range = self.line_range(line);
        let byte = byte.clamp(range.start, range.end);
        let mut w = 0.0;
        for ch in self.buffer.text()[range.start..byte].chars() {
            w += self.char_w(ch);
        }
        w
    }

    /// 单字符宽度。
    ///
    /// 事件/渲染阶段没有 `TextMetrics`，所以这里用**排版表推导出的近似**：
    /// 取同级字符的平均宽度会让 CJK 与 ASCII 混排点偏，因此按 Unicode 范围粗分
    /// （CJK/全角 = 2 倍宽，其它 = 1 倍宽）。它只影响点击落点与光标 x，
    /// 真实绘制仍走 `RenderCtx::draw_text` 的度量 —— 两者误差被限制在半个字符内。
    fn char_w(&self, ch: char) -> f32 {
        // **零宽组合记号不占格**：`TextBuffer::char_width` 对组合记号返回 0
        // （`Grid::width_of_char`），排版按 0 走；若这里仍按一格算，
        // 光标与点击在 NFD/天城文这类文本上会逐字累计偏移。
        if is_zero_width_mark(ch) {
            return 0.0;
        }
        if (ch as u32) >= 0x2E80 {
            self.wide_w
        } else {
            self.cell_w
        }
    }

    /// 某条可视行的起始 y（相对可视区顶部，已含滚动）。
    fn line_top(&self, index: usize) -> f32 {
        index as f32 * self.line_h - self.scroll_y
    }

    /// 给定行内 x，返回最近的字节边界。
    fn byte_of_line_x(&self, line_index: usize, x: f32) -> usize {
        let Some(line) = self.lines.get(line_index) else {
            return self.buffer.cursor();
        };
        let line = *line;
        let range = self.line_range(&line);
        let mut acc = 0.0;
        for (off, ch) in self.buffer.text()[range.start..range.end].char_indices() {
            let cw = self.char_w(ch);
            if x < acc + cw * 0.5 {
                return range.start + off;
            }
            acc += cw;
        }
        range.end
    }

    /// 把屏幕坐标（客户端坐标）映射成字节偏移。
    fn byte_at_screen(&self, pos: Pos) -> usize {
        let pad = self.padding_scaled(self.ui_scale);
        let local_x = pos.x - (self.rect.x + pad.left);
        let local_y = pos.y - (self.rect.y + pad.top) + self.scroll_y;
        let line_index = if self.line_h > 0.0 {
            (local_y / self.line_h).floor().max(0.0) as usize
        } else {
            0
        };
        let line_index = line_index.min(self.lines.len().saturating_sub(1));
        self.byte_of_line_x(line_index, local_x)
    }

    /// 保证光标在可视区内（垂直方向）。
    fn ensure_caret_visible(&mut self) {
        let index = self.line_index_of_byte(self.buffer.cursor());
        let top = index as f32 * self.line_h;
        let bottom = top + self.line_h;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if self.inner_h > 0.0 && bottom > self.scroll_y + self.inner_h {
            self.scroll_y = bottom - self.inner_h;
        }
        self.scroll_y = self.scroll_y.max(0.0);
    }

    /// 上下移动：保持目标 x（像素），跨可视行。
    fn move_vertical(&mut self, delta: i32, extend: bool) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        let cursor = self.buffer.cursor();
        let index = self.line_index_of_byte(cursor) as i32;
        let goal = self
            .goal_x
            .unwrap_or_else(|| self.x_in_line(&self.lines[index as usize], cursor));
        let last = self.lines.len() as i32 - 1;
        // **越界时退化到文档首/尾**（与主流编辑器一致）：
        // 第一行再按 ↑ 回到全文开头，最后一行再按 ↓ 回到全文末尾。
        let byte = if index + delta < 0 {
            if extend {
                let anchor = self.buffer.anchor();
                self.buffer.set_selection(anchor, 0);
            } else {
                self.buffer.set_cursor(0);
            }
            0
        } else if index + delta > last {
            let end = self.buffer.text().len();
            if extend {
                let anchor = self.buffer.anchor();
                self.buffer.set_selection(anchor, end);
            } else {
                self.buffer.set_cursor(end);
            }
            end
        } else {
            let target = (index + delta) as usize;
            let b = self.byte_of_line_x(target, goal);
            if extend {
                let anchor = self.buffer.anchor();
                self.buffer.set_selection(anchor, b);
            } else {
                self.buffer.set_cursor(b);
            }
            b
        };
        self.goal_x = Some(goal);
        self.caret_visible = true;
        byte != cursor
    }

    // ---------- 键盘 ----------

    fn handle_key(&mut self, key: Key, mods: Modifiers) -> EventResult {
        // Ctrl 组合优先（与 TextInput 同序：先组合键，再普通键）
        if mods.ctrl {
            match key {
                Key::Char('a') | Key::Char('A') => {
                    self.buffer.select_all();
                    return EventResult::Redraw;
                }
                Key::Char('c') | Key::Char('C') => {
                    if let Some(text) = self.buffer.selected_text() {
                        let owned = text.to_string();
                        self.clipboard.set(&owned);
                    }
                    return EventResult::Handled;
                }
                Key::Char('x') | Key::Char('X') => {
                    if let Some(text) = self.buffer.selected_text() {
                        let owned = text.to_string();
                        self.clipboard.set(&owned);
                        self.buffer.delete_selection();
                        self.touch();
                        return EventResult::Redraw;
                    }
                    return EventResult::Handled;
                }
                Key::Char('v') | Key::Char('V') => {
                    if let Some(text) = self.clipboard.get() {
                        self.buffer.insert_str(&text);
                        self.touch();
                        return EventResult::Redraw;
                    }
                    return EventResult::Handled;
                }
                _ => return EventResult::Ignored,
            }
        }

        // 组字中不碰正文：删字归输入法管（与 TextInput 同一条防线）。
        if !self.preedit.is_empty() && matches!(key, Key::Backspace | Key::Delete) {
            return EventResult::Handled;
        }

        match key {
            Key::Backspace => {
                if self.buffer.backspace() {
                    self.touch();
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Delete => {
                if self.buffer.delete_forward() {
                    self.touch();
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Left => {
                if self.buffer.move_left(mods.shift) {
                    self.touch_caret();
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Right => {
                if self.buffer.move_right(mods.shift) {
                    self.touch_caret();
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Up => {
                if self.move_vertical(-1, mods.shift) {
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Down => {
                if self.move_vertical(1, mods.shift) {
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Home => {
                // 按**可视行**行首（软换行后 Home 应该回到这一段的开头）
                let index = self.line_index_of_byte(self.buffer.cursor());
                let start = self.lines[index].start;
                if mods.shift {
                    let anchor = self.buffer.anchor();
                    self.buffer.set_selection(anchor, start);
                } else {
                    self.buffer.set_cursor(start);
                }
                self.touch_caret();
                EventResult::Redraw
            }
            Key::End => {
                let index = self.line_index_of_byte(self.buffer.cursor());
                let end = self.lines[index].end;
                if mods.shift {
                    let anchor = self.buffer.anchor();
                    self.buffer.set_selection(anchor, end);
                } else {
                    self.buffer.set_cursor(end);
                }
                self.touch_caret();
                EventResult::Redraw
            }
            // **回车真的换行**（与单行输入框的关键区别）。
            Key::Enter => {
                self.buffer.insert_char('\n');
                self.touch();
                EventResult::Redraw
            }
            Key::Char(c) if !c.is_control() => {
                self.buffer.insert_char(c);
                self.touch();
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    /// 光标移动（不改变文本）：只重置闪烁与目标列。
    fn touch_caret(&mut self) {
        self.caret_visible = true;
        self.last_blink = Instant::now();
        self.goal_x = None;
    }
}

impl Widget for TextArea {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        // WAI-ARIA 里多行输入是 `textbox`；用同一角色 + `state("multiline")` 标注，
        // 与 TextInput 保持同一套映射（`y901_a11y::aria` 只认 role 字符串）。
        Role::TEXT_INPUT
    }

    fn ink_overflow(&self, metrics: &dyn TextMetrics) -> Option<f32> {
        Some(y901_ui::ink_margin_for(metrics, &self.style))
    }

    fn focusable(&self) -> bool {
        true
    }

    fn is_disabled(&self) -> bool {
        self.disabled
    }

    fn cursor(&self) -> y901_ui::Cursor {
        y901_ui::Cursor::Text
    }

    fn revision(&self) -> u64 {
        self.version
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn layout_version(&self) -> u64 {
        self.version
    }

    fn wants_tick(&self) -> bool {
        self.focused
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn padding(&self) -> Edges {
        self.padding
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.ui_scale = metrics.scale();
        let pad = self.padding_scaled(metrics.scale());
        let padding_h = pad.horizontal();
        let padding_v = pad.vertical();
        self.line_h = metrics.line_height(&self.style).max(1.0);

        // 宽度口径与 `TextInput` 完全一致：**不跟着内容走**。
        // 内容变长只影响软换行与滚动，不触发布局（否则每敲一个字整屏重排）。
        let content_w = match self.item.width {
            Dimension::Px(v) => v,
            _ if constraints.max.w.is_finite() => constraints.max.w,
            _ => self.inner_w + padding_h,
        };
        // 高度：**默认只占 `rows` 行**，只有显式声明"要伸缩/百分比"时才吃满父级。
        //
        // 旧实现无条件 `constraints.max.h`（= 父容器整个内高），后果很实际：
        // `Flex::column([TextArea, Button])` 里 TextArea 报满高度、按顺序排下去，
        // 后面的按钮直接被挤到容器底部之外（看不见），而 `rows(4)` 永远不生效。
        // 这与 `TextInput` 的口径也不一致（它只报一行高）。
        //
        // 想让编辑器占满剩余空间时，用 `.flex_item(FlexItem::fill())` 显式表达。
        let default_h = self.line_h * self.rows as f32 + padding_v;
        let want_fill = matches!(self.item.height, Dimension::Flex(_) | Dimension::Percent(_));
        let content_h = if want_fill && constraints.max.h.is_finite() {
            constraints.max.h
        } else {
            default_h
        };
        constraints.constrain(Size::new(
            content_w.max(padding_h + 1.0),
            content_h.max(self.line_h + padding_v),
        ))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        metrics: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        self.ui_scale = metrics.scale();
        let pad = self.padding_scaled(metrics.scale());
        self.inner_w = (rect.w - pad.horizontal()).max(0.0);
        self.inner_h = (rect.h - pad.vertical()).max(0.0);
        self.relayout(metrics);
        self.needs_relayout = false;
        self.ensure_caret_visible();
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if self.disabled {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::Focus(on) => {
                self.focused = *on;
                self.caret_visible = *on;
                self.last_blink = Instant::now();
                EventResult::Redraw
            }

            UiEvent::MouseDown { pos, button } => {
                if !matches!(button, y901_ui::MouseButton::Left) {
                    return EventResult::Ignored;
                }
                if !self.rect.contains(*pos) {
                    return EventResult::Ignored;
                }
                // 只**请求**焦点：是否真的给由框架（Modal 作用域）决定。
                ctx.request_focus(self.id.clone());
                ctx.set_pointer_capture(self.id.clone());
                self.dragging = true;
                let byte = self.byte_at_screen(*pos);
                self.buffer.set_cursor(byte);
                self.goal_x = None;
                EventResult::Redraw
            }

            UiEvent::MouseMove { pos } => {
                if !self.dragging {
                    return EventResult::Ignored;
                }
                let byte = self.byte_at_screen(*pos);
                let anchor = self.buffer.anchor();
                self.buffer.set_selection(anchor, byte);
                self.caret_visible = true;
                EventResult::Redraw
            }

            UiEvent::MouseUp { .. } => {
                if self.dragging {
                    self.dragging = false;
                    ctx.release_pointer_capture();
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }

            UiEvent::Text(text) => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                // 提交即消费预编辑串（否则候选串会叠在正文上重复显示）。
                self.preedit.clear();
                self.buffer.insert_str(text);
                self.touch();
                EventResult::Redraw
            }

            UiEvent::Key { key, modifiers } => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                self.handle_key(*key, *modifiers)
            }

            UiEvent::Compose(text) => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                self.preedit = text.clone();
                // 把 IME 候选窗钉在光标处（多行必须用光标所在行的 y）
                let index = self.line_index_of_byte(self.buffer.cursor());
                let x = self.rect.x
                    + self.padding_scaled(self.ui_scale).left
                    + self.x_in_line(&self.lines[index], self.buffer.cursor());
                let y = self.rect.y + self.padding_scaled(self.ui_scale).top + self.line_top(index);
                ctx.set_ime_candidate(Pos::new(x, y + self.line_h));
                EventResult::Redraw
            }

            UiEvent::Wheel { delta, .. } => {
                // 多行编辑器必须能滚：内容超过可视高度时，其它可滚动控件
                // （ScrollView / ListView / Table / TreeView）都有 Wheel 分支，
                // 这里缺了就只能靠移动光标来"滚"，鼠标滚轮会一路传给祖先。
                let max_scroll = (self.lines.len() as f32 * self.line_h - self.inner_h).max(0.0);
                if max_scroll <= 0.0 {
                    return EventResult::Ignored;
                }
                let before = self.scroll_y;
                // `delta` 为正 = 向上滚（与 `UiEvent::Wheel` 的约定一致）→ scroll_y 减小。
                self.scroll_y = (self.scroll_y - *delta).clamp(0.0, max_scroll);
                if (self.scroll_y - before).abs() > 0.01 {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }

            UiEvent::Tick => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                if self.last_blink.elapsed() >= BLINK {
                    self.caret_visible = !self.caret_visible;
                    self.last_blink = Instant::now();
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }

            UiEvent::Action { name, args } => match name.as_str() {
                "set_text" => {
                    let text = args
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.buffer = TextBuffer::from_text(text);
                    self.touch();
                    EventResult::Redraw
                }
                "focus" => {
                    ctx.request_focus(self.id.clone());
                    EventResult::Redraw
                }
                "select_all" => {
                    self.buffer.select_all();
                    EventResult::Redraw
                }
                "insert_text" => {
                    let text = args
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.buffer.insert_str(&text);
                    self.touch();
                    EventResult::Redraw
                }
                _ => EventResult::Ignored,
            },

            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let rect = self.rect;
        let pad = self.padding_scaled(self.ui_scale);
        let inner = Rect::new(
            rect.x + pad.left,
            rect.y + pad.top,
            (rect.w - pad.horizontal()).max(0.0),
            (rect.h - pad.vertical()).max(0.0),
        );
        if inner.is_empty() {
            return;
        }
        // 尺寸可能在 `arrange` 之后才变（例如父级二次分配），这里兜一次。
        self.inner_w = inner.w;
        self.inner_h = inner.h;

        // **软换行必须跟着文本走**：编辑只返回 `Redraw`（不触发布局），
        // 而 `RenderCtx` 恰好实现了 `TextMetrics`，所以重排放在这里。
        if self.needs_relayout || (self.inner_w - self.inner_w_last).abs() > 0.01 {
            self.inner_w_last = self.inner_w;
            self.relayout(ctx);
            self.needs_relayout = false;
        }

        // 1) 背景（主题取色，disabled / focused 各有状态）
        let state = if self.disabled {
            State::Disabled
        } else if self.focused {
            State::Focused
        } else {
            State::Normal
        };
        let themed = ctx.theme().style(Role::TEXT_INPUT, state);
        ctx.fill_rect(rect, themed.bg);
        if self.focused {
            ctx.stroke_round_rect(rect, themed.fg, 1.0, 0.0);
        }

        // 2) 光标跟随滚动（放在渲染期：编辑只返回 Redraw，不会再 arrange）
        if self.focused {
            self.ensure_caret_visible();
        }

        ctx.push_clip(Rect::new(inner.x, inner.y - 4.0, inner.w, inner.h + 8.0));
        let line_h = self.line_h.max(1.0);
        let text = self.buffer.text().to_string();

        // 3) 逐可视行画正文（只画可见范围：长文档也不会遍历全部行去画）
        let first = ((self.scroll_y / line_h).floor().max(0.0) as usize).min(self.lines.len());
        let visible = (inner.h / line_h).ceil() as usize + 2;
        for i in first..(first + visible).min(self.lines.len()) {
            let line = self.lines[i];
            let y = inner.y + self.line_top(i);
            if y + line_h < inner.y || y > inner.bottom() {
                continue;
            }
            if line.end > line.start {
                let mut style = self.style;
                style.bg = Color::TRANSPARENT;
                style.underline = false;
                ctx.draw_text(Pos::new(inner.x, y), &text[line.start..line.end], &style);
            }
        }

        // 4) 选区高亮（画在文字上面、透明叠加，避免遮住墨迹）
        if let Some((sel_start, sel_end)) = self.buffer.selection() {
            for i in first..(first + visible).min(self.lines.len()) {
                let line = self.lines[i];
                let s = sel_start.max(line.start);
                let e = sel_end.min(line.end);
                let y = inner.y + self.line_top(i);
                if y + line_h < inner.y || y > inner.bottom() {
                    continue;
                }
                if s < e {
                    let x0 = inner.x + self.x_in_line(&line, s);
                    let x1 = inner.x + self.x_in_line(&line, e);
                    ctx.fill_rect(
                        Rect::new(x0, y, (x1 - x0).max(1.0), line_h),
                        self.selection_color,
                    );
                }
                // 选到行尾的换行符：把这一行剩余宽度也高亮，否则看不出"换行被选中"
                if sel_end > line.end && sel_start <= line.end {
                    let x0 = inner.x + self.x_in_line(&line, line.end);
                    if x0 < inner.right() {
                        ctx.fill_rect(
                            Rect::new(x0, y, inner.right() - x0, line_h),
                            self.selection_color,
                        );
                    }
                }
            }
        }

        // 5) IME 预编辑：画在光标处，带下划线
        let caret_index = self.line_index_of_byte(self.buffer.cursor());
        let caret_y = inner.y + self.line_top(caret_index);
        let mut caret_x = inner.x + self.x_in_line(&self.lines[caret_index], self.buffer.cursor());
        if !self.preedit.is_empty() {
            let preedit_style = TextStyle::fg(self.style.fg);
            let mut pen = caret_x;
            for ch in self.preedit.chars() {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                let w = ctx.measure_text(s, &preedit_style).w;
                if pen + w >= inner.x && pen <= inner.right() {
                    ctx.draw_text(Pos::new(pen, caret_y), s, &preedit_style);
                    ctx.fill_rect(
                        Rect::new(pen, caret_y + line_h - 2.0, w, 1.5),
                        Color::rgba(0.55, 0.72, 1.0, 1.0),
                    );
                }
                pen += w;
            }
            caret_x = pen;
        }

        // 6) 光标
        if self.focused && self.caret_visible && caret_x <= inner.right() {
            ctx.fill_rect(
                Rect::new(caret_x, caret_y, CARET_W, line_h),
                self.caret_color,
            );
        }

        // 7) 占位提示（完全空时）
        if text.is_empty() && self.preedit.is_empty() {
            if let Some(hint) = self.placeholder.as_deref() {
                if !hint.is_empty() {
                    ctx.draw_text(
                        Pos::new(inner.x, inner.y),
                        hint,
                        &TextStyle::fg(Color::rgba(0.55, 0.58, 0.64, 1.0)),
                    );
                }
            }
        }

        ctx.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        let text = self.buffer.text().to_string();
        let label = if text.is_empty() {
            "TextArea".to_string()
        } else {
            text.clone()
        };
        Semantics::default()
            .exposed(true)
            .label(label)
            .value(text.clone())
            .action_spec("set_text", y901_ui::InputKind::Text)
            .action("focus")
            .prop("multiline", "true")
            .prop("cursor", self.buffer.cursor().to_string())
            .prop("anchor", self.buffer.anchor().to_string())
            .prop("len", self.buffer.len_bytes().to_string())
            .prop("lines", self.lines.len().to_string())
            .prop("focused", self.focused.to_string())
            .state("text", text)
            .state("multiline", true)
            .state("caret", self.buffer.cursor())
            .state("anchor", self.buffer.anchor())
            .disabled(self.disabled)
    }
}

/// 是否为**不占格**的组合记号（近似判定：覆盖主要组合区段）。
///
/// 为什么用区段而不是 Unicode 类别表：框架不引入 `unicode-properties` 这类依赖，
/// 而这些区段已经涵盖实战中会遇到的组合记号（拉丁附加、希腊、希伯来点、阿拉伯元音、
/// 天城文/泰文附标、通用组合符号）。误判的代价是"少算一格"，比"多算一格"轻。
fn is_zero_width_mark(ch: char) -> bool {
    matches!(ch as u32,
        0x0300..=0x036F   // Combining Diacritical Marks
        | 0x0483..=0x0489 // Cyrillic 组合
        | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4 | 0x06E7..=0x06E8 | 0x06EA..=0x06ED
        | 0x0711 | 0x0730..=0x074A
        | 0x07A6..=0x07B0 | 0x07EB..=0x07F3
        | 0x0816..=0x0819 | 0x081B..=0x0823 | 0x0825..=0x0827 | 0x0829..=0x082D
        | 0x0859..=0x085B | 0x08D3..=0x08E1 | 0x08E3..=0x0903
        | 0x093A..=0x093C | 0x093E..=0x094F | 0x0951..=0x0957 | 0x0962..=0x0963
        | 0x0981..=0x0983 | 0x09BC | 0x09BE..=0x09C4 | 0x09C7..=0x09C8
        | 0x09CB..=0x09CD | 0x09D7 | 0x09E2..=0x09E3
        | 0x0A01..=0x0A03 | 0x0A3C | 0x0A3E..=0x0A42 | 0x0A47..=0x0A48
        | 0x0A4B..=0x0A4D | 0x0A51 | 0x0A70..=0x0A71 | 0x0A75
        | 0x0A81..=0x0A83 | 0x0ABC | 0x0ABE..=0x0AC5 | 0x0AC7..=0x0AC9
        | 0x0ACB..=0x0ACD | 0x0AE2..=0x0AE3
        | 0x0B01..=0x0B03 | 0x0B3C | 0x0B3E..=0x0B44 | 0x0B47..=0x0B48
        | 0x0B4B..=0x0B4D | 0x0B56..=0x0B57 | 0x0B62..=0x0B63
        | 0x0BBE..=0x0BC2 | 0x0BC6..=0x0BC8 | 0x0BCA..=0x0BCD | 0x0BD7
        | 0x0C00..=0x0C04 | 0x0C3E..=0x0C44 | 0x0C46..=0x0C48 | 0x0C4A..=0x0C4D
        | 0x0C55..=0x0C56 | 0x0C62..=0x0C63
        | 0x0D00..=0x0D03 | 0x0D3B..=0x0D3C | 0x0D3E..=0x0D44 | 0x0D46..=0x0D48
        | 0x0D4A..=0x0D4D | 0x0D57 | 0x0D62..=0x0D63
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x0EB1 | 0x0EB4..=0x0EBC | 0x0EC8..=0x0ECD
        | 0x0F18..=0x0F19 | 0x0F35 | 0x0F37 | 0x0F39 | 0x0F71..=0x0F7E
        | 0x0F80..=0x0F84 | 0x0F86..=0x0F87
        | 0x102B..=0x103E | 0x1056..=0x1059
        | 0x135D..=0x135F
        | 0x1712..=0x1714 | 0x1732..=0x1734 | 0x1752..=0x1753
        | 0x1772..=0x1773 | 0x17B4..=0x17D3
        | 0x180B..=0x180D | 0x1885..=0x1886
        | 0x1A17..=0x1A1B | 0x1AB0..=0x1AFF
        | 0x1B00..=0x1B04 | 0x1B34..=0x1B44
        | 0x1DC0..=0x1DFF // Combining Diacritical Marks Supplement
        | 0x20D0..=0x20F0 // Combining Diacritical Marks for Symbols
        | 0x2CEF..=0x2CF1 | 0x2D7F | 0x2DE0..=0x2DFF
        | 0xA66F..=0xA672 | 0xA674..=0xA67D | 0xA69E..=0xA69F
        | 0xA6F0..=0xA6F1 | 0xA802 | 0xA806 | 0xA80B | 0xA823..=0xA827
        | 0xFE00..=0xFE0F // Variation Selectors
        | 0xFE20..=0xFE2F // Combining Half Marks
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use y901_ui::MouseButton;

    /// 等宽度量：ASCII 8px、CJK 16px、行高 16px（与 TextInput 的测试口径一致）。
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

    fn mount(widget: &mut TextArea, w: f32, h: f32) {
        let mut cache = LayoutCache::new();
        widget.measure(&Mono, Constraints::loose(Size::new(w, h)), &mut cache);
        widget.arrange(Rect::new(0.0, 0.0, w, h), &Mono, &mut cache);
    }

    fn ctx() -> EventCtx {
        EventCtx::new(None, Size::new(240.0, 120.0))
    }

    fn key(area: &mut TextArea, k: Key) -> EventResult {
        let mut c = ctx();
        area.event(
            &mut c,
            &UiEvent::Key {
                key: k,
                modifiers: Modifiers::default(),
            },
        )
    }

    fn shift_key(area: &mut TextArea, k: Key) -> EventResult {
        let mut c = ctx();
        area.event(
            &mut c,
            &UiEvent::Key {
                key: k,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
        )
    }

    fn focus(area: &mut TextArea) {
        let mut c = ctx();
        area.event(&mut c, &UiEvent::Focus(true));
    }

    #[test]
    fn empty_area_has_one_visual_line() {
        let mut area = TextArea::new("a");
        mount(&mut area, 240.0, 100.0);
        assert_eq!(area.visual_line_count(), 1);
    }

    #[test]
    fn long_line_soft_wraps_by_width() {
        let mut area = TextArea::new("a").with_text("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"); // 32 chars * 8px
                                                                                         // inner_w = 240 - 16 = 224 → 224/8 = 28 字符/行 → 32 字符要 2 行
        mount(&mut area, 240.0, 100.0);
        assert_eq!(
            area.visual_line_count(),
            2,
            "32 个 8px 字符在 224px 内应换行"
        );
    }

    #[test]
    fn newline_creates_visual_line() {
        let mut area = TextArea::new("a").with_text("ab\ncd");
        mount(&mut area, 240.0, 100.0);
        assert_eq!(area.visual_line_count(), 2);
    }

    #[test]
    fn empty_lines_are_clickable() {
        let mut area = TextArea::new("a").with_text("a\n\nb");
        mount(&mut area, 240.0, 100.0);
        assert_eq!(area.visual_line_count(), 3, "空行也必须占一条可视行");
    }

    #[test]
    fn down_moves_to_next_visual_line_and_keeps_goal_column() {
        let mut area = TextArea::new("a").with_text("abcdefghij\nklmnopqrst");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        // 光标放到第一行第 5 个字符
        area.buffer.set_cursor(5);
        key(&mut area, Key::Down);
        assert_eq!(area.cursor(), 5 + 11, "应落在第二行同一列（跨过换行符）");
        // 最后一行再按 ↓：退化到全文末尾（主流编辑器行为）
        key(&mut area, Key::Down);
        assert_eq!(area.cursor(), area.text().len());
    }

    #[test]
    fn up_at_first_line_stays() {
        let mut area = TextArea::new("a").with_text("abc\ndef");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        area.buffer.set_cursor(2);
        key(&mut area, Key::Up);
        assert_eq!(area.cursor(), 0, "第一行按 ↑ 应回到全文开头而不是原地");
    }

    #[test]
    fn enter_inserts_newline() {
        let mut area = TextArea::new("a").with_text("ab");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        key(&mut area, Key::Enter);
        assert_eq!(area.text(), "ab\n");
        // 编辑阶段没有度量，重排发生在 render；测试里直接调内部方法验证同一逻辑。
        area.relayout(&Mono);
        assert_eq!(area.visual_line_count(), 2, "换行后应变成两条可视行");
    }

    #[test]
    fn backspace_at_line_start_merges_lines() {
        let mut area = TextArea::new("a").with_text("ab\ncd");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        area.buffer.set_cursor(3); // 第二行行首
        key(&mut area, Key::Backspace);
        assert_eq!(area.text(), "abcd");
        assert_eq!(area.cursor(), 2);
    }

    #[test]
    fn selection_across_lines_reports_range() {
        let mut area = TextArea::new("a").with_text("abc\ndef");
        mount(&mut area, 240.0, 100.0);
        area.buffer.set_selection(1, 5); // "bc\nd"
        assert_eq!(area.selection(), Some((1, 5)));
        assert_eq!(area.buffer.selected_text(), Some("bc\nd"));
    }

    #[test]
    fn typing_appends_and_bumps_revision() {
        let mut area = TextArea::new("a");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        let before = area.revision();
        let mut c = ctx();
        area.event(&mut c, &UiEvent::Text("你好".into()));
        assert_eq!(area.text(), "你好");
        assert!(area.revision() > before, "编辑必须 bump revision");
    }

    #[test]
    fn preedit_does_not_enter_buffer() {
        let mut area = TextArea::new("a");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        let mut c = ctx();
        area.event(&mut c, &UiEvent::Compose("nihao".into()));
        assert_eq!(area.text(), "", "预编辑串不能写进缓冲区");
        assert_eq!(area.preedit(), "nihao");
        area.event(&mut c, &UiEvent::Text("你好".into()));
        assert_eq!(area.text(), "你好");
        assert!(area.preedit().is_empty(), "提交后必须清空预编辑串");
    }

    #[test]
    fn copy_and_paste_use_injected_clipboard() {
        let mut area = TextArea::new("a")
            .with_clipboard(Box::new(crate::text_input::MemoryClipboard::default()));
        area.set_text("hello");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        area.buffer.set_selection(1, 4); // "ell"
        let mut c = ctx();
        // Ctrl+C
        area.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Char('c'),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
        );
        area.buffer.set_cursor(5);
        // Ctrl+V
        area.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Char('v'),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
        );
        assert_eq!(area.text(), "helloell");
    }

    #[test]
    fn disabled_area_ignores_events() {
        let mut area = TextArea::new("a").disabled(true);
        mount(&mut area, 240.0, 100.0);
        let mut c = ctx();
        let r = area.event(&mut c, &UiEvent::Text("x".into()));
        assert!(matches!(r, EventResult::Ignored));
        assert_eq!(area.text(), "");
    }

    #[test]
    fn click_places_caret_on_clicked_line() {
        let mut area = TextArea::new("a").with_text("abc\ndef");
        mount(&mut area, 240.0, 100.0);
        // 第二行行首：x 取 inner.x（= padding.left 8），y 取第二行中部
        let mut c = ctx();
        area.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 4.0 + 16.0 + 8.0),
                button: MouseButton::Left,
            },
        );
        assert_eq!(area.cursor(), 4, "点第二行行首应落在 'd' 之前");
    }

    #[test]
    fn caret_scrolls_into_view_when_moving_down() {
        let mut area = TextArea::new("a").with_text("1\n2\n3\n4\n5\n6\n7\n8\n9\n10");
        // 高度只够 2.5 行：inner_h = 48 - 8 = 40
        mount(&mut area, 240.0, 48.0);
        focus(&mut area);
        // 回到开头：此时不该有滚动
        area.buffer.set_cursor(0);
        area.ensure_caret_visible();
        assert_eq!(area.scroll_y(), 0.0);
        // 把光标放到末尾：必须滚到能看见最后一行
        area.buffer.set_cursor(area.text().len());
        area.ensure_caret_visible();
        assert!(
            area.scroll_y() > 0.0,
            "光标在最后一行时必须滚动，scroll_y={}",
            area.scroll_y()
        );
    }

    /// **回归（P0）**：编辑让文本变短后，同一帧内的坐标命中不能越界 panic。
    ///
    /// `lines` 只在 `relayout`（arrange/render）里重建，而编辑只回 `Redraw`；
    /// 一帧里"先删字、再点击"时行表还是旧的，切片 `text[start..end]` 会越界。
    /// **回归**：组合记号不占格，光标 x 不应把它算成一整格。
    ///
    /// 排版用的是 `TextBuffer::char_width`（组合记号 = 0 格），旧 `char_w`
    /// 却给它一整格 —— NFD/天城文里点击与光标会逐字累计偏移。
    #[test]
    fn combining_marks_do_not_advance_the_caret() {
        let mut area = TextArea::new("a").with_text("e\u{0301}x");
        mount(&mut area, 240.0, 100.0);
        // 光标放在 "e" 之后（字节 1），此时 x 应为一格宽（"e" 本身）
        area.buffer.set_cursor(1);
        let at_e = area.x_in_line(&area.lines[0], 1);
        assert!((at_e - area.cell_w).abs() < 0.01, "e 之后应为一格：{at_e}");
        // 越过后面的组合记号（字节 3，即 "e"+U+0301 之后）仍是同一格
        let after_mark = area.x_in_line(&area.lines[0], 3);
        assert!(
            (after_mark - area.cell_w).abs() < 0.01,
            "组合记号不应推进光标：{after_mark}"
        );
        // 到 "x" 之后才是两格
        let after_x = area.x_in_line(&area.lines[0], 4);
        assert!(
            (after_x - 2.0 * area.cell_w).abs() < 0.01,
            "x 之后应为两格：{after_x}"
        );
    }

    #[test]
    fn click_right_after_shrinking_edit_does_not_panic() {
        let mut area = TextArea::new("a").with_text("abc");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        key(&mut area, Key::Backspace); // 文本变 "ab"，行表仍是 (0,3)
        assert_eq!(area.text(), "ab");

        let mut c = ctx();
        let r = area.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(20.0, 8.0),
                button: MouseButton::Left,
            },
        );
        assert!(matches!(r, EventResult::Redraw | EventResult::Handled));
        assert!(area.cursor() <= area.text().len(), "光标必须落在文本范围内");
    }

    /// **回归**：文本大幅变短后 `scroll_y` 必须跟着收回去。
    ///
    /// 否则第一条可视行被推到窗口外，`render` 画 0 行 —— 看起来像 set_text 没生效。
    #[test]
    fn shrinking_text_resets_scroll() {
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("line{i}\n"));
        }
        let mut area = TextArea::new("a").with_text(text);
        mount(&mut area, 240.0, 48.0); // 只够 2.5 行
        focus(&mut area);
        area.buffer.set_cursor(area.text().len());
        area.ensure_caret_visible();
        assert!(area.scroll_y() > 0.0, "前置：应先滚动起来");

        area.set_text("hello");
        area.relayout(&Mono);
        assert_eq!(area.scroll_y(), 0.0, "内容只剩一行时必须回到顶部");
    }

    /// **回归**：鼠标滚轮应能滚动多行编辑器（其它可滚动控件都有 Wheel 分支）。
    /// **回归**：鼠标滚轮应能滚动多行编辑器（其它可滚动控件都有 Wheel 分支）。
    ///
    /// 注意前置状态：`with_text` 把光标放在**末尾**，`arrange` 里的
    /// `ensure_caret_visible` 会把视图滚到底 —— 所以先向上滚、再向下滚，
    /// 两个方向都要能动（旧实现完全没有 Wheel 分支，两者都不动）。
    #[test]
    fn wheel_scrolls_the_editor() {
        let mut text = String::new();
        for i in 0..40 {
            text.push_str(&format!("line{i}\n"));
        }
        let mut area = TextArea::new("a").with_text(text);
        mount(&mut area, 240.0, 48.0); // 只够 2.5 行
        let at_bottom = area.scroll_y();
        assert!(at_bottom > 0.0, "前置：光标在末尾应已滚到底");

        let mut c = ctx();
        // delta 为正 = 向上滚
        area.event(
            &mut c,
            &UiEvent::Wheel {
                delta: 120.0,
                pos: Pos::new(10.0, 10.0),
                mods: Modifiers::default(),
            },
        );
        let after_up = area.scroll_y();
        assert!(
            after_up < at_bottom,
            "向上滚轮应减小 scroll_y（旧实现没有 Wheel 分支，纹丝不动）：{at_bottom} -> {after_up}"
        );

        // 再向下滚回去
        area.event(
            &mut c,
            &UiEvent::Wheel {
                delta: -120.0,
                pos: Pos::new(10.0, 10.0),
                mods: Modifiers::default(),
            },
        );
        assert!(
            area.scroll_y() > after_up,
            "向下滚轮应增大 scroll_y：{} -> {}",
            after_up,
            area.scroll_y()
        );
    }
    #[test]
    fn shift_down_extends_selection() {
        let mut area = TextArea::new("a").with_text("ab\ncd");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        area.buffer.set_cursor(0);
        shift_key(&mut area, Key::Down);
        let sel = area.selection().expect("应形成选区");
        assert_eq!(sel.0, 0);
        assert!(sel.1 > 0);
    }

    /// 记录绘制指令的后端（同时提供度量）。
    #[derive(Default)]
    struct Recorder {
        fills: Vec<Rect>,
        texts: Vec<String>,
    }

    impl TextMetrics for Recorder {
        fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
            Mono.measure_text(t, s)
        }
        fn line_height(&self, s: &TextStyle) -> f32 {
            Mono.line_height(s)
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    impl RenderCtx for Recorder {
        fn size(&self) -> Size {
            Size::new(400.0, 200.0)
        }
        fn fill_rect(&mut self, r: Rect, _c: Color) {
            self.fills.push(r);
        }
        fn stroke_rect(&mut self, r: Rect, _c: Color, _w: f32) {
            self.fills.push(r);
        }
        fn draw_text(&mut self, _o: Pos, t: &str, _s: &TextStyle) -> f32 {
            self.texts.push(t.to_string());
            t.chars().count() as f32 * 8.0
        }
        fn push_clip(&mut self, _r: Rect) {}
        fn pop_clip(&mut self) {}
    }

    /// **渲染路径必须自己重排**：编辑只返回 `Redraw`，不触发布局，
    /// 所以"换行后立即渲染"必须看到两行（否则界面会停在旧排版上）。
    #[test]
    fn render_relayouts_after_edit() {
        let mut area = TextArea::new("a");
        mount(&mut area, 240.0, 100.0);
        focus(&mut area);
        let mut c = ctx();
        area.event(&mut c, &UiEvent::Text("ab".into()));
        area.event(&mut c, &UiEvent::Text("\n".into()));
        area.event(&mut c, &UiEvent::Text("cd".into()));

        let mut rec = Recorder::default();
        area.render(&mut rec);
        assert!(
            rec.texts.iter().any(|t| t == "ab"),
            "第一行应被画出：{:?}",
            rec.texts
        );
        assert!(
            rec.texts.iter().any(|t| t == "cd"),
            "第二行应被画出（重排生效）：{:?}",
            rec.texts
        );
        assert_eq!(area.visual_line_count(), 2);
    }

    /// 画出来的行数受可视高度约束：长文档只画可见行（虚拟化）。
    #[test]
    fn render_only_draws_visible_lines() {
        let mut text = String::new();
        for i in 0..200 {
            text.push_str(&format!("line{i}\n"));
        }
        let mut area = TextArea::new("a").with_text(text);
        mount(&mut area, 240.0, 48.0); // 只够 2.5 行
        let mut rec = Recorder::default();
        area.render(&mut rec);
        assert!(
            rec.texts.len() < 20,
            "200 行文本只应画可见的几行，实际画了 {} 行",
            rec.texts.len()
        );
    }
}
