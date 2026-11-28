//! **双向文本（Bidi）视觉重排的不变量**（属性测试，确定性伪随机）。
//!
//! 视觉顺序算错的表现是"字的位置对了但内容错位"—— 混合 LTR/RTL 段里一个字符被
//! 漏掉或重复，画面上就是整行错位，而单看某个字符的坐标很难发现。这里钉三条：
//!
//! 1. `reorder_visual` 的结果必须是**逻辑字符位置的一个排列**（不重不漏，且每个位置
//!    都是 char 边界）；
//! 2. 纯 LTR 顺序不变、纯 RTL 整段反转（含多字节字符的字节偏移也要正确反转）；
//! 3. `visual_runs_of` 的区间在**逻辑上**首尾相接、覆盖整串、无空段。
//!
//! 注：`y901-render` 与 `y901-ui` 各有一套 cluster 实现，**它们按设计就不同**
//! （一个服务 shaping 分段、一个服务光标/删除粒度），不要试图断言两者相等。

fn he(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a' => '\u{05D0}', // א
            'b' => '\u{05D1}', // ב
            'c' => '\u{05D2}', // ג
            'd' => '\u{05D3}', // ד
            _ => c,
        })
        .collect()
}
fn ar(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a' => '\u{0645}', // م
            'b' => '\u{0631}', // ر
            'c' => '\u{062D}', // ح
            'd' => '\u{0628}', // ب
            _ => c,
        })
        .collect()
}

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

/// 视觉顺序必须是**逻辑字符位置的一个排列**：不重不漏，且每个位置都是 char 边界。
fn check_permutation(text: &str, ctx: &str) {
    let visual = y901_render::text::reorder_visual(text);
    let logical: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    assert_eq!(
        visual.len(),
        logical.len(),
        "[{ctx}] 视觉序列长度 {} != 逻辑字符数 {}（text={text:?}）",
        visual.len(),
        logical.len()
    );
    let mut sorted = visual.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted, logical,
        "[{ctx}] 视觉序列不是逻辑字符的一个排列（重排丢了或重复了字符）text={text:?}"
    );
    for &i in &visual {
        assert!(
            text.is_char_boundary(i),
            "[{ctx}] 视觉序列里 {i} 不是 char 边界（text={text:?}）"
        );
    }
}

#[test]
fn visual_order_is_a_permutation_of_logical_positions() {
    let mut rng = Lcg(0xb1d1_2026);
    let corpus = [
        "",
        "abc",
        "abcd",
        "ab ab",
        "a b",
        "abcd ef",
        " ab",
        "ab ",
        " a ",
        "12 34",
        "\u{05D0}\u{05D1}\u{05D2}\u{05D3}",
        "\u{0645}\u{0631}\u{062D}\u{0628}",
    ];
    for (ci, base) in corpus.iter().enumerate() {
        check_permutation(base, &format!("corpus{ci}"));
    }
    // 随机混合：拉丁词 + 希伯来/阿拉伯词 + 空格 + 数字
    for case in 0..2000 {
        let parts = ["abc", "ab", "a", "12", "b c", "  "];
        let n = 1 + (rng.next() % 4) as usize;
        let mut s = String::new();
        for _ in 0..n {
            let k = rng.next() % 3;
            let base = parts[(rng.next() as usize) % parts.len()];
            match k {
                0 => s.push_str(base),
                1 => s.push_str(&he(base)),
                _ => s.push_str(&ar(base)),
            }
        }
        check_permutation(&s, &format!("case{case}"));
    }
}

#[test]
fn pure_ltr_and_pure_rtl_have_known_orders() {
    // 纯 LTR：视觉 == 逻辑
    let ltr = "abcd";
    let v = y901_render::text::reorder_visual(ltr);
    let logical: Vec<usize> = ltr.char_indices().map(|(i, _)| i).collect();
    assert_eq!(v, logical, "纯 LTR 顺序不变");

    // 纯 RTL：视觉 == 逻辑反转（单字节 UTF-8 之外用多字节字符验证，偏移也要反）
    let rtl = he("abcd");
    let v = y901_render::text::reorder_visual(&rtl);
    let mut expected: Vec<usize> = rtl.char_indices().map(|(i, _)| i).collect();
    expected.reverse();
    assert_eq!(v, expected, "纯 RTL 段必须整段反转");
}

#[test]
fn visual_runs_cover_the_text_contiguously() {
    let mut rng = Lcg(0x5ee_1122);
    for case in 0..1500 {
        let parts = ["abc", "a", "12", " "];
        let n = 1 + (rng.next() % 4) as usize;
        let mut s = String::new();
        for _ in 0..n {
            let base = parts[(rng.next() as usize) % parts.len()];
            match rng.next() % 3 {
                0 => s.push_str(base),
                1 => s.push_str(&he(base)),
                _ => s.push_str(&ar(base)),
            }
        }
        let runs = y901_render::text::visual_runs_of(&s);
        if s.is_empty() {
            assert!(runs.is_empty(), "[case{case}] 空串不应有 run");
            continue;
        }
        // 逻辑上必须无重叠、无空洞地覆盖整串
        let mut logical: Vec<_> = runs.iter().map(|(r, _)| r.clone()).collect();
        logical.sort_by_key(|r| r.start);
        let mut pos = 0usize;
        for r in &logical {
            assert_eq!(
                r.start, pos,
                "[case{case}] run 区间不连续（空洞/重叠）：{logical:?}，text={s:?}"
            );
            pos = r.end;
        }
        assert_eq!(pos, s.len(), "[case{case}] run 未覆盖到串尾：{logical:?}");
        // 每段都必须是非空的
        for (r, _) in &runs {
            assert!(!r.is_empty(), "[case{case}] 出现空 run：{runs:?}");
        }
    }
}
