//! **`Table` 的列布局与选中不变量**（属性测试，确定性伪随机）。
//!
//! 表格有两个容易出事的地方，都属于"看得出但说不清"的类型：
//!
//! 1. **列宽总额**：`column_layout()` 同时供绘制、命中测试与表头使用。一旦总宽超过
//!    行区域，最后一列会画在可视区之外 —— 命中测试却仍指向那块看不见的地方。
//!    2026-11-28 这里抓到过真 bug：定宽列本身超宽时"超宽按比例压缩"被跳过
//!    （判据用了 `rest`，而定宽列超宽时 `rest` 恰好是 0）。
//! 2. **选中下标**：`set_rows` 变短、排序改变显示顺序、`select` 越界 —— 选中若越界，
//!    键盘导航会读到越界行。
//!
//! 约定：**显式 `Px` 定宽可以故意超出**（用户要 114px 而表只有 102px，我们不横向滚动，
//! 尊重用户的宽度并裁掉超出部分）；除此之外一律不得超出行区域。

use y901_ui::layout::LayoutCache;
use y901_ui::{Constraints, Rect, Size, TextMetrics, TextStyle, Widget};
use y901_widgets::{Table, TableColumn};

struct Mono;
impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
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

fn layout_table(t: &mut Table, w: f32, h: f32) {
    let mut cache = LayoutCache::new();
    let c = Constraints::loose(Size::new(w, h));
    let _ = Widget::measure(t, &Mono, c, &mut cache);
    Widget::arrange(t, Rect::new(0.0, 0.0, w, h), &Mono, &mut cache);
}

#[test]
fn column_layout_never_exceeds_the_row_area() {
    let mut rng = Lcg(0x7ab_1e00);
    for case in 0..200 {
        let ncols = 1 + (rng.next() % 5) as usize;
        let mut t = Table::new("t");
        let mut fixed_sum = 0.0f32;
        let mut has_flex = false;
        for i in 0..ncols {
            match rng.next() % 4 {
                0 => {
                    let v = (rng.next() % 120) as f32;
                    fixed_sum += v;
                    t = t.column(TableColumn::fixed(format!("c{i}"), v));
                }
                _ => {
                    has_flex = true;
                    t = t.column(TableColumn::flex(format!("c{i}")));
                }
            }
        }
        for r in 0..5 {
            t = t.row(vec![format!("r{r}"); ncols]);
        }
        let w = 80.0 + (rng.next() % 400) as f32;
        let h = 100.0 + (rng.next() % 200) as f32;
        layout_table(&mut t, w, h);

        let layout = t.column_layout();
        assert_eq!(layout.len(), ncols, "[case{case}] 每列都要有几何");
        let mut prev_right = f32::NEG_INFINITY;
        for (i, (x, cw)) in layout.iter().enumerate() {
            assert!(
                cw.is_finite() && *cw >= 0.0,
                "[case{case}] 列 {i} 宽度非法：{cw}"
            );
            assert!(x.is_finite(), "[case{case}] 列 {i} 起点非法：{x}");
            assert!(
                *x >= prev_right - 0.01,
                "[case{case}] 列 {i} 与前一层叠：x={x} prev_right={prev_right}（layout={layout:?}）"
            );
            prev_right = x + cw;
        }
        // 契约：**显式定宽可以故意超出**（用户要 114px，表只有 102px —— 我们不做横向滚动，
        // 尊重用户给的宽度并裁掉超出部分）；除此之外一律不得超出可用宽度。
        let total: f32 = layout.iter().map(|(_, cw)| *cw).sum();
        let explicit_overflow = fixed_sum > w + 0.5 && !has_flex;
        if !explicit_overflow {
            assert!(
                total <= w + 0.5,
                "[case{case}] 列总宽 {total} 超过表格宽 {w}（fixed_sum={fixed_sum} has_flex={has_flex} layout={layout:?}）"
            );
        }
    }
}

#[test]
fn selection_and_row_count_stay_consistent() {
    let mut rng = Lcg(0x1357_2468);
    for case in 0..150 {
        let n = (rng.next() % 40) as usize;
        let mut t = Table::new("t").column(TableColumn::fixed("c", 100.0));
        for r in 0..n {
            t = t.row(vec![format!("{r}")]);
        }
        // 随机选中 + 改行数
        for step in 0..10 {
            match rng.next() % 4 {
                0 => {
                    let i = (rng.next() % ((n.max(1) + 5) as u64)) as usize;
                    t.select(Some(i));
                    if let Some(sel) = t.selected_index() {
                        assert!(
                            sel < t.len(),
                            "[case{case} step{step}] 选中 {sel} 越界（len={}）",
                            t.len()
                        );
                    }
                }
                1 => {
                    let m = (rng.next() % 30) as usize;
                    let rows: Vec<Vec<String>> = (0..m).map(|r| vec![format!("{r}")]).collect();
                    t.set_rows(rows);
                }
                2 => {
                    t.select(None);
                }
                _ => {
                    let _ = t.sort_by(0, y901_widgets::SortOrder::Ascending);
                }
            }
            if let Some(sel) = t.selected_index() {
                assert!(
                    sel < t.len(),
                    "[case{case} step{step}] 选中 {sel} 越界（len={}）",
                    t.len()
                );
            }
            layout_table(&mut t, 300.0, 200.0);
            if let Some(sel) = t.selected_index() {
                assert!(sel < t.len(), "[case{case} step{step}] 布局后选中越界");
            }
        }
    }
}
