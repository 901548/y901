//! **字形集群边界**（grapheme-ish clusters）与光标吸附。
//!
//! ## 为什么 UI 层需要它
//!
//! 文本编辑器的光标**不能落在集群中间**：`कि`（辅音 + 元音符号）、`e` + 组合重音、
//! `👩‍👩‍👧` 这类序列如果在中间插光标，后续编辑会把它们拆成两个孤立字符 —— 视觉上
//! 就是"删一个符号却删掉了半个字"。
//!
//! 本模块提供**最小可用**的集群判定（不是完整的 UAX #29）：
//!
//! 1. 组合记号（Mn/Mc/Me 的主要区间）及其后的零宽连接符不单独成界；
//! 2. 同一脚本的连续字符归入同一群；
//! 3. 空白与脚本切换处成界。
//!
//! 覆盖"阿拉伯连写 + 印度语系音节 + 拉丁重音"这三类 shaping 场景；
//! emoji ZWJ 序列/旗帜等由后续切片补齐（注释里写清，不假装支持）。

/// 判断是否为**组合记号**（组合附加符号、元音符号、零宽连接符）。
///
/// 为什么不用 `char::is_combining_mark`：标准库没有稳定 API；这里按 Unicode 的
/// Mn/Me/Mc 主要区块判断。漏判的后果只是"集群切细一点"，不会画错或删错。
pub fn is_combining_mark(ch: char) -> bool {
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

/// 集群边界（升序，含 `0` 与文本长度）。
///
/// 规则（**按字素簇而不是按单词**）：
/// - **每个字符都成界**，除非它是组合记号 / ZWJ / ZWNJ，或它的前一个字符是 ZWJ
///   （emoji 的连接序列）；
/// - 于是阿拉伯语是"每个字母一簇"（Backspace 删一个字母，符合编辑器惯例），
///   而 `कि`、`e◌́` 这类"基础字符 + 附加符号"是一簇（不会删掉半个音节）。
///
/// 早期实现按"同一脚本归一群"切分，那会让 `abc` 整体成为一个簇、←/→ 直接跳过整个词，
/// 与用户预期相反 —— 已改掉。
pub fn cluster_boundaries(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    let mut prev_was_zwj = false;
    for (i, ch) in text.char_indices() {
        if i > 0 && !is_combining_mark(ch) && !prev_was_zwj {
            out.push(i);
        }
        prev_was_zwj = ch == '\u{200D}';
    }
    out.push(text.len());
    out.dedup();
    out
}

/// 把任意字节位置**吸附到最近的集群边界**（优先向前，避免越过用户想放的位置）。
pub fn snap(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if byte == 0 || byte >= text.len() {
        return byte.min(text.len());
    }
    let bounds = cluster_boundaries(text);
    // 落在边界上就原样返回；否则退到前一个边界。
    match bounds.binary_search(&byte) {
        Ok(_) => byte,
        Err(idx) => bounds.get(idx.saturating_sub(1)).copied().unwrap_or(0),
    }
}

/// 前一个集群边界（用于 ←/Backspace）。已在开头返回 `None`。
pub fn prev_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = snap(text, byte);
    let bounds = cluster_boundaries(text);
    bounds.iter().rev().copied().find(|b| *b < byte)
}

/// 后一个集群边界（用于 →/Delete）。已在末尾返回 `None`。
pub fn next_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = snap(text, byte);
    let bounds = cluster_boundaries(text);
    bounds.iter().copied().find(|b| *b > byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_has_one_boundary_per_char_plus_ends() {
        assert_eq!(cluster_boundaries("abc"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn arabic_letters_are_individual_clusters() {
        // 与编辑器惯例一致：Backspace 删一个字母，而不是整个词。
        let text = "مرحبا";
        let b = cluster_boundaries(text);
        assert_eq!(b.len(), 6, "5 个字母 → 6 个边界，实得 {b:?}");
    }

    #[test]
    fn devanagari_vowel_sign_stays_with_its_consonant() {
        // "कि"：辅音 + 元音符号，必须同一集群（否则 Backspace 会拆掉半个音节）。
        let text = "कि";
        let b = cluster_boundaries(text);
        assert_eq!(b, vec![0, text.len()], "实得 {b:?}");
    }

    #[test]
    fn combining_accent_attaches_to_the_letter() {
        let text = "e\u{0301}x";
        let b = cluster_boundaries(text);
        assert_eq!(b, vec![0, 3, 4], "e+重音应同群，x 单群；实得 {b:?}");
    }

    #[test]
    fn snap_never_lands_inside_a_cluster() {
        let text = "कि";
        // 任意中间位置都必须吸附到 0 或末尾。
        for i in 0..=text.len() {
            let s = snap(text, i);
            assert!(s == 0 || s == text.len(), "snap({i}) = {s}");
        }
    }

    #[test]
    fn snap_clamps_out_of_range_input() {
        assert_eq!(snap("abc", 99), 3);
        assert_eq!(snap("abc", 0), 0);
        assert_eq!(snap("", 5), 0);
    }

    #[test]
    fn prev_and_next_boundary_walk_over_whole_clusters() {
        let text = "e\u{0301}x";
        assert_eq!(
            prev_boundary(text, 3),
            Some(0),
            "从 x 往左应跨过整个 e+重音"
        );
        assert_eq!(
            next_boundary(text, 0),
            Some(3),
            "从 0 往右应跨过整个 e+重音"
        );
        assert_eq!(prev_boundary(text, 0), None);
        assert_eq!(next_boundary(text, text.len()), None);
    }

    #[test]
    fn empty_text_has_one_boundary_and_no_movement() {
        assert_eq!(cluster_boundaries(""), vec![0]);
        assert_eq!(prev_boundary("", 0), None);
        assert_eq!(next_boundary("", 0), None);
    }

    #[test]
    fn orbiting_spaces_create_boundaries() {
        let b = cluster_boundaries("ab cd");
        assert!(b.contains(&3), "空格处应成界：{b:?}");
    }
}
