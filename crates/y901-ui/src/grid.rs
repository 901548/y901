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

use y901_core::interval::IntervalSet;

use crate::layout::LayoutCache;
use std::collections::HashMap;
use std::ops::Range;

use crate::{
    Constraints, Edges, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, TextMetrics,
    UiEvent, Widget,
};

/// 把轨道表里的非法值（NaN / ±∞ / 负数）收敛为 `Fixed(0)`。
///
/// 为什么在入口做：`Fixed(NaN)` 会让轨道长度与子控件矩形变成 NaN，
/// 而 NaN 矩形在光栅化里是"不可见但不报错"——难排查。
fn sanitize_tracks(tracks: Vec<GridTrack>) -> Vec<GridTrack> {
    tracks
        .into_iter()
        .map(|t| match t {
            GridTrack::Fixed(v) if !v.is_finite() || v < 0.0 => GridTrack::Fixed(0.0),
            GridTrack::Fraction(f) if !f.is_finite() || f < 0.0 => GridTrack::Fraction(0.0),
            other => other,
        })
        .collect()
}

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
    /// 在 Flex 中的尺寸约束
    item: crate::FlexItem,
    row_heights: Vec<f32>,
    /// 显式定位：子控件 id → 占用区间。未出现在这里的走顺序排布。
    placements: HashMap<String, GridSpan>,
    /// 每行一个列占用集；二维冲突检测就是"所有相关行都不重叠"。
    ///
    /// 为什么是"每行一个一维集"而不是一个二维结构：行与行之间互不影响，
    /// 而列方向是一维区间问题——直接复用 `IntervalSet` 比自造矩阵更少出错。
    occupancy: Vec<IntervalSet>,
    /// 内容版本：`place()` 改变布局时 +1，让布局缓存知道要重算。
    version: u64,
}

impl Grid {

    /// 设置在 Flex 中的尺寸约束（链式）。
    pub fn flex_item(mut self, item: crate::FlexItem) -> Self {
        self.item = item;
        self
    }
    /// 创建网格。
    ///
    /// 输入：`id`、列轨道、行轨道。轨道为空时视为"单条弹性轨道"。
    pub fn new(id: impl Into<String>, cols: Vec<GridTrack>, rows: Vec<GridTrack>) -> Self {
        // 空轨道表本身已被 `resolve_tracks` 当作"单条弹性轨道"，
        // 但调用方传入全 0 宽/高的轨道会让单元格退化成负数。
        // 这里在入口就把"非正值"剔除，保证内部不会出现负尺寸。
        let cols = sanitize_tracks(cols);
        let rows = sanitize_tracks(rows);
        Self {
            id: id.into(),
            cols,
            rows,
            gap: (0.0, 0.0),
            padding: Edges::ZERO,
            children: Vec::new(),
            rect: Rect::ZERO,
            col_widths: Vec::new(),
            item: crate::FlexItem::default(),
            row_heights: Vec::new(),
            placements: HashMap::new(),
            occupancy: Vec::new(),
            version: 0,
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
                if let Some(span) = self.placements.get(self.children[i].id()) {
                    return self.rect_for_span(*span);
                }
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

/// 一个子控件显式占用的网格区间：列 `[col_start, col_end)`、行 `[row_start, row_end)`。
///
/// 半开区间与 `Rect` 一致：`0..1` 表示"第 0 列"，`0..3` 表示"横跨三列"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSpan {
    /// 列区间起点（含）。
    pub col_start: u32,
    /// 列区间终点（不含）。
    pub col_end: u32,
    /// 行区间起点（含）。
    pub row_start: u32,
    /// 行区间终点（不含）。
    pub row_end: u32,
}

impl GridSpan {
    /// 列方向占用的格数。
    pub fn col_span(&self) -> u32 {
        self.col_end.saturating_sub(self.col_start)
    }

    /// 行方向占用的格数。
    pub fn row_span(&self) -> u32 {
        self.row_end.saturating_sub(self.row_start)
    }

    /// 两个区间是否相交（半开语义：仅相邻不算相交）。
    fn intersects(&self, other: &GridSpan) -> bool {
        self.col_start < other.col_end
            && other.col_start < self.col_end
            && self.row_start < other.row_end
            && other.row_start < self.row_end
    }
}

/// `Grid::place` 失败的原因。
///
/// 为什么返回 `Result` 而不是 panic：`place` 接收的是**用户输入**，
/// 冲突是调用方的编程错误，应当可被捕获与报告，而不是让整个界面崩掉。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlacementError {
    /// 区间为空或反向（`start >= end`）。
    EmptyRange {
        /// 出错的轴：`"col"` 或 `"row"`。
        axis: &'static str,
        /// 起点。
        start: u32,
        /// 终点。
        end: u32,
    },
    /// 区间超出声明的轨道数量。
    OutOfBounds {
        /// 出错的轴。
        axis: &'static str,
        /// 请求的终点（不含）。
        end: u32,
        /// 该轴已有的轨道数上限。
        limit: u32,
    },
    /// 树上没有这个 id 的子控件。
    UnknownChild {
        /// 请求的 id。
        id: String,
    },
    /// 与已有控件占用重叠。
    Overlap {
        /// 本次请求的 id。
        id: String,
        /// 与之冲突的既有 id（若能在现有定位里找到）。
        with: Option<String>,
        /// 本次请求的区间。
        requested: GridSpan,
        /// 冲突方的区间（若找到）。
        existing: Option<GridSpan>,
    },
}

impl std::fmt::Display for PlacementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlacementError::EmptyRange { axis, start, end } => {
                write!(f, "{axis} 区间无效：{start}..{end}")
            }
            PlacementError::OutOfBounds { axis, end, limit } => {
                write!(f, "{axis} 超出范围：请求 {end} 列/行，上限 {limit}")
            }
            PlacementError::UnknownChild { id } => write!(f, "不存在的子控件 id：{id}"),
            PlacementError::Overlap { id, with, requested, .. } => {
                write!(f, "定位冲突：{id}")?;
                if let Some(w) = with {
                    write!(f, " 与 {w}")?;
                }
                write!(
                    f,
                    " 在 {}..{}×{}..{}",
                    requested.col_start, requested.col_end, requested.row_start, requested.row_end
                )
            }
        }
    }
}

impl Grid {
    /// 显式定位：把子控件 `id` 放到 `col`×`row` 区间。
    ///
    /// 输入：`id`（必须已在 `children` 里）、列区间、行区间（半开）。
    /// 输出：`Ok` = 已记录；`Err` = 空区间 / 越界 / 未知 id / 与已有定位重叠。
    /// 副作用：成功时 **触发重排**（自动 bump `layout_version`）——
    /// 否则用户改了定位却看不到变化，是更难排查的 bug。
    ///
    /// 对同一个 id 重复调用 = **重新定位**（先释放旧占用再检查新位置）。
    pub fn place(
        &mut self,
        id: impl Into<String>,
        col: Range<u32>,
        row: Range<u32>,
    ) -> Result<(), PlacementError> {
        let id = id.into();
        let span = GridSpan {
            col_start: col.start,
            col_end: col.end,
            row_start: row.start,
            row_end: row.end,
        };

        if span.col_start >= span.col_end {
            return Err(PlacementError::EmptyRange {
                axis: "col",
                start: span.col_start,
                end: span.col_end,
            });
        }
        if span.row_start >= span.row_end {
            return Err(PlacementError::EmptyRange {
                axis: "row",
                start: span.row_start,
                end: span.row_end,
            });
        }

        let col_limit = self.cols.len().max(1) as u32;
        let row_limit = self.rows.len().max(1) as u32;
        if span.col_end > col_limit {
            return Err(PlacementError::OutOfBounds {
                axis: "col",
                end: span.col_end,
                limit: col_limit,
            });
        }
        if span.row_end > row_limit {
            return Err(PlacementError::OutOfBounds {
                axis: "row",
                end: span.row_end,
                limit: row_limit,
            });
        }

        if !self.children.iter().any(|c| c.id() == id) {
            return Err(PlacementError::UnknownChild { id });
        }

        // 重新定位：先把自己的旧占用拿掉
        if self.placements.remove(&id).is_some() {
            self.rebuild_occupancy();
        }

        // 先查所有相关行（本次覆盖的行范围），全部通过才写入。
        self.sync_occupancy_rows();
        for r in span.row_start..span.row_end {
            if self.occupancy[r as usize].overlaps(span.col_start, span.col_end) {
                let with = self
                    .placements
                    .iter()
                    .find(|(_, other)| other.intersects(&span))
                    .map(|(k, _)| k.clone());
                let existing = with.as_ref().and_then(|k| self.placements.get(k)).copied();
                return Err(PlacementError::Overlap {
                    id,
                    with,
                    requested: span,
                    existing,
                });
            }
        }

        for r in span.row_start..span.row_end {
            // 已在上面检查过不重叠，这里的 insert 不会失败
            let _ = self.occupancy[r as usize].insert(span.col_start, span.col_end);
        }
        self.placements.insert(id, span);
        self.version = self.version.wrapping_add(1);
        Ok(())
    }

    /// 子控件是否有显式定位。
    pub fn placement_of(&self, id: &str) -> Option<GridSpan> {
        self.placements.get(id).copied()
    }

    /// 按已有定位重建占用表（重新定位时用）。
    fn rebuild_occupancy(&mut self) {
        self.occupancy.clear();
        self.sync_occupancy_rows();
        let spans: Vec<GridSpan> = self.placements.values().copied().collect();
        for span in spans {
            for r in span.row_start..span.row_end {
                let _ = self.occupancy[r as usize].insert(span.col_start, span.col_end);
            }
        }
    }

    /// 保证 `occupancy` 的行数与声明的行数一致。
    fn sync_occupancy_rows(&mut self) {
        let rows = self.rows.len().max(1);
        if self.occupancy.len() < rows {
            self.occupancy.resize(rows, IntervalSet::new());
        }
    }

    /// 一个跨格区间对应的屏幕矩形（含区间内的间距）。
    fn rect_for_span(&self, span: GridSpan) -> Rect {
        let c0 = span.col_start as usize;
        let c1 = span.col_end as usize;
        let r0 = span.row_start as usize;
        let r1 = span.row_end as usize;

        let x = self.rect.x
            + self.padding.left
            + self.col_widths.iter().take(c0).sum::<f32>()
            + self.gap.0 * c0 as f32;
        let y = self.rect.y
            + self.padding.top
            + self.row_heights.iter().take(r0).sum::<f32>()
            + self.gap.1 * r0 as f32;

        let w: f32 = self.col_widths.iter().take(c1).skip(c0).sum::<f32>()
            + self.gap.0 * span.col_span().saturating_sub(1) as f32;
        let h: f32 = self.row_heights.iter().take(r1).skip(r0).sum::<f32>()
            + self.gap.1 * span.row_span().saturating_sub(1) as f32;
        Rect::new(x, y, w.max(0.0), h.max(0.0))
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

    fn flex(&self) -> crate::FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    /// 每个子控件裁到自己的格子里。
    ///
    /// 这是 Grid 存在的意义之一：固定列宽就应该是硬约束，
    /// 内容太大就裁掉，而不是把整个布局撞开。
    fn layout_version(&self) -> u64 {
        self.version
    }

    fn children_clip(&self, cell_rect: Rect) -> Option<Rect> {
        Some(cell_rect)
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

    // ================= 显式定位（place）=================

    /// 一个 3×3 网格，每列 50 / 每行 20，间距 (10, 5)，原点 (100, 200)。
    fn grid_3x3() -> Grid {
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0); 3],
            vec![GridTrack::Fixed(20.0); 3],
        )
        .gap(10.0, 5.0)
        .with_children(vec![
            Box::new(TestLeaf::new("a")),
            Box::new(TestLeaf::new("b")),
            Box::new(TestLeaf::new("c")),
            Box::new(TestLeaf::new("d")),
        ]);
        g.rect = Rect::new(100.0, 200.0, 200.0, 100.0);
        g.col_widths = vec![50.0, 50.0, 50.0];
        g.row_heights = vec![20.0, 20.0, 20.0];
        g
    }

    fn rect_of(g: &Grid, id: &str) -> Rect {
        let idx = g
            .children
            .iter()
            .position(|c| c.id() == id)
            .expect("子控件存在");
        g.child_rects()[idx]
    }

    #[test]
    fn place_single_cell_matches_sequential_layout() {
        let mut g = grid_3x3();
        g.place("a", 0..1, 0..1).expect("单格");
        // 顺序排布时 "a" 也在 (0,0)：两者必须一致
        assert_eq!(rect_of(&g, "a"), Rect::new(100.0, 200.0, 50.0, 20.0));
    }

    #[test]
    fn place_spanning_columns_matches_union_plus_gaps() {
        let mut g = grid_3x3();
        g.place("a", 0..3, 0..1).expect("跨三列");
        // 宽 = 50 + 10 + 50 + 10 + 50 = 170
        assert_eq!(rect_of(&g, "a"), Rect::new(100.0, 200.0, 170.0, 20.0));
    }

    #[test]
    fn place_spanning_rows_matches_union_plus_gaps() {
        let mut g = grid_3x3();
        g.place("a", 1..2, 0..3).expect("跨三行");
        // x = 100 + 50 + 10 = 160；高 = 20 + 5 + 20 + 5 + 20 = 70
        assert_eq!(rect_of(&g, "a"), Rect::new(160.0, 200.0, 50.0, 70.0));
    }

    #[test]
    fn place_spanning_both_axes() {
        let mut g = grid_3x3();
        g.place("a", 1..3, 1..3).expect("跨两列两行");
        // x = 100 + 50 + 10 = 160；y = 200 + 20 + 5 = 225
        // w = 50 + 10 + 50 = 110；h = 20 + 5 + 20 = 45
        assert_eq!(rect_of(&g, "a"), Rect::new(160.0, 225.0, 110.0, 45.0));
    }

    #[test]
    fn place_rejects_overlapping_spans() {
        let mut g = grid_3x3();
        g.place("a", 0..2, 0..1).expect("先放 a");
        let err = g.place("b", 1..3, 0..1).expect_err("应与 a 冲突");
        match &err {
            PlacementError::Overlap { id, with, .. } => {
                assert_eq!(id, "b");
                assert_eq!(with.as_deref(), Some("a"), "应指出冲突方");
            }
            other => panic!("应为 Overlap，实际 {other:?}"),
        }
    }

    #[test]
    fn place_allows_adjacent_but_disjoint_cells() {
        let mut g = grid_3x3();
        g.place("a", 0..1, 0..1).expect("a");
        g.place("b", 1..2, 0..1).expect("相邻不算冲突");
        g.place("c", 0..1, 1..2).expect("下一行不算冲突");
    }

    #[test]
    fn place_rejects_out_of_bounds_and_unknown_ids() {
        let mut g = grid_3x3();
        assert_eq!(
            g.place("a", 0..4, 0..1),
            Err(PlacementError::OutOfBounds {
                axis: "col",
                end: 4,
                limit: 3
            })
        );
        assert_eq!(
            g.place("a", 0..1, 0..4),
            Err(PlacementError::OutOfBounds {
                axis: "row",
                end: 4,
                limit: 3
            })
        );
        assert_eq!(
            g.place("zzz", 0..1, 0..1),
            Err(PlacementError::UnknownChild {
                id: "zzz".to_string()
            })
        );
        assert!(matches!(
            g.place("a", 2..2, 0..1),
            Err(PlacementError::EmptyRange { axis: "col", .. })
        ));
    }

    #[test]
    fn place_bumps_layout_version_only_on_success() {
        let mut g = grid_3x3();
        let before = g.version;
        g.place("a", 0..1, 0..1).expect("ok");
        assert_eq!(g.version, before + 1, "成功应 bump");

        let v = g.version;
        let _ = g.place("b", 0..1, 0..1); // 与 a 冲突
        assert_eq!(g.version, v, "失败不应 bump");
    }

    #[test]
    fn replace_placement_frees_the_old_cells() {
        let mut g = grid_3x3();
        g.place("a", 0..2, 0..1).expect("a 占 0..2");
        g.place("a", 2..3, 0..1).expect("改到 2..3（已释放旧格）");
        g.place("b", 0..2, 0..1).expect("旧格已空出，应能放 b");
    }
}
