//! [临时] `TextBuffer` 的光标不变量：随机操作后检查"光标/锚点始终是合法的字素簇边界"。

use y901_ui::TextBuffer;

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

fn is_cluster_boundary(text: &str, byte: usize) -> bool {
    // 用公开的 cluster API 交叉验证：把任意位置 snap 回来应当等于自己。
    y901_ui::cluster::snap(text, byte) == byte
}

fn check(buf: &TextBuffer, ctx: &str) {
    let text = buf.text();
    let c = buf.cursor();
    let a = buf.anchor();
    assert!(c <= text.len(), "[{ctx}] cursor {c} > len {}", text.len());
    assert!(a <= text.len(), "[{ctx}] anchor {a} > len {}", text.len());
    assert!(
        text.is_char_boundary(c),
        "[{ctx}] cursor {c} 不在 char 边界"
    );
    assert!(
        text.is_char_boundary(a),
        "[{ctx}] anchor {a} 不在 char 边界"
    );
    assert!(
        is_cluster_boundary(text, c),
        "[{ctx}] cursor {c} 不在字素簇边界（text={text:?}）"
    );
    assert!(
        is_cluster_boundary(text, a),
        "[{ctx}] anchor {a} 不在字素簇边界（text={text:?}）"
    );
    if let Some((s, e)) = buf.selection() {
        assert!(s <= e, "[{ctx}] 选区反了 {s}..{e}");
        assert!(
            text.is_char_boundary(s) && text.is_char_boundary(e),
            "[{ctx}] 选区边界不在 char 边界"
        );
    }
}

#[test]
fn random_editing_keeps_the_caret_on_a_cluster_boundary() {
    let seeds = [
        "",
        "abc",
        "e\u{301}x\u{308}",
        "\u{915}\u{93F}\u{930}\u{94D}\u{92F}",
        "\u{645}\u{631}\u{62D}\u{628}\u{627}",
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        "a\nb\n",
        "  ",
    ];
    let mut rng = Lcg(0xfeed_face);
    for (si, seed) in seeds.iter().enumerate() {
        for round in 0..300 {
            let mut buf = TextBuffer::from_text(*seed);
            for step in 0..25 {
                let op = rng.next() % 12;
                match op {
                    0 => {
                        buf.insert_char('x');
                    }
                    1 => {
                        buf.insert_str("中");
                    }
                    2 => {
                        buf.backspace();
                    }
                    3 => {
                        buf.delete_forward();
                    }
                    4 => {
                        buf.move_left(rng.next().is_multiple_of(2));
                    }
                    5 => {
                        buf.move_right(rng.next().is_multiple_of(2));
                    }
                    6 => {
                        buf.move_word_left(rng.next().is_multiple_of(2));
                    }
                    7 => {
                        buf.move_word_right(rng.next().is_multiple_of(2));
                    }
                    8 => {
                        // 把光标放到一个**任意字节位置**（含字符中间）—— 必须被吸附
                        let p = (rng.next() as usize) % (buf.text().len() + 1);
                        buf.set_cursor(p);
                    }
                    9 => {
                        let n = buf.text().len();
                        let p = (rng.next() as usize) % (n + 1);
                        let q = (rng.next() as usize) % (n + 1);
                        buf.set_selection(p, q);
                    }
                    10 => {
                        if rng.next().is_multiple_of(2) {
                            buf.undo();
                        } else {
                            buf.redo();
                        }
                    }
                    _ => {
                        if rng.next().is_multiple_of(4) {
                            let n = buf.text().len();
                            let p = (rng.next() as usize) % (n + 1);
                            buf.select_word_at(p);
                        }
                    }
                }
                check(&buf, &format!("seed{si} round{round} step{step} op{op}"));
            }
        }
    }
}
