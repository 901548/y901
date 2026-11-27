//! **折行的不变量**（属性测试，确定性伪随机，不引入额外依赖）。
//!
//! 为什么要有它：`wrap` 的规则（优先空格断、否则按字符断、保留 `\n`）写起来容易，
//! 但"切点算错一个字节"这类 bug 用举例测试很难覆盖 —— 手写用例总是那几条。
//! 这里用 4000 + 1500 组随机文本钉三条不变量：
//!
//! 1. 每行的 `start` 是 char 边界，且 `text[start..]` 确实以该行内容开头
//!    （否则点击命中/光标定位会落到别的行）；
//! 2. 多字符行（除"单字符比整行还宽"的退化情况）宽度必须真的放得下；
//! 3. **不丢字符**：去掉所有空白后，原文与各行的拼接必须逐字符相等
//!    （CJK 允许按字符断开，所以不能按"空白分隔的词"比）。

use y901_ui::wrap::wrap;

fn mono(s: &str) -> f32 {
    s.chars().count() as f32 * 10.0
}

/// 确定性伪随机（避免引入 rand 依赖）。
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

fn check(text: &str, w: f32, ctx: &str) {
    let lines = wrap(text, w, mono);
    assert!(!lines.is_empty(), "[{ctx}] 必须至少一行: {text:?}");

    // 1. start 必须指向该行文本在原文里的位置
    for (i, l) in lines.iter().enumerate() {
        assert!(
            l.start <= text.len(),
            "[{ctx}] 第 {i} 行 start={} 越界（len={}）text={text:?} lines={lines:?}",
            l.start,
            text.len()
        );
        assert!(
            text.is_char_boundary(l.start),
            "[{ctx}] 第 {i} 行 start={} 不在 char 边界 text={text:?}",
            l.start
        );
        if !l.text.is_empty() {
            assert!(
                text[l.start..].starts_with(&l.text),
                "[{ctx}] 第 {i} 行文本 {:?} 与 start={} 处的原文不符。text={text:?} lines={lines:?}",
                l.text,
                l.start
            );
        }
    }

    // 2. 非超宽行必须真的放得下
    for (i, l) in lines.iter().enumerate() {
        let wl = mono(&l.text);
        if l.text.chars().count() > 1 {
            assert!(
                wl <= w + 0.01,
                "[{ctx}] 第 {i} 行宽 {wl} > {w}（多字符行不该超宽）: {:?} text={text:?}",
                l.text
            );
        }
    }

    // 3. 内容不丢：CJK/长词允许**按字符**断开，所以只能比"去掉所有空白后的字符序列"。
    let strip = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let joined: String = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        strip(text),
        strip(&joined),
        "[{ctx}] 折行丢了或多了字符。text={text:?} lines={lines:?}"
    );
}

#[test]
fn wrap_invariants_random() {
    let alphabets: [&[&str]; 5] = [
        &["a", "b", "c", " "],
        &["ab", " ", " ", "cd"],
        &["中", "文", "字", " "],
        &["a", "中", " ", "bb"],
        &["e\u{301}", "x", " ", "yy"],
    ];
    let mut rng = Lcg(0x1234_5678);
    for case in 0..4000 {
        let alpha: &[&str] = alphabets[(rng.next() as usize) % alphabets.len()];
        let n = (rng.next() % 14) as usize;
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(alpha[(rng.next() as usize) % alpha.len()]);
        }
        let w = ((rng.next() % 6) as f32 + 1.0) * 10.0; // 10..60
        check(&s, w, &format!("case{case}"));
    }
}

#[test]
fn wrap_invariants_with_newlines() {
    let mut rng = Lcg(0xdead_beef);
    for case in 0..1500 {
        let n = (rng.next() % 10) as usize;
        let mut s = String::new();
        for _ in 0..n {
            match rng.next() % 4 {
                0 => s.push('\n'),
                1 => s.push(' '),
                2 => s.push('a'),
                _ => s.push('中'),
            }
        }
        let w = ((rng.next() % 5) as f32 + 1.0) * 10.0;
        check(&s, w, &format!("nl{case}"));
    }
}
