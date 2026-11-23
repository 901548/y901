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

/// 把一段文字按 **Bidi 视觉 run** 切分，返回 `(字节区间, 是否 RTL)`，**已按视觉顺序排列**。
///
/// 用途：shaping 之前必须先按方向切段 —— 一段里既有 LTR 又有 RTL 时，
/// 整段交给 shaper 会把它们按同一方向排（`"hello שלום"` 会把希伯来段当成拉丁段排），
/// 跨段的视觉顺序也无从体现。`unicode_bidi` 的 `visual_runs` 正好给出这个切分。
///
/// 返回的区间在**逻辑**上连续且互不重叠；顺序就是视觉顺序（从左到右）。
pub fn visual_runs_of(text: &str) -> Vec<(std::ops::Range<usize>, bool)> {
    if text.is_empty() {
        return Vec::new();
    }
    let info = BidiInfo::new(text, None);
    let Some(para) = info.paragraphs.first() else {
        return vec![(0..text.len(), false)];
    };
    let (levels, runs) = info.visual_runs(para, para.range.clone());
    runs.iter()
        .map(|r| {
            let rtl = levels.get(r.start).map(|l| l.is_rtl()).unwrap_or(false);
            (r.clone(), rtl)
        })
        .collect()
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
    fn visual_runs_split_mixed_directions_in_visual_order() {
        let text = "hello שלום";
        let runs = visual_runs_of(text);
        assert!(runs.len() >= 2, "混排应至少切成两段，实际 {runs:?}");
        // 视觉顺序：拉丁在前、希伯来在后
        assert!(!runs[0].1, "第一段应是 LTR：{runs:?}");
        assert!(runs.last().unwrap().1, "最后一段应是 RTL：{runs:?}");
        // 区间必须覆盖整串且不重叠
        let mut pos = 0usize;
        let mut logical: Vec<_> = runs.iter().map(|(r, _)| r.clone()).collect();
        logical.sort_by_key(|r| r.start);
        for r in &logical {
            assert_eq!(r.start, pos, "区间应连续覆盖：{logical:?}");
            pos = r.end;
        }
        assert_eq!(pos, text.len(), "区间应覆盖到末尾");
    }

    #[test]
    fn pure_rtl_is_one_rtl_run() {
        let runs = visual_runs_of("שלום");
        assert_eq!(runs.len(), 1, "单方向应只有一段：{runs:?}");
        assert!(runs[0].1, "应为 RTL");
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
