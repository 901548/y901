//! Table：多列 + 表头 + 选中行的**最小可用**数据表格。
//!
//! 为什么需要它：`ListView` 是单列的虚拟列表，而工具型应用的第一个界面几乎都是
//! "名称 / 大小 / 修改时间"这种**多列表格**（Helm 的文件表就是手写的 800 行）。
//!
//! 当前版本**刻意不做**：排序、拖拽列宽、多选、单元格编辑、列冻结、虚拟滚动。
//! 行数据用 `Vec<String>` 简化（后续可泛型化）。

use y901_ui::widget::{Dimension, FlexItem};
use y901_ui::{
    Align, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextStyle, TextMetrics, UiEvent, Widget,
};

/// 一列的定义。
#[derive(Clone, Debug)]
pub struct TableColumn {
    /// 列标题。
    pub title: String,
    /// 列宽：`Px(w)` 固定；`Flex(f)` 按剩余宽度分配。
    pub width: Dimension,
    /// 单元格文字对齐。
    pub align: Align,
}

impl TableColumn {
    /// 固定宽度的列。
    pub fn fixed(title: impl Into<String>, width: f32) -> Self {
        Self {
            title: title.into(),
            width: Dimension::Px(width),
            align: Align::Start,
        }
    }

    /// 弹性列（均分剩余宽度）。
    pub fn flex(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: Dimension::Flex(1.0),
            align: Align::Start,
        }
    }

    /// 设置对齐。
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }
}

/// 表头高度（逻辑像素）。
const HEADER_H: f32 = 32.0;
/// 行高。
const ROW_H: f32 = 28.0;
/// 单元格水平内边距。
const CELL_PAD: f32 = 12.0;
/// 弹性列的默认最小宽度。
const FLEX_MIN_W: f32 = 80.0;

/// 多列数据表格（最小可用版）。
pub struct Table {
    id: String,
    columns: Vec<TableColumn>,
    rows: Vec<Vec<String>>,
    selected: Option<usize>,
    /// 选中行时发出的命令（args = {"row": index}）。
    command: Option<y901_ui::Command>,
    rect: Rect,
    item: FlexItem,
    /// 可选样式覆盖（仅容器型表面）。
    style: Option<crate::style::WidgetStyle>,
    /// **DPI 缩放**（`measure` 里唯一写入点；未 measure 时 1.0 = 兜底）。
    scale: f32,
    hovered_row: Option<usize>,
}

impl Table {
    /// 创建空表。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            columns: Vec::new(),
            rows: Vec::new(),
            selected: None,
            command: None,
            rect: Rect::ZERO,
            item: FlexItem::default(),
            style: None,
            scale: 1.0,
            hovered_row: None,
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
        self
    }

    /// 设置选中行。
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index.filter(|i| *i < self.rows.len());
        self
    }

    /// 点击行回调。
    pub fn command(mut self, cmd: y901_ui::Command) -> Self {
        self.command = Some(cmd);
        self
    }

    /// **覆盖容器型表面**（背景 / 描边 / 圆角）。
    ///
    /// 注意：语义色（如 switch 轨道、error 红、empty 灰）**不**由它覆盖 ——
    /// 那些走 `Theme`，否则会破坏 DARK/LIGHT 自适应。
    pub fn style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.style = Some(style);
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

    /// 当前选中行。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 总高度（表头 + 所有行）。
    fn content_height(&self) -> f32 {
        HEADER_H * self.scale + self.rows.len() as f32 * ROW_H * self.scale
    }

    /// 按最终宽度算每列的实际 x 与宽度。
    fn column_layout(&self) -> Vec<(f32, f32)> {
        let n = self.columns.len();
        if n == 0 {
            return Vec::new();
        }
        let mut fixed = 0.0f32;
        let mut flex_weight = 0.0f32;
        // **Percent 基准** = 表格内容总宽（`self.rect.w`）：百分比列按"整表宽度"计算，
        // 与 CSS 的 percentage 宽度语义一致。
        let avail = self.rect.w.max(0.0);
        let mut percent_total = 0.0f32;
        for c in &self.columns {
            match c.width {
                Dimension::Px(v) => fixed += v.max(0.0) * self.scale,
                Dimension::Flex(f) => flex_weight += f.max(0.0),
                Dimension::Percent(p) => percent_total += avail * p.max(0.0) / 100.0,
                Dimension::Auto => fixed += FLEX_MIN_W,
            }
        }
        let rest = (self.rect.w - fixed - percent_total).max(0.0);
        let mut out = Vec::with_capacity(n);
        let mut x = self.rect.x;
        for c in &self.columns {
            let w = match c.width {
                Dimension::Px(v) => v.max(0.0) * self.scale,
                // **百分比列**：直接取总宽的 p%（不参与余量分配）
                Dimension::Percent(p) => avail * p.max(0.0) / 100.0,
                Dimension::Flex(f) if flex_weight > 0.0 => {
                    (rest * f.max(0.0) / flex_weight).max(FLEX_MIN_W.min(rest))
                }
                _ => rest.max(FLEX_MIN_W),
            };
            out.push((x, w));
            x += w;
        }
        out
    }

    /// 表头绘制区域。UI 坐标 y 轴向下，因此表头位于矩形顶部。
    fn header_rect(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y, self.rect.w, HEADER_H * self.scale)
    }

    /// 第 `index` 行的绘制区域（第 0 行紧贴表头下方）。
    fn row_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y + (HEADER_H + index as f32 * ROW_H) * self.scale,
            self.rect.w,
            ROW_H * self.scale,
        )
    }

    /// 命中测试：返回鼠标所在行（不含表头），与绘制坐标从上往下对应。
    fn row_at(&self, pos: Pos) -> Option<usize> {
        if !self.rect.contains(pos) {
            return None;
        }
        let y = pos.y - self.rect.y - HEADER_H * self.scale;
        if y < 0.0 {
            return None;
        }
        let idx = (y / (ROW_H * self.scale)) as usize;
        (idx < self.rows.len()).then_some(idx)
    }
}

impl Widget for Table {
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
        let want_w = c.max.w;
        c.constrain(Size::new(want_w, self.content_height()))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseMove { pos } => {
                let was = self.hovered_row;
                self.hovered_row = self.row_at(*pos);
                if was != self.hovered_row {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::MouseUp { pos, .. } => {
                if let Some(idx) = self.row_at(*pos) {
                    self.selected = Some(idx);
                    if let Some(cmd) = self.command.as_ref() {
                    let args = y901_ui::json::Json::obj().set("row", idx as i32);
                    ctx.emit_command(crate::command_with_args(cmd, args));
                }
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            // **键盘选行**：↑↓ 移动选中，**到边界停住**（不循环，与 ComboBox 一致）。
            //
            // 滚动：`Table` 目前**没有滚动机制**（渲染直接遍历全部行，
            // 溢出部分被父容器裁剪），因此**不存在"跟随滚动"可做**；
            // 等 Table 真正引入虚拟列表时再加。
            UiEvent::Key { key, .. } => {
                let n = self.rows.len();
                if n == 0 {
                    return EventResult::Ignored;
                }
                // 无选中时：↓ 选第一行（不是第二行），↑ 选最后一行。
                let next = match (key, self.selected) {
                    (y901_ui::Key::Down, None) => Some(0),
                    (y901_ui::Key::Down, Some(cur)) => Some((cur + 1).min(n - 1)),
                    (y901_ui::Key::Up, None) => Some(n - 1),
                    (y901_ui::Key::Up, Some(cur)) => Some(cur.saturating_sub(1)),
                    _ => None,
                };
                match next {
                    Some(idx) if Some(idx) != self.selected => {
                        self.selected = Some(idx);
                        if let Some(cmd) = self.command.as_ref() {
                            let args = y901_ui::json::Json::obj().set("row", idx as i32);
                            ctx.emit_command(crate::command_with_args(cmd, args));
                        }
                        EventResult::Redraw
                    }
                    // 到边界：不变更也不重绘，但表示"我处理了这个键"
                    Some(_) => EventResult::Handled,
                    None => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let theme = ctx.theme();
        let header_bg = theme.style(Role::LIST_ITEM, State::Hovered).bg;
        let row_bg = theme.style(Role::PANEL, State::Normal).bg;
        let alt_bg = theme.style(Role::PANEL, State::Hovered).bg;
        let hover_bg = theme.style(Role::LIST_ITEM, State::Hovered).bg;
        let sel_bg = theme.style(Role::LIST_ITEM, State::Pressed).bg;
        let text_fg = theme.style(Role::TEXT, State::Normal).fg;
        let muted = theme.muted_text_color();

        ctx.push_clip(self.rect);

        // UI 坐标系和后端缓冲都以左上为原点，Y 向下；表头绘制在顶部。
        let header = self.header_rect();
        let header_bg = self.style.and_then(|s| s.bg).unwrap_or(header_bg);
        ctx.fill_rect(header, header_bg);
        let cols = self.column_layout();
        let header_style = TextStyle::fg(muted);
        for (i, col) in self.columns.iter().enumerate() {
            let Some((x, w)) = cols.get(i).copied() else {
                break;
            };
            draw_cell(ctx, col, &col.title, x, header.y, w, HEADER_H * self.scale, &header_style, muted, self.scale);
            let _ = text_fg;
        }

        // 行
        let mut visible = 0usize;
        for (r, cells) in self.rows.iter().enumerate() {
            let row_rect = self.row_rect(r);
            let y = row_rect.y;
            if y >= self.rect.y + self.rect.h {
                break;
            }
            let bg = if self.selected == Some(r) {
                sel_bg
            } else if self.hovered_row == Some(r) {
                hover_bg
            } else if r % 2 == 1 {
                alt_bg
            } else {
                row_bg
            };
            ctx.fill_rect(Rect::new(self.rect.x, y, self.rect.w, ROW_H * self.scale), bg);
            let style = TextStyle::fg(text_fg);
            for (i, col) in self.columns.iter().enumerate() {
                let Some((x, w)) = cols.get(i).copied() else {
                    break;
                };
                let text = cells.get(i).map(String::as_str).unwrap_or("");
                draw_cell(ctx, col, text, x, y, w, ROW_H * self.scale, &style, text_fg, self.scale);
            }
            visible += 1;
        }

        ctx.pop_clip();
        let _ = visible;
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(true)
            .label(format!("table {} ({} rows)", self.id, self.rows.len()))
            .prop("rows", self.rows.len().to_string())
            .prop("columns", self.columns.len().to_string())
            .action("select_row");
        if let Some(i) = self.selected {
            s = s.value(format!("row {i}"));
        }
        s
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
    fallback: y901_ui::Color,
    scale: f32,
) {
    let _ = fallback;
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
    use y901_ui::Color;
    use y901_ui::layout::LayoutCache;

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
        let s = t.measure(&Metrics, Constraints::loose(Size::new(400.0, 400.0)), &mut cache);
        assert_eq!(s.h, HEADER_H + 2.0 * ROW_H);
        assert!(s.w <= 400.0);
    }

    #[test]
    fn top_down_row_geometry_matches_hit_testing() {
        let mut t = Table::new("t")
            .column(TableColumn::flex("c"))
            .row(vec!["first".into()])
            .row(vec!["second".into()]);
        t.rect = Rect::new(10.0, 20.0, 200.0, 120.0);

        let header = t.header_rect();
        let row0 = t.row_rect(0);
        let row1 = t.row_rect(1);
        assert_eq!(header, Rect::new(10.0, 20.0, 200.0, HEADER_H));
        assert_eq!(row0, Rect::new(10.0, 20.0 + HEADER_H, 200.0, ROW_H));
        assert_eq!(row1, Rect::new(10.0, 20.0 + HEADER_H + ROW_H, 200.0, ROW_H));
        assert!(header.bottom() <= row0.y, "表头应在第一行上方");
        assert!(row0.bottom() <= row1.y, "数据行应从上到下排列");

        let x = t.rect.x + 4.0;
        assert_eq!(t.row_at(Pos::new(x, header.y + 1.0)), None, "表头不应命中行");
        assert_eq!(t.row_at(Pos::new(x, row0.y + row0.h * 0.5)), Some(0));
        assert_eq!(t.row_at(Pos::new(x, row1.y + row1.h * 0.5)), Some(1));
        assert_eq!(t.row_at(Pos::new(x, row0.bottom())), Some(1), "行边界归属下方行");

        let mut ctx = EventCtx::new(None, Size::new(400.0, 300.0));
        t.event(
            &mut ctx,
            &UiEvent::MouseUp {
                pos: Pos::new(x, row1.y + row1.h * 0.5),
                button: y901_ui::MouseButton::Left,
            },
        );
        assert_eq!(t.selected_index(), Some(1), "点击第二行应选择第二行");
    }

    #[test]
    fn render_draws_header_then_rows_top_down() {
        let mut t = Table::new("t")
            .column(TableColumn::fixed("c", 100.0))
            .row(vec!["first".into()])
            .row(vec!["second".into()]);
        t.rect = Rect::new(10.0, 20.0, 200.0, 120.0);
        let mut ctx = RectRecorder::default();

        t.render(&mut ctx);

        // fill order: header, row 0, row 1. Recorded geometry is the visible contract.
        assert_eq!(ctx.fills.len(), 3);
        assert_eq!(ctx.fills[0], t.header_rect());
        assert_eq!(ctx.fills[1], t.row_rect(0));
        assert_eq!(ctx.fills[2], t.row_rect(1));
        assert!(ctx.fills[0].y < ctx.fills[1].y);
        assert!(ctx.fills[1].y < ctx.fills[2].y);
    }

    #[test]
    fn percent_column_takes_share_of_total_width() {
        let mut t = Table::new("t")
            .column(TableColumn::flex("a"))
            .column(TableColumn {
                title: "b".to_string(),
                width: Dimension::Percent(50.0),
                align: Align::Start,
            })
            .row(vec!["a".into(), "b".into()]);
        let mut cache = LayoutCache::new();
        t.arrange(Rect::new(0.0, 0.0, 400.0, 200.0), &Metrics, &mut cache);
        let cols = t.column_layout();
        // Percent(50) 占 400 的一半
        assert!((cols[1].1 - 200.0).abs() < 0.5, "Percent(50) 应为 200，实测 {}", cols[1].1);
        // flex 列吃剩下的 200
        assert!((cols[0].1 - 200.0).abs() < 0.5, "flex 应剩 200，实测 {}", cols[0].1);
    }

    #[test]
    fn exposes_select_row_action() {
        let t = Table::new("t").column(TableColumn::flex("a")).row(vec!["x".into()]);
        let acts = t.semantics().actions;
        assert!(acts.iter().any(|a| a.name == "select_row"), "table 应暴露 select_row");
    }

    #[test]
    fn style_is_stored_for_container_surface() {
        let st = crate::style::WidgetStyle::default().bg(Color::rgba(0.2, 0.2, 0.2, 1.0));
        let t = Table::new("t").column(TableColumn::flex("a")).style(st);
        assert_eq!(t.style, Some(st), "style 应被保存（render 里用它覆盖表头底色）");
    }
}
