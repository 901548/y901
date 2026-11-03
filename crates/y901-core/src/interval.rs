//! 一维区间集合：不相交的 `[start, end)` 段，支持插入去重与"找连续空位"。
//!
//! 为什么放在 `y901-core` 而不是 `y901-ui`：它只依赖标准库与整数，
//! 但至少三处要用同一套语义——
//!
//! - `Grid` 的跨格冲突检测与自动流找位（列/行掩码的一维模型）；
//! - `TreeView` 虚拟化：可见行区间与"未渲染行"的求差；
//! - 语义树合并：多个窗口暴露的节点集合求并。
//!
//! 三处如果各写一遍，边界条件（相邻算不算重叠、零长度请求算什么）必然出现分歧，
//! 所以这里把语义**固定下来并写成测试**。

/// 一段半开区间 `[start, end)`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: u32,
    end: u32,
}

/// 插入失败的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntervalError {
    /// `start >= end`：空区间或反向区间没有意义。
    EmptyRange {
        /// 调用方给的起点。
        start: u32,
        /// 调用方给的终点。
        end: u32,
    },
    /// 与已有区间重叠。
    ///
    /// 带上**冲突双方的区间**：光说"冲突"无法定位是哪个控件压了哪个，
    /// 而跨格冲突的排查恰恰要这一条信息。
    Overlap {
        /// 本次想插入的区间起点。
        start: u32,
        /// 本次想插入的区间终点。
        end: u32,
        /// 与它重叠的既有区间起点。
        existing_start: u32,
        /// 与它重叠的既有区间终点。
        existing_end: u32,
    },
}

/// 不相交区间的有序集合（相邻段自动合并）。
///
/// 不变量：`spans` 按 `start` 升序，且任意两段的 `end <= 下一段的 start`；
/// 相邻（`end == 下一段的 start`）会被**合并**——如果不合并，
/// `first_gap` 会在两段之间认为存在一个"长度为 0 的空位"，毫无意义。
#[derive(Clone, Debug, Default)]
pub struct IntervalSet {
    spans: Vec<Span>,
}

impl IntervalSet {
    /// 建立空集合。
    pub fn new() -> Self {
        Self::default()
    }

    /// 已占用的段数（合并后的数量，不是插入次数）。
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// 是否还没有任何段。
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// 插入 `[start, end)`；与已有段重叠时返回 `Overlap` 且**不改动集合**。
    ///
    /// 相邻段（`end == 另一段的 start`）会被合并成一个连续段。
    pub fn insert(&mut self, start: u32, end: u32) -> Result<(), IntervalError> {
        if start >= end {
            return Err(IntervalError::EmptyRange { start, end });
        }

        // 找到第一个"结束位置不晚于 start"的既有段位置。
        // 注意是 `<=` 而非 `<`：`end == start` 是**相邻**（要合并），不是重叠。
        let pos = self.spans.partition_point(|s| s.end <= start);

        // 检查与前后是否重叠（pos 之前那段可能延伸到 start 之后）
        if let Some(prev) = pos.checked_sub(1).and_then(|i| self.spans.get(i)) {
            if prev.end > start {
                return Err(IntervalError::Overlap {
                    start,
                    end,
                    existing_start: prev.start,
                    existing_end: prev.end,
                });
            }
        }
        if let Some(next) = self.spans.get(pos) {
            if next.start < end {
                return Err(IntervalError::Overlap {
                    start,
                    end,
                    existing_start: next.start,
                    existing_end: next.end,
                });
            }
        }

        // 合并相邻段
        let mut new_start = start;
        let mut new_end = end;
        let mut remove_from = pos;
        let mut remove_count = 0;
        if let Some(prev) = pos.checked_sub(1).and_then(|i| self.spans.get(i)) {
            if prev.end == start {
                new_start = prev.start;
                remove_from = pos - 1;
                remove_count += 1;
            }
        }
        if let Some(next) = self.spans.get(pos) {
            if next.start == end {
                new_end = next.end;
                remove_count += 1;
            }
        }

        self.spans
            .splice(remove_from..remove_from + remove_count, [Span {
                start: new_start,
                end: new_end,
            }]);
        Ok(())
    }

    /// 第一个能放下长度 `len` 的空位（**不设上界**，找不到就接在末尾）。
    ///
    /// `len == 0` 返回 `Some(0)`：零长度区间在任何位置都成立，
    /// 返回 0 是最小解，也让调用方不必特判。
    pub fn first_gap(&self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let mut cursor: u32 = 0;
        for span in &self.spans {
            if span.start.saturating_sub(cursor) >= len {
                return Some(cursor);
            }
            cursor = cursor.max(span.end);
        }
        // 末尾之后永远有空间
        Some(cursor)
    }

    /// 在 `[from, limit)` 内找第一个能放下 `len` 的空位；放不下返回 `None`。
    ///
    /// 为什么要上界：自动流必须落在**网格已有的行**里，
    /// 不能因为"末尾永远有空"就无限往下长。
    pub fn first_gap_within(&self, from: u32, limit: u32, len: u32) -> Option<u32> {
        if len == 0 {
            return if from <= limit { Some(from) } else { None };
        }
        if from >= limit {
            return None;
        }
        let mut cursor = from;
        let end_limit = limit;
        for span in &self.spans {
            if span.end <= cursor {
                continue;
            }
            if span.start >= end_limit {
                break;
            }
            if span.start.saturating_sub(cursor) >= len {
                return Some(cursor);
            }
            cursor = cursor.max(span.end);
        }
        if end_limit.saturating_sub(cursor) >= len {
            Some(cursor)
        } else {
            None
        }
    }

    /// 已占用的总长度（各段长度之和）。
    pub fn covered_len(&self) -> u32 {
        self.spans
            .iter()
            .map(|s| s.end.saturating_sub(s.start))
            .sum()
    }

    /// 某个位置是否已被占用。
    pub fn contains(&self, point: u32) -> bool {
        self.spans
            .iter()
            .any(|s| s.start <= point && point < s.end)
    }

    /// 按起点升序返回所有段 `(start, end)`。
    ///
    /// 用于诊断与渲染（`Grid` 需要按段算并集矩形），不暴露内部结构。
    pub fn spans(&self) -> Vec<(u32, u32)> {
        self.spans.iter().map(|s| (s.start, s.end)).collect()
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.spans.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_reports_no_span_and_gap_at_zero() {
        let set = IntervalSet::new();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.covered_len(), 0);
        assert_eq!(set.first_gap(3), Some(0), "空集合里第一个空位就是 0");
        assert!(!set.contains(0));
    }

    #[test]
    fn insert_rejects_empty_and_reversed_ranges() {
        let mut set = IntervalSet::new();
        assert_eq!(
            set.insert(5, 5),
            Err(IntervalError::EmptyRange { start: 5, end: 5 })
        );
        assert_eq!(
            set.insert(7, 3),
            Err(IntervalError::EmptyRange { start: 7, end: 3 })
        );
        assert!(set.is_empty(), "失败不应改动集合");
    }

    #[test]
    fn insert_rejects_overlap_and_reports_both_sides() {
        let mut set = IntervalSet::new();
        set.insert(0, 2).expect("首个区间");

        // 与既有段部分重叠
        assert_eq!(
            set.insert(1, 3),
            Err(IntervalError::Overlap {
                start: 1,
                end: 3,
                existing_start: 0,
                existing_end: 2,
            })
        );
        // 被既有段完全包住
        assert_eq!(
            set.insert(0, 1),
            Err(IntervalError::Overlap {
                start: 0,
                end: 1,
                existing_start: 0,
                existing_end: 2,
            })
        );
        // 覆盖既有段
        assert_eq!(
            set.insert(0, 9),
            Err(IntervalError::Overlap {
                start: 0,
                end: 9,
                existing_start: 0,
                existing_end: 2,
            })
        );
        assert_eq!(set.spans(), vec![(0, 2)], "冲突后集合保持不变");
    }

    #[test]
    fn adjacent_spans_merge_into_one() {
        let mut set = IntervalSet::new();
        set.insert(0, 2).expect("左段");
        set.insert(2, 5).expect("右段应合并");
        assert_eq!(set.len(), 1, "相邻段必须合并，否则会出现长度 0 的假空位");
        assert_eq!(set.spans(), vec![(0, 5)]);
        assert_eq!(set.covered_len(), 5);

        // 反向相邻（插在前面）
        set.insert(5, 6).expect("接尾");
        assert_eq!(set.spans(), vec![(0, 6)]);
    }

    #[test]
    fn insert_bridges_two_spans() {
        let mut set = IntervalSet::new();
        set.insert(0, 2).expect("a");
        set.insert(4, 6).expect("b");
        set.insert(2, 4).expect("中间的桥应把两段连起来");
        assert_eq!(set.spans(), vec![(0, 6)], "应合并成一段");
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn out_of_order_inserts_stay_sorted() {
        let mut set = IntervalSet::new();
        for (s, e) in [(10, 12), (0, 2), (5, 7), (20, 21)] {
            set.insert(s, e).expect("互不重叠");
        }
        assert_eq!(set.spans(), vec![(0, 2), (5, 7), (10, 12), (20, 21)]);
        assert_eq!(set.covered_len(), 2 + 2 + 2 + 1);
    }

    #[test]
    fn first_gap_finds_the_first_fitting_hole() {
        let mut set = IntervalSet::new();
        set.insert(0, 2).expect("a");
        set.insert(5, 7).expect("b");

        assert_eq!(set.first_gap(1), Some(2), "2..5 之间有 3 格");
        assert_eq!(set.first_gap(3), Some(2), "正好放满 2..5");
        assert_eq!(set.first_gap(4), Some(7), "3 格不够，应跳到末尾之后");
        assert_eq!(set.first_gap(0), Some(0), "零长度 → 0");
    }

    #[test]
    fn first_gap_within_respects_the_limit() {
        let mut set = IntervalSet::new();
        set.insert(0, 2).expect("a");
        set.insert(4, 6).expect("b");

        assert_eq!(set.first_gap_within(0, 6, 2), Some(2), "2..4 正好");
        assert_eq!(set.first_gap_within(0, 6, 3), None, "6 格内放不下 3 格连续");
        assert_eq!(set.first_gap_within(2, 4, 2), Some(2));
        assert_eq!(set.first_gap_within(4, 6, 1), None, "起点已被占用");
        assert_eq!(set.first_gap_within(9, 9, 1), None, "空区间");
        assert_eq!(set.first_gap_within(0, 6, 0), Some(0), "零长度 → from");
    }

    #[test]
    fn contains_uses_half_open_semantics() {
        let mut set = IntervalSet::new();
        set.insert(2, 5).expect("a");
        assert!(!set.contains(1));
        assert!(set.contains(2), "左端含");
        assert!(set.contains(4));
        assert!(!set.contains(5), "右端不含——半开区间");
    }

    #[test]
    fn huge_ranges_do_not_overflow() {
        let mut set = IntervalSet::new();
        set.insert(0, u32::MAX).expect("满区间");
        assert_eq!(set.covered_len(), u32::MAX);
        assert_eq!(set.first_gap(1), Some(u32::MAX), "只能接在末尾");
        assert_eq!(set.first_gap_within(0, u32::MAX, 1), None);
    }

    #[test]
    fn clear_resets_everything() {
        let mut set = IntervalSet::new();
        set.insert(0, 4).expect("a");
        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.first_gap(2), Some(0));
    }

    #[test]
    fn inserting_ten_thousand_cells_keeps_one_merged_span() {
        // 模拟一行 10k 格全部被占：合并后应只有 1 段，`len()` 不是 10k
        let mut set = IntervalSet::new();
        for i in 0..10_000u32 {
            set.insert(i, i + 1).expect("连续插入不重叠");
        }
        assert_eq!(set.len(), 1, "相邻段必须合并");
        assert_eq!(set.covered_len(), 10_000);
        assert_eq!(set.first_gap(1), Some(10_000));
    }
}
