//! 极简 VT 过滤器：把 PTY 字节流变成 Grid 操作。
//!
//! 支持：CR/LF/BS/TAB、光标定位、擦除、SGR 颜色（CSI ... m）。
//! 忽略：OSC、私有模式、滚动区、字符集切换。

use y901_core::Grid;

use crate::palette::SgrState;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum VtState {
    #[default]
    Ground,
    Esc,
    Csi,
    Osc,
    OscEsc,
    Charset,
}

#[derive(Default)]
pub struct VtFilter {
    state: VtState,
    params: Vec<u8>,
    sgr: SgrState,
    /// 当前 SGR 对应的 StyleTable 索引（缓存，避免每个字符都哈希）。
    style_id: u16,
    style_dirty: bool,
}

impl VtFilter {
    /// 新建 ANSI 转义序列过滤器（地面状态，默认样式）。
    ///
    /// 返回：可直接 `feed` 的实例。副作用：无。
    pub fn new() -> Self {
        Self {
            state: VtState::Ground,
            params: Vec::new(),
            sgr: SgrState::default(),
            style_id: 0,
            style_dirty: true,
        }
    }

    /// 取当前样式的表索引；SGR 变化时才重新驻留。
    fn current_style(&mut self, grid: &mut Grid) -> u16 {
        if self.style_dirty {
            self.style_id = grid.intern_style(self.sgr.style());
            self.style_dirty = false;
        }
        self.style_id
    }

    /// 吃进一个字符，处理 CSI/SGR 等转义序列并写入网格。
    ///
    /// 输入：`ch`（来自 PTY 的 UTF-8 解码结果）、目标 `grid`。
    /// 返回：无。
    /// 副作用：会修改 `grid`（写字符、移动光标、改样式），并推进内部状态机；
    /// **不完整**的序列留在状态机里等后续字符（不要假设"一次 feed 一次刷新"）。
    pub fn feed(&mut self, ch: char, grid: &mut Grid) {
        match self.state {
            VtState::Ground => match ch {
                '\x1b' => self.state = VtState::Esc,
                '\r' => grid.carriage_return(),
                '\n' => grid.line_feed(),
                '\x08' => grid.backspace(),
                '\t' => grid.tab(),
                c if (c as u32) < 0x20 => {}
                c => {
                    let style = self.current_style(grid);
                    grid.place_cell(c, style);
                }
            },
            VtState::Esc => {
                self.state = match ch {
                    '[' => {
                        self.params.clear();
                        VtState::Csi
                    }
                    ']' => VtState::Osc,
                    '(' | ')' | '*' | '+' | '#' | '%' => VtState::Charset,
                    'D' => {
                        grid.line_feed();
                        VtState::Ground
                    }
                    'E' => {
                        grid.carriage_return();
                        grid.line_feed();
                        VtState::Ground
                    }
                    'M' => {
                        grid.move_cursor(-1, 0);
                        VtState::Ground
                    }
                    _ => VtState::Ground,
                };
            }
            VtState::Csi => {
                if ('\x40'..='\x7e').contains(&ch) {
                    self.dispatch(ch, grid);
                    self.state = VtState::Ground;
                } else {
                    self.params.push(ch as u8);
                }
            }
            VtState::Osc => match ch {
                '\x07' => self.state = VtState::Ground,
                '\x1b' => self.state = VtState::OscEsc,
                _ => {}
            },
            VtState::OscEsc => {
                self.state = if ch == '\\' {
                    VtState::Ground
                } else {
                    VtState::Osc
                };
            }
            VtState::Charset => self.state = VtState::Ground,
        }
    }

    fn dispatch(&mut self, final_ch: char, grid: &mut Grid) {
        let raw: String = self.params.iter().map(|b| *b as char).collect();
        let private = raw.starts_with('?') || raw.starts_with('>') || raw.starts_with('!');
        let body = raw.trim_start_matches(['?', '>', '!']);
        if private {
            // 私有模式：只关心 vim/htop 真正用到的几个
            let set = final_ch == 'h';
            let clear = final_ch == 'l';
            if set || clear {
                for part in body.split(';') {
                    match part.parse::<u32>().unwrap_or(0) {
                        1049 | 1047 | 47 => {
                            if set {
                                grid.enter_alt_screen();
                            } else {
                                grid.leave_alt_screen();
                            }
                        }
                        25 => grid.cursor_hidden = !set,
                        7 => grid.autowrap = set,
                        _ => {}
                    }
                }
            }
            return;
        }

        let p: Vec<u32> = body
            .split(';')
            .map(|s| s.parse::<u32>().unwrap_or(0))
            .collect();
        let n = |i: usize| p.get(i).copied().unwrap_or(0);
        let positive = |i: usize| n(i).max(1) as isize;

        match final_ch {
            // SGR：颜色与样式
            'm' => {
                self.sgr.apply(&p);
                self.style_dirty = true;
            }

            'H' | 'f' => {
                let r = n(0).max(1) as usize - 1;
                let c = n(1).max(1) as usize - 1;
                grid.set_cursor(r, c);
            }
            'A' => grid.move_cursor(-positive(0), 0),
            'B' => grid.move_cursor(positive(0), 0),
            'C' => grid.move_cursor(0, positive(0)),
            'D' => grid.move_cursor(0, -positive(0)),
            'G' => {
                let c = n(0).max(1) as usize - 1;
                grid.set_cursor(grid.cursor_row, c);
            }
            'd' => {
                let r = n(0).max(1) as usize - 1;
                grid.set_cursor(r, grid.cursor_col);
            }
            'J' => grid.erase_in_display(n(0) as u16),
            'K' => grid.erase_in_line(n(0) as u16),
            _ => {}
        }
    }
}

/// UTF-8 增量解码：跨 chunk 的半个汉字留在 pending 里等下一次。
pub fn feed_bytes(vt: &mut VtFilter, pending: &mut Vec<u8>, grid: &mut Grid, bytes: &[u8]) {
    pending.extend_from_slice(bytes);

    loop {
        let (text, valid, incomplete) = match std::str::from_utf8(pending) {
            Ok(s) => (s.to_owned(), pending.len(), false),
            Err(err) => {
                let valid = err.valid_up_to();
                let text = std::str::from_utf8(&pending[..valid])
                    .unwrap_or("")
                    .to_owned();
                (text, valid, err.error_len().is_none())
            }
        };

        if valid > 0 {
            pending.drain(..valid);
            for ch in text.chars() {
                vt.feed(ch, grid);
            }
        }
        if incomplete {
            break;
        }
        if !pending.is_empty() {
            pending.remove(0);
            continue;
        }
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(vt: &mut VtFilter, grid: &mut Grid, s: &str) {
        let mut pending = Vec::new();
        feed_bytes(vt, &mut pending, grid, s.as_bytes());
    }

    #[test]
    fn sgr_red_then_reset() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(20, 2);

        feed(&mut vt, &mut grid, "\x1b[31mR\x1b[0mN");
        assert_eq!(grid.line_text(0), "RN");

        let red = grid.style_at(grid.screen[0].cells[0].style);
        assert_eq!(red.fg, y901_core::ColorSpec::Indexed(1)); // red
        assert!(grid.style_at(grid.screen[0].cells[1].style).is_plain());
    }

    #[test]
    fn sgr_background() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(20, 2);
        feed(&mut vt, &mut grid, "\x1b[41mX");
        let style = grid.style_at(grid.screen[0].cells[0].style);
        assert_eq!(style.bg, y901_core::ColorSpec::Indexed(1));
    }

    #[test]
    fn sgr_bold_underline_reverse() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(20, 2);
        feed(&mut vt, &mut grid, "\x1b[1;4;7mX");
        let style = grid.style_at(grid.screen[0].cells[0].style);
        assert!(style.bold && style.underline && style.reverse);
    }

    #[test]
    fn sgr_truecolor_roundtrip() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(20, 2);
        feed(&mut vt, &mut grid, "\x1b[38;2;255;100;0mO\x1b[0m");
        let style = grid.style_at(grid.screen[0].cells[0].style);
        assert_eq!(style.fg, y901_core::ColorSpec::Rgb(255, 100, 0));
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(20, 2);
        let bytes = "中".as_bytes();
        let mut pending = Vec::new();
        feed_bytes(&mut vt, &mut pending, &mut grid, &bytes[..1]);
        assert_eq!(grid.line_text(0), "");
        feed_bytes(&mut vt, &mut pending, &mut grid, &bytes[1..]);
        assert_eq!(grid.line_text(0), "中");
    }

    #[test]
    fn color_persists_across_lines() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(20, 3);
        feed(&mut vt, &mut grid, "\x1b[32mA\r\nB");
        let a = grid.style_at(grid.screen[0].cells[0].style);
        let b = grid.style_at(grid.screen[1].cells[0].style);
        assert_eq!(a.fg, y901_core::ColorSpec::Indexed(2));
        assert_eq!(b.fg, y901_core::ColorSpec::Indexed(2));
    }

    #[test]
    fn scrollback_collects_evicted_rows() {
        let mut vt = VtFilter::new();
        let mut grid = Grid::new(10, 3);
        // 5 行内容塞进 3 行视口，前 2 行应进历史
        feed(&mut vt, &mut grid, "L0\r\nL1\r\nL2\r\nL3\r\nL4");
        assert_eq!(grid.history.len(), 2);
        assert_eq!(grid.history[0].text(), "L0");
        assert_eq!(grid.history[1].text(), "L1");
        assert_eq!(grid.screen[0].text(), "L2");

        grid.scroll_up(2);
        assert_eq!(grid.visible_row(0).unwrap().text(), "L0");
        grid.scroll_down(5);
        assert_eq!(grid.scroll_offset, 0);
    }
}
