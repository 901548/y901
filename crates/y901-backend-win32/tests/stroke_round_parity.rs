//! D3D 与 Soft 的 `stroke_round_rect` 像素对拍。
//!
//! P1c-2 让 Soft 用与 D3D 着色器**同一个** SDF 公式描边（`y901-render/src/sdf.rs`），
//! 本测试验证两后端现在逐像素可比。
//!
//! 需要真实 D3D11 设备与窗口，默认 `#[ignore]`；由真机（demo 会话）执行：
//!
//! ```text
//! cargo test -p y901-backend-win32 --release --test stroke_round_parity -- --ignored --nocapture
//! ```

use y901_render::soft::SoftBackend;
use y901_ui::{Color, Rect, RenderCtx, Size, TextMetrics, UiTree};

/// 场景：一个圆角描边，可选用 transform 放大。
struct Scene {
    rect: Rect,
    /// `None` = 无 transform；`Some(s)` = 以左上角为原点放大 `s` 倍
    scale: Option<f32>,
}

impl y901_ui::Widget for Scene {
    fn id(&self) -> &str {
        "stroke-scene"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: y901_ui::Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, c.max.h))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut y901_ui::EventCtx, _e: &y901_ui::UiEvent) -> y901_ui::EventResult {
        y901_ui::EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let r = self.rect;
        // 背景先铺满，避免两后端对"未绘制区域"的初始值不同
        ctx.fill_rect(r, Color::rgba(0.05, 0.05, 0.07, 1.0));

        let target = Rect::new(r.x + 20.0, r.y + 20.0, 120.0, 80.0);
        match self.scale {
            None => {
                // 无 transform：radius=8, width=2
                ctx.stroke_round_rect(target, Color::rgba(1.0, 0.25, 0.2, 1.0), 2.0, 8.0);
            }
            Some(s) => {
                // 有 transform：同一描边在 1.5倍缩放下。
                // 这里验证的是 map_rect + radius/width 缩放一致性。
                ctx.push_transform(y901_ui::Pos::new(r.x, r.y), s);
                ctx.stroke_round_rect(
                    Rect::new(20.0, 20.0, 120.0, 80.0),
                    Color::rgba(1.0, 0.25, 0.2, 1.0),
                    2.0,
                    8.0,
                );
                ctx.pop_transform();
            }
        }
    }
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

/// 两后端同场景渲染后逐像素比较。
///
/// 阈值与 `soft_backend.rs::soft_matches_d3d_within_antialiasing_tolerance` **完全一致**：
/// `|ΔR|+|ΔG|+|ΔB| <= 24` 的像素必须 ≥ 99%。
fn compare(scale: Option<f32>) -> f64 {
    use y901_backend_win32::backend::font as wfont;
    use y901_backend_win32::window::Window;
    use y901_runtime::Options;

    let options = Options {
        title: "stroke parity".to_string(),
        width: 320,
        height: 200,
        theme: None,
        font: None,
    
            fullscreen: false,
        };

    // ---- A：D3D11 ----
    let mut win = Window::create(&options).expect("需要 D3D11 设备");
    let mut tree_a = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        scale,
    }));
    win.resize(&mut tree_a);
    tree_a.invalidate_all();
    win.request_snapshot();
    win.render(&mut tree_a);
    let (px_d3d, w, h) = win.take_snapshot().expect("快照失败");

    // ---- B：CPU 光栅器（同一个场景、同一尺寸） ----
    let (cw, ch) = wfont::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = Box::new(wfont::TextEngine::new("Segoe UI", 15.0, cw, ch).expect("dwrite"));
    let mut soft = SoftBackend::new(w, h, engine, cell, None).expect("soft init");
    let mut tree_b = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        scale,
    }));
    tree_b.layout(Size::new(w as f32, h as f32), &soft);
    soft.clear();
    tree_b.invalidate_all();
    tree_b.render(&mut soft);
    let px_soft = soft.pixels().to_vec();

    assert_eq!(px_d3d.len(), px_soft.len(), "两边像素缓存大小应相同");

    let total = (w * h) as usize;
    let mut exact = 0usize;
    let mut within = 0usize;
    let mut max_diff = 0i32;
    for i in 0..total {
        let a = i * 4;
        let d = (px_d3d[a] as i32 - px_soft[a] as i32).abs()
            + (px_d3d[a + 1] as i32 - px_soft[a + 1] as i32).abs()
            + (px_d3d[a + 2] as i32 - px_soft[a + 2] as i32).abs();
        if d == 0 {
            exact += 1;
        }
        if d <= 24 {
            within += 1;
        }
        max_diff = max_diff.max(d);
    }
    let exact_pct = 100.0 * exact as f64 / total as f64;
    let within_pct = 100.0 * within as f64 / total as f64;
    eprintln!(
        "[stroke-parity] scale={scale:?} {w}x{h}: exact={exact_pct:.1}% within(<=24)={within_pct:.1}% max_diff={max_diff}"
    );
    within_pct
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn stroke_round_rect_matches_without_transform() {
    let within = compare(None);
    assert!(
        within >= 99.0,
        "无 transform 时 Soft 与 D3D 应逐像素可比，实测 within={within:.1}%"
    );
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn stroke_round_rect_matches_under_transform() {
    // 1.5 倍缩放：验证 map_rect 与 radius/width 缩放两后端一致。
    let within = compare(Some(1.5));
    assert!(
        within >= 99.0,
        "1.5倍 transform 下 Soft 与 D3D 应逐像素可比，实测 within={within:.1}%"
    );
}
