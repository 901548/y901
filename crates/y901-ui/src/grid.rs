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
use std::collections::{HashMap, HashSet};
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
    /// 子控件 id → 下标索引（避免 `place` 里做 O(n) 字符串扫描）。
    ///
    /// 失效机制：`children_mut()` 会把 `index_dirty` 置为 `true`；`place()` 入口发现
    /// 脏标记或长度不匹配时重建。宁可多比一次长度，也不能拿到错的下标。
    id_index: HashMap<String, usize>,
    /// 索引是否可能已失效（`children_mut()` 被调用过）。
    index_dirty: bool,
    /// 自动流子控件声明的跨格尺寸：`id → (列数, 行数)`。
    ///
    /// 与 `placements` **互斥**：`place` 定"在哪"，`span` 只定"多大"，
    /// 位置由自动流找。两者都没调 = 自动流 1×1。
    auto_spans: HashMap<String, (u32, u32)>,
    /// 已经警告过"放不下退化 1×1"的 id。
    ///
    /// 为什么去重：布局每帧都跑，不去重的话一个错误布局会刷屏。
    warned_span_ids: HashSet<String>,
    /// 每个子控件**实际生效**的跨格尺寸（与 `children` 下标对齐）。
    ///
    /// 为什么不直接读 `auto_spans`：声明了 2×1 但放不下时会**退化为 1×1**，
    /// 渲染必须用退化后的尺寸，否则会画出一个不存在的跨格区。
    auto_effective: Vec<(u32, u32)>,
    /// 自动落位缓存（与 `children` 下标对齐）。
    ///
    /// 为什么要缓存：`auto_cells` 本身是 O(n·rows·cols)。若 `cell_of(i)`
    /// 每次都重算，布局就退化为 **O(n²·rows·cols)**——实测在 10k 节点上直接卡死。
    auto_cache: Vec<(usize, usize)>,
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
            id_index: HashMap::new(),
            index_dirty: false,
            auto_spans: HashMap::new(),
            warned_span_ids: HashSet::new(),
            auto_effective: Vec::new(),
            auto_cache: Vec::new(),
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
        self.rebuild_index();
        self
    }

    /// 重建 id → 下标索引（同名 id 保留第一个）。
    fn rebuild_index(&mut self) {
        self.id_index.clear();
        for (i, c) in self.children.iter().enumerate() {
            self.id_index.entry(c.id().to_string()).or_insert(i);
        }
        self.index_dirty = false;
    }

    /// 第 `i` 个子控件落在 `(col, row)`。
    ///
    /// 显式定位过的走 `placement`（见 `child_rects`），这里只处理**自动流**：
    /// 按树序逐个找"第一个未被占用的格子"（行优先），而不是按下标硬算。
    ///
    /// 为什么不能用 `index % cols`：一旦有显式定位，下标与格子就脱钩了。
    /// 例：`place("a", 1..2, 0..1)` 之后，第二个子控件应在 `(0,0)`，而 `index % 2` 会给出 `(1,0)`——那格已被 a 占了。
    fn cell_of(&self, index: usize) -> (usize, usize) {
        // 超出子控件个数的查询（诊断/测试用）退回纯下标映射：
        // 它不对应任何实体控件，也就不受显式定位影响。
        if index >= self.children.len() {
            let cols = self.cols.len().max(1);
            return (index % cols, index / cols);
        }
        if self.auto_cache.is_empty() {
            // 未缓存（例如布局前的探查）：退回纯下标映射，不触发重计算
            let cols = self.cols.len().max(1);
            return (index % cols, index / cols);
        }
        self.auto_cache.get(index).copied().unwrap_or((0, 0))
    }

    /// 计算所有子控件的**自动**落位（显式定位的跳过）。
    ///
    /// 返回的下标与 `children` 对齐；被显式定位的位置写 `(0,0)`（不会被用到）。
    fn auto_cells(&mut self) -> Vec<(usize, usize)> {
        let cols = self.cols.len().max(1);
        let rows = self.rows.len().max(1);

        // 每行一个占用位图：点查询 O(1)（不用 `IntervalSet::contains`——它是线性扫描）。
        let mut taken: Vec<Vec<bool>> = vec![vec![false; cols]; rows];
        for (r, set) in self.occupancy.iter().enumerate().take(rows) {
            for (a, b) in set.spans() {
                let from = (a as usize).min(cols);
                let to = (b as usize).min(cols);
                if from < to {
                    taken[r][from..to].fill(true);
                }
            }
        }

        let mut cursor = vec![0usize; rows];
        let mut out = Vec::with_capacity(self.children.len());
        let mut effective: Vec<(u32, u32)> = Vec::with_capacity(self.children.len());
        // 警告先收集，循环结束后再写（否则与 `&self.children` 的借用冲突）
        let mut to_warn: Vec<String> = Vec::new();

        for child in &self.children {
            if self.placements.contains_key(child.id()) {
                out.push((0, 0));
                effective.push((1, 1));
                continue;
            }
            let (w, h) = self.auto_spans.get(child.id()).copied().unwrap_or((1, 1));
            let (w, h) = (w.max(1) as usize, h.max(1) as usize);

            let mut placed = None;
            if w == 1 && h == 1 {
                // 快路：单格用游标，列只增不减
                for r in 0..rows {
                    let mut c = cursor[r];
                    while c < cols && taken[r][c] {
                        c += 1;
                    }
                    cursor[r] = c;
                    if c < cols {
                        taken[r][c] = true;
                        placed = Some((c, r));
                        break;
                    }
                }
            } else {
                // 跨格：找"连续 w 列 × h 行全空"的起点（行优先）
                'search: for r in 0..=rows.saturating_sub(h) {
                    for c in 0..=cols.saturating_sub(w) {
                        let free = taken[r..r + h]
                            .iter()
                            .all(|row| !row[c..c + w].iter().any(|x| *x));
                        if free {
                            for row in taken[r..r + h].iter_mut() {
                                row[c..c + w].fill(true);
                            }
                            placed = Some((c, r));
                            break 'search;
                        }
                    }
                }
                if placed.is_none() {
                    // 放不下：退化 1×1（不动态加行），并警告一次
                    to_warn.push(child.id().to_string());
                    for r in 0..rows {
                        let mut c = cursor[r];
                        while c < cols && taken[r][c] {
                            c += 1;
                        }
                        cursor[r] = c;
                        if c < cols {
                            taken[r][c] = true;
                            placed = Some((c, r));
                            break;
                        }
                    }
                }
            }
            let got_span = placed.is_some() && (w > 1 || h > 1);
            effective.push(if got_span {
                (w as u32, h as u32)
            } else {
                (1, 1)
            });
            out.push(placed.unwrap_or((0, 0)));
        }

        for id in to_warn {
            if self.warned_span_ids.insert(id.clone()) {
                eprintln!(
                    "[y901] Grid::span 放不下：{id} 需要的连续格子在声明的行列内不存在，已退化为 1×1"
                );
            }
        }

        self.auto_cache = out.clone();
        self.auto_effective = effective;
        out
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

    /// 按**区间约束的最长路**求轨道尺寸（跨格归因的精确解）。
    ///
    /// 形式化：设前缀和 `P[0] = 0`、`w_j = P[j] - P[j-1]`，
    /// 每条约束 `(区间 a..b, 最小尺寸 d)` 变为
    ///
    /// ```text
    /// P[b] >= P[a] + d + gap * (b - a - 1)
    /// ```
    ///
    /// 因为 `a < b`，所有边都指向更大下标 → 约束图是 **DAG**，
    /// 下标顺序本身就是拓扑序，一遍松弛即得**最小总宽的精确解**（`O(m + n)`）。
    ///
    /// 为什么不用两遍 min/max-content 启发式：那是近似解，误差上界 `n·d_max`；
    /// 而这里的精确解反而更快（纯浮点运算，不再调 `measure`）。
    ///
    /// 注意：跨格约束**可以撑开 `Fixed` 轨道**（否则跨格内容放不下）。
    fn resolve_by_spans(
        tracks: &[GridTrack],
        available: f32,
        gap: f32,
        spans: &[(usize, usize, f32)],
    ) -> Vec<f32> {
        let m = tracks.len().max(1);
        let mut p = vec![0.0f32; m + 1];

        // 约束按"终点 b"分桶，便于一遍扫描
        let mut by_end: Vec<Vec<(usize, f32)>> = vec![Vec::new(); m + 1];
        // 固定轨道自身也是约束：长度 >= v
        for (j, t) in tracks.iter().enumerate() {
            if let GridTrack::Fixed(v) = t {
                if j < m {
                    by_end[j + 1].push((j, v.max(0.0)));
                }
            }
        }
        for &(a, b, d) in spans {
            if a < b && b <= m {
                let extra = gap * (b - a - 1) as f32;
                // 与固定轨道的下界取 max
                by_end[b].push((a, d + extra));
            }
        }

        for k in 1..=m {
            let mut best = p[k - 1];
            for &(a, need) in &by_end[k] {
                best = best.max(p[a] + need);
            }
            p[k] = best;
        }

        let mut out: Vec<f32> = (0..m).map(|j| (p[j + 1] - p[j]).max(0.0)).collect();

        // 剩余空间给弹性轨道（与旧 `resolve_tracks` 的 Fraction 语义一致）
        let weight: f32 = tracks
            .iter()
            .filter_map(|t| t.fraction())
            .map(|f| f.max(0.0))
            .sum();
        if weight > 0.0 {
            let used: f32 = out.iter().sum::<f32>() + gap * (m.saturating_sub(1)) as f32;
            let free = (available - used).max(0.0);
            for (j, t) in tracks.iter().enumerate() {
                if let Some(f) = t.fraction() {
                    out[j] += free * f.max(0.0) / weight;
                }
            }
        }
        out
    }

    /// 每个子控件的目标矩形（解析后的结果，供测试与调试直接断言）。
    ///
    /// 前提：需要先跑过一次 `measure`（它会缓存自动落位），否则自动流子控件退回纯下标映射。
    ///
    /// 为什么公开：它是纯函数（输入轨道尺寸 + 定位，输出矩形），
    /// 几何断言比像素断言精确得多，错了能直接报出哪个数字不对。
    pub fn child_rects(&self) -> Vec<Rect> {
        (0..self.children.len())
            .map(|i| {
                if let Some(span) = self.placements.get(self.children[i].id()) {
                    return self.rect_for_span(*span);
                }
                let (col, row) = self.cell_of(i);
                // 自动流但声明了跨格：用"**生效**"尺寸算并集
                if let Some((w, h)) = self.auto_effective.get(i).copied() {
                    if w > 1 || h > 1 {
                        return self.rect_for_span(GridSpan {
                            col_start: col as u32,
                            col_end: col as u32 + w,
                            row_start: row as u32,
                            row_end: row as u32 + h,
                        });
                    }
                }
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
            PlacementError::Overlap {
                id,
                with,
                requested,
                ..
            } => {
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

        // 索引可能失效：长度不匹配或被标脏就重建。
        if self.index_dirty || self.id_index.len() != self.children.len() {
            self.rebuild_index();
        }
        if !self.id_index.contains_key(&id) {
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

    /// 声明**自动流子控件**的跨格尺寸（不指定位置）。
    ///
    /// 输入：`id`、列数、行数。
    /// 输出：`Ok` = 已记录；`Err` = 未知 id / 零尺寸 / 超出声明的行列数。
    /// 副作用：成功时 bump `layout_version`（与 `place` 一致）。
    ///
    /// 与 `place` 的关系：`place` 定"在哪"，`span` 只定"多大"，
    /// 两者**互斥**。对同一个 id 先 `place` 后 `span`：**以 `place` 为准**并警告一次（不报错，
    /// 因为这是"多写了一句"而不是"请求无法满足"）。
    ///
    /// 放不下时的行为（布局阶段，无法返回 `Err`）：**退化为 1×1 并警告一次**。
    /// 不动态加行——那会让声明式布局的结果不可预测。
    pub fn span(
        &mut self,
        id: impl Into<String>,
        cols: u32,
        rows: u32,
    ) -> Result<(), PlacementError> {
        let id = id.into();
        if cols == 0 {
            return Err(PlacementError::EmptyRange {
                axis: "cols",
                start: 0,
                end: cols,
            });
        }
        if rows == 0 {
            return Err(PlacementError::EmptyRange {
                axis: "rows",
                start: 0,
                end: rows,
            });
        }
        let col_limit = self.cols.len().max(1) as u32;
        let row_limit = self.rows.len().max(1) as u32;
        if cols > col_limit {
            return Err(PlacementError::OutOfBounds {
                axis: "cols",
                end: cols,
                limit: col_limit,
            });
        }
        if rows > row_limit {
            return Err(PlacementError::OutOfBounds {
                axis: "rows",
                end: rows,
                limit: row_limit,
            });
        }
        if self.index_dirty || self.id_index.len() != self.children.len() {
            self.rebuild_index();
        }
        if !self.id_index.contains_key(&id) {
            return Err(PlacementError::UnknownChild { id });
        }
        if self.placements.contains_key(&id) {
            // 以 `place` 为准：不覆盖已定位的控件，但要告诉用户"你这句没生效"
            if self.warned_span_ids.insert(id.clone()) {
                eprintln!("[y901] Grid::span 忽略：{id} 已通过 place() 显式定位");
            }
            return Ok(());
        }
        self.auto_spans.insert(id, (cols, rows));
        self.version = self.version.wrapping_add(1);
        Ok(())
    }

    /// 自动流子控件声明的跨格尺寸（未声明则 `None`）。
    pub fn span_of(&self, id: &str) -> Option<(u32, u32)> {
        self.auto_spans.get(id).copied()
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
        // 外部可以直接改这个 Vec，索引必须标脏；
        // 标脏代价是 O(1)，而"拿到错下标"会定位到错的控件。
        self.index_dirty = true;
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
        // 先算一次自动落位并缓存：`cell_of` 后续被 O(n) 次调用，不能每次重算。
        let _ = self.auto_cells();
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
                // 跨格控件**不能**用单格宽去限制：那样它的 min-content
                // 会被裁到一格（实测遇到过：300px 内容询回 133px）。
                // 它的上界是整个内容区宽，真正的轨道分配由最长路决定。
                let (col, _) = self.cell_of(i);
                let spanning = self
                    .placements
                    .get(self.children[i].id())
                    .is_some_and(|s| s.col_span() > 1);
                if spanning {
                    return inner_w.max(0.0);
                }
                match self.cols.get(col) {
                    Some(GridTrack::Fixed(v)) => v.max(0.0),
                    _ => (inner_w - self.gap.0 * (cols - 1) as f32) / cols as f32,
                }
            })
            .collect();

        let mut col_content = vec![0.0f32; cols];
        let mut row_content = vec![0.0f32; rows];
        // 跨格约束：列方向 `(a, b, 内在宽)`、行方向 `(a, b, 内在高)`
        let mut col_spans: Vec<(usize, usize, f32)> = Vec::new();
        let mut row_spans: Vec<(usize, usize, f32)> = Vec::new();
        // 先把每个子控件的 (col,row) 算好：循环内再调 `cell_of` 会同时借 self。
        let cells: Vec<(usize, usize)> =
            (0..self.children.len()).map(|i| self.cell_of(i)).collect();
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
            // 跨格控件：它的尺寸不能归给起始格（那会让起始轨道过宽、其余过窄），
            // 而是变成一条"区间下界"约束交给最长路求解。
            let span = self
                .placements
                .get(child.id())
                .copied()
                .unwrap_or(crate::grid::GridSpan {
                    col_start: col as u32,
                    col_end: col as u32 + 1,
                    row_start: row as u32,
                    row_end: row as u32 + 1,
                });
            if span.col_span() > 1 {
                col_spans.push((span.col_start as usize, span.col_end as usize, size.w));
            } else {
                col_content[col] = col_content[col].max(size.w);
            }
            if row < rows {
                if span.row_span() > 1 {
                    row_spans.push((span.row_start as usize, span.row_end as usize, size.h));
                } else {
                    row_content[row] = row_content[row].max(size.h);
                }
            }
        }

        // 没有跨格时走原路径：保证单格布局逐位不变（向后兼容）。
        self.col_widths = if col_spans.is_empty() {
            Self::resolve_tracks(&self.cols, inner_w, self.gap.0, |i| {
                col_content.get(i).copied().unwrap_or(0.0)
            })
        } else {
            let mut spans = col_spans;
            // 单格子控件也要作为约束参与求解，否则有跨格时它们的内容会被忽略
            for (j, v) in col_content.iter().enumerate() {
                if *v > 0.0 {
                    spans.push((j, j + 1, *v));
                }
            }
            Self::resolve_by_spans(&self.cols, inner_w, self.gap.0, &spans)
        };
        let inner_h = (constraints.max.h - self.padding.top - self.padding.bottom).max(0.0);
        self.row_heights = if row_spans.is_empty() {
            Self::resolve_tracks(&self.rows, inner_h, self.gap.1, |i| {
                row_content.get(i).copied().unwrap_or(0.0)
            })
        } else {
            let mut spans = row_spans;
            for (j, v) in row_content.iter().enumerate() {
                if *v > 0.0 {
                    spans.push((j, j + 1, *v));
                }
            }
            Self::resolve_by_spans(&self.rows, inner_h, self.gap.1, &spans)
        };

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
        // 跨格控件：裁剪到**区间并集**（含区间内的间距），而不是它起始那一格。
        // 否则"跨三列"的控件只能画在第一列里，跨格就白跨了。
        if let Some(id) = self
            .children
            .iter()
            .find(|c| c.rect() == cell_rect)
            .map(|c| c.id())
        {
            if let Some(span) = self.placements.get(id) {
                return Some(self.rect_for_span(*span));
            }
        }
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
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(Size::new(10.0, 10.0))
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
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
        assert_eq!(
            r[1],
            Rect::new(160.0, 200.0, 50.0, 20.0),
            "右上（含列间距 10）"
        );
        assert_eq!(
            r[2],
            Rect::new(100.0, 225.0, 50.0, 20.0),
            "左下（含行间距 5）"
        );
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

    // ================= 跨格归因（最长路）=================

    /// 可指定内在尺寸的叶子（模拟 min-content）。
    struct SizedLeaf {
        id: String,
        size: Size,
        rect: Rect,
    }

    impl SizedLeaf {
        fn new(id: &str, w: f32, h: f32) -> Self {
            Self {
                id: id.to_string(),
                size: Size::new(w, h),
                rect: Rect::ZERO,
            }
        }
    }

    impl Widget for SizedLeaf {
        fn id(&self) -> &str {
            &self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(self.size)
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn render(&mut self, _ctx: &mut dyn crate::RenderCtx) {}
    }

    /// 本地等宽度量（与其他测试同构）。
    struct Mono;

    impl TextMetrics for Mono {
        fn measure_text(&self, t: &str, _s: &crate::TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 8.0, 16.0)
        }
        fn line_height(&self, _s: &crate::TextStyle) -> f32 {
            16.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    /// 建一个只有 Auto 轨道的网格（内容决定宽度）。
    fn auto_grid(cols: usize, rows: usize, children: Vec<Box<dyn Widget>>) -> Grid {
        Grid::new(
            "g",
            vec![GridTrack::Auto; cols],
            vec![GridTrack::Auto; rows],
        )
        .gap(0.0, 0.0)
        .with_children(children)
    }

    fn measured(mut g: Grid, w: f32, h: f32) -> Grid {
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(w, h));
        let _ = g.measure(&Mono, c, &mut cache);
        g
    }

    #[test]
    fn spanning_columns_distribute_min_content_across_tracks() {
        // 跨 2 列，内在宽 150：两列总和必须 >= 150，且不得让第一列独吞。
        let mut g = auto_grid(2, 1, vec![Box::new(SizedLeaf::new("wide", 150.0, 10.0))]);
        g.place("wide", 0..2, 0..1).expect("跨两列");
        let g = measured(g, 400.0, 100.0);

        let total: f32 = g.col_widths.iter().sum();
        assert!(total >= 149.9, "两列总宽应 >= 150，实测 {total}");
        assert!(
            g.col_widths[0] <= 150.1 && g.col_widths[1] <= 150.1,
            "不应把 150 全塞进一列：{:?}",
            g.col_widths
        );
    }

    #[test]
    fn spanning_both_axes_solves_rows_and_columns_independently() {
        // 3×3：big 占左上 2×2，small 放在第三列第一行（不会冲突）
        let mut g = auto_grid(
            3,
            3,
            vec![
                Box::new(SizedLeaf::new("big", 150.0, 90.0)),
                Box::new(SizedLeaf::new("small", 40.0, 20.0)),
            ],
        );
        g.place("big", 0..2, 0..2).expect("跨 2×2");
        g.place("small", 2..3, 0..1).expect("右上格");
        let g = measured(g, 400.0, 300.0);

        let cw: f32 = g.col_widths.iter().sum();
        let rh: f32 = g.row_heights.iter().sum();
        assert!(cw >= 149.9, "列总宽应 >= 150，实测 {cw}");
        assert!(rh >= 89.9, "行总高应 >= 90，实测 {rh}");
        assert!(
            g.col_widths[0] >= 39.9,
            "单格控件的 40px 也要被满足，实测 {:?}",
            g.col_widths
        );
    }

    #[test]
    fn single_cell_children_are_not_affected_by_spans() {
        // 混合：跨格 + 单格。单格控件的内容宽不不应被跨格约束压缩。
        // 行 0 被 tall 占满，单格控件放在行 1
        let mut g = auto_grid(
            3,
            2,
            vec![
                Box::new(SizedLeaf::new("tall", 300.0, 10.0)),
                Box::new(SizedLeaf::new("one", 80.0, 10.0)),
            ],
        );
        g.place("tall", 0..3, 0..1).expect("跨三列");
        g.place("one", 0..1, 1..2).expect("单格");
        let g = measured(g, 500.0, 100.0);

        assert!(
            g.col_widths[0] >= 79.9,
            "单格控件 80px 应被满足，实测 {:?}",
            g.col_widths
        );
        let total: f32 = g.col_widths.iter().sum();
        assert!(total >= 299.9, "跨三列需 300，实测 {total}");
    }

    #[test]
    fn no_span_keeps_the_old_resolution_path() {
        // 一个都没跨格：走旧 `resolve_tracks`，Auto 就是各自内容宽。
        let g = auto_grid(
            2,
            1,
            vec![
                Box::new(SizedLeaf::new("a", 30.0, 10.0)),
                Box::new(SizedLeaf::new("b", 70.0, 10.0)),
            ],
        );
        let g = measured(g, 400.0, 100.0);
        assert_eq!(g.col_widths, vec![30.0, 70.0]);
    }

    #[test]
    fn empty_grid_does_not_panic() {
        // 0 个子控件、轨道为空：不能 panic，且尺寸退化为 0 而非 NaN
        let mut g = Grid::new("g", vec![], vec![]);
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(100.0, 100.0));
        let size = g.measure(&Mono, c, &mut cache);
        assert!(
            size.w.is_finite() && size.h.is_finite(),
            "尺寸必须有限：{size:?}"
        );
        assert!(size.w >= 0.0 && size.h >= 0.0);
        g.arrange(Rect::new(0.0, 0.0, 100.0, 100.0), &Mono, &mut cache);
        assert!(g.child_rects().is_empty());
    }

    #[test]
    fn single_cell_grid_gives_the_child_the_content_area() {
        // 1x1 网格 + 1 个子控件：子控件应占满内容区（扣除 padding）
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fraction(1.0)],
            vec![GridTrack::Fraction(1.0)],
        )
        .padding(Edges::all(10.0))
        .with_children(vec![Box::new(SizedLeaf::new("only", 10.0, 10.0))]);
        let mut cache = LayoutCache::new();
        let c = Constraints::loose(Size::new(100.0, 50.0));
        let _ = g.measure(&Mono, c, &mut cache);
        g.arrange(Rect::new(0.0, 0.0, 100.0, 50.0), &Mono, &mut cache);
        let r = g.child_rects()[0];
        assert_eq!(r, Rect::new(10.0, 10.0, 80.0, 30.0), "应占满内容区");
    }

    // ================= 自动流 + 跨格（span）=================

    #[test]
    fn span_places_auto_flow_child_in_next_free_slot() {
        // 4 列：a 显式占 0..2；b 要 2 列 → 下一个连续空位是 (2,0)
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0); 4],
            vec![GridTrack::Fixed(20.0); 2],
        )
        .gap(10.0, 5.0)
        .with_children(vec![
            Box::new(TestLeaf::new("a")),
            Box::new(TestLeaf::new("b")),
        ]);
        g.rect = Rect::new(0.0, 0.0, 400.0, 100.0);
        g.col_widths = vec![50.0; 4];
        g.row_heights = vec![20.0, 20.0];
        g.place("a", 0..2, 0..1).expect("a");
        g.span("b", 2, 1).expect("b 要 2 列");
        let _ = g.auto_cells(); // 生产中由 measure 调用；单元测试需显式跑一次
        let rects = g.child_rects();
        // x = 0 + (50+10)*2 = 120
        assert_eq!(
            rects[1],
            Rect::new(120.0, 0.0, 110.0, 20.0),
            "b 应落在 (2,0)"
        );
    }

    #[test]
    fn span_wraps_to_next_row_when_no_fit() {
        // 3 列：a 占 0..2；b 要 2 列 → 行 0 剩下只有 1 列，应换到行 1 的 (0,1)
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0); 3],
            vec![GridTrack::Fixed(20.0); 2],
        )
        .with_children(vec![
            Box::new(TestLeaf::new("a")),
            Box::new(TestLeaf::new("b")),
        ]);
        g.rect = Rect::ZERO;
        g.col_widths = vec![50.0; 3];
        g.row_heights = vec![20.0, 20.0];
        g.place("a", 0..2, 0..1).expect("a");
        g.span("b", 2, 1).expect("b 要 2 列");
        let _ = g.auto_cells();
        let rects = g.child_rects();
        assert_eq!(
            rects[1],
            Rect::new(0.0, 20.0, 100.0, 20.0),
            "b 应换行到 (0,1)"
        );
    }

    #[test]
    fn span_falls_back_to_1x1_when_the_grid_is_full() {
        // 场景设计（2x1 网格）：a 声明 2x1 占满两列；b 也要 2x1，但已无连续空位 →
        // 退化 1x1；而 1x1 也已没空格 → 按现有兜底落在 (0,0)。
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0); 2],
            vec![GridTrack::Fixed(20.0); 1],
        )
        .with_children(vec![
            Box::new(TestLeaf::new("a")),
            Box::new(TestLeaf::new("b")),
        ]);
        g.rect = Rect::ZERO;
        g.col_widths = vec![50.0, 50.0];
        g.row_heights = vec![20.0];
        g.span("a", 2, 1).expect("a 占满两列");
        g.span("b", 2, 1).expect("b 也要两列，应退化");
        let _ = g.auto_cells();
        let rects = g.child_rects();
        assert_eq!(rects[0], Rect::new(0.0, 0.0, 100.0, 20.0), "a 应占满");
        assert_eq!(
            rects[1],
            Rect::new(0.0, 0.0, 50.0, 20.0),
            "b 退化为 1×1 并兜底到 (0,0)"
        );
    }

    #[test]
    fn span_warns_only_once_per_id() {
        let mut g = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0); 2],
            vec![GridTrack::Fixed(20.0); 1],
        )
        .with_children(vec![Box::new(TestLeaf::new("a"))]);
        g.col_widths = vec![50.0, 50.0];
        g.row_heights = vec![20.0];
        g.span("a", 2, 1).expect("ok");
        let _ = g.auto_cells();
        assert!(g.warned_span_ids.is_empty(), "能放下时不应警告");

        // 再放一个放不下的
        let mut g2 = Grid::new(
            "g",
            vec![GridTrack::Fixed(50.0); 2],
            vec![GridTrack::Fixed(20.0); 1],
        )
        .with_children(vec![
            Box::new(TestLeaf::new("a")),
            Box::new(TestLeaf::new("b")),
        ]);
        g2.col_widths = vec![50.0, 50.0];
        g2.row_heights = vec![20.0];
        g2.span("a", 2, 1).expect("ok");
        g2.span("b", 2, 1).expect("ok");
        let _ = g2.auto_cells();
        let _ = g2.auto_cells();
        let _ = g2.auto_cells();
        assert_eq!(g2.warned_span_ids.len(), 1, "同一 id 只警告一次");
    }

    #[test]
    fn span_unknown_id_returns_err() {
        let mut g = grid_3x3();
        assert_eq!(
            g.span("nope", 2, 1),
            Err(PlacementError::UnknownChild {
                id: "nope".to_string()
            })
        );
    }

    #[test]
    fn span_zero_returns_err() {
        let mut g = grid_3x3();
        assert!(matches!(
            g.span("a", 0, 1),
            Err(PlacementError::EmptyRange { axis: "cols", .. })
        ));
        assert!(matches!(
            g.span("a", 1, 0),
            Err(PlacementError::EmptyRange { axis: "rows", .. })
        ));
        assert!(matches!(
            g.span("a", 9, 1),
            Err(PlacementError::OutOfBounds { axis: "cols", .. })
        ));
    }

    #[test]
    fn span_with_existing_place_keeps_the_placement() {
        let mut g = grid_3x3();
        g.place("a", 1..2, 0..1).expect("a 在 (1,0)");
        g.span("a", 2, 1).expect("span 不报错，但被忽略");
        assert_eq!(g.span_of("a"), None, "place 优先，span 不生效");
        assert_eq!(
            rect_of(&g, "a"),
            Rect::new(160.0, 200.0, 50.0, 20.0),
            "a 位置不变"
        );
    }
}
