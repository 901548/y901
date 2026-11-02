//! ProgressBar：只读进度条。
//!
//! 视觉：轨道（背景色，圆角 4px，高 8px）+ 进度条（主题色，宽度 = value × 轨道宽度）。
//! 无交互。`value` 越界时 **clamp** 到 `0..=1`，不 panic、不外溢。

use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Role, Semantics, Size, State, TextMetrics,
    UiEvent, Widget,
};

/// 轨道高度（逻辑像素）。
const TRACK_H: f32 = 8.0;
/// 圆角半径。
const RADIUS: f32 = 4.0;

/// 只读进度条。
pub struct ProgressBar {
    id: String,
    /// 进度（0..=1，越界自动 clamp）
    value: f32,
    /// 是否在轨道中间显示百分比文字
    show_text: bool,
    rect: Rect,
}

impl ProgressBar {
    /// 创建进度条。`value` 会被 clamp 到 `0..=1`。
    pub fn new(id: impl Into<String>, value: f32) -> Self {
        Self {
            id: id.into(),
            value: value.clamp(0.0, 1.0),
            show_text: false,
            rect: Rect::ZERO,
        }
    }

    /// 是否显示百分比文字。
    pub fn show_text(mut self, show: bool) -> Self {
        self.show_text = show;
        self
    }

    /// 设置进度（自动 clamp）。
    pub fn set_value(&mut self, value: f32) {
        self.value = value.clamp(0.0, 1.0);
    }

    /// 当前进度（已 clamp）。
    pub fn value(&self) -> f32 {
        self.value
    }

    /// 轨道矩形（垂直居中于自身矩形）。
    fn track(&self) -> Rect {
        let h = TRACK_H.min(self.rect.h);
        Rect::new(
            self.rect.x,
            self.rect.y + (self.rect.h - h) * 0.5,
            self.rect.w.max(0.0),
            h,
        )
    }
}

impl Widget for ProgressBar {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, TRACK_H))
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

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let track = self.track();
        if track.w <= 0.0 || track.h <= 0.0 {
            return;
        }
        let bg = ctx.theme().style(Role::PANEL, State::Normal);
        let fg = ctx.theme().style(Role::BUTTON, State::Selected);

        // 轨道
        ctx.fill_round_rect(track, bg.bg, RADIUS);

        // 进度：宽度 = value × 轨道宽。
        // value=0 时宽度为 0 → 直接跳过（画 0 宽矩形没有意义，还会在部分后端产生 1px 伪影）
        let w = track.w * self.value;
        if w > 0.0 {
            ctx.fill_round_rect(Rect::new(track.x, track.y, w, track.h), fg.bg, RADIUS);
        }

        if self.show_text {
            let text = format!("{:.0}%", self.value * 100.0);
            let style = y901_ui::TextStyle::fg(fg.fg);
            let tw = ctx.measure_text(&text, &style).w;
            let tx = track.x + (track.w - tw) * 0.5;
            let ty = track.y + (track.h - ctx.line_height(&style)) * 0.5;
            ctx.draw_text(y901_ui::Pos::new(tx, ty), &text, &style);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("progress {}", self.id))
            .value(format!("{:.2}", self.value))
            .prop("value", format!("{:.2}", self.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;

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

    /// 渲染一个进度条并回读像素。
    fn render(value: f32) -> (SoftBackend, Rect) {
        use y901_ui::layout::LayoutCache;
        let mut s = soft();
        s.clear();
        let mut bar = ProgressBar::new("p", value);
        let rect = Rect::new(0.0, 0.0, 100.0, 20.0);
        let mut cache = LayoutCache::new();
        bar.arrange(rect, &s, &mut cache);
        bar.render(&mut s);
        (s, rect)
    }

    /// 在**轨道范围内**统计进度色像素数。
    ///
    /// 为什么必须限定范围：画布背景也"不等于轨道色"，
    /// 不限定就会把整幅画布算成进度（实测踩过：150px vs 预期 50px）。
    fn progress_pixels(s: &SoftBackend, y: u32, track_w: u32, color: (u8, u8, u8)) -> u32 {
        (0..track_w)
            .filter(|&x| px(s, x, y) == color)
            .count() as u32
    }

    fn px(s: &SoftBackend, x: u32, y: u32) -> (u8, u8, u8) {
        let p = s.pixel(x, y).expect("像素");
        (
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
        )
    }

    #[test]
    fn value_is_clamped_on_construction() {
        assert_eq!(ProgressBar::new("a", 1.5).value(), 1.0);
        assert_eq!(ProgressBar::new("b", -0.5).value(), 0.0);
        assert_eq!(ProgressBar::new("c", 0.25).value(), 0.25);
    }

    #[test]
    fn zero_value_draws_no_progress() {
        let (s, rect) = render(0.0);
        let y = (rect.y + rect.h * 0.5) as u32;
        // 轨道左端应是轨道色（没有被进度覆盖）
        let track_color = px(&s, 1, y);
        let mid = px(&s, 50, y);
        assert_eq!(mid, track_color, "value=0 时中点应仍是轨道色");
    }

    #[test]
    fn half_value_covers_half_the_track() {
        // 先用 value=1.0 学习"进度色"（不硬编码主题色）
        let (full, frect) = render(1.0);
        let fy = (frect.y + frect.h * 0.5) as u32;
        let progress_color = px(&full, 1, fy);

        let (s, rect) = render(0.5);
        let y = (rect.y + rect.h * 0.5) as u32;
        let w = progress_pixels(&s, y, 100, progress_color);
        eprintln!("[progress] value=0.5 的进度像素 = {w}px（轨道宽 100）");
        assert!((w as i32 - 50).abs() <= 2, "value=0.5 应覆盖半宽，实际 {w}");
    }

    #[test]
    fn full_value_covers_the_whole_track() {
        let (s, rect) = render(1.0);
        let y = (rect.y + rect.h * 0.5) as u32;
        let left = px(&s, 1, y);
        let right = px(&s, 98, y);
        assert_eq!(left, right, "value=1 时两端应是同一进度色");
        // 与 value=0 的轨道色不同
        let (s0, r0) = render(0.0);
        let y0 = (r0.y + r0.h * 0.5) as u32;
        assert_ne!(left, px(&s0, 1, y0), "进度色应与轨道色不同");
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let (hi, rect) = render(1.5);
        let y = (rect.y + rect.h * 0.5) as u32;
        let (lo, r0) = render(-0.5);
        let y0 = (r0.y + r0.h * 0.5) as u32;
        assert_eq!(
            px(&hi, 50, y),
            px(&hi, 2, y),
            "value=1.5 应 clamp 成满格"
        );
        assert_ne!(
            px(&lo, 50, y0),
            px(&hi, 50, y),
            "value=-0.5 应 clamp 成空格"
        );
    }
}
