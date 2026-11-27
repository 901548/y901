//! **Grid 的不变量**（属性测试，确定性伪随机，不引入额外依赖）。
//!
//! 这一层是终端内核算术最容易出错的地方：宽字符占两格（`WideHead` + `WideTail`），
//! 任何"只动一格"的操作都会把一对劈成两半 —— 渲染时半格标记会被跳过，画面上少
//! 半个字；`line_text()` / `text()` 里则多出一个无头的字符位。
//!
//! 本文件随机混合 14 种 VT 操作（写窄/宽字、LF、CR、退格、Tab、定位、相对移动、
//! 擦行、擦屏、改列宽重排）× 300 个网格，每一步都检查：
//!
//! 1. 每行格数等于 `cols`，`len <= cols`；
//! 2. `WideHead` 的下一个格必须是 `WideTail`，且不能在末列（否则就是"被劈开"）；
//! 3. `WideTail` 不能在第 0 列，且前一个格必须是 `WideHead`。
//!
//! 它抓到过两个真 bug（2026-11-27）：
//! - `blank_from` / `blank_upto` 逐格填空白 → 擦到宽字中间时留下孤立的 `WideTail`；
//! - `reflow` 裁剪历史时用"每行减去 base_off"修偏移 → 同一逻辑行 id 不连续时
//!   `u32` 下溢 panic（现在改为从行 0 连续前缀重新累积）。

use y901_core::{CellKind, Grid, STYLE_DEFAULT};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn check(g: &Grid, ctx: &str) {
    let cols = g.cols as usize;
    assert!(g.rows >= 1 && g.cols >= 1, "[{ctx}] cols/rows 至少 1");

    for (ri, row) in g.screen.iter().enumerate() {
        assert_eq!(
            row.cells.len(),
            cols,
            "[{ctx}] 第 {ri} 行有 {} 个格，应为 {cols}",
            row.cells.len()
        );
        assert!(
            row.len as usize <= cols,
            "[{ctx}] 第 {ri} 行 len={} 超过 cols={cols}",
            row.len
        );
        for (ci, cell) in row.cells.iter().enumerate() {
            match cell.kind {
                CellKind::WideHead => {
                    assert!(
                        ci + 1 < cols,
                        "[{ctx}] 第 {ri} 行第 {ci} 列是宽字头，但它是最后一个格 —— \
                         宽字符被劈开了（行宽 {cols}）"
                    );
                    assert_eq!(
                        row.cells[ci + 1].kind,
                        CellKind::WideTail,
                        "[{ctx}] 第 {ri} 行第 {ci} 列是宽字头，下一个却不是宽字尾"
                    );
                }
                CellKind::WideTail => {
                    assert!(ci > 0, "[{ctx}] 第 {ri} 行第 0 列是宽字尾（没有头）");
                    assert_eq!(
                        row.cells[ci - 1].kind,
                        CellKind::WideHead,
                        "[{ctx}] 第 {ri} 行第 {ci} 列是宽字尾，上一个却不是宽字头"
                    );
                }
                _ => {}
            }
        }
    }
}

#[test]
fn random_terminal_ops_keep_grid_consistent() {
    let mut rng = Lcg(0x0bad_f00d);
    for case in 0..300 {
        let cols = (rng.next() % 8) as u16 + 1;
        let rows = (rng.next() % 5) as u16 + 1;
        let mut g = Grid::new(cols, rows);
        for step in 0..120 {
            match rng.next() % 14 {
                0..=2 => g.place_cell('a', STYLE_DEFAULT),
                3..=4 => g.place_cell('中', STYLE_DEFAULT),
                5 => g.line_feed(),
                6 => g.carriage_return(),
                7 => g.backspace(),
                8 => g.tab(),
                9 => {
                    let r = (rng.next() % 6) as usize;
                    let c = (rng.next() % 9) as usize;
                    g.set_cursor(r, c);
                }
                10 => {
                    let dr = (rng.next() % 5) as isize - 2;
                    let dc = (rng.next() % 5) as isize - 2;
                    g.move_cursor(dr, dc);
                }
                11 => g.erase_in_line((rng.next() % 3) as u16),
                12 => g.erase_in_display((rng.next() % 4) as u16),
                _ => {
                    let nc = (rng.next() % 9) as u16;
                    g.reflow(nc);
                }
            }
            check(&g, &format!("case{case} step{step}"));
        }
    }
}
