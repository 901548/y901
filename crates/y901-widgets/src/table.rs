//! Table：多列表格，**支持排序、虚拟滚动与键盘导航**。
//!
//! 为什么需要它：`ListView` 是单列的虚拟列表，而工具型应用的第一个界面几乎都是
//! "名称 / 大小 / 修改时间"这种**多列表格**。
//!
//! ## 为什么是"重写"而不是在旧实现上打补丁
//!
//! v0.2.x 的 `Table` 是"直接遍历全部行 + 溢出交给父容器裁剪"：数据一多就每帧
//! 遍历上万个 cell，而且没有滚动能力（滚轮事件无处可去）。这次按 `ListView` 的
//! 虚拟化模型重写：**只画可见行**，表头固定，滚动条独立，行高统一。
//!
//! ## 仍然刻意不做
//!
//! 可变行高、单元格编辑、列宽拖拽、列冻结、多选、列内联过滤。
//! 行高统一是虚拟滚动的前提（`ListView` 的可变行高走 prefix 累加，表格暂不需要）。
//!
//! ## 排序语义
//!
//! - 点击表头：升序 → 降序 → 取消（三态循环），与桌面文件列表一致；
//! - 比较规则：两侧都能解析为数字时**按数值**，否则按小写字典序（稳定排序）；
//! - 排序只改**显示顺序**（`order`），不动 `rows` 本身；`selected_index()` 返回
//!   **数据下标**，因此排序不会让业务手里的选中行"变成另一行"；
//! - 设置 `sort_command` 时额外发一条命令，业务想自己排（例如服务端排序）就忽略内部排序。

use y901_ui::widget::{Dimension, FlexItem};
use y901_ui::{
    Align, Constraints, EventCtx, EventPhase, EventResult, InputKind, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 一列的定义。
///
/// `#[non_exhaustive]`：列的可选属性（排序开关、未来可能出现的列宽拖拽）只会增加，
/// 不设这个属性的话每加一项都是破坏性变更。
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TableColumn {
    /// 列标题。
    pub title: String,
    /// 列宽：`Px(w)` 固定；`Flex(f)` 按剩余宽度分配；`Percent(p)` 取整表宽度百分比。
    pub width: Dimension,
    /// 单元格文字对齐。
    pub align: Align,
    /// 是否允许点击表头排序（默认 `true`）。
    pub sortable: bool,
}

impl TableColumn {
    /// 固定宽度的列。
    pub fn fixed(title: impl Into<String>, width: f32) -> Self {
        Self {
            title: title.into(),
            width: Dimension::Px(width),
            align: Align::Start,
            sortable: true,
        }
    }

    /// 弹性列（均分剩余宽度）。
    pub fn flex(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: Dimension::Flex(1.0),
            align: Align::Start,
            sortable: true,
        }
    }

    /// 设置对齐。
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// 禁止点击本列表头排序（例如纯图标列）。
    pub fn not_sortable(mut self) -> Self {
        self.sortable = false;
        self
    }
}

/// 排序方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    /// 升序（A→Z / 小→大）。
    Ascending,
    /// 降序（Z→A / 大→小）。
    Descending,
}

impl SortOrder {
    /// 反转方向。
    pub fn reversed(self) -> Self {
        match self {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        }
    }
}

/// 表头高度（逻辑像素）。
const HEADER_H: f32 = 32.0;
/// 行高（所有行统一 —— 虚拟滚动的前提）。
const ROW_H: f32 = 28.0;
/// 单元格水平内边距。
const CELL_PAD: f32 = 12.0;
/// 弹性列的默认最小宽度。
const FLEX_MIN_W: f32 = 80.0;
/// 滚动条宽度。
const BAR_W: f32 = 6.0;

/// 多列数据表格。
pub struct Table {
    id: String,
    /// 无障碍名称（读屏器念它）。空 = 不提供名称。
    label: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
    columns: Vec<TableColumn>,
    rows: Vec<Vec<String>>,
    /// 显示顺序：`order[display] = data_index`。排序只动它。
    order: Vec<usize>,
    /// `order` 是否失效（排序变更或行数变更后置位）。
    order_dirty: bool,
    /// 选中行（**数据下标**，不随排序漂移）。
    selected: Option<usize>,
    /// 当前排序。
    sort: Option<(usize, SortOrder)>,
    /// 选中行时发出的命令（args = {"row": 数据下标}）。
    command: Option<y901_ui::Command>,
    /// 点击表头时额外发出的命令（args = {"column","descending"}）。
    sort_command: Option<y901_ui::Command>,
    /// 整个控件的矩形（含表头）。
    viewport: Rect,
    /// 行区域矩形（表头之下、滚动条之左）。
    inner: Rect,
    item: FlexItem,
    /// 可选样式覆盖（仅容器型表面）。
    style: Option<crate::style::WidgetStyle>,
    /// **DPI 缩放**（`measure` 里唯一写入点；未 measure 时 1.0 = 兜底）。
    scale: f32,
    /// 悬停行（数据下标）。
    hovered_row: Option<usize>,
    /// 悬停列（表头）。
    hovered_header: Option<usize>,
    /// 竖向滚动偏移（像素，作用在行区域上）。
    offset: f32,
    /// 是否显示滚动条（内容高于行区域时）。
    show_bar: bool,
    /// 滚动条几何（自持，不用 `Scrollbar`）—— 后者内含 `Box<dyn FnMut>`，
    /// 会把 `Table` 从 `Send + Sync` 变成 `!Send + !Sync`（实测被 cargo-semver-checks 抓到）。
    bar_rect: Rect,
    bar_visible: bool,
    bar_thumb_ratio: f32,
    bar_start_ratio: f32,
    bar_dragging: bool,
    bar_style: Option<crate::style::WidgetStyle>,
    /// 最近一次安排时可见的整行数（PageUp/PageDown 用）。
    page_rows: usize,
}

impl Table {
    /// 无障碍名称（读屏器念的名字；空 = 不提供）。
    ///
    /// 语义里曾经把内部 id（和行数）拼进名字，读屏器会把实现细节念出来。
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// 创建空表。
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            label: String::new(),
            id,
            key,
            columns: Vec::new(),
            rows: Vec::new(),
            order: Vec::new(),
            order_dirty: true,
            selected: None,
            sort: None,
            command: None,
            sort_command: None,
            viewport: Rect::ZERO,
            inner: Rect::ZERO,
            item: FlexItem::default(),
            style: None,
            scale: 1.0,
            hovered_row: None,
            hovered_header: None,
            offset: 0.0,
            show_bar: false,
            bar_rect: Rect::ZERO,
            bar_visible: false,
            bar_thumb_ratio: 1.0,
            bar_start_ratio: 0.0,
            bar_dragging: false,
            bar_style: None,
            page_rows: 1,
        }
    }

    /// 追加一列。
    pub fn column(mut self, column: TableColumn) -> Self {
        self.columns.push(column);
        self
    }

    /// 追加一行（单元格数量不足时补空串，多余时忽略）。
    pub fn row(mut self, cells: Vec<String>) -> Self {
        self.rows.push(cells);
        self.order_dirty = true;
        self.selected = self.selected.filter(|i| *i < self.rows.len());
        self
    }

    /// 批量替换全部行（数据刷新用；保留排序与选中）。
    pub fn set_rows(&mut self, rows: Vec<Vec<String>>) -> bool {
        if self.rows.len() == rows.len() && self.rows == rows {
            return false;
        }
        self.rows = rows;
        self.order_dirty = true;
        if let Some(sel) = self.selected {
            if sel >= self.rows.len() {
                self.selected = None;
            }
        }
        true
    }

    /// 设置选中行（数据下标；越界会被忽略）。
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index.filter(|i| *i < self.rows.len());
        self
    }

    /// 以编程方式设置选中行，并把该行滚入可见区。
    pub fn select(&mut self, index: Option<usize>) -> bool {
        let index = index.filter(|i| *i < self.rows.len());
        if self.selected == index {
            return false;
        }
        self.selected = index;
        self.scroll_selected_into_view();
        true
    }

    /// 点击行回调。
    pub fn command(mut self, cmd: y901_ui::Command) -> Self {
        self.command = Some(cmd);
        self
    }

    /// 点击表头排序时额外发出的命令（不设置则不通知，只做内部排序）。
    pub fn sort_command(mut self, cmd: y901_ui::Command) -> Self {
        self.sort_command = Some(cmd);
        self
    }

    /// 以编程方式设置排序（`None` = 恢复原始顺序）。
    pub fn sort_by(&mut self, column: usize, order: SortOrder) -> bool {
        if column >= self.columns.len() {
            return false;
        }
        if self.sort == Some((column, order)) {
            return false;
        }
        self.sort = Some((column, order));
        self.order_dirty = true;
        self.offset = 0.0;
        true
    }

    /// 当前排序。
    pub fn sort_state(&self) -> Option<(usize, SortOrder)> {
        self.sort
    }

    /// **覆盖容器型表面**（背景 / 描边 / 圆角）。
    ///
    /// 注意：语义色（如选中行、悬停行）**不**由它覆盖 —— 那些走 `Theme`，
    /// 否则会破坏 DARK/LIGHT 自适应。
    pub fn style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// 注入滚动条样式。
    pub fn bar_style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.bar_style = Some(style);
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 行数。
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// 当前选中行（**数据下标**）。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    // ---------------------------------------------------------------- 内部几何

    /// 行区域总高度（不含表头）。
    fn rows_height(&self) -> f32 {
        self.rows.len() as f32 * ROW_H * self.scale
    }

    /// 控件需要的高度（表头 + 全部行）。
    fn wanted_height(&self) -> f32 {
        HEADER_H * self.scale + self.rows_height()
    }

    /// 行区域最大滚动偏移。
    fn max_offset(&self) -> f32 {
        (self.rows_height() - self.inner.h).max(0.0)
    }

    fn row_h(&self) -> f32 {
        ROW_H * self.scale
    }

    /// 按最终宽度算每列的实际 `(x, w)`（宽度基准是**行区域**宽度）。
    ///
    /// 为什么公开：它是纯函数（输入列定义 + 当前 inner 宽度，输出几何），
    /// 几何断言比像素断言精确得多 —— 能直接报出"第 2 列起点越界"这种错误，
    /// 而不是靠截图差几个像素去猜。命中测试、表头绘制与行绘制共用它。
    pub fn column_layout(&self) -> Vec<(f32, f32)> {
        let n = self.columns.len();
        if n == 0 {
            return Vec::new();
        }
        let mut fixed = 0.0f32;
        let mut flex_weight = 0.0f32;
        let avail = self.inner.w.max(0.0);
        let mut percent_total = 0.0f32;
        for c in &self.columns {
            match c.width {
                Dimension::Px(v) => fixed += v.max(0.0) * self.scale,
                Dimension::Flex(f) => flex_weight += f.max(0.0),
                Dimension::Percent(p) => percent_total += avail * p.max(0.0) / 100.0,
                Dimension::Auto => fixed += FLEX_MIN_W,
            }
        }
        let rest = (avail - fixed - percent_total).max(0.0);
        // **`FLEX_MIN_W` 不能按列叠加**：旧实现每个 flex 列都取
        // `max(share, min(80, rest))`，两列各要 80 而 `rest` 只有 120 时总宽 160 > 120，
        // 第二列被视口裁掉、表头命中测试还会指向被裁的区域。
        // 现在的规则：share 先按权重分；只有当**所有 flex 列都分不到 80** 时才用
        // "每列 80 但整体按比例压缩"，保证 Σflex ≤ rest。
        let min_per_flex = FLEX_MIN_W.min(rest / (flex_weight.max(1.0)).max(1.0));

        let mut out = Vec::with_capacity(n);
        let mut x = self.inner.x;
        for c in &self.columns {
            let w = match c.width {
                Dimension::Px(v) => v.max(0.0) * self.scale,
                Dimension::Percent(p) => avail * p.max(0.0) / 100.0,
                Dimension::Flex(f) if flex_weight > 0.0 => {
                    // 按权重分份额，再保证不小于"每列最小宽"，但**总量不超过 rest**：
                    // `rest * f / weight` 已经满足 Σ = rest，最小值只是把它往上抬，
                    // 所以抬完要按剩余空间再夹一次。
                    let share = rest * f.max(0.0) / flex_weight;
                    share.max(min_per_flex).min(rest)
                }
                _ => rest.max(FLEX_MIN_W.min(rest)),
            };
            out.push((x, w));
            x += w;
        }
        // 超宽时按比例压缩（保证最后不超出行区域；`Percent` 列也可能超）。
        //
        // **判据必须是"存在 flex 列"（`flex_weight > 0.0`），不能是 `flex_total_w > 0.0`**：
        // `flex_total_w` 等于 `rest`（分给 flex 列的剩余空间），而定宽列**本身就超宽**时
        // `rest` 恰好是 0 —— 于是压缩被跳过、总宽超出可用宽度：
        //
        // ```text
        // 列：Px(7) Flex Flex Flex Px(119)，表格宽 93
        // 旧：fixed=126 → rest=0 → flex 列宽 0、flex_total_w=0 → 不压缩 → 总宽 126 > 93
        //     最后一列画在行区域之外（被裁掉），表头命中还会指向这块看不见的区域
        // 新：flex_weight=3 > 0 → 按 93/126 压缩 → 总宽 = 93
        // ```
        let total: f32 = out.iter().map(|(_, w)| *w).sum();
        if total > avail && total > 0.0 && flex_weight > 0.0 {
            let k = avail / total;
            for (_, w) in out.iter_mut() {
                *w *= k;
            }
            // 重新累加 x，保持列首尾相接
            let mut cx = self.inner.x;
            for (x, w) in out.iter_mut() {
                *x = cx;
                cx += *w;
            }
        }
        out
    }

    /// 表头绘制区域（固定在控件顶部）。
    fn header_rect(&self) -> Rect {
        Rect::new(
            self.inner.x,
            self.viewport.y,
            self.inner.w,
            HEADER_H * self.scale,
        )
    }

    /// 第 `display` 行（**显示序号**）的绘制区域。
    fn display_row_rect(&self, display: usize) -> Rect {
        Rect::new(
            self.inner.x,
            self.inner.y + display as f32 * self.row_h() - self.offset,
            self.inner.w,
            self.row_h(),
        )
    }

    /// 可见的显示行范围（半开区间）。
    fn visible_range(&self) -> std::ops::Range<usize> {
        let n = self.rows.len();
        if n == 0 || self.inner.h <= 0.0 {
            return 0..0;
        }
        let rh = self.row_h();
        let first = (self.offset / rh).floor().max(0.0) as usize;
        let count = (self.inner.h / rh).ceil() as usize + 1;
        let first = first.min(n);
        let end = (first + count).min(n);
        first..end
    }

    /// 滚动条轨道内的比例（0 = 顶部，1 = 底部）。
    fn bar_ratio_at(&self, y: f32) -> f32 {
        if self.bar_rect.h <= 0.0 {
            return 0.0;
        }
        ((y - self.bar_rect.y) / self.bar_rect.h).clamp(0.0, 1.0)
    }

    /// 滑块矩形（与 `Scrollbar` 的几何保持一致：最小 24px 高、左右留 2px）。
    fn bar_thumb(&self) -> Rect {
        let h = (self.bar_rect.h * self.bar_thumb_ratio).max(24.0);
        let y = self.bar_rect.y + (self.bar_rect.h - h) * self.bar_start_ratio;
        Rect::new(
            self.bar_rect.x + 2.0,
            y,
            (self.bar_rect.w - 4.0).max(2.0),
            h,
        )
    }

    /// 画滚动条（轨道 + 滑块）。
    fn render_bar(&self, ctx: &mut dyn RenderCtx) {
        if !self.bar_visible {
            return;
        }
        let (track, thumb) = {
            let theme = ctx.theme();
            let state = if self.bar_dragging {
                State::Pressed
            } else {
                State::Normal
            };
            (
                theme.style(Role("scrollbar"), State::Disabled).bg,
                theme.style(Role("scrollbar"), state).bg,
            )
        };
        match self.bar_style {
            Some(s) => {
                let w = if s.bar_width > 0.0 {
                    s.bar_width.min(self.bar_rect.w)
                } else {
                    self.bar_rect.w
                };
                let x = self.bar_rect.right() - w;
                let track_rect = Rect::new(x, self.bar_rect.y, w, self.bar_rect.h);
                ctx.fill_round_rect(track_rect, s.bar_track.unwrap_or(track), s.bar_radius);
                let t = self.bar_thumb();
                ctx.fill_round_rect(
                    Rect::new(x, t.y, w, t.h),
                    s.bar_thumb.unwrap_or(thumb),
                    s.bar_radius,
                );
            }
            None => {
                ctx.fill_rect(self.bar_rect, track);
                ctx.fill_rect(self.bar_thumb(), thumb);
            }
        }
    }

    /// 命中测试：返回表头列号（`pos` 在表头内且该列可排序）。
    fn header_at(&self, pos: Pos) -> Option<usize> {
        let header = self.header_rect();
        if !header.contains(pos) {
            return None;
        }
        let cols = self.column_layout();
        let idx = cols
            .iter()
            .position(|(x, w)| pos.x >= *x && pos.x < *x + *w)?;
        self.columns.get(idx).filter(|c| c.sortable).map(|_| idx)
    }

    /// 命中测试：返回**数据下标**（`pos` 在行区域内）。
    fn row_at(&self, pos: Pos) -> Option<usize> {
        if !self.inner.contains(pos) {
            return None;
        }
        let y = pos.y - self.inner.y + self.offset;
        if y < 0.0 {
            return None;
        }
        let display = (y / self.row_h()) as usize;
        self.order.get(display).copied()
    }

    /// 重建显示顺序（排序 + 稳定比较）。
    fn rebuild_order(&mut self) {
        let n = self.rows.len();
        self.order.clear();
        self.order.extend(0..n);
        if let Some((col, order)) = self.sort {
            let rows = &self.rows;
            self.order.sort_by(|a, b| {
                let va = rows[*a].get(col).map(String::as_str).unwrap_or("");
                let vb = rows[*b].get(col).map(String::as_str).unwrap_or("");
                let cmp = compare_cells(va, vb);
                match order {
                    SortOrder::Ascending => cmp,
                    SortOrder::Descending => cmp.reverse(),
                }
            });
        }
        self.order_dirty = false;
    }

    /// 保证 `order` 可用（长度与行数一致）。
    fn ensure_order(&mut self) {
        if self.order_dirty || self.order.len() != self.rows.len() {
            self.rebuild_order();
        }
    }

    /// 数据下标在显示顺序中的位置。
    fn display_of(&mut self, data_index: usize) -> Option<usize> {
        self.ensure_order();
        self.order.iter().position(|i| *i == data_index)
    }

    fn set_offset(&mut self, v: f32) -> bool {
        let v = v.clamp(0.0, self.max_offset());
        if (self.offset - v).abs() < 0.01 {
            return false;
        }
        self.offset = v;
        true
    }

    /// 把选中行滚进可见区（键盘导航后调用）。
    fn scroll_selected_into_view(&mut self) -> bool {
        let Some(sel) = self.selected else {
            return false;
        };
        let Some(display) = self.display_of(sel) else {
            return false;
        };
        let rh = self.row_h();
        let top = display as f32 * rh;
        let bottom = top + rh;
        if top < self.offset {
            return self.set_offset(top);
        }
        if bottom > self.offset + self.inner.h {
            return self.set_offset(bottom - self.inner.h);
        }
        false
    }

    /// 三态循环：无 → 升 → 降 → 无。
    fn cycle_sort(&mut self, column: usize) {
        self.sort = match self.sort {
            Some((c, SortOrder::Ascending)) if c == column => Some((column, SortOrder::Descending)),
            Some((c, SortOrder::Descending)) if c == column => None,
            _ => Some((column, SortOrder::Ascending)),
        };
        self.order_dirty = true;
        self.offset = 0.0;
    }

    fn emit_select(&self, ctx: &mut EventCtx, data_index: usize) {
        if let Some(cmd) = self.command.as_ref() {
            let args = y901_ui::json::Json::obj().set("row", data_index as i32);
            ctx.emit_command(crate::command_with_args(cmd, args));
        }
    }
}

impl Widget for Table {
    /// **键盘导航可达**：`UiTree` 只把 `Key` 发给焦点节点，且 `set_focus` 拒绝
    /// 非 `focusable` 节点。不声明这一条，`event` 里整套 ↓/↑/Home/End/PageUp/PageDown
    /// 与 `scroll_selected_into_view` 永远不会被调用（模块文档承诺的键盘导航是死的）。
    fn focusable(&self) -> bool {
        true
    }
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        self.ensure_order();
        // **小表按内容高度、大表按可用高度**：这样 v0.2.x 的"表格自然撑开"
        // 行为对小数据不变，只有超出可用高度的表才进入虚拟滚动。
        let want_h = self.wanted_height().min(c.max.h);
        let want_w = c.max.w;
        c.constrain(Size::new(want_w, want_h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.ensure_order();
        self.viewport = rect;
        let header_h = HEADER_H * self.scale;
        let rows_h = (rect.h - header_h).max(0.0);
        // 先按"无滚动条"算一次，再判断是否需要滚动条（避免滚动条自身引起反复抖动）。
        self.show_bar = self.rows_height() > rows_h + 0.5;
        let bar_w = if self.show_bar { BAR_W } else { 0.0 };
        self.inner = Rect::new(rect.x, rect.y + header_h, (rect.w - bar_w).max(0.0), rows_h);
        self.offset = self.offset.clamp(0.0, self.max_offset());
        self.page_rows = ((rows_h / self.row_h()).floor() as usize).max(1);

        let content = self.rows_height();
        self.bar_visible = self.show_bar;
        self.bar_rect = Rect::new(rect.right() - BAR_W, rect.y + header_h, BAR_W, rows_h);
        self.bar_thumb_ratio = if content > 0.0 {
            (rows_h / content).clamp(0.05, 1.0)
        } else {
            1.0
        };
        let start = if self.max_offset() > 0.0 {
            self.offset / self.max_offset()
        } else {
            0.0
        };
        self.bar_start_ratio = start.clamp(0.0, 1.0 - self.bar_thumb_ratio);
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.viewport
    }

    fn clip(&self) -> Option<Rect> {
        Some(self.viewport)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if ctx.phase == EventPhase::Capture {
            return EventResult::Ignored;
        }
        // 顺序即优先级：滚动条 → 表头 → 行。
        match event {
            UiEvent::MouseDown { pos, .. } if self.bar_visible && self.bar_rect.contains(*pos) => {
                ctx.set_pointer_capture(self.id.clone());
                self.bar_dragging = true;
                let ratio = self.bar_ratio_at(pos.y) - self.bar_thumb_ratio * 0.5;
                if self.set_offset(ratio * self.max_offset()) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::MouseMove { pos } if self.bar_dragging => {
                let ratio = self.bar_ratio_at(pos.y) - self.bar_thumb_ratio * 0.5;
                if self.set_offset(ratio * self.max_offset()) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::MouseUp { .. } if self.bar_dragging => {
                self.bar_dragging = false;
                ctx.release_pointer_capture();
                EventResult::Redraw
            }
            UiEvent::MouseMove { pos } => {
                let header = self.header_at(*pos);
                let row = self.row_at(*pos);
                let changed = header != self.hovered_header || row != self.hovered_row;
                self.hovered_header = header;
                self.hovered_row = row;
                if changed {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::MouseUp { pos, .. } => {
                if let Some(column) = self.header_at(*pos) {
                    self.cycle_sort(column);
                    if let Some(cmd) = self.sort_command.as_ref() {
                        // **第三态（取消排序）也要如实上报**：旧实现用
                        // `unwrap_or((column, Ascending))`，于是"点第三次把排序清掉"时
                        // 应用收到的却是"升序" —— 表头没有箭头、命令却说在升序排。
                        // `column = -1` 表示不排序（与 `sort` 为 `None` 同义）。
                        let (col, descending) = match self.sort {
                            Some((c, SortOrder::Descending)) => (c as i32, 1),
                            Some((c, SortOrder::Ascending)) => (c as i32, 0),
                            None => (-1, 0),
                        };
                        let args = y901_ui::json::Json::obj()
                            .set("column", col)
                            .set("descending", descending);
                        ctx.emit_command(crate::command_with_args(cmd, args));
                    }
                    return EventResult::Relayout;
                }
                if let Some(idx) = self.row_at(*pos) {
                    self.selected = Some(idx);
                    self.emit_select(ctx, idx);
                    return EventResult::Redraw;
                }
                EventResult::Ignored
            }
            UiEvent::Wheel { pos, delta, .. } => {
                if !self.viewport.contains(*pos) {
                    return EventResult::Ignored;
                }
                let step = *delta / 120.0 * self.row_h() * 3.0;
                if self.set_offset(self.offset - step) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Key { key, .. } => {
                use y901_ui::Key;
                let n = self.rows.len();
                if n == 0 {
                    return EventResult::Ignored;
                }
                let cur_display = match self.selected {
                    Some(sel) => self.display_of(sel),
                    None => None,
                };
                let target_display = match (key, cur_display) {
                    (Key::Down, None) => Some(0),
                    (Key::Down, Some(d)) => Some((d + 1).min(n - 1)),
                    (Key::Up, None) => Some(n - 1),
                    (Key::Up, Some(d)) => Some(d.saturating_sub(1)),
                    (Key::Home, _) => Some(0),
                    (Key::End, _) => Some(n - 1),
                    (Key::PageDown, None) => Some(0),
                    (Key::PageDown, Some(d)) => Some((d + self.page_rows).min(n - 1)),
                    (Key::PageUp, None) => Some(n - 1),
                    (Key::PageUp, Some(d)) => Some(d.saturating_sub(self.page_rows)),
                    _ => None,
                };
                match target_display {
                    Some(d) => {
                        let data = self.order.get(d).copied().unwrap_or(d);
                        if Some(data) == self.selected {
                            // 到边界：不重绘，但表示"键已被消费"（避免父容器再滚一层）。
                            return EventResult::Handled;
                        }
                        self.selected = Some(data);
                        self.scroll_selected_into_view();
                        self.emit_select(ctx, data);
                        EventResult::Relayout
                    }
                    None => EventResult::Ignored,
                }
            }
            UiEvent::Action { name, args } if name == "sort" => {
                let col = args.get("column").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                if col < 0.0 || col as usize >= self.columns.len() {
                    return EventResult::Ignored;
                }
                let desc = args
                    .get("descending")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    > 0.0;
                let order = if desc {
                    SortOrder::Descending
                } else {
                    SortOrder::Ascending
                };
                if self.sort_by(col as usize, order) {
                    EventResult::Relayout
                } else {
                    EventResult::Handled
                }
            }
            UiEvent::Action { name, args } if name == "select_row" => {
                let row = args.get("row").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                if row < 0.0 || row as usize >= self.rows.len() {
                    return EventResult::Ignored;
                }
                self.select(Some(row as usize));
                EventResult::Relayout
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.viewport.w <= 0.0 || self.viewport.h <= 0.0 {
            return;
        }
        self.ensure_order();
        // **先把用到的颜色全部取成值**：`ctx.theme()` 是借用，取完才能再可变借用
        // `ctx` 去绘制（否则每个 `fill_rect` 都会被借用检查判冲突）。
        let (header_bg, row_bg, alt_bg, hover_bg, sel_bg, text_fg, muted, sep) = {
            let theme = ctx.theme();
            (
                self.style
                    .and_then(|s| s.bg)
                    .unwrap_or(theme.style(Role::LIST_ITEM, State::Hovered).bg),
                self.style
                    .and_then(|s| s.bg)
                    .unwrap_or(theme.style(Role::PANEL, State::Normal).bg),
                theme.style(Role::PANEL, State::Hovered).bg,
                theme.style(Role::LIST_ITEM, State::Hovered).bg,
                theme.style(Role::LIST_ITEM, State::Pressed).bg,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.muted_text_color(),
                theme.style(Role::PANEL, State::Disabled).bg,
            )
        };

        ctx.push_clip(self.viewport);

        // ---- 行（先画，表头覆盖在上面，保证滚动时表头是"粘"的）
        ctx.fill_rect(self.inner, row_bg);
        let cols = self.column_layout();
        let range = self.visible_range();
        for display in range {
            let Some(&data) = self.order.get(display) else {
                continue;
            };
            let row_rect = self.display_row_rect(display);
            if row_rect.bottom() < self.inner.y || row_rect.y > self.inner.bottom() {
                continue;
            }
            let bg = if self.selected == Some(data) {
                sel_bg
            } else if self.hovered_row == Some(data) {
                hover_bg
            } else if display % 2 == 1 {
                alt_bg
            } else {
                row_bg
            };
            ctx.fill_rect(row_rect, bg);
            let style = TextStyle::fg(text_fg);
            if let Some(cells) = self.rows.get(data) {
                for (i, col) in self.columns.iter().enumerate() {
                    let Some((x, w)) = cols.get(i).copied() else {
                        break;
                    };
                    let text = cells.get(i).map(String::as_str).unwrap_or("");
                    draw_cell(
                        ctx, col, text, x, row_rect.y, w, row_rect.h, &style, self.scale,
                    );
                }
            }
        }

        // ---- 表头（固定）
        let header = self.header_rect();
        ctx.fill_rect(header, header_bg);
        let header_style = TextStyle::fg(muted);
        let active_style = TextStyle::fg(text_fg);
        for (i, col) in self.columns.iter().enumerate() {
            let Some((x, w)) = cols.get(i).copied() else {
                break;
            };
            let sorted = self.sort.filter(|(c, _)| *c == i);
            let style = if sorted.is_some() {
                &active_style
            } else {
                &header_style
            };
            let label = match sorted {
                Some((_, SortOrder::Ascending)) => format!("{} ▲", col.title),
                Some((_, SortOrder::Descending)) => format!("{} ▼", col.title),
                None => col.title.clone(),
            };
            draw_cell(
                ctx, col, &label, x, header.y, w, header.h, style, self.scale,
            );
            // 表头底部分隔线（1px，固定 DPI 缩放后仍为 1 物理像素附近）
            ctx.fill_rect(Rect::new(x, header.bottom() - 1.0, w, 1.0), sep);
        }

        ctx.pop_clip();
        self.render_bar(ctx);
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(true)
            // 名字里不念 id（真机 UIA 验收的缺陷之一）；行数留在 `state`/`prop` 里。
            .label(self.label.clone())
            .prop("rows", self.rows.len().to_string())
            .prop("rows", self.rows.len().to_string())
            .prop("columns", self.columns.len().to_string())
            .action_spec("select_row", InputKind::Number)
            .action_spec("sort", InputKind::Number)
            .state("offset", self.offset)
            .state("first_visible", self.visible_range().start as u32);
        if let Some(i) = self.selected {
            s = s.value(format!("row {i}")).state("selected", i as u32);
        }
        if let Some((col, order)) = self.sort {
            s = s.prop("sort_column", col.to_string()).prop(
                "sort_order",
                match order {
                    SortOrder::Ascending => "ascending",
                    SortOrder::Descending => "descending",
                },
            );
        }
        s
    }
}

/// 单元格比较：两侧都是数字时比数值，否则比小写字典序。
fn compare_cells(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

/// 在单元格矩形内按 `col.align` 画一行文字。
#[allow(clippy::too_many_arguments)] // 纯内部绘制辅助：坐标/尺寸/样式都是显式输入
fn draw_cell(
    ctx: &mut dyn RenderCtx,
    col: &TableColumn,
    text: &str,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    style: &TextStyle,
    scale: f32,
) {
    if text.is_empty() {
        return;
    }
    let tw = ctx.measure_text(text, style).w;
    let tx = match col.align {
        Align::Center => x + (w - tw) * 0.5,
        Align::End => x + w - CELL_PAD * scale - tw,
        _ => x + CELL_PAD * scale,
    };
    let ty = y + (h - ctx.line_height(style)) * 0.5;
    ctx.draw_text(Pos::new(tx, ty), text, style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use y901_ui::Color;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    #[derive(Default)]
    struct RectRecorder {
        fills: Vec<Rect>,
    }

    impl TextMetrics for RectRecorder {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    impl RenderCtx for RectRecorder {
        fn size(&self) -> Size {
            Size::new(400.0, 300.0)
        }
        fn fill_rect(&mut self, rect: Rect, _color: Color) {
            self.fills.push(rect);
        }
        fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {}
        fn draw_text(&mut self, _origin: Pos, _text: &str, _style: &TextStyle) -> f32 {
            0.0
        }
        fn push_clip(&mut self, _rect: Rect) {}
        fn pop_clip(&mut self) {}
    }

    fn arrange(t: &mut Table, w: f32, h: f32) -> &mut Table {
        let mut cache = LayoutCache::new();
        t.measure(&Metrics, Constraints::loose(Size::new(w, h)), &mut cache);
        t.arrange(Rect::new(0.0, 0.0, w, h), &Metrics, &mut cache);
        t
    }

    fn sorted_table(rows: Vec<Vec<String>>) -> Table {
        let mut t = Table::new("t").column(TableColumn::fixed("name", 100.0));
        t.set_rows(rows);
        t
    }

    fn names(t: &mut Table) -> Vec<String> {
        t.ensure_order();
        t.order
            .iter()
            .map(|i| t.rows[*i][0].clone())
            .collect::<Vec<_>>()
    }

    #[test]
    fn construct_and_measure() {
        let mut t = Table::new("t")
            .column(TableColumn::flex("名称"))
            .column(TableColumn::fixed("大小", 80.0))
            .row(vec!["a".into(), "1".into()])
            .row(vec!["b".into(), "2".into()])
            .selected(Some(1));
        assert_eq!(t.len(), 2);
        assert_eq!(t.selected_index(), Some(1));
        let mut cache = LayoutCache::new();
        let s = t.measure(
            &Metrics,
            Constraints::loose(Size::new(400.0, 400.0)),
            &mut cache,
        );
        assert_eq!(s.h, HEADER_H + 2.0 * ROW_H, "小表按内容高度");
        assert!(s.w <= 400.0);
    }

    #[test]
    fn large_table_is_capped_to_available_height() {
        let mut t = Table::new("t").column(TableColumn::flex("a"));
        for i in 0..10_000 {
            t = t.row(vec![format!("r{i}")]);
        }
        let mut cache = LayoutCache::new();
        let s = t.measure(
            &Metrics,
            Constraints::loose(Size::new(400.0, 300.0)),
            &mut cache,
        );
        assert_eq!(s.h, 300.0, "超出可用高度时被裁到可用高度");
    }

    #[test]
    fn top_down_row_geometry_matches_hit_testing() {
        let mut t = Table::new("t")
            .column(TableColumn::flex("c"))
            .row(vec!["first".into()])
            .row(vec!["second".into()]);
        arrange(&mut t, 200.0, 120.0);

        let header = t.header_rect();
        let row0 = t.display_row_rect(0);
        let row1 = t.display_row_rect(1);
        assert_eq!(header, Rect::new(0.0, 0.0, 200.0, HEADER_H));
        assert_eq!(row0, Rect::new(0.0, HEADER_H, 200.0, ROW_H));
        assert_eq!(row1, Rect::new(0.0, HEADER_H + ROW_H, 200.0, ROW_H));
        assert!(header.bottom() <= row0.y, "表头应在第一行上方");

        let x = 4.0;
        assert_eq!(t.row_at(Pos::new(x, header.y + 1.0)), None, "表头不命中行");
        assert_eq!(t.row_at(Pos::new(x, row0.y + row0.h * 0.5)), Some(0));
        assert_eq!(t.row_at(Pos::new(x, row1.y + row1.h * 0.5)), Some(1));
        assert_eq!(
            t.row_at(Pos::new(x, row0.bottom())),
            Some(1),
            "边界归属下行"
        );

        let mut ctx = EventCtx::new(None, Size::new(400.0, 300.0));
        t.event(
            &mut ctx,
            &UiEvent::MouseUp {
                pos: Pos::new(x, row1.y + row1.h * 0.5),
                button: y901_ui::MouseButton::Left,
            },
        );
        assert_eq!(t.selected_index(), Some(1));
    }

    #[test]
    fn render_draws_rows_then_sticky_header() {
        let mut t = Table::new("t")
            .column(TableColumn::fixed("c", 100.0))
            .row(vec!["first".into()])
            .row(vec!["second".into()]);
        arrange(&mut t, 200.0, 120.0);
        let mut ctx = RectRecorder::default();
        t.render(&mut ctx);
        // 背景 + 2 行 + 表头 + 表头分隔线
        assert!(ctx.fills.len() >= 4, "至少画背景/两行/表头");
        assert_eq!(ctx.fills[0], t.inner, "第一笔是行区域背景");
        assert_eq!(
            ctx.fills[ctx.fills.len() - 2],
            t.header_rect(),
            "表头在行之后绘制（因此滚动时始终可见）"
        );
    }

    #[test]
    fn only_visible_rows_are_drawn() {
        let mut t = Table::new("t").column(TableColumn::fixed("c", 100.0));
        for i in 0..5000 {
            t = t.row(vec![format!("r{i}")]);
        }
        arrange(&mut t, 200.0, 200.0);
        let mut ctx = RectRecorder::default();
        t.render(&mut ctx);
        // 200 高处约 6 行 + 背景 + 表头 + 分隔线；远小于 5000
        assert!(ctx.fills.len() < 20, "只画可见行，实画 {}", ctx.fills.len());
        assert!(t.visible_range().len() <= 8);
    }

    #[test]
    fn sorting_numbers_and_text_and_toggles_three_states() {
        let mut t = sorted_table(vec![vec!["10".into()], vec!["2".into()], vec!["1".into()]]);
        t.sort_by(0, SortOrder::Ascending);
        assert_eq!(names(&mut t), vec!["1", "2", "10"], "数字按数值排序");
        t.sort_by(0, SortOrder::Descending);
        assert_eq!(names(&mut t), vec!["10", "2", "1"]);

        let mut text = sorted_table(vec![vec!["b".into()], vec!["A".into()], vec!["c".into()]]);
        text.sort_by(0, SortOrder::Ascending);
        assert_eq!(names(&mut text), vec!["A", "b", "c"], "文本忽略大小写");

        // 三态循环：升 → 降 → 取消
        text.cycle_sort(0);
        assert_eq!(
            text.sort_state(),
            Some((0, SortOrder::Descending)),
            "升 → 降"
        );
        text.cycle_sort(0);
        assert_eq!(text.sort_state(), None, "降 → 取消排序");
        assert_eq!(names(&mut text), vec!["b", "A", "c"], "取消后恢复原始顺序");
    }

    #[test]
    fn selection_follows_data_row_not_display_position() {
        let mut t = sorted_table(vec![vec!["3".into()], vec!["1".into()], vec!["2".into()]]);
        t.selected = Some(0); // 数据行 "3"
        t.sort_by(0, SortOrder::Ascending);
        assert_eq!(names(&mut t), vec!["1", "2", "3"]);
        assert_eq!(
            t.selected_index(),
            Some(0),
            "排序不应改变选中的数据行（仍是 '3'）"
        );
        // 显示位置是最后一个
        assert_eq!(t.display_of(0), Some(2));
    }

    #[test]
    fn keyboard_navigation_and_page_keys() {
        let mut t = Table::new("t").column(TableColumn::fixed("c", 100.0));
        for i in 0..100 {
            t = t.row(vec![format!("r{i}")]);
        }
        arrange(&mut t, 200.0, 200.0);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 300.0));
        let press = |t: &mut Table, ctx: &mut EventCtx, key| {
            t.event(
                ctx,
                &UiEvent::Key {
                    key,
                    modifiers: y901_ui::Modifiers::default(),
                },
            )
        };
        press(&mut t, &mut ctx, y901_ui::Key::Down);
        assert_eq!(t.selected_index(), Some(0), "↓ 从无选中开始选第一行");
        press(&mut t, &mut ctx, y901_ui::Key::End);
        assert_eq!(t.selected_index(), Some(99), "End 选最后一行");
        press(&mut t, &mut ctx, y901_ui::Key::Home);
        assert_eq!(t.selected_index(), Some(0), "Home 选第一行");
        press(&mut t, &mut ctx, y901_ui::Key::PageDown);
        // 旧版只断言 `>= 3`：page_rows 恰好是 3 时它永远成立，
        // 就算 PageDown 被实现成"前进 3 行"（一页的巧合）也抓不到。
        // 现在要求**精确等于一页的步长**，并从第 0 行出发（不是从中间）。
        assert!(
            t.page_rows >= 2,
            "前置：PageDown 才有意义（page_rows={}，200px 高 / 每行高度算出来至少 2 行）",
            t.page_rows
        );
        assert_eq!(
            t.selected_index(),
            Some(t.page_rows.min(99)),
            "PageDown 必须恰好前进 page_rows 行（page_rows={}）",
            t.page_rows
        );
        press(&mut t, &mut ctx, y901_ui::Key::PageUp);
        assert_eq!(
            t.selected_index(),
            Some(0),
            "PageUp 必须恰好退回一页（不是退到 -1 也不是原地不动）"
        );
    }

    #[test]
    fn wheel_scrolls_and_clamps() {
        let mut t = Table::new("t").column(TableColumn::fixed("c", 100.0));
        for i in 0..100 {
            t = t.row(vec![format!("r{i}")]);
        }
        arrange(&mut t, 200.0, 200.0);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 300.0));
        t.event(
            &mut ctx,
            &UiEvent::Wheel {
                pos: Pos::new(10.0, 100.0),
                delta: -360.0,
                mods: y901_ui::Modifiers::default(),
            },
        );
        assert!(t.offset > 0.0, "向下滚动应增加 offset");
        let max = t.max_offset();
        t.set_offset(100_000.0);
        assert_eq!(t.offset, max, "offset 被 clamp 到最大偏移");
        assert_eq!(t.selected_index(), None, "滚动不改变选中");
    }

    #[test]
    fn sticky_header_hit_test_sorts_on_click() {
        let mut t = sorted_table(vec![vec!["b".into()], vec!["a".into()]]);
        arrange(&mut t, 200.0, 200.0);
        let header = t.header_rect();
        assert_eq!(
            t.header_at(Pos::new(header.x + 5.0, header.y + 5.0)),
            Some(0)
        );
        let mut ctx = EventCtx::new(None, Size::new(400.0, 300.0));
        t.event(
            &mut ctx,
            &UiEvent::MouseUp {
                pos: Pos::new(header.x + 5.0, header.y + 5.0),
                button: y901_ui::MouseButton::Left,
            },
        );
        assert_eq!(t.sort_state(), Some((0, SortOrder::Ascending)));
        assert_eq!(names(&mut t), vec!["a", "b"]);
    }

    #[test]
    fn percent_column_takes_share_of_total_width() {
        let mut t = Table::new("t")
            .column(TableColumn::flex("a"))
            .column(TableColumn {
                title: "b".to_string(),
                width: Dimension::Percent(50.0),
                align: Align::Start,
                sortable: true,
            })
            .row(vec!["a".into(), "b".into()]);
        arrange(&mut t, 400.0, 200.0);
        let cols = t.column_layout();
        assert!(
            (cols[1].1 - 200.0).abs() < 0.5,
            "Percent(50) 应为 200，实测 {}",
            cols[1].1
        );
        assert!(
            (cols[0].1 - 200.0).abs() < 0.5,
            "flex 应剩 200，实测 {}",
            cols[0].1
        );
    }

    #[test]
    fn exposes_select_and_sort_actions() {
        let t = Table::new("t")
            .column(TableColumn::flex("a"))
            .row(vec!["x".into()]);
        let acts = t.semantics().actions;
        assert!(acts.iter().any(|a| a.name == "select_row"));
        assert!(acts.iter().any(|a| a.name == "sort"));
    }

    #[test]
    fn semantics_reports_sort_and_visible_window() {
        let mut t = Table::new("t").column(TableColumn::flex("a"));
        for i in 0..50 {
            t = t.row(vec![format!("r{i}")]);
        }
        arrange(&mut t, 200.0, 200.0);
        t.sort_by(0, SortOrder::Descending);
        let s = t.semantics();
        assert_eq!(
            s.state.get("first_visible").and_then(|v| v.as_f64()),
            Some(0.0)
        );
        let props: Vec<_> = s
            .props
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(props.contains(&("sort_column", "0")));
        assert!(props.contains(&("sort_order", "descending")));
    }

    #[test]
    fn style_is_stored_for_container_surface() {
        let st = crate::style::WidgetStyle::default().bg(Color::rgba(0.2, 0.2, 0.2, 1.0));
        let t = Table::new("t").column(TableColumn::flex("a")).style(st);
        assert_eq!(t.style, Some(st));
    }

    #[test]
    fn set_rows_preserves_sort_and_drops_out_of_range_selection() {
        let mut t = sorted_table(vec![vec!["2".into()], vec!["1".into()]]);
        t.sort_by(0, SortOrder::Ascending);
        t.selected = Some(1);
        t.set_rows(vec![vec!["9".into()]]);
        assert_eq!(t.selected_index(), None, "越界选中被清空");
        assert_eq!(names(&mut t), vec!["9"], "新行沿用当前排序");
    }
}
