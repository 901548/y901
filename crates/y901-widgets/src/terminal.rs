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
    Semantics, Size, TextStyle, TextMetrics, UiEvent, Widget,
};

use crate::palette::{self, DEFAULT_BG, DEFAULT_FG};
use crate::vt::{feed_bytes, VtFilter};

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
                feed_bytes(&mut self.text, &mut self.pending_utf8, &mut self.grid, &bytes);
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
            UiEvent::MouseDown { .. } => {
                ctx.request_focus(self.id.clone());
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
            UiEvent::Key { key, .. } => {
                if let Some(bytes) = Self::key_bytes(*key) {
                    self.grid.reset_scroll_on_input();
                    self.write(bytes);
                    EventResult::Handled
                } else {
                    EventResult::Ignored
                }
            }
            // RPC 投递、IME 提交、粘贴都走这里
            UiEvent::Text(text) => {
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
                if st.reverse {
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
        let mut sem = Semantics::default().exposed(true)
            .label(format!("terminal {}", self.grid.cols as u32 * self.grid.rows as u32))
            .value(self.grid.line_text(self.grid.cursor_row.min(self.grid.rows as usize - 1)))
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
