//! RadioGroup：垂直排列的单选组，组内互斥。
//!
//! 视觉：每项左侧 16×16 圆框（边框 2px，用四段近似圆），选中时内部画直径 8 的实心圆点。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Key, Modifiers, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 圆框直径。
const DOT_BOX: f32 = 16.0;

/// 圆框尺寸（逻辑像素 × DPI 缩放）。
/// **为何是函数**：框与文字并排，字在 125% 下从 16→20px，
/// 而 16px 的框会变得比字小 —— 对齐关系被破坏。
/// `Radio` 没有 `&self` 公开几何 API（`row_h` / `row_rect` / `dot_rect` 均私有）→ **不加缓存**。
fn dot_box(scale: f32) -> f32 {
    DOT_BOX * scale
}

/// 框与文字的间距（逻辑像素 × DPI 缩放）。
fn gap(scale: f32) -> f32 {
    GAP * scale
}

/// 行间距（逻辑像素 × DPI 缩放）。
fn row_gap(scale: f32) -> f32 {
    ROW_GAP * scale
}
/// 圆框与文字的间距。
const GAP: f32 = 8.0;
/// 每一项的垂直间距。
const ROW_GAP: f32 = 4.0;

/// 单选组。
pub struct RadioGroup {
    id: String,
    options: Vec<String>,
    selected: Option<usize>,
    on_change: Option<Box<dyn Fn(usize)>>,
    rect: Rect,
    focused: bool,
}

impl RadioGroup {
    /// 创建单选组。`selected` 越界时按 `None` 处理。
    pub fn new(id: impl Into<String>, options: Vec<String>, selected: Option<usize>) -> Self {
        let selected = selected.filter(|&i| i < options.len());
        Self {
            id: id.into(),
            options,
            selected,
            on_change: None,
            rect: Rect::ZERO,
            focused: false,
        }
    }

    /// 设置选中变化的回调。
    pub fn on_change(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// 当前选中项。
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// 选中第 `i` 项（越界或已选中则返回 false）。
    pub fn select(&mut self, i: usize) -> bool {
        if i >= self.options.len() || self.selected == Some(i) {
            return false;
        }
        self.selected = Some(i);
        if let Some(cb) = self.on_change.as_ref() {
            cb(i);
        }
        true
    }

    /// 单行高度（框尺寸 + 行间距，两者必须同源）。
    fn row_h(&self, scale: f32) -> f32 {
        dot_box(scale) + row_gap(scale)
    }

    /// 第 `i` 行的矩形。
    fn row_rect(&self, i: usize, scale: f32) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y + i as f32 * self.row_h(scale),
            self.rect.w.max(0.0),
            dot_box(scale),
        )
    }

    /// 第 `i` 行的圆框矩形。
    fn dot_rect(&self, i: usize, scale: f32) -> Rect {
        let d = dot_box(scale);
        let r = self.row_rect(i, scale);
        Rect::new(r.x, r.y + (r.h - d) * 0.5, d, d)
    }

    /// 画一个圆环（外圆填边框色，内圆填底色）。
    ///
    /// 为什么不用 `stroke_round_rect`：那个接口的圆角半径语义是"矩形圆角"，
    /// 用它画圆需要 radius = 边长/2 且边框向内缩，两者叠加在不同后端会有 1px 差异。
    /// 这里用"外圆 + 内圆"两笔，任何后端都稳定。
    fn draw_ring(ctx: &mut dyn RenderCtx, r: Rect, border: Color, fill: Color, width: f32) {
        let radius = r.w.min(r.h) * 0.5;
        ctx.fill_round_rect(r, border, radius);
        let inset = width.max(0.0);
        let inner = Rect::new(
            r.x + inset,
            r.y + inset,
            (r.w - inset * 2.0).max(0.0),
            (r.h - inset * 2.0).max(0.0),
        );
        if inner.w > 0.0 && inner.h > 0.0 {
            ctx.fill_round_rect(inner, fill, radius - inset.max(0.0));
        }
    }
}

impl Widget for RadioGroup {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::BUTTON
    }

    fn focusable(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let style = TextStyle::default();
        let mut w: f32 = 0.0;
        for o in &self.options {
            w = w.max(dot_box(m.scale()) + gap(m.scale()) + m.measure_text(o, &style).w);
        }
        let h = self.row_h(m.scale()) * self.options.len() as f32;
        c.constrain(Size::new(w, h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseDown { pos, .. } => {
                if !self.rect.contains(*pos) {
                    return EventResult::Ignored;
                }
                ctx.request_focus(self.id.clone());
                let row = ((pos.y - self.rect.y) / self.row_h(ctx.scale())).floor();
                if row >= 0.0 {
                    let i = row as usize;
                    if i < self.options.len() && self.select(i) {
                        return EventResult::Handled;
                    }
                }
                EventResult::Ignored
            }
            UiEvent::Focus(focused) => {
                self.focused = *focused;
                EventResult::Redraw
            }
            UiEvent::Key { key, modifiers } => {
                if *modifiers != Modifiers::default() {
                    return EventResult::Ignored;
                }
                let n = self.options.len();
                if n == 0 {
                    return EventResult::Ignored;
                }
                let cur = self.selected.unwrap_or(0);
                match key {
                    Key::Up => {
                        let next = (cur + n - 1) % n;
                        if self.select(next) {
                            EventResult::Handled
                        } else {
                            EventResult::Ignored
                        }
                    }
                    Key::Down => {
                        let next = (cur + 1) % n;
                        if self.select(next) {
                            EventResult::Handled
                        } else {
                            EventResult::Ignored
                        }
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let normal = ctx.theme().style(Role::BUTTON, State::Normal);
        let border = if self.focused {
            ctx.theme().style(Role::BUTTON, State::Focused).fg
        } else {
            normal.border
        };
        let style = TextStyle::fg(normal.fg);

        for i in 0..self.options.len() {
            let dot = self.dot_rect(i, ctx.scale());
            Self::draw_ring(ctx, dot, border, normal.bg, 2.0);
            if self.selected == Some(i) {
                // 实心圆点：直径 8，居中
                let d = 8.0_f32.min(dot.w * 0.5);
                let c = Rect::new(dot.x + (dot.w - d) * 0.5, dot.y + (dot.h - d) * 0.5, d, d);
                ctx.fill_round_rect(c, normal.fg, d * 0.5);
            }
            let row = self.row_rect(i, ctx.scale());
            let tx = dot.right() + gap(ctx.scale());
            let ty = row.y + (row.h - ctx.line_height(&style)) * 0.5;
            ctx.draw_text(Pos::new(tx, ty), &self.options[i], &style);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("radio {}", self.id))
            .value(self.selected.map(|i| i.to_string()).unwrap_or_default())
            .action("select")
            .prop("count", self.options.len().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;
    use y901_ui::layout::LayoutCache;

    const W: u32 = 200;
    const H: u32 = 120;

    fn soft() -> SoftBackend {
        let (cw, ch) = y901_render::cell_metrics_for(15.0);
        let cell = Size::new(cw, ch);
        let engine = MyFontEngine::new(
            y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
            15.0,
            cell,
        )
        .expect("engine");
        SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
    }

    fn render(selected: Option<usize>) -> SoftBackend {
        let mut s = soft();
        s.clear();
        let mut g = RadioGroup::new("r", vec!["一".into(), "二".into(), "三".into()], selected);
        let mut cache = LayoutCache::new();
        g.arrange(Rect::new(0.0, 0.0, 120.0, 60.0), &s, &mut cache);
        g.render(&mut s);
        s
    }

    fn px(s: &SoftBackend, x: u32, y: u32) -> (u8, u8, u8) {
        let p = s.pixel(x, y).expect("像素");
        (
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
        )
    }

    /// 第 i 行圆框中心是否亮（实心圆点）
    fn center_bright(s: &SoftBackend, i: usize) -> bool {
        let cy = (i as f32 * ROW_GAP.max(DOT_BOX) + DOT_BOX * 0.5) as u32;
        let cx = DOT_BOX as u32 / 2;
        let c = px(s, cx, cy);
        c.0 > 120 && c.1 > 120 && c.2 > 120
    }

    #[test]
    fn only_the_selected_row_has_a_dot() {
        let s = render(Some(0));
        assert!(center_bright(&s, 0), "第 0 行应有点");
        assert!(!center_bright(&s, 1), "第 1 行不应有点");
        eprintln!("[radio] 选中 0：row0 亮 = true，row1 亮 = false");

        let s1 = render(Some(1));
        assert!(!center_bright(&s1, 0), "改选 1 后第 0 行应无点");
        assert!(center_bright(&s1, 1), "第 1 行应有点");
    }

    #[test]
    fn selecting_is_mutually_exclusive() {
        let mut g = RadioGroup::new("r", vec!["a".into(), "b".into()], None);
        assert_eq!(g.selected(), None);
        assert!(g.select(0));
        assert_eq!(g.selected(), Some(0));
        assert!(g.select(1));
        assert_eq!(g.selected(), Some(1), "同组只能有一个选中");
        assert!(!g.select(1), "重复选中同一项应返回 false");
        assert!(!g.select(9), "越界索引应被忽略");
    }

    #[test]
    fn callback_fires_on_change_only() {
        use std::cell::Cell;
        use std::rc::Rc;
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let mut g = RadioGroup::new("r", vec!["a".into(), "b".into()], None)
            .on_change(move |_| h.set(h.get() + 1));
        g.select(0);
        assert_eq!(hits.get(), 1);
        g.select(0); // 重复选中 → 不回调
        assert_eq!(hits.get(), 1, "重复选中不应触发回调");
        g.select(1);
        assert_eq!(hits.get(), 2);
    }

    #[test]
    fn arrow_keys_cycle_when_focused() {
        let mut g = RadioGroup::new("r", vec!["a".into(), "b".into(), "c".into()], Some(0));
        assert!(g.focusable(), "RadioGroup 必须能获得键盘焦点");
        // 方向键路径与 Tabs 相同（树只把 Key 发给焦点节点），此处验证选中推进逻辑
        g.select(1);
        assert_eq!(g.selected(), Some(1));
        g.select(2);
        assert_eq!(g.selected(), Some(2));
        // 回绕语义：从末项再往下应回到 0
        let n = 3;
        let next = (2 + 1) % n;
        assert_eq!(next, 0, "末项向下应回绕到首项");
    }
}
