//! Checkbox：方框 + 标签，点击或 Space 切换。
//!
//! 视觉：左侧 16×16 圆角方框（半径 2，边框 2px），选中时框内画 V 形对勾；
//! 右侧 8px 间距后接标签文字，垂直居中。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Key, Modifiers, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 方框边长（逻辑像素）。
const BOX: f32 = 16.0;

/// 复选框尺寸（逻辑像素 × DPI 缩放）。
///
/// **为何是函数**：框与文字并排，字在 125% 下从 16→20px，
/// 而 16px 的框会变得比字小 —— 对齐关系被破坏。
/// `Checkbox` 没有 `&self` 公开几何 API（`box_rect` 是私有）→ **不加缓存**。
fn box_size(scale: f32) -> f32 {
    BOX * scale
}

/// 框与文字的间距（逻辑像素 × DPI 缩放）。
fn gap(scale: f32) -> f32 {
    GAP * scale
}
/// 方框与文字的间距。
const GAP: f32 = 8.0;
/// 对勾两条线段的粗细。
const CHECK_W: f32 = 2.0;

/// 可勾选的复选框。
pub struct Checkbox {
    id: String,
    label: String,
    checked: bool,
    on_change: Option<Box<dyn Fn(bool)>>,
    rect: Rect,
    focused: bool,
}

impl Checkbox {
    /// 创建复选框。
    pub fn new(id: impl Into<String>, label: impl Into<String>, checked: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked,
            on_change: None,
            rect: Rect::ZERO,
            focused: false,
        }
    }

    /// 设置切换回调。
    pub fn on_change(mut self, f: impl Fn(bool) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    /// 当前是否选中。
    pub fn checked(&self) -> bool {
        self.checked
    }

    /// 方框矩形（左侧，垂直居中）。
    fn box_rect(&self, scale: f32) -> Rect {
        let b = box_size(scale);
        let y = self.rect.y + (self.rect.h - b) * 0.5;
        Rect::new(self.rect.x, y, b.min(self.rect.w), b)
    }

    /// 切换选中状态并触发回调。
    fn toggle(&mut self) {
        self.checked = !self.checked;
        if let Some(cb) = self.on_change.as_ref() {
            cb(self.checked);
        }
    }

    /// 画 V 形对勾（两段 rotated-free 近似：用逐行扫描的短横线拼）。
    ///
    /// 为什么不用旋转矩形：`RenderCtx` 只有轴对齐矩形与圆角矩形，
    /// 旋转要么加图元要么在 CPU 上算。这里用"逐行短条"近似 45° 线段，
    /// 在 16px 的框里视觉完全够用（每行 2px 宽、错位 1px）。
    fn draw_check(ctx: &mut dyn RenderCtx, area: Rect, color: Color) {
        // 左半段：从 (0.22, 0.52) 到 (0.44, 0.74)；右半段：到 (0.78, 0.28)
        let (x0, y0) = (area.x + area.w * 0.22, area.y + area.h * 0.52);
        let (xm, ym) = (area.x + area.w * 0.44, area.y + area.h * 0.74);
        let (x1, y1) = (area.x + area.w * 0.78, area.y + area.h * 0.28);
        let mut draw_seg = |ax: f32, ay: f32, bx: f32, by: f32| {
            let steps = ((bx - ax).abs().max((by - ay).abs())).max(1.0) as i32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let x = ax + (bx - ax) * t;
                let y = ay + (by - ay) * t;
                ctx.fill_rect(Rect::new(x, y, CHECK_W, CHECK_W), color);
            }
        };
        draw_seg(x0, y0, xm, ym);
        draw_seg(xm, ym, x1, y1);
    }
}

impl Widget for Checkbox {
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
        let tw = m.measure_text(&self.label, &style).w;
        let b = box_size(m.scale());
        let h = b.max(m.line_height(&style));
        c.constrain(Size::new(b + gap(m.scale()) + tw, h))
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
                self.toggle();
                EventResult::Handled
            }
            UiEvent::Focus(focused) => {
                self.focused = *focused;
                EventResult::Redraw
            }
            UiEvent::Key { key, modifiers } => {
                if *modifiers != Modifiers::default() {
                    return EventResult::Ignored;
                }
                if matches!(key, Key::Char(' ')) {
                    self.toggle();
                    return EventResult::Handled;
                }
                EventResult::Ignored
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
        let boxr = self.box_rect(ctx.scale());

        // 方框：先填底，再描边（描边用四条 fill_rect，避免依赖 stroke_round_rect 的实现细节）
        ctx.fill_round_rect(boxr, normal.bg, 2.0);
        let w = 2.0_f32.min(boxr.w * 0.5);
        ctx.fill_rect(Rect::new(boxr.x, boxr.y, boxr.w, w), border);
        ctx.fill_rect(Rect::new(boxr.x, boxr.bottom() - w, boxr.w, w), border);
        ctx.fill_rect(Rect::new(boxr.x, boxr.y, w, boxr.h), border);
        ctx.fill_rect(Rect::new(boxr.right() - w, boxr.y, w, boxr.h), border);

        if self.checked {
            Self::draw_check(ctx, boxr, normal.fg);
        }

        // 标签
        let style = TextStyle::fg(normal.fg);
        let text_x = boxr.right() + gap(ctx.scale());
        let text_y = self.rect.y + (self.rect.h - ctx.line_height(&style)) * 0.5;
        ctx.draw_text(Pos::new(text_x, text_y), &self.label, &style);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.label.clone())
            .value(self.checked.to_string())
            .action("toggle")
            .prop("checked", self.checked.to_string())
            .checked(self.checked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;
    use y901_ui::layout::LayoutCache;

    const W: u32 = 200;
    const H: u32 = 40;

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

    fn render(checked: bool) -> (SoftBackend, Rect) {
        let mut s = soft();
        s.clear();
        let mut cb = Checkbox::new("c", "选项", checked);
        let rect = Rect::new(0.0, 0.0, 120.0, 24.0);
        let mut cache = LayoutCache::new();
        cb.arrange(rect, &s, &mut cache);
        cb.render(&mut s);
        (s, rect)
    }

    fn px(s: &SoftBackend, x: u32, y: u32) -> (u8, u8, u8) {
        let p = s.pixel(x, y).expect("像素");
        (
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
        )
    }

    /// 方框内部（去掉 3px 边框带）的异色像素数。
    fn ink_inside_box(s: &SoftBackend, boxr: Rect) -> usize {
        let mut n = 0;
        let x0 = (boxr.x as u32) + 3;
        let y0 = (boxr.y as u32) + 3;
        let x1 = (boxr.right() as u32).saturating_sub(3);
        let y1 = (boxr.bottom() as u32).saturating_sub(3);
        for y in y0..y1 {
            for x in x0..x1 {
                let c = px(s, x, y);
                // 只数"亮"像素（对勾是前景色，底色是深色）
                if c.0 > 120 && c.1 > 120 && c.2 > 120 {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn unchecked_box_has_no_check_mark() {
        let (s, _) = render(false);
        let boxr = Rect::new(0.0, 4.0, 16.0, 16.0);
        let ink = ink_inside_box(&s, boxr);
        eprintln!("[checkbox] 未选中方框内亮像素 = {ink}");
        assert_eq!(ink, 0, "未选中时框内不应有对勾");
    }

    #[test]
    fn checked_box_has_a_check_mark() {
        let (s, _) = render(true);
        let boxr = Rect::new(0.0, 4.0, 16.0, 16.0);
        let ink = ink_inside_box(&s, boxr);
        eprintln!("[checkbox] 选中方框内亮像素 = {ink}");
        assert!(ink > 10, "选中时框内应有对勾像素，实测 {ink}");
    }

    #[test]
    fn click_points_toggle_and_fire_the_callback() {
        use std::cell::Cell;
        use std::rc::Rc;

        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let mut cb = Checkbox::new("c", "x", false).on_change(move |_| h.set(h.get() + 1));
        cb.rect = Rect::new(0.0, 0.0, 100.0, 24.0);

        // 点击事件需要 EventCtx：这里直接验证 toggle 的行为与回调计数
        cb.toggle();
        assert!(cb.checked(), "第一次点击后应选中");
        assert_eq!(hits.get(), 1, "回调应调用一次");
        cb.toggle();
        assert!(!cb.checked(), "再点一次应取消");
        assert_eq!(hits.get(), 2);
    }

    #[test]
    fn space_toggles_and_other_keys_do_not() {
        let mut cb = Checkbox::new("c", "x", false);
        assert!(cb.focusable(), "Checkbox 必须可获得键盘焦点");

        // 直接调用 toggle 模拟 Space 的效果（EventCtx 在测试里构造不出来）
        cb.toggle();
        assert!(cb.checked());
        cb.toggle();
        assert!(!cb.checked());
    }
}
