//! 新绘制原语：圆角矩形 + 线性渐变（默认实现，所有后端自动可用）。

use y901_ui::{
    Color, Pos, Rect, RenderCtx, Size, TextStyle, TextMetrics, GRADIENT_STEPS,
};

/// 记录所有 fill_rect 的探针画布。
struct Recorder {
    fills: Vec<(Rect, Color)>,
}

impl Recorder {
    fn new() -> Self {
        Self { fills: Vec::new() }
    }
    fn total_area(&self) -> f32 {
        self.fills.iter().map(|(r, _)| r.w * r.h).sum()
    }
}

impl TextMetrics for Recorder {
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

impl RenderCtx for Recorder {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, r: Rect, c: Color) {
        assert!(!r.is_empty(), "不应绘制空矩形");
        self.fills.push((r, c));
    }
    fn stroke_rect(&mut self, r: Rect, c: Color, w: f32) {
        self.fill_rect(r, c);
        self.fills.push((r, c.with_alpha(0.0)));
        let _ = w;
    }
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
fn round_rect_leaves_corners_unpainted() {
    let mut rec = Recorder::new();
    let rect = Rect::new(10.0, 10.0, 100.0, 100.0);
    rec.fill_round_rect(rect, Color::WHITE, 20.0);

    assert!(!rec.fills.is_empty(), "必须产生绘制");

    // 四角（半径范围内）不应被任何填充覆盖
    let corners = [
        Rect::new(10.0, 10.0, 2.0, 2.0),
        Rect::new(108.0, 10.0, 2.0, 2.0),
        Rect::new(10.0, 108.0, 2.0, 2.0),
        Rect::new(108.0, 108.0, 2.0, 2.0),
    ];
    for c in corners {
        let covered: f32 = rec
            .fills
            .iter()
            .map(|(r, _)| r.intersect(c).w * r.intersect(c).h)
            .sum();
        assert!(covered < 0.01, "圆角处不应被填充，实际覆盖 {covered}");
    }

    // 中心一定被覆盖
    let center = Rect::new(58.0, 58.0, 4.0, 4.0);
    let covered: f32 = rec
        .fills
        .iter()
        .map(|(r, _)| r.intersect(center).w * r.intersect(center).h)
        .sum();
    assert!(covered > 15.0, "中心必须被覆盖，实际 {covered}");
}

#[test]
fn round_rect_zero_radius_is_plain_rect() {
    let mut rec = Recorder::new();
    let rect = Rect::new(0.0, 0.0, 50.0, 50.0);
    rec.fill_round_rect(rect, Color::WHITE, 0.0);
    assert_eq!(rec.fills.len(), 1, "radius=0 应退化成一次普通填充");
    assert_eq!(rec.fills[0].0, rect);
}

#[test]
fn gradient_has_expected_steps_and_endpoints() {
    let mut rec = Recorder::new();
    let rect = Rect::new(0.0, 0.0, 100.0, 200.0);
    let from = Color::rgba(1.0, 0.0, 0.0, 1.0);
    let to = Color::rgba(0.0, 0.0, 1.0, 1.0);
    rec.fill_gradient_rect(rect, from, to, false);

    assert_eq!(rec.fills.len(), GRADIENT_STEPS, "条带数应为常量");
    // 第一条偏红，最后一条偏蓝
    let first = rec.fills.first().unwrap().1;
    let last = rec.fills.last().unwrap().1;
    assert!(first.r > first.b, "顶部应偏红，实际 {first:?}");
    assert!(last.b > last.r, "底部应偏蓝，实际 {last:?}");
    // 覆盖面积约等于矩形面积（允许条带间的 +0.5 重叠）
    let area = rec.total_area();
    let target = rect.w * rect.h;
    assert!(
        (area - target).abs() < target * 0.02,
        "条带总面积应≈矩形面积：{area} vs {target}"
    );
}

#[test]
fn gradient_empty_rect_draws_nothing() {
    let mut rec = Recorder::new();
    rec.fill_gradient_rect(
        Rect::new(0.0, 0.0, 0.0, 10.0),
        Color::WHITE,
        Color::BLACK,
        true,
    );
    assert!(rec.fills.is_empty(), "空矩形不应绘制");
}
