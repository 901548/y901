//! **折行**：把一段文字按可用宽度切成多行。
//!
//! ## 为什么单独一个模块
//!
//! 折行是**纯几何**问题："给定宽度与测量函数，切在哪"。它不需要窗口、图集或字体引擎，
//! 因此可以完全单测；而组件（`TextLabel` / `RichText`）只需要接上测量函数。
//!
//! ## 规则（最小可用版，但每条都有理由）
//!
//! 1. **优先在空格处断行**（拉丁文按词）——这是排版惯例，也是"词不被劈开"的唯一保证；
//! 2. **没有空格可断时按字符断**（CJK 没有词间空格，长 URL 也没有）；
//! 3. **单字符比整行还宽时**：照样单独成行（宁可超宽，也不要死循环或丢字符）；
//! 4. **保留原有换行符** `\n`（显式换行永远优先，折叠它会让"多行文本"变成一行）。
//!
//! 不做：连字符断词（hyphenation）、两端对齐、避头尾标点规则。那些需要语言数据，
//! 属于后续切片；当前实现宁可朴素也不要假装支持。

/// 一行折好的文本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedLine {
    /// 该行内容（不含行尾空白）。
    pub text: String,
    /// 该行在原始字符串里的字节起点。
    pub start: usize,
}

/// 按 `max_w` 折行，`measure` 返回一段文字的宽度。
///
/// 参数：
/// - `text`：原始文本（可含 `\n`）；
/// - `max_w`：可用宽度（像素）；非正数 / 非有限值 = **不折行**（只按 `\n` 切）；
/// - `measure`：测量函数（通常是 `TextMetrics::measure_text(..).w`）。
///
/// 返回：至少一行（空文本返回一个空行），每行 `text` 已去掉行尾空格。
pub fn wrap(text: &str, max_w: f32, measure: impl Fn(&str) -> f32) -> Vec<WrappedLine> {
    let wrap_enabled = max_w.is_finite() && max_w > 0.0;
    let mut out = Vec::new();
    let mut para_start = 0usize;
    for para in text.split('\n') {
        let para_end = para_start + para.len();
        if !wrap_enabled {
            out.push(WrappedLine {
                text: para.trim_end().to_string(),
                start: para_start,
            });
        } else {
            wrap_paragraph(para, para_start, max_w, &measure, &mut out);
        }
        para_start = para_end + 1; // 跳过 '\n'
    }
    if out.is_empty() {
        out.push(WrappedLine {
            text: String::new(),
            start: 0,
        });
    }
    out
}

/// 折一个段落（不含 `\n`）。
fn wrap_paragraph(
    para: &str,
    base: usize,
    max_w: f32,
    measure: &impl Fn(&str) -> f32,
    out: &mut Vec<WrappedLine>,
) {
    let mut line_start = 0usize; // 当前行在 `para` 内的字节起点
    let mut last_break: Option<usize> = None; // 最近一个可断点（空格之后）
    let mut cur_end = 0usize; // 当前行已纳入的字节终点

    let chars: Vec<(usize, char)> = para.char_indices().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let (offset, ch) = chars[i];
        let next_offset = offset + ch.len_utf8();
        // 候选行：line_start..next_offset
        let candidate = &para[line_start..next_offset];
        if measure(candidate.trim_end()) <= max_w || candidate.trim_end().is_empty() {
            if ch == ' ' {
                last_break = Some(next_offset);
            }
            cur_end = next_offset;
            i += 1;
            continue;
        }
        // 放不下了：在最近的断点处切（没有断点就按字符切）
        let cut = match last_break {
            Some(b) if b > line_start => b,
            // 当前行至少有一个字符：切在它之前，保证不空转
            _ if cur_end > line_start => cur_end,
            // 单字符就超宽：让这个字符单独成行（宁可超宽，也不丢字符/死循环）
            _ => next_offset,
        };
        out.push(WrappedLine {
            text: para[line_start..cut].trim_end().to_string(),
            start: base + line_start,
        });
        line_start = cut;
        // 跳掉切点处的空格，避免下一行以空格开头
        while line_start < para.len() && para[line_start..].starts_with(' ') {
            line_start += 1;
        }
        last_break = None;
        cur_end = line_start;
        i = 0; // 从新行首重新扫（简单、正确；折行不是热路径）
               // 对齐 i 到 line_start
        while i < chars.len() && chars[i].0 < line_start {
            i += 1;
        }
    }
    // 收尾：line_start 已经在末尾时不要再补一个空行（否则每段末尾多一行）。
    if line_start < para.len() {
        out.push(WrappedLine {
            text: para[line_start..].trim_end().to_string(),
            start: base + line_start,
        });
    } else if para.is_empty() {
        out.push(WrappedLine {
            text: String::new(),
            start: base,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 等宽测量：每字符 10px（便于手算）。
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    fn texts(lines: &[WrappedLine]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn short_text_stays_on_one_line() {
        let out = wrap("hello", 100.0, mono);
        assert_eq!(texts(&out), vec!["hello"]);
        assert_eq!(out[0].start, 0);
    }

    #[test]
    fn wraps_at_spaces_and_never_splits_a_word() {
        // 每词 5 字符 = 50px；宽度 60 → 一行一个词。
        let out = wrap("aaaaa bbbbb ccccc", 60.0, mono);
        assert_eq!(texts(&out), vec!["aaaaa", "bbbbb", "ccccc"]);
        // start 指向每行首字符在原文里的位置
        assert_eq!(out[1].start, 6);
        assert_eq!(out[2].start, 12);
    }

    #[test]
    fn trailing_spaces_are_trimmed_from_wrapped_lines() {
        let out = wrap("aa bb", 50.0, mono);
        for l in &out {
            assert!(!l.text.ends_with(' '), "行尾不该留下空格：{l:?}");
        }
    }

    #[test]
    fn cjk_without_spaces_breaks_by_character() {
        // 6 个汉字 ×10px，宽度 30 → 每行 3 个。
        let out = wrap("中文折行测试", 30.0, mono);
        assert_eq!(texts(&out), vec!["中文折", "行测试"]);
    }

    #[test]
    fn explicit_newlines_are_preserved() {
        let out = wrap("ab\ncd", 100.0, mono);
        assert_eq!(texts(&out), vec!["ab", "cd"]);
        assert_eq!(out[1].start, 3);
    }

    #[test]
    fn non_positive_or_infinite_width_disables_wrapping() {
        let long = "aaaaaaaaaa bbbbbbbbbb";
        assert_eq!(texts(&wrap(long, 0.0, mono)), vec![long]);
        assert_eq!(texts(&wrap(long, -5.0, mono)), vec![long]);
        assert_eq!(texts(&wrap(long, f32::INFINITY, mono)), vec![long]);
        assert_eq!(texts(&wrap(long, f32::NAN, mono)), vec![long]);
    }

    #[test]
    fn single_char_wider_than_the_line_still_gets_its_own_line() {
        // 宽度 5px，而每个字符 10px：不许死循环，也不许丢字符。
        let out = wrap("abc", 5.0, mono);
        assert_eq!(texts(&out), vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_text_yields_one_empty_line() {
        let out = wrap("", 50.0, mono);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "");
    }

    #[test]
    fn text_that_exactly_fits_is_not_wrapped() {
        // 10 字符 = 100px，宽度正好 100 → 不折。
        let out = wrap("abcdefghij", 100.0, mono);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn wrapping_is_lossless_except_for_collapsed_spaces() {
        // 每个非空格字符都要出现且顺序不变（折行不能吞字）。
        let src = "alpha beta gamma delta";
        let out = wrap(src, 60.0, mono);
        let joined: String = out
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let strip = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(strip(&joined), strip(src));
    }
}
