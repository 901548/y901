//! 终端组件。
//!
//! 它不知道 D3D11，也不知道 Win32 —— 只实现 `Widget`，
//! 通过 `RenderCtx` 出图，通过 `TextMetrics` 量字。
//! PTY 是它自己的实现细节，框架完全看不见。

use std::io::{Read, Write};

use crossbeam_channel::{unbounded, Receiver};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use y901_core::{CellKind, ColorSpec, Grid};
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, FlexItem, Key, Pos, Rect, RenderCtx, Role,
    Semantics, Size, TextMetrics, TextStyle, UiEvent, Widget,
};

use crate::palette::{self, DEFAULT_BG, DEFAULT_FG};
use crate::vt::{feed_bytes, VtFilter};

/// 终端需要的剪贴板能力就这么两个方法。
///
/// 为什么在 `y901-extras` 里另定义一个而不用 `y901-shell::clipboard::Clipboard`：
/// 依赖方向是 `shell → extras`，反过来会成环。宿主只需写一个 6 行适配器。
pub trait TextClipboard {
    /// 取文本（`None` = 空或不可用）。
    fn get(&self) -> Option<String>;
    /// 写文本。
    fn set(&self, text: &str);
}

/// 终端组件：Grid + PTY + VT 解析。
pub struct TerminalWidget {
    id: String,
    grid: Grid,
    text: VtFilter,
    pending_utf8: Vec<u8>,
    pty: Option<PtySession>,
    rect: Rect,
    /// 单格尺寸（逻辑像素），由字体度量得出。
    cell: Size,
    item: FlexItem,
    focused: bool,
    /// IME 预编辑串：只画在覆盖层，绝不进 Grid。
    preedit: String,
    /// 只读演示模式：不 spawn shell，用于测试/注入。
    read_only: bool,
    /// 语义版本：每收到一次 PTY 输出 +1（供增量推送感知变化）。
    revision: u64,
    /// bracketed paste 探测器（shell 通过 `ESC[?2004h` 开启）
    paste_detect: crate::paste::BracketedPasteDetector,
    /// 系统剪贴板（宿主注入）
    clipboard: Option<Box<dyn TextClipboard>>,
    /// 当前选区（`None` = 没选中）
    selection: Option<crate::selection::Selection>,
}

struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    rx: Receiver<Vec<u8>>,
    _child: Box<dyn Child + Send + Sync>,
}

impl TerminalWidget {
    /// 建一个不启动 shell 的终端（测试、或后面手动 `attach_shell`）。
    pub fn new(id: impl Into<String>, cols: u16, rows: u16) -> Self {
        Self {
            id: id.into(),
            grid: Grid::new(cols.max(1), rows.max(1)),
            text: VtFilter::new(),
            pending_utf8: Vec::new(),
            pty: None,
            paste_detect: crate::paste::BracketedPasteDetector::default(),
            clipboard: None,
            selection: None,
            rect: Rect::ZERO,
            cell: Size::new(8.0, 16.0),
            item: FlexItem::fill(),
            focused: false,
            preedit: String::new(),
            read_only: false,
            revision: 0,
        }
    }

    /// 只读模式：键盘输入不再写入 PTY（仍可滚动与选择）。
    ///
    /// 返回：自身。副作用：无。用于"查看日志"场景。
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// 设置 flex 伸缩参数（终端通常用 `FlexItem::fill()` 占满）。
    ///
    /// 返回：自身。副作用：无。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 终端网格（只读）：行、单元格、光标、scrollback 都在这里。
    ///
    /// 返回：`&Grid`。副作用：无。
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// 终端网格（可变）：直接改它会绕过 PTY 与 reflow。
    ///
    /// 返回：`&mut Grid`。副作用：**不会**自动标记布局失效或重绘；
    /// 只给测试与"注入历史输出"的场景用，产品路径请走 PTY。
    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    /// 注入系统剪贴板（由 app/shell 层提供适配器）。
    ///
    /// 为什么用一个 trait 而不是直接依赖平台实现：`y901-extras` 不能往上依赖
    /// `y901-shell`，否则依赖方向就倒了。
    pub fn set_clipboard(&mut self, cb: Box<dyn TextClipboard>) {
        self.clipboard = Some(cb);
    }

    /// 屏幕坐标 → 网格单元格（`None` = 不能映射，如布局前）。
    ///
    /// 纯函数在 `selection::pos_to_cell`（已单测覆盖 rect=ZERO / 边界 / 负坐标）。
    fn cell_at(&self, pos: y901_ui::Pos) -> Option<(usize, usize)> {
        crate::selection::pos_to_cell(
            (pos.x, pos.y),
            (self.rect.x, self.rect.y, self.rect.w, self.rect.h),
            self.cell.w,
            self.cell.h,
            self.grid.cols as usize,
            self.grid.rows as usize,
        )
    }

    /// **仅测试用**：直接设定 rect 与单格尺寸，绕开 `arrange`。
    #[doc(hidden)]
    pub fn set_layout_for_test(&mut self, rect: Rect, cell: Size) {
        self.rect = rect;
        self.cell = cell;
    }

    /// 当前选区的活动端坐标（测试与诊断用）。
    #[doc(hidden)]
    pub fn selected_cell(&self) -> Option<(usize, usize)> {
        self.selection.map(|s| s.end)
    }

    /// 在屏幕坐标处开始一个选区（鼠标按下路径）。
    ///
    /// 返回：`false` = 坐标无法映射（如 `arrange` 之前），此时**不建选区也不 panic**。
    pub fn begin_selection(&mut self, pos: y901_ui::Pos) -> bool {
        let Some((r, c)) = self.cell_at(pos) else {
            return false;
        };
        self.selection = Some(crate::selection::Selection::new(r, c));
        true
    }

    /// 把选区的活动端拖到屏幕坐标处。
    ///
    /// 返回：`false` = 当前没选区或坐标无法映射。
    pub fn drag_selection_to(&mut self, pos: y901_ui::Pos) -> bool {
        if self.selection.is_none() {
            return false;
        }
        let Some((r, c)) = self.cell_at(pos) else {
            return false;
        };
        if let Some(sel) = self.selection.as_mut() {
            sel.drag_to(r, c);
        }
        true
    }

    /// 把当前选区取成文本（`None` = 没选中或选个空）。
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection?;
        if sel.is_empty() {
            return None;
        }
        let text = sel.text(&self.grid, self.grid.cols as usize);
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// 清除选区。
    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    /// 把一段文本当作"粘贴"写进 PTY（过滤控制字符 + bracketed 包裹）。
    ///
    /// 返回：`true` = 已写入。副作用：写 PTY、重置滚动。
    pub fn paste(&mut self, text: &str) -> bool {
        if self.read_only {
            return false;
        }
        self.selection = None;
        let bytes = crate::paste::wrap(text, self.paste_detect.enabled());
        if bytes.is_empty() {
            return false;
        }
        self.grid.reset_scroll_on_input();
        self.write(&bytes);
        true
    }

    /// 启动 shell 并把 PTY 接上。
    pub fn attach_shell(&mut self) -> Result<(), String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: self.grid.rows,
                cols: self.grid.cols,
                pixel_width: (self.grid.cols as f32 * self.cell.w) as u16,
                pixel_height: (self.grid.rows as f32 * self.cell.h) as u16,
            })
            .map_err(|e| e.to_string())?;

        let mut cmd = if cfg!(windows) {
            let mut c = CommandBuilder::new("powershell.exe");
            c.arg("-NoLogo");
            c
        } else {
            CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "bash".into()))
        };
        cmd.env("TERM", "xterm-256color");
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let (tx, rx) = unbounded::<Vec<u8>>();

        std::thread::Builder::new()
            .name("y901-pty".to_string())
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .ok();

        self.pty = Some(PtySession {
            master: pair.master,
            writer,
            rx,
            _child: child,
        });
        Ok(())
    }

    /// 是否已接上真实 shell（`attach_shell` 成功过）。
    ///
    /// 返回：`true` = 输入会写进 PTY；`false` = 纯展示（测试/日志视图）。
    /// 副作用：无。
    pub fn has_shell(&self) -> bool {
        self.pty.is_some()
    }

    /// 把已经到达的 PTY 数据吃进 Grid。返回是否真的读到了东西。
    pub fn pump(&mut self) -> bool {
        let mut any = false;
        if let Some(pty) = self.pty.as_ref() {
            while let Ok(bytes) = pty.rx.try_recv() {
                // 顺路追踪 bracketed paste 开关（shell 会发 `ESC[?2004h`）
                self.paste_detect.feed(&bytes);
                feed_bytes(
                    &mut self.text,
                    &mut self.pending_utf8,
                    &mut self.grid,
                    &bytes,
                );
                any = true;
            }
        }
        if any {
            // 任何输出都算一次语义变化（哪怕屏幕最后看起来一样）
            self.revision = self.revision.wrapping_add(1);
        }
        any
    }

    /// 写入 PTY（键盘/注入都走这里）。
    pub fn write(&mut self, bytes: &[u8]) {
        if let Some(pty) = self.pty.as_mut() {
            let _ = pty.writer.write_all(bytes);
            let _ = pty.writer.flush();
        }
    }

    fn sync_pty_size(&mut self) {
        if let Some(pty) = self.pty.as_mut() {
            let _ = pty.master.resize(PtySize {
                rows: self.grid.rows,
                cols: self.grid.cols,
                pixel_width: (self.grid.cols as f32 * self.cell.w) as u16,
                pixel_height: (self.grid.rows as f32 * self.cell.h) as u16,
            });
        }
    }

    /// 按 rect 与字体度量重算 cols/rows。
    fn retile(&mut self, rect: Rect, metrics: &dyn TextMetrics) {
        let cell_w = metrics
            .measure_text("M", &TextStyle::fg(Color::WHITE))
            .w
            .max(1.0);
        let cell_h = metrics.line_height(&TextStyle::fg(Color::WHITE)).max(1.0);
        self.cell = Size::new(cell_w, cell_h);

        let cols = ((rect.w / cell_w).floor() as i64).clamp(1, 1000) as u16;
        let rows = ((rect.h / cell_h).floor() as i64).clamp(1, 500) as u16;

        if cols != self.grid.cols {
            self.grid.reflow(cols);
        }
        if rows != self.grid.rows {
            self.grid.set_rows(rows);
        }
        self.sync_pty_size();
    }

    fn key_bytes(key: Key) -> Option<&'static [u8]> {
        Some(match key {
            Key::Enter => b"\r",
            Key::Backspace => b"\x7f",
            Key::Tab => b"\t",
            Key::Escape => b"\x1b",
            Key::Up => b"\x1b[A",
            Key::Down => b"\x1b[B",
            Key::Right => b"\x1b[C",
            Key::Left => b"\x1b[D",
            Key::Home => b"\x1b[H",
            Key::End => b"\x1b[F",
            Key::Delete => b"\x1b[3~",
            Key::PageUp => b"\x1b[5~",
            Key::PageDown => b"\x1b[6~",
            _ => return None,
        })
    }
}

impl Widget for TerminalWidget {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        crate::ROLE_TERMINAL
    }

    /// 向下转型：Host 驱动 PTY（`write` / `pump` / `attach_shell`）的唯一路径。
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn revision(&self) -> u64 {
        self.revision
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    /// 终端必须可聚焦，否则收不到键盘输入（此前缺失，导致 Tab 无法进入终端）。
    fn focusable(&self) -> bool {
        true
    }

    /// 终端消费 Tab：shell 的补全就是用 Tab 触发的。
    fn wants_tab(&self) -> bool {
        true
    }

    /// 终端订阅每帧心跳：`Tick` 里拉 PTY 输出（不订阅就永远收不到）。
    fn wants_tick(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let cell_w = metrics
            .measure_text("M", &TextStyle::fg(Color::WHITE))
            .w
            .max(1.0);
        let cell_h = metrics.line_height(&TextStyle::fg(Color::WHITE)).max(1.0);
        c.constrain(Size::new(
            self.grid.cols as f32 * cell_w,
            self.grid.rows as f32 * cell_h,
        ))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        metrics: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        self.retile(rect, metrics);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::Tick => {
                if self.pump() {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::Compose(text) => {
                self.preedit = text.clone();
                EventResult::Redraw
            }
            UiEvent::Focus(focused) => {
                self.focused = *focused;
                EventResult::Redraw
            }
            UiEvent::MouseDown { pos, .. } => {
                ctx.request_focus(self.id.clone());
                self.begin_selection(*pos);
                EventResult::Redraw
            }
            UiEvent::MouseMove { pos } => {
                if self.selection.is_none() {
                    return EventResult::Ignored;
                }
                self.drag_selection_to(*pos);
                EventResult::Redraw
            }
            UiEvent::MouseUp { .. } => {
                // 抬起只是"结束拖拽"，选区保留（否则 Ctrl+Shift+C 就没东西可复制）
                EventResult::Redraw
            }
            UiEvent::Wheel { delta, .. } => {
                let lines = (delta.abs() / 120.0 * 3.0).round() as usize;
                if *delta > 0.0 {
                    self.grid.scroll_up(lines);
                } else {
                    self.grid.scroll_down(lines);
                }
                EventResult::Redraw
            }
            UiEvent::Key { key, modifiers: mods } => {
                // Ctrl+Shift+C / V：终端专用剪贴板快捷键
                // 规则：有选中才复制；没选中时 Ctrl+Shift+C 不拦截（交给 shell 处理 SIGINT）
                if mods.ctrl && mods.shift {
                    match key {
                        Key::Char('c') | Key::Char('C') => {
                            // 有选区：复制并消耗快捷键；没选区：**不拦截**，信号照常送 shell
                            if let Some(text) = self.selected_text() {
                                if !text.is_empty() {
                                    if let Some(cb) = self.clipboard.as_ref() {
                                        cb.set(&text);
                                    }
                                    return EventResult::Handled;
                                }
                            }
                            return EventResult::Ignored;
                        }
                        Key::Char('v') | Key::Char('V') => {
                            if let Some(cb) = self.clipboard.as_ref() {
                                if let Some(text) = cb.as_ref().get() {
                                    if self.paste(&text) {
                                        return EventResult::Handled;
                                    }
                                }
                            }
                            return EventResult::Handled;
                        }
                        _ => {}
                    }
                }
                if let Some(bytes) = Self::key_bytes(*key) {
                    self.selection = None;
                    self.grid.reset_scroll_on_input();
                    self.write(bytes);
                    EventResult::Handled
                } else {
                    EventResult::Ignored
                }
            }
            // RPC 投递、IME 提交、粘贴都走这里
            UiEvent::Text(text) => {
                // 粘贴路径必须过滤控制字符 + 按 bracketed 模式包装
                if self.paste(text) {
                    return EventResult::Redraw;
                }
                self.grid.reset_scroll_on_input();
                if self.read_only {
                    // 只读演示：直接喂给 VT 解析器，不碰 PTY
                    feed_bytes(
                        &mut self.text,
                        &mut self.pending_utf8,
                        &mut self.grid,
                        &text.clone().into_bytes(),
                    );
                } else {
                    self.write(text.as_bytes());
                }
                EventResult::Redraw
            }
            // 动作：名字由组件自己定义，框架和其它组件都不认识。
            UiEvent::Action { name, args } => match name.as_str() {
                "inject_text" => {
                    let text = args
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.grid.reset_scroll_on_input();
                    if self.read_only {
                        feed_bytes(
                            &mut self.text,
                            &mut self.pending_utf8,
                            &mut self.grid,
                            &text.into_bytes(),
                        );
                    } else {
                        self.write(text.as_bytes());
                    }
                    EventResult::Redraw
                }
                "clear" => {
                    // 清屏 + 回首页，纯组件内部实现
                    self.grid.erase_in_display(2);
                    self.grid.set_cursor(0, 0);
                    EventResult::Redraw
                }
                _ => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let rect = self.rect;
        if rect.is_empty() {
            return;
        }
        let cell_w = self.cell.w;
        let cell_h = self.cell.h;

        ctx.push_clip(rect);
        ctx.fill_rect(rect, to_color(DEFAULT_BG));

        let cols = self.grid.cols as usize;
        let rows = self.grid.rows as usize;

        for view_row in 0..rows {
            let Some(line) = self.grid.visible_row(view_row) else {
                continue;
            };
            let y = rect.y + view_row as f32 * cell_h;
            let len = (line.len as usize).min(cols);
            for col in 0..len {
                let cell = line.cells[col];
                let st = self.grid.style_at(cell.style);
                let (mut fg_spec, mut bg_spec) = (st.fg, st.bg);
                // 选区反色：与 SGR reverse 共用同一个 swap，因此两者叠加时会翻回来——
                // 这是可以接受的（选中反色文字本来就看不出反色效果）。
                let selected = self
                    .selection
                    .is_some_and(|sel| sel.contains(view_row, col));
                if st.reverse || selected {
                    std::mem::swap(&mut fg_spec, &mut bg_spec);
                }

                let bg = palette::resolve(bg_spec, DEFAULT_BG);
                if bg != DEFAULT_BG {
                    ctx.fill_rect(
                        Rect::new(rect.x + col as f32 * cell_w, y, cell_w, cell_h),
                        to_color(bg),
                    );
                }

                if cell.kind == CellKind::WideTail || cell.ch == ' ' {
                    continue;
                }

                let mut ts = TextStyle::fg(to_color(palette::resolve(fg_spec, DEFAULT_FG)));
                ts.bold = st.bold;
                ts.underline = st.underline;

                // P1-2：每格单字符，用 draw_char 避免每格一次 String 分配。
                ctx.draw_char(Pos::new(rect.x + col as f32 * cell_w, y), cell.ch, &ts);
            }
        }

        // 光标
        if self.grid.scroll_offset == 0 && !self.grid.cursor_hidden && self.focused {
            let cx = rect.x + self.grid.cursor_col.min(cols.saturating_sub(1)) as f32 * cell_w;
            let cy = rect.y + self.grid.cursor_row.min(rows.saturating_sub(1)) as f32 * cell_h;
            ctx.fill_rect(
                Rect::new(cx, cy, cell_w, cell_h),
                Color::rgba(0.35, 0.55, 0.85, 0.55),
            );
        }

        // IME 预编辑：画在光标处，带下划线；不进 Grid
        if !self.preedit.is_empty() && self.focused && self.grid.scroll_offset == 0 {
            let cx = rect.x + self.grid.cursor_col.min(cols.saturating_sub(1)) as f32 * cell_w;
            let cy = rect.y + self.grid.cursor_row.min(rows.saturating_sub(1)) as f32 * cell_h;
            let mut pen = cx;
            let ts = TextStyle::fg(Color::WHITE);
            for ch in self.preedit.chars() {
                ctx.draw_char(Pos::new(pen, cy), ch, &ts);
                // 宽字符判定与网格同源（unicode-width）；旧代码用 `> 0x2E80` 的启发式，
                // 对 emoji（0x1F600）会错判成 1 格。组合字符宽度 0 → 不推进（正确行为）。
                pen += cell_w * y901_core::Grid::width_of_char(ch) as f32;
            }
            ctx.fill_rect(
                Rect::new(cx, cy + cell_h - 2.0, (pen - cx).max(cell_w), 2.0),
                Color::rgba(0.55, 0.72, 1.0, 1.0),
            );
        }
        ctx.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        let mut sem = Semantics::default()
            .exposed(true)
            .label(format!(
                "terminal {}",
                self.grid.cols as u32 * self.grid.rows as u32
            ))
            .value(
                self.grid
                    .line_text(self.grid.cursor_row.min(self.grid.rows as usize - 1)),
            )
            .action_spec("inject_text", y901_ui::InputKind::Text)
            .action("clear")
            .prop("cols", self.grid.cols.to_string())
            .prop("rows", self.grid.rows.to_string())
            .prop("cursor_row", self.grid.cursor_row.to_string())
            .prop("cursor_col", self.grid.cursor_col.to_string())
            .prop("scroll_offset", self.grid.scroll_offset.to_string())
            .prop("history", self.grid.history.len().to_string())
            .prop("shell", if self.has_shell() { "pty" } else { "none" })
            // 结构化状态：Agent 与增量推送都靠它感知"真的变了"
            .state("cols", self.grid.cols as u32)
            .state("rows", self.grid.rows as u32)
            .state("cursor_row", self.grid.cursor_row as u32)
            .state("cursor_col", self.grid.cursor_col as u32)
            .state("scroll_offset", self.grid.scroll_offset as u32)
            .state("history", self.grid.history.len() as u32);

        // 颜色数量能反映 SGR 是否生效
        sem = sem.prop("styles", self.grid.styles.len().to_string());
        sem
    }
}

/// 调色板返回 `[f32; 4]`，框架用 `Color`：这里是唯一的转换点。
pub fn to_color(c: [f32; 4]) -> Color {
    Color::rgba(c[0], c[1], c[2], c[3])
}

/// 方便构造：把 `ColorSpec` 转成框架色（测试/调试用）。
pub fn color_of(spec: ColorSpec) -> Color {
    to_color(palette::resolve(spec, DEFAULT_FG))
}
