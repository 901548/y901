//! **选区抽取 / 粘贴清洗的不变量**（属性测试，确定性伪随机）。
//!
//! 四条性质，各自对应一种"用户会真的踩到"的失败：
//!
//! 1. `Selection::text()` 必须与 `Selection::contains()` 的判定**逐格一致** ——
//!    两者不一致时，界面上高亮的格子和复制出来的内容不是同一块；
//! 2. **复制 → 粘贴 → 再复制必须无损** —— 终端里最被信任的一条链路，
//!    少一个字符就是"看不见的数据丢失"；
//! 3. `paste::sanitize()` 之后**不得残留任何控制字符**（`\x1b` 能伪造转义序列、
//!    `\x08` 能改写行内内容）—— 这是安全红线；
//! 4. `sanitize()` **幂等**（清洗两次 == 一次），否则"先清洗再转发"的链路会二次变化。

use y901_core::{CellKind, Grid, STYLE_DEFAULT};
use y901_extras::selection::Selection;

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

fn build(cols: u16, rows: u16, rng: &mut Lcg) -> Grid {
    let mut g = Grid::new(cols, rows);
    for r in 0..rows as usize {
        g.set_cursor(r, 0);
        let n = (rng.next() % (cols as u64 + 2)) as usize;
        for _ in 0..n {
            let ch = match rng.next() % 5 {
                0 => '中',
                1 => 'a',
                2 => ' ',
                3 => 'x',
                _ => '1',
            };
            g.place_cell(ch, STYLE_DEFAULT);
        }
        g.carriage_return();
    }
    g
}

/// 按 `contains()` 的语义重建期望文本（独立实现，用于对拍）。
fn expected(g: &Grid, sel: &Selection, cols: usize) -> String {
    let ((r0, _), (r1, _)) = sel.normalized();
    let mut lines: Vec<String> = Vec::new();
    for row in r0..=r1 {
        let Some(line) = g.visible_row(row) else {
            lines.push(String::new());
            continue;
        };
        let len = (line.len as usize).min(cols);
        let mut s = String::new();
        for col in 0..len {
            if !sel.contains(row, col) {
                continue;
            }
            let cell = &line.cells[col];
            if cell.kind == CellKind::WideTail {
                continue;
            }
            s.push(cell.ch);
        }
        lines.push(s.trim_end().to_string());
    }
    lines.join("\n")
}

#[test]
fn text_matches_the_contains_predicate() {
    let mut rng = Lcg(0x5e1_ec70);
    for case in 0..800 {
        let cols = 1 + (rng.next() % 10) as u16;
        let rows = 1 + (rng.next() % 5) as u16;
        let g = build(cols, rows, &mut rng);
        let (r0, c0) = (
            (rng.next() % rows as u64) as usize,
            (rng.next() % (cols as u64 + 2)) as usize,
        );
        let (r1, c1) = (
            (rng.next() % rows as u64) as usize,
            (rng.next() % (cols as u64 + 2)) as usize,
        );
        let sel = Selection {
            start: (r0, c0),
            end: (r1, c1),
        };
        let got = sel.text(&g, cols as usize);
        let want = expected(&g, &sel, cols as usize);
        assert_eq!(
            got,
            want,
            "[case{case}] 选区文本与 contains() 不一致：sel={sel:?} cols={cols} rows={rows} \
             grid_lines={:?}",
            (0..rows as usize)
                .map(|r| g.line_text(r))
                .collect::<Vec<_>>()
        );
    }
}
/// **安全性质**：`sanitize` 之后不得残留任何 `0x00..=0x1f`（除 `\r`/`\t`）与 DEL。
///
/// 这些字符能直接伪造终端转义序列（`\x1b`）、改写行内内容（`\x08`）或截断输出
/// （`\x00`），所以"过滤干净"是安全红线，不是可选项。
#[test]
fn sanitize_leaves_no_control_characters() {
    let mut rng = Lcg(0x5a_1170);
    for case in 0..2000 {
        let n = (rng.next() % 30) as usize;
        let mut s = String::new();
        for _ in 0..n {
            let c = match rng.next() % 8 {
                0 => '\u{1b}', // ESC
                1 => '\u{8}',  // BS
                2 => '\u{0}',  // NUL
                3 => '\u{7f}', // DEL
                4 => '\n',
                5 => '\r',
                6 => '\t',
                _ => char::from_u32((rng.next() % 0x2000) as u32 + 0x20).unwrap_or('a'),
            };
            s.push(c);
        }
        let out = y901_extras::paste::sanitize(&s);
        for c in out.chars() {
            assert!(
                c == '\r' || c == '\t' || (c as u32) >= 0x20,
                "[case{case}] 清洗后仍残留控制字符 {:?}（输入 {:?} 输出 {:?}）",
                c,
                s,
                out
            );
            assert_ne!(c as u32, 0x7f, "[case{case}] DEL 未被过滤");
        }
        // 换行必须统一成 CR（PTY 语义）
        assert!(
            !out.contains('\n'),
            "[case{case}] 清洗后不应再有 \\n：{:?}",
            out
        );
    }
}

/// **往返恒等**：清洗两次 == 清洗一次（幂等）。
#[test]
fn sanitize_is_idempotent() {
    let mut rng = Lcg(0x1d_e000);
    for case in 0..1500 {
        let n = (rng.next() % 20) as usize;
        let mut s = String::new();
        for _ in 0..n {
            let c = match rng.next() % 6 {
                0 => '\u{1b}',
                1 => '\n',
                2 => '\r',
                3 => '\t',
                _ => char::from_u32((rng.next() % 0x1000) as u32 + 0x20).unwrap_or('z'),
            };
            s.push(c);
        }
        let once = y901_extras::paste::sanitize(&s);
        let twice = y901_extras::paste::sanitize(&once);
        assert_eq!(once, twice, "[case{case}] sanitize 不幂等：输入 {:?}", s);
    }
}
/// **复制 → 粘贴 → 再复制**必须无损（在同一列宽的目标网格里）。
///
/// 这是终端里最常被信任的一条链路：选中一段 → Ctrl+Shift+C → 粘到另一个窗口/编辑器。
/// 只要抽取时少一个字符、或者粘贴时把宽字符挤到下一行，用户就在"看不见的地方丢了数据"。
#[test]
fn copy_paste_roundtrip_is_lossless() {
    let mut rng = Lcg(0xc0_9a57);
    for case in 0..400 {
        let cols = 1 + (rng.next() % 8) as u16;
        let rows = 1 + (rng.next() % 4) as u16;
        let g = build(cols, rows, &mut rng);
        let (r0, c0) = (
            (rng.next() % rows as u64) as usize,
            (rng.next() % (cols as u64 + 2)) as usize,
        );
        let (r1, c1) = (
            (rng.next() % rows as u64) as usize,
            (rng.next() % (cols as u64 + 2)) as usize,
        );
        let sel = Selection {
            start: (r0, c0),
            end: (r1, c1),
        };
        let copied = sel.text(&g, cols as usize);

        // 粘贴：按行写进一个同宽的新网格（每行 CR→回到行首 + 换行）
        let mut g2 = Grid::new(cols, rows.max(copied.lines().count() as u16 + 1));
        let mut pasted_rows = 0usize;
        for (i, line) in copied.split('\n').enumerate() {
            g2.set_cursor(i, 0);
            for ch in line.chars() {
                g2.place_cell(ch, STYLE_DEFAULT);
            }
            pasted_rows = i + 1;
        }
        // 再抽取整个粘贴区域
        let sel2 = Selection {
            start: (0, 0),
            end: (pasted_rows.saturating_sub(1), cols as usize + 1),
        };
        let again = sel2.text(&g2, cols as usize);
        assert_eq!(
            copied,
            again,
            "[case{case}] 复制/粘贴往返丢数据：sel={sel:?} cols={cols} rows={rows} \
             original_lines={:?}",
            (0..rows as usize)
                .map(|r| g.line_text(r))
                .collect::<Vec<_>>()
        );
    }
}
