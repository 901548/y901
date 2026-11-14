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
        for c in &self.columns {
            match c.width {
                Dimension::Px(v) => fixed += v.max(0.0),
                Dimension::Flex(f) => flex_weight += f.max(0.0),
                _ => fixed += FLEX_MIN_W,
            }
        }
        let rest = (self.rect.w - fixed).max(0.0);
        let mut out = Vec::with_capacity(n);
        let mut x = self.rect.x;
        for c in &self.columns {
            let w = match c.width {
                Dimension::Px(v) => v.max(0.0),
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

    /// 命中测试：返回鼠标所在行（不含表头）。
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

        // 表头
        let header = Rect::new(self.rect.x, self.rect.y, self.rect.w, HEADER_H * self.scale);
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
            let y = self.rect.y + HEADER_H * self.scale + r as f32 * ROW_H * self.scale;
            if y >= self.rect.bottom() {
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
            .prop("columns", self.columns.len().to_string());
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
    fn row_hit_test_respects_header() {
        let mut t = Table::new("t").column(TableColumn::flex("c")).row(vec!["x".into()]);
        t.rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(t.row_at(Pos::new(10.0, 10.0)), None, "表头不应命中行");
        assert_eq!(t.row_at(Pos::new(10.0, HEADER_H + 5.0)), Some(0));
    }
}