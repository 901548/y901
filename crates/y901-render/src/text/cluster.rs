//! 字形集群（cluster）与光标映射。
//!
//! shaping 之后"一个字形"可能对应"多个字符"（连字、印度语系音节），
//! 反之亦然（一个字符拆成多个字形）。因此**光标不能按字形数走**，必须按集群。

use unicode_script::{Script, UnicodeScript};

/// 字节偏移处的**集群边界**列表（升序，含 0 与文本长度）。
///
/// 规则（最小可用版）：
/// 1. 组合字符（`Mn`/`Mc`/`Me` 与零宽连接符）不单独成界；
/// 2. 同一脚本的连续字符归入同一集群，直到脚本切换；
/// 3. 空格与标点单独成界（便于"按词删除"）。
///
/// 这不是完整的字素簇（UAX #29）实现：emoji ZWJ 序列、旗帜等由后续切片补齐，
/// 但对"阿拉伯连字 + 印度语系音节"这类 shaping 场景已经够用。
pub fn cluster_boundaries(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    let mut prev_script: Option<Script> = None;
    let mut prev_was_space = false;
    for (i, ch) in text.char_indices() {
        if i == 0 {
            prev_script = Some(ch.script());
            prev_was_space = ch.is_whitespace();
            continue;
        }
        let script = ch.script();
        let is_space = ch.is_whitespace();
        let is_mark = is_combining_mark(ch);
        // **组合记号永远不单独成界**：它是前一个字素的一部分（元音符号、变音符）。
        // 其余情况在"空白边界"或"脚本切换"处切分。
        let boundary =
            !is_mark && (is_space || prev_was_space || script != prev_script.unwrap_or(script));
        if boundary {
            out.push(i);
        }
        prev_script = Some(script);
        prev_was_space = is_space;
    }
    out.push(text.len());
    out.dedup();
    out
}

/// 视觉位置（集群序号）→ 逻辑字节偏移。
pub fn visual_to_logical(text: &str, cluster_index: usize) -> usize {
    let bounds = cluster_boundaries(text);
    match bounds.get(cluster_index) {
        Some(v) => *v,
        None => text.len(),
    }
}

/// 是否为组合记号（粗略判定：Unicode 组合类别的常见码位区间）。
///
/// 为什么不用 `char::is_combining_mark`：标准库没有稳定 API；
/// 这里按 Unicode 的 Mn/Me/Mc 主要区块判断，覆盖阿拉伯语变音符、天城文元音符号、
/// 泰文声调符等 shaping 必需场景。漏判的后果只是"集群切得细一点"，不会画错。
fn is_combining_mark(ch: char) -> bool {
    let c = ch as u32;
    matches!(c,
        0x0300..=0x036F   // Combining Diacritical Marks
        | 0x0483..=0x0489 // Cyrillic combining
        | 0x0591..=0x05BD // Hebrew points
        | 0x0610..=0x061A // Arabic marks
        | 0x064B..=0x065F // Arabic vowel marks
        | 0x0670
        | 0x06D6..=0x06DC
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x0900..=0x0903 // Devanagari signs
        | 0x093A..=0x094F // Devanagari vowel signs (incl. virama)
        | 0x0951..=0x0957
        | 0x0962..=0x0963
        | 0x0E31 | 0x0E34..=0x0E3A // Thai
        | 0x0E47..=0x0E4E
        | 0x1AB0..=0x1AFF // Combining Diacritical Marks Extended
        | 0x20D0..=0x20F0 // Combining Diacritical Marks for Symbols
        | 0xFE00..=0xFE0F // Variation Selectors
        | 0xFE20..=0xFE2F // Combining Half Marks
    ) || ch == '\u{200D}' // ZWJ
        || ch == '\u{200C}' // ZWNJ
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_words_split_on_spaces() {
        let text = "hello world";
        let b = cluster_boundaries(text);
        assert_eq!(b.first(), Some(&0));
        assert_eq!(b.last(), Some(&text.len()));
        assert!(b.contains(&6), "空格处应成界：{b:?}");
    }

    #[test]
    fn arabic_letters_form_one_cluster() {
        // "مرحبا" —— 同一脚本、无空格，应是一个集群。
        let text = "مرحبا";
        let b = cluster_boundaries(text);
        assert_eq!(b, vec![0, text.len()], "阿拉伯词不应被拆成单字符");
    }

    #[test]
    fn devanagari_syllable_keeps_marks_attached() {
        // "कि" = ka + i-matra：元音符号必须和辅音同属一个集群。
        let text = "कि";
        let b = cluster_boundaries(text);
        assert_eq!(b, vec![0, text.len()], "元音符号不能单独成界：{b:?}");
    }

    #[test]
    fn script_switch_starts_a_new_cluster() {
        let text = "hiمرحبا";
        let b = cluster_boundaries(text);
        assert!(b.len() >= 3, "拉丁与阿拉伯之间应有边界：{b:?}");
        assert!(b.contains(&2), "脚本切换点 2 应在边界里：{b:?}");
    }

    #[test]
    fn visual_to_logical_clamps_past_the_end() {
        let text = "abc";
        assert_eq!(visual_to_logical(text, 0), 0);
        assert_eq!(visual_to_logical(text, 999), text.len());
    }

    #[test]
    fn empty_text_has_one_boundary() {
        assert_eq!(cluster_boundaries(""), vec![0]);
    }
}
