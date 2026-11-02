//! Grid 布局：按列/行轨道分配空间。
//!
//! ## 定位方式：**顺序排布**（不做 `grid_cell(col, row)`）
//!
//! 第一版按"从左到右、从上到下、满列换行"排布子控件。
//! 显式定位需要跨单元格（span）与空洞语义，那会把算法从"按索引算矩形"变成
//! "占位矩阵 + 冲突检测"。顺序排布能覆盖绝大多数表单/图片墙场景。
//!
//! ## Auto 轨道：按子控件内容尺寸自适应（不是返回 0）
//!
//! `Auto` 取该轨道内子控件测得尺寸的最大值。`measure` 本来就要量子控件，没有额外成本。

use crate::layout::LayoutCache;
use crate::{
    Constraints, Edges, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, TextMetrics,
    UiEvent, Widget,
};

/// 一条轨道的尺寸策略。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridTrack {
    /// 固定像素长度
    Fixed(f32),
    /// 按比例瓜分剩余空间（类似 CSS 的 `fr`）
    Fraction(f32),
    /// 按该轨道内子控件的内容尺寸自适应
    Auto,
}

impl GridTrack {
    /// 比例权重（非 `Fraction` 返回 `None`）。
    fn fraction(self) -> Option<f32> {
        match self {
            GridTrack::Fraction(f) => Some(f.max(0.0)),
            _ => None,
        }
    }
}

/// 二维网格容器。
pub struct Grid {
    id: String,
    cols: Vec<GridTrack>,
    rows: Vec<GridTrack>,
    gap: (f32, f32),
    padding: Edges,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    /// `measure` 算出的轨道长度，供 `arrange` 复用
    col_widths: Vec<f32>,
    row_heights: Vec<f32>,
}

impl Grid {
    /// 创建网格。
    ///
    /// 输入：`id`、列轨道、行轨道。轨道为空时视为"单条弹性轨道"。
    pub fn new(id: impl Into<String>, cols: Vec<GridTrack>, rows: Vec<GridTrack>) -> Self {
        Self {
            id: id.into(),
            cols,
            rows,
            gap: (0.0, 0.0),
            padding: Edges::ZERO,
            children: Vec::new(),
            rect: Rect::ZERO,
            col_widths: Vec::new(),
            row_heights: Vec::new(),
        }
    }

    /// 设置行列间距。
    pub fn gap(mut self, horizontal: f32, vertical: f32) -> Self {
        self.gap = (horizontal.max(0.0), vertical.max(0.0));
        self
    }

    /// 设置内边距。
    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    /// 批量设置子控件（链式）。
    pub fn with_children(mut self, children: Vec<Box<dyn Widget>>) -> Self {
        self.children = children;
        self
    }

    /// 第 `i` 个子控件落在 `(col, row)`（顺序排布）。
    fn cell_of(&self, index: usize) -> (usize, usize) {
        let cols = self.cols.len().max(1);
        (index % cols, index / cols)
    }

    /// 计算轨道长度。
    ///
    /// 规则：Fixed 先占，Auto 取该轨道内容最大值，剩余按 Fraction 权重分配。
    /// 总量不足时**不压缩** Fixed（允许溢出、由裁剪兜底），否则"固定像素"名不副实。
    fn resolve_tracks(
        tracks: &[GridTrack],
        available: f32,
        gap: f32,
        content_of: impl Fn(usize) -> f32,
    ) -> Vec<f32> {
        if tracks.is_empty() {
            return vec![available.max(0.0)];
        }
        let n = tracks.len();
        let gaps = gap * (n.saturating_sub(1)) as f32;
        let mut out = vec![0.0f32; n];
        let mut used = 0.0f32;
        let mut weight = 0.0f32;
        for (i, t) in tracks.iter().enumerate() {
            match t {
                GridTrack::Fixed(v) => {
                    out[i] = v.max(0.0);
                    used += out[i];
                }
                GridTrack::Auto => {
                    out[i] = content_of(i).max(0.0);
                    used += out[i];
                }
                GridTrack::Fraction(f) => weight += f.max(0.0),
            }
        }
        let free = (available - gaps - used).max(0.0);
        if weight > 0.0 {
            for (i, t) in tracks.iter().enumerate() {
                if let Some(f) = t.fraction() {
                    out[i] = free * f / weight;
                }
            }
        }
        out
    }

    /// 每个子控件的目标矩形。
    fn child_rects(&self) -> Vec<Rect> {
        (0..self.children.len())
            .map(|i| {
                let (col, row) = self.cell_of(i);
                let x = self.rect.x
                    + self.padding.left
                    + self.col_widths.iter().take(col).sum::<f32>()
                    + self.gap.0 * col as f32;
                let y = self.rect.y
                    + self.padding.top
                    + self.row_heights.iter().take(row).sum::<f32>()
                    + self.gap.1 * row as f32;
                let w = self.col_widths.get(col).copied().unwrap_or(0.0).max(0.0);
                let h = self.row_heights.get(row).copied().unwrap_or(0.0).max(0.0);
                Rect::new(x, y, w, h)
            })
            .collect()
    }

    /// 没有 `measure` 结果时，按 rect 直接推算轨道（`arrange` 的兜底）。
    fn fallback_tracks(&mut self, rect: Rect) {
        let cols = self.cols.len().max(1);
        let inner_w = (rect.w - self.padding.left - self.padding.right).max(0.0);
        self.col_widths = Self::resolve_tracks(&self.cols, inner_w, self.gap.0, |_| 0.0);
        let rows = self
            .children
            .len()
            .div_ceil(cols)
            .max(self.rows.len())
            .max(1);
        let inner_h = (rect.h - self.padding.top - self.padding.bottom).max(0.0);
        self.row_heights =
            Self::resolve_tracks(&self.rows, inner_h, self.gap.1, |_| inner_h / rows as f32);
    }
}

impl Widget for Grid {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }

    fn padding(&self) -> Edges {
        self.padding
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
        cache: &mut LayoutCache,
    ) -> Size {
        let cols = self.cols.len().max(1);
        let inner_w = (constraints.max.w - self.padding.left - self.padding.right).max(0.0);
        let rows = self
            .children
            .len()
            .div_ceil(cols)
            .max(self.rows.len())
            .max(1);

        // 每个子控件的可用宽度先算好：循环里同时借 `children`（mut）与 `self`（只读）会冲突。
        let widths: Vec<f32> = (0..self.children.len())
            .map(|i| {
                let (col, _) = self.cell_of(i);
                match self.cols.get(col) {
                    Some(GridTrack::Fixed(v)) => v.max(0.0),
                    _ => (inner_w - self.gap.0 * (cols - 1) as f32) / cols as f32,
                }
            })
            .collect();

        let mut col_content = vec![0.0f32; cols];
        let mut row_content = vec![0.0f32; rows];
        // 先把每个子控件的 (col,row) 算好：循环内再调 `cell_of` 会同时借 self。
        let cells: Vec<(usize, usize)> = (0..self.children.len()).map(|i| self.cell_of(i)).collect();
        for ((child, cell_w), (col, row)) in self.children.iter_mut().zip(widths).zip(cells) {
            let size = child.measure(
                metrics,
                Constraints {
                    min: Size::ZERO,
                    max: Size::new(cell_w.max(0.0), constraints.max.h.max(0.0)),
                    ..Default::default()
                },
                cache,
            );
            col_content[col] = col_content[col].max(size.w);
            if row < rows {
                row_content[row] = row_content[row].max(size.h);
            }
        }

        self.col_widths = Self::resolve_tracks(&self.cols, inner_w, self.gap.0, |i| {
            col_content.get(i).copied().unwrap_or(0.0)
        });
        let inner_h = (constraints.max.h - self.padding.top - self.padding.bottom).max(0.0);
        self.row_heights = Self::resolve_tracks(&self.rows, inner_h, self.gap.1, |i| {
            row_content.get(i).copied().unwrap_or(0.0)
        });

        let total_w = self.col_widths.iter().sum::<f32>()
            + self.gap.0 * (self.col_widths.len().saturating_sub(1)) as f32
            + self.padding.left
            + self.padding.right;
        let total_h = self.row_heights.iter().sum::<f32>()
            + self.gap.1 * (self.row_heights.len().saturating_sub(1)) as f32
            + self.padding.top
            + self.padding.bottom;
        constraints.constrain(Size::new(total_w, total_h))
    }

    fn arrange(&mut self, rect: Rect, metrics: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        if self.col_widths.is_empty() {
            self.fallback_tracks(rect);
        }
        let rects = self.child_rects();
        for (child, r) in self.children.iter_mut().zip(rects) {
            child.arrange(r, metrics, cache);
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, _ctx: &mut dyn RenderCtx) {
        // **不画子控件**：框架的渲染模型是"树遍历每个节点各自画自己"
        // （`render_pass` 递归）。容器若再遍历一遍，每个子控件会被**画两次**——
        // 半透明元素会变深，且第二次不带任何剪裁。这是实测抓到的。
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("grid {}", self.id))
            .prop("cols", self.cols.len().to_string())
            .prop("rows", self.row_heights.len().to_string())
            .prop("children", self.children.len().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 最小占位控件（只用于验证布局矩形）。
    struct TestLeaf {
        id: String,
        rect: Rect,
    }

    impl TestLeaf {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                rect: Rect::ZERO,
            }
        }
    }

    impl Widget for TestLeaf {
        fn id(&self) -> &str {
            &self.id
        }
        fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut LayoutCache) -> Size {
            c.constrain(Size::new(10.0, 10.0))
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    fn resolve(tracks: &[GridTrack], avail: f32, gap: f32) -> Vec<f32> {
        Grid::resolve_tracks(tracks, avail, gap, |_| 0.0)
    }

    #[test]
    fn fixed_columns_keep_their_width() {
        let w = resolve(
            &[
                GridTrack::Fixed(100.0),
                GridTrack::Fixed(100.0),
                GridTrack::Fixed(100.0),
            ],
            400.0,
            10.0,
        );
        assert_eq!(w, vec![100.0, 100.0, 100.0], "固定列不应被拉伸");
    }

    #[test]
    fn fractions_split_remaining_space() {
        let w = resolve(
            &[
                GridTrack::Fixed(100.0),
                GridTrack::Fraction(1.0),
                GridTrack::Fraction(1.0),
            ],
            400.0,
            0.0,
        );
        assert_eq!(w, vec![100.0, 150.0, 150.0], "剩余 300 应各分 150");
    }

    #[test]
    fn mixed_tracks_account_for_gaps_and_auto() {
        // 400 - 间距 20 - Fixed 100 - Auto 50 = 230 给 Fraction
        let w = Grid::resolve_tracks(
            &[
                GridTrack::Fixed(100.0),
                GridTrack::Auto,
                GridTrack::Fraction(1.0),
            ],
            400.0,
            10.0,
            |i| if i == 1 { 50.0 } else { 0.0 },
        );
        assert_eq!(w, vec![100.0, 50.0, 230.0]);
    }

    #[test]
    fn auto_uses_largest_content_in_that_track() {
        let w = Grid::resolve_tracks(
            &[GridTrack::Auto, GridTrack::Fraction(1.0)],
            300.0,
            0.0,
            |i| if i == 0 { 120.0 } else { 0.0 },
        );
        assert_eq!(w, vec![120.0, 180.0]);
    }

    #[test]
    fn empty_tracks_take_everything() {
        assert_eq!(resolve(&[], 250.0, 8.0), vec![250.0]);
    }

    #[test]
    fn rows_resolve_independently_of_columns() {
        let h = resolve(
            &[GridTrack::Fixed(40.0), GridTrack::Fraction(1.0)],
            100.0,
            0.0,
        );
        assert_eq!(h, vec![40.0, 60.0]);
    }

    #[test]
    fn cell_mapping_wraps_by_column_count() {
        let g = Grid::new("g", vec![GridTrack::Fixed(10.0); 3], vec![GridTrack::Auto]);
        assert_eq!(g.cell_of(0), (0, 0));
        assert_eq!(g.cell_of(2), (2, 0));
        assert_eq!(g.cell_of(3), (0, 1), "第 4 个应换到第二行首列");
        assert_eq!(g.cell_of(4), (1, 1));
    }

    #[test]
    fn child_rects_follow_tracks_and_gaps() {
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0), GridTrack::Fixed(50.0)],
            vec![GridTrack::Fixed(20.0), GridTrack::Fixed(20.0)],
        )
        .gap(10.0, 5.0);
        // 用最小占位控件就行：本测试只验证矩形计算。
        for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
            let _ = i;
            g.children.push(Box::new(TestLeaf::new(name)));
        }
        g.rect = Rect::new(100.0, 200.0, 110.0, 45.0);
        g.col_widths = vec![50.0, 50.0];
        g.row_heights = vec![20.0, 20.0];

        let r = g.child_rects();
        assert_eq!(r[0], Rect::new(100.0, 200.0, 50.0, 20.0), "左上");
        assert_eq!(r[1], Rect::new(160.0, 200.0, 50.0, 20.0), "右上（含列间距 10）");
        assert_eq!(r[2], Rect::new(100.0, 225.0, 50.0, 20.0), "左下（含行间距 5）");
        assert_eq!(r[3], Rect::new(160.0, 225.0, 50.0, 20.0), "右下");
    }
}
