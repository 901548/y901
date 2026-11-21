//! 双向文本（Unicode Bidi）：段落方向判定与视觉重排。

use unicode_bidi::BidiInfo;

/// 判定一个段落的**基础方向**：`true` = 从右往左。
///
/// 规则（与 Unicode Bidi 的 P2/P3 一致）：取段落里第一个**强类型**字符
/// （L / R / AL）决定方向；没有强类型字符时按 LTR。
pub fn paragraph_level(text: &str) -> bool {
    let info = BidiInfo::new(text, None);
    info.paragraphs
        .first()
        .map(|p| p.level.is_rtl())
        .unwrap_or(false)
}

/// 把一个段落按视觉顺序重排，返回**逻辑字节下标 → 视觉顺序**的序列。
///
/// 返回值：`Vec<usize>`，`visual[i] = 逻辑字符起始位置`。
/// 纯 LTR 文本返回 `0..n` 的自然顺序；含 RTL 时按 Bidi 算法重排连续段。
///
/// 为什么返回"逻辑下标序列"而不是重排后的字符串：调用方要绘制的是字形，
/// 而字形与逻辑字符的对应关系必须保留（光标、选区、集群都依赖它）。
pub fn reorder_visual(text: &str) -> Vec<usize> {
    let info = BidiInfo::new(text, None);
    let Some(para) = info.paragraphs.first() else {
        return Vec::new();
    };
    // 整段当作一行（本模块不做折行，折行后的重排属于布局层）。
    let line = para.range.clone();
    let (_levels, runs) = info.visual_runs(para, line);
    let mut out = Vec::new();
    for range in runs {
        let level = info.levels[range.start];
        let mut idx: Vec<usize> = text[range.clone()]
            .char_indices()
            .map(|(i, _)| range.start + i)
            .collect();
        // 奇数 level = RTL：该 run 内部按视觉顺序反转。
        if level.is_rtl() {
            idx.reverse();
        }
        out.extend(idx);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_ltr_is_left_to_right() {
        let text = "abc";
        assert!(!paragraph_level(text));
        let visual = reorder_visual(text);
        assert_eq!(visual, vec![0, 1, 2], "纯 LTR 顺序不变");
    }

    #[test]
    fn hebrew_paragraph_is_rtl() {
        // "שלום" = hello
        let text = "שלום";
        assert!(paragraph_level(text), "希伯来语段落应为 RTL");
    }

    #[test]
    fn arabic_paragraph_is_rtl() {
        // "مرحبا" = hello
        assert!(paragraph_level("مرحبا"), "阿拉伯语段落应为 RTL");
    }

    #[test]
    fn mixed_paragraph_takes_first_strong_direction() {
        // 以英文开头、含希伯来语 → 段落仍是 LTR
        assert!(!paragraph_level("hello שלום"));
        // 以希伯来语开头 → RTL
        assert!(paragraph_level("שלום hello"));
    }

    #[test]
    fn rtl_run_is_reversed_in_visual_order() {
        let text = "שלום";
        let visual = reorder_visual(text);
        assert_eq!(visual.len(), 4, "4 个希伯来字符");
        // 视觉顺序应是逻辑顺序的反转
        let mut expected: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        expected.reverse();
        assert_eq!(visual, expected, "RTL run 内部按视觉顺序反转");
    }

    #[test]
    fn empty_text_has_no_visual_order() {
        assert!(!paragraph_level(""));
        assert!(reorder_visual("").is_empty());
    }

    #[test]
    fn digits_inside_rtl_keep_ltr_order() {
        // 数字是弱类型：在 RTL 段落里仍按 LTR 显示（Bidi 的 EN 规则）。
        let text = "שלום 123";
        let visual = reorder_visual(text);
        // 取出数字在视觉序列里的相对顺序：1,2,3 应保持递增
        let digits: Vec<usize> = visual
            .iter()
            .copied()
            .filter(|i| text[*i..].starts_with(['1', '2', '3']))
            .collect();
        let positions: Vec<usize> = digits
            .iter()
            .map(|i| text[*i..].chars().next().unwrap().to_digit(10).unwrap() as usize)
            .collect();
        assert_eq!(positions, vec![1, 2, 3], "数字在视觉顺序里保持 123");
    }
}
