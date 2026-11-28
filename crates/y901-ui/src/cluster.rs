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

/// ZWJ 之后的字符是否**仍然属于同一个簇**（emoji/连字序列）。
///
/// 为什么不一律"ZWJ 后都同簇"（旧实现）：`\u{200D}` 后面跟制表符、换行、空格时，
/// 那不是 emoji 连接序列 —— 一律同簇会让 **Backspace 一次删掉"👨‍" + 制表符**，
/// 也会让 ←/→ 跨过空白落到意外的位置。UAX #29 里 ZWJ 只对 Extended_Pictographic
/// 生效（GB11），控制/空白永远成界（GB4/GB5）。
///
/// 这里用"非控制、非空白、非 ASCII"作近似：emoji 与 CJK 全在 U+1F000 以上或全角区，
/// 拉丁字母/标点则应当断开。判错的代价只是"簇切得细一点"，不会丢字符。
fn joins_after_zwj(ch: char) -> bool {
    !ch.is_control() && !ch.is_whitespace() && !ch.is_ascii()
}
/// 集群边界（升序，含 `0` 与文本长度）。
///
/// 规则（**按字素簇而不是按单词**）：
/// - **每个字符都成界**，除非它是组合记号 / ZWJ / ZWNJ，或它前一个是 ZWJ **且**
///   它本身不是控制/空白/ASCII（真正的 emoji 连接序列）；`ZWJ + 制表符` 不是一簇；
/// - 于是阿拉伯语是"每个字母一簇"（Backspace 删一个字母，符合编辑器惯例），
///   而 `कि`、`e◌́` 这类"基础字符 + 附加符号"是一簇（不会删掉半个音节）。
///
/// 早期实现按"同一脚本归一群"切分，那会让 `abc` 整体成为一个簇、←/→ 直接跳过整个词，
/// 与用户预期相反 —— 已改掉。
pub fn cluster_boundaries(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    let mut prev_was_zwj = false;
    for (i, ch) in text.char_indices() {
        if i > 0 {
            let joins = is_combining_mark(ch) || (prev_was_zwj && joins_after_zwj(ch));
            if !joins {
                out.push(i);
            }
        }
        prev_was_zwj = ch == '\u{200D}';
    }
    out.push(text.len());
    out.dedup();
    out
}

/// `i` 是否是集群边界（`i` 必须是某个字符的起始偏移，且 `0 < i < len`）。
///
/// 与 [`cluster_boundaries`] 的推导规则**逐字对应**：
/// 位置 `i` 成界 ⟺ `i` 处的字符不是组合记号 ⟺ 且它前面那个字符不是 ZWJ。
#[inline]
fn is_boundary_at(text: &str, i: usize) -> bool {
    debug_assert!(text.is_char_boundary(i));
    if i == 0 || i >= text.len() {
        return false; // 两端由调用方单独处理（0 恒为界，len 也是界）
    }
    let ch = text[i..].chars().next().unwrap_or('\0');
    if is_combining_mark(ch) {
        return false;
    }
    // ZWJ 之后只有"非控制/空白/ASCII"才继续同簇（见 `joins_after_zwj`）。
    if text[..i].ends_with('\u{200D}') {
        return !joins_after_zwj(ch);
    }
    true
}

/// 从 `byte` 往回找最近的边界（含 `byte` 自己）。
///
/// 复杂度：O(一个簇的长度) —— 常见情况是 1 个字符（拉丁/中文/阿拉伯），
/// 最坏是一个超长组合记号序列。**不分配内存**（旧实现每次建整个 `Vec<usize>`）。
fn boundary_at_or_before(text: &str, byte: usize) -> usize {
    let mut end = byte;
    while end > 0 {
        let Some((start, _)) = text[..end].char_indices().next_back() else {
            return 0;
        };
        if start == 0 {
            return 0; // 0 恒为界
        }
        if is_boundary_at(text, start) {
            return start;
        }
        end = start;
    }
    0
}

/// 把任意字节位置**吸附到最近的集群边界**（优先向前，避免越过用户想放的位置）。
///
/// 不分配内存：只为"调用它的那条光标"走一个簇。
pub fn snap(text: &str, byte: usize) -> usize {
    let byte = byte.min(text.len());
    if byte == 0 {
        return 0;
    }
    if byte >= text.len() {
        return text.len();
    }
    // 落在字符中间时先退到字符起始，再看它是不是簇边界。
    let mut at = byte;
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    if text.is_char_boundary(at) && (is_boundary_at(text, at) || at == 0) {
        return at;
    }
    boundary_at_or_before(text, at)
}

/// 前一个集群边界（用于 ←/Backspace）。已在开头返回 `None`。
pub fn prev_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = snap(text, byte);
    if byte == 0 {
        return None;
    }
    // `snap` 已经把位置对齐到边界，往回走一个字符再找边界即可。
    let mut end = byte;
    while end > 0 {
        let (start, _) = text[..end].char_indices().next_back()?;
        if start == 0 {
            return Some(0); // 0 恒为界，且 `0 < byte`
        }
        if is_boundary_at(text, start) {
            return Some(start);
        }
        end = start;
    }
    Some(0)
}

/// 后一个集群边界（用于 →/Delete）。已在末尾返回 `None`。
pub fn next_boundary(text: &str, byte: usize) -> Option<usize> {
    let byte = snap(text, byte);
    if byte >= text.len() {
        return None;
    }
    // 从 `byte` 之后逐字符前进；每个候选位置自己判断是否成界。
    let mut it = text[byte..].char_indices();
    // 跳过起点自身（`byte` 已经是边界）。
    it.next();
    for (offset, ch) in it {
        let i = byte + offset;
        // i == len 时 `chars().next()` 为空 → 不可能是组合记号，直接用规则判定。
        let _ = ch;
        // 统一走 `is_boundary_at`：规则只写一处，两条实现路径（走簇 / 物化表）不会分叉。
        if is_boundary_at(text, i) {
            return Some(i);
        }
    }
    // 走到头：`len` 本身是边界。
    Some(text.len())
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

    /// 旧实现（物化整个 `Vec`）的逐字副本，只用于**对拍**新实现。
    ///
    /// 为什么保留它：`snap` / `prev_boundary` / `next_boundary` 改成"按簇游走、不分配"
    /// 之后，正确性不再能靠"读代码"保证 —— 需要一个独立参照物，在每个字节位置逐一对比。
    fn reference_snap_prev_next(text: &str, byte: usize) -> (usize, Option<usize>, Option<usize>) {
        fn bounds(text: &str) -> Vec<usize> {
            let mut out = vec![0usize];
            let mut prev_was_zwj = false;
            for (i, ch) in text.char_indices() {
                if i > 0 {
                    // 与 `cluster_boundaries` **同一规则**（含 `joins_after_zwj` 的细化）。
                    let joins = is_combining_mark(ch) || (prev_was_zwj && joins_after_zwj(ch));
                    if !joins {
                        out.push(i);
                    }
                }
                prev_was_zwj = ch == '\u{200D}';
            }
            out.push(text.len());
            out.dedup();
            out
        }
        let byte = byte.min(text.len());
        let b = bounds(text);
        let snapped = if byte == 0 || byte >= text.len() {
            byte
        } else {
            match b.binary_search(&byte) {
                Ok(_) => byte,
                Err(idx) => b.get(idx.saturating_sub(1)).copied().unwrap_or(0),
            }
        };
        let prev = b.iter().rev().copied().find(|x| *x < snapped);
        let next = b.iter().copied().find(|x| *x > snapped);
        (snapped, prev, next)
    }

    #[test]
    fn allocation_free_walk_matches_the_materialised_reference() {
        // 覆盖面：ASCII（每字符一簇）、组合重音、天城文、阿拉伯文、ZWJ emoji 序列、
        // 以及"零散空格 + 组合记号"的混合串。
        let corpus = [
            "",
            "abc",
            "ab cd",
            "e\u{301}",
            "e\u{301}x\u{308}",
            "\u{915}\u{93F}\u{930}\u{94D}\u{92F}",
            "\u{645}\u{631}\u{62D}\u{628}\u{627}",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "a\u{200D}b\u{200C}c",
            "  \u{301}  ",
        ];
        for text in corpus {
            for byte in 0..=text.len() {
                let (want_snap, want_prev, want_next) = reference_snap_prev_next(text, byte);
                assert_eq!(snap(text, byte), want_snap, "snap({text:?}, {byte})");
                assert_eq!(
                    prev_boundary(text, byte),
                    want_prev,
                    "prev_boundary({text:?}, {byte})"
                );
                assert_eq!(
                    next_boundary(text, byte),
                    want_next,
                    "next_boundary({text:?}, {byte})"
                );
            }
        }
    }

    #[test]
    fn walk_stays_correct_on_a_long_combining_run() {
        // 一个"基字 + 500 个组合记号"的超长簇：`snap` 必须落回基字起点，
        // `next_boundary` 必须一步跨到簇尾（不能因为簇长就退化成按字符走）。
        let mut text = String::from("a");
        for _ in 0..500 {
            text.push('\u{301}');
        }
        text.push('b');
        // "a" + 500 个组合记号（各 2 字节）= 1001 字节，'b' 从 1001 开始。
        assert_eq!(snap(&text, 250), 0, "簇中间必须吸附到簇首");
        assert_eq!(next_boundary(&text, 0), Some(1001), "下一步应一次跨到 'b'");
        assert_eq!(prev_boundary(&text, text.len()), Some(1001));
        let huge = "x".repeat(200_000);
        // 长 ASCII 文本上做 1000 次移动：旧实现每次物化一个 20 万元素的 Vec
        //（约 1.6 MB 分配 × 1000 次），新实现只走一个字符。
        let mut pos = huge.len();
        for _ in 0..1000 {
            pos = prev_boundary(&huge, pos).unwrap_or(0);
        }
        assert_eq!(pos, huge.len() - 1000, "1000 次 ← 应正好后退 1000 个字符");
    }

    /// **`ZWJ` 之后跟控制符/空白时必须断开**（UAX #29 的 GB4/GB5/GB11）。
    ///
    /// 旧实现一律"前一个是 ZWJ ⇒ 同簇"，于是 `👨‍\t`（emoji + ZWJ + 制表符）被当成
    /// 一个簇：Backspace 一次会**连制表符一起删掉**，←/→ 也会跨过空白。
    #[test]
    fn zwj_does_not_glue_control_or_space_into_the_same_cluster() {
        // ZWJ + 制表符：制表符必须自己成界（可以单独删）
        let text = "\u{1F468}\u{200D}\t";
        let b = cluster_boundaries(text);
        assert!(
            b.contains(&(text.len() - 1)),
            "制表符之前必须成界（否则它和前一个 emoji 粘成一簇）：{b:?}"
        );
        // 于是从末尾退格一次只删掉制表符
        assert_eq!(
            prev_boundary(text, text.len()),
            Some(text.len() - 1),
            "Backspace 只能删掉制表符，不能连 emoji 一起删"
        );

        // ZWJ + 换行、ZWJ + 空格同理
        for tail in ["\n", " "] {
            let t = format!("\u{1F468}\u{200D}{tail}");
            let b = cluster_boundaries(&t);
            assert!(
                b.contains(&(t.len() - tail.len())),
                "ZWJ 后的 {tail:?} 必须成界：{b:?}"
            );
        }
    }

    /// 真正的 emoji 连接序列仍然是一簇（`👨‍👩` 不能被劈开）。
    #[test]
    fn emoji_zwj_sequence_stays_one_cluster() {
        let text = "\u{1F468}\u{200D}\u{1F469}";
        let b = cluster_boundaries(text);
        assert_eq!(b, vec![0, text.len()], "emoji ZWJ 序列必须是一簇：{b:?}");
        assert_eq!(prev_boundary(text, text.len()), Some(0));
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
