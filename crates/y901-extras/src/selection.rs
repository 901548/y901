//! 终端选区：**拖拽选中 + 取文本 + 反色展示**。
//!
//! ## 为什么选区要独立一个模块
//!
//! 它有三个独立的不变量，且每个都容易写错：
//! 1. **归一化**：拖拽可从右下向左上（start > end）；
//! 2. **行跨度**：首行从 `col` 到行尾，中间行全选，末行从行首到 `col`；
//! 3. **取文本时要 trim 行末空格**（否则粘贴到编辑器里全是尾随空白）。
//!
//! 把它们放在一个可单测的类里，比埋在 `TerminalWidget::event` 里安全得多。

use y901_core::Cell;

/// 一个矩形选区（行/列坐标，均为屏幕内偏移）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// 锚点（按下鼠标处）
    pub start: (usize, usize),
    /// 活动端（当前鼠标处）
    pub end: (usize, usize),
}

impl Selection {
    /// 新建一个单元格选区（按下时的初始状态）。
    pub fn new(row: usize, col: usize) -> Self {
        Self {
            start: (row, col),
            end: (row, col),
        }
    }

    /// 把活动端移到 `(row, col)`。
    pub fn drag_to(&mut self, row: usize, col: usize) {
        self.end = (row, col);
    }

    /// 归一化后的 `(上行, 上列, 下行, 下列)`。
    ///
    /// 为什么要归一化：拖拽方向不定，下游全部按"从左上到右下"处理。
    pub fn normalized(&self) -> ((usize, usize), (usize, usize)) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    /// 某个单元格是否在选区内。
    ///
    /// 规则：首行 `col >= 起始列`；末行 `col <= 结束列`；中间行全选。
    pub fn contains(&self, row: usize, col: usize) -> bool {
        let ((r0, c0), (r1, c1)) = self.normalized();
        if row < r0 || row > r1 {
            return false;
        }
        if r0 == r1 {
            return col >= c0 && col <= c1;
        }
        if row == r0 {
            return col >= c0;
        }
        if row == r1 {
            return col <= c1;
        }
        true
    }

    /// 选区是否为空（没拖动过 = 只点了一下）。
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// 把选区内的文本拼成字符串。
    ///
    /// 输入：`visible_row` 回调（行号 → 该行）、`cols`。
    /// 输出：行间用 `\n` 连接，**每行末尾空格已 trim**。
    /// 副作用：无。
    pub fn text(&self, grid: &y901_core::Grid, cols: usize) -> String {
        let ((r0, c0), (r1, c1)) = self.normalized();
        let mut out = String::new();
        for row in r0..=r1 {
            let Some(line) = grid.visible_row(row) else {
                continue;
            };
            let len = (line.len as usize).min(cols);
            let from = if row == r0 { c0.min(len) } else { 0 };
            let to = if row == r1 { (c1 + 1).min(len) } else { len };
            let mut s = String::new();
            for col in from..to {
                let cell: &Cell = &line.cells[col];
                // 宽字符的尾部占位不重复输出
                if cell.kind == y901_core::CellKind::WideTail {
                    continue;
                }
                s.push(cell.ch);
            }
            // 行末空格 trim：不然粘到编辑器里全是尾随空白
            let trimmed = s.trim_end();
            out.push_str(trimmed);
            if row != r1 {
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_core::Grid;

    /// 造一个已填充文本的网格（每行一个短串）。
    fn grid_with(lines: &[&str]) -> Grid {
        let mut g = Grid::new(20, lines.len().max(1) as u16);
        for (i, s) in lines.iter().enumerate() {
            g.set_cursor(i, 0);
            for ch in s.chars() {
                g.place_cell(ch, 0);
            }
        }
        g
    }

    #[test]
    fn single_row_selection_takes_the_substring() {
        let g = grid_with(&["hello world"]);
        let sel = Selection {
            start: (0, 0),
            end: (0, 4),
        };
        assert_eq!(sel.text(&g, 20), "hello");
    }

    #[test]
    fn cross_row_selection_includes_middle_rows_fully() {
        let g = grid_with(&["aaa", "bbbbbb", "cc"]);
        // 从第 0 行第 1 列 → 第 2 行第 0 列
        let sel = Selection {
            start: (0, 1),
            end: (2, 0),
        };
        assert_eq!(sel.text(&g, 20), "aa\nbbbbbb\nc");
    }

    #[test]
    fn reverse_drag_is_normalized() {
        let g = grid_with(&["abc", "def"]);
        let sel = Selection {
            start: (1, 2),
            end: (0, 0),
        };
        assert_eq!(sel.text(&g, 20), "abc\ndef");
        // 归一化后 start 在前
        assert_eq!(sel.normalized().0, (0, 0));
    }

    #[test]
    fn trailing_spaces_are_trimmed_per_line() {
        let mut g = Grid::new(20, 2);
        // 第 0 行："ab" + 其余留空（不写入）
        g.set_cursor(0, 0);
        g.place_cell('a', 0);
        g.place_cell('b', 0);
        let sel = Selection {
            start: (0, 0),
            end: (0, 19),
        };
        assert_eq!(sel.text(&g, 20), "ab", "行尾空格应被 trim");
    }

    #[test]
    fn contains_respects_row_bounds() {
        let sel = Selection {
            start: (1, 3),
            end: (3, 5),
        };
        assert!(!sel.contains(0, 4), "首行上方不选");
        assert!(!sel.contains(1, 2), "首行左侧不选");
        assert!(sel.contains(1, 3), "首行起点选");
        assert!(sel.contains(2, 0), "中间行全选");
        assert!(sel.contains(3, 5), "末行终点选");
        assert!(!sel.contains(3, 6), "末行右侧不选");
        assert!(!sel.contains(4, 0), "末行下方不选");
    }

    #[test]
    fn single_cell_click_is_an_empty_selection() {
        let sel = Selection::new(2, 5);
        assert!(sel.is_empty(), "只点一下不算选中");
    }
}
