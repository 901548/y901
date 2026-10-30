//! 新绘制原语：圆角（覆盖率 AA）+ 全方向线性渐变。

use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle};

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

    /// 某个 1×1 像素被覆盖的比例（按 alpha 加权）。
    fn coverage_at(&self, px: f32, py: f32) -> f32 {
        let p = Rect::new(px, py, 1.0, 1.0);
        self.fills
            .iter()
            .map(|(r, c)| r.intersect(p).w * r.intersect(p).h * c.a)
            .sum()
    }

    /// 某一行被填了多少宽度。
    fn row_width(&self, y: f32) -> f32 {
        self.fills
            .iter()
            .filter(|(r, _)| r.y <= y && r.bottom() > y)
            .map(|(r, _)| r.w)
            .sum()
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
    fn stroke_rect(&mut self, r: Rect, c: Color, _w: f32) {
        self.fill_rect(r, c);
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

    // 四个角的最外一像素：几乎不被覆盖
    let corners = [(10.0, 10.0), (109.0, 10.0), (10.0, 109.0), (109.0, 109.0)];
    for (x, y) in corners {
        let cov = rec.coverage_at(x, y);
        assert!(cov < 0.05, "({x},{y}) 圆角外不应被覆盖，实际 {cov}");
    }

    // 中心必须完全覆盖
    let center = rec.coverage_at(59.0, 59.0);
    assert!(center > 0.99, "中心应完全覆盖，实际 {center}");
}

#[test]
fn round_rect_edges_have_partial_coverage() {
    // 覆盖率 AA 的核心：圆角过渡带必须出现 0 < alpha < 1 的像素
    let mut rec = Recorder::new();
    rec.fill_round_rect(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Color::rgba(1.0, 1.0, 1.0, 1.0),
        16.0,
    );

    let partial = rec
        .fills
        .iter()
        .filter(|(_, c)| c.a > 0.01 && c.a < 0.99)
        .count();
    assert!(
        partial >= 8,
        "圆角边缘必须有半透明像素（覆盖率 AA），实际 {partial} 个"
    );
}

#[test]
fn round_rect_rows_are_monotonic() {
    // 从上到下，圆角行的宽度应当单调增（直到直边），不能出现锯齿抖动
    let mut rec = Recorder::new();
    rec.fill_round_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::WHITE, 12.0);
    let widths: Vec<f32> = (0..12).map(|i| rec.row_width(i as f32 + 0.5)).collect();
    for w in widths.windows(2) {
        assert!(w[1] >= w[0] - 0.01, "圆角行宽必须单调不减：{widths:?}");
    }
    assert!(
        *widths.last().unwrap() > 98.0,
        "圆角结束处应接近满宽：{widths:?}"
    );
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
fn gradient_vertical_is_exact_and_seamless() {
    let mut rec = Recorder::new();
    let rect = Rect::new(0.0, 0.0, 100.0, 64.0);
    let from = Color::rgba(1.0, 0.0, 0.0, 1.0);
    let to = Color::rgba(0.0, 0.0, 1.0, 1.0);
    rec.fill_gradient_rect_dir(rect, from, to, (0.0, 1.0));

    // 逐行条带：行数 = 高度，每行 1px 高（无缝无重叠）
    assert_eq!(rec.fills.len(), 64, "应逐行切条带");
    let mut y = 0.0;
    for (r, _) in &rec.fills {
        assert_eq!(r.y, y, "条带必须首尾相接");
        assert_eq!(r.h, 1.0, "条带高度应为 1px");
        y += r.h;
    }
    let first = rec.fills.first().unwrap().1;
    let last = rec.fills.last().unwrap().1;
    assert!(first.r > 0.95 && first.b < 0.05, "顶部应是 from：{first:?}");
    assert!(last.b > 0.95 && last.r < 0.05, "底部应是 to：{last:?}");
    assert_eq!(rec.total_area(), rect.w * rect.h, "覆盖面积必须精确");
}

#[test]
fn gradient_horizontal_is_exact() {
    let mut rec = Recorder::new();
    let rect = Rect::new(10.0, 20.0, 32.0, 8.0);
    rec.fill_gradient_rect_dir(rect, Color::WHITE, Color::BLACK, (1.0, 0.0));
    assert_eq!(rec.fills.len(), 32, "水平方向应逐列切");
    let mut x = 10.0;
    for (r, _) in &rec.fills {
        assert_eq!(r.x, x);
        assert_eq!(r.w, 1.0);
        assert_eq!(r.h, 8.0);
        x += 1.0;
    }
}

#[test]
fn gradient_diagonal_follows_dominant_axis() {
    // dir=(0.3, 1.0) → 主导轴是 y，逐行；颜色沿 y 单调变化
    let mut rec = Recorder::new();
    let rect = Rect::new(0.0, 0.0, 40.0, 40.0);
    rec.fill_gradient_rect_dir(
        rect,
        Color::rgba(1.0, 0.0, 0.0, 1.0),
        Color::rgba(0.0, 0.0, 1.0, 1.0),
        (0.3, 1.0),
    );
    assert_eq!(rec.fills.len(), 40, "主导轴 y → 逐行");
    let reds: Vec<f32> = rec.fills.iter().map(|(_, c)| c.r).collect();
    for w in reds.windows(2) {
        assert!(w[1] <= w[0] + 1e-4, "红通道必须单调递减：{reds:?}");
    }
    assert_eq!(rec.total_area(), 1600.0, "面积必须精确");
}

#[test]
fn gradient_empty_rect_draws_nothing() {
    let mut rec = Recorder::new();
    rec.fill_gradient_rect_dir(
        Rect::new(0.0, 0.0, 0.0, 10.0),
        Color::WHITE,
        Color::BLACK,
        (0.0, 1.0),
    );
    assert!(rec.fills.is_empty(), "空矩形不应绘制");
}

#[test]
fn gradient_zero_vector_is_solid() {
    let mut rec = Recorder::new();
    let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
    rec.fill_gradient_rect_dir(rect, Color::WHITE, Color::BLACK, (0.0, 0.0));
    assert_eq!(rec.fills.len(), 1, "零向量应退化成纯色");
    assert_eq!(rec.fills[0].1, Color::WHITE);
}

#[test]
fn legacy_one_axis_wrapper_still_works() {
    let mut rec = Recorder::new();
    rec.fill_gradient_rect(
        Rect::new(0.0, 0.0, 64.0, 16.0),
        Color::WHITE,
        Color::BLACK,
        true,
    );
    assert_eq!(rec.fills.len(), 64, "horizontal=true 应逐列");
}
