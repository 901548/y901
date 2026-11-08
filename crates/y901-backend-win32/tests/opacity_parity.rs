//! D3D vs Soft 的 `opacity` 对拍。
//!
//! 验证两条实现同语义：
//! - D3D：shader 末尾乘 `input.params.w`（`push_instance` 写入 `current_opacity()`）
//! - Soft：`pack` 乘 `state.current_opacity()`
//!
//! 需要真实 GPU，默认 `#[ignore]`；由真机执行：
//!
//! ```text
//! cargo test -p y901-backend-win32 --release --test opacity_parity -- --ignored --nocapture
//! ```
//!
//! 场景与 `y901-render/tests/opacity_soft.rs` **完全相同**（黑底 + 红块 + N 层 opacity），
//! 保证两边画的是同一个东西。

use y901_render::soft::SoftBackend;
use y901_ui::{Color, Rect, RenderCtx, Size, TextMetrics, UiTree};

/// 黑底 + 红块，外层套 `depth` 层 0.5 不透明度。
struct Scene {
    rect: Rect,
    depth: usize,
}

impl y901_ui::Widget for Scene {
    fn id(&self) -> &str {
        "opacity-scene"
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
    fn event(
        &mut self,
        _c: &mut y901_ui::EventCtx,
        _e: &y901_ui::UiEvent,
    ) -> y901_ui::EventResult {
        y901_ui::EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let r = self.rect;
        // 不透明黑底：不透明度只体现在 RGB 混合比例上，底色必须是黑
        ctx.fill_rect(r, Color::rgba(0.0, 0.0, 0.0, 1.0));
        for _ in 0..self.depth {
            ctx.push_opacity(0.5);
        }
        // 居中红块（64x64），与 CPU 侧测试同一尺寸
        let side = 64.0;
        ctx.fill_rect(
            Rect::new(
                r.x + (r.w - side) * 0.5,
                r.y + (r.h - side) * 0.5,
                side,
                side,
            ),
            Color::rgba(1.0, 0.0, 0.0, 1.0),
        );
        for _ in 0..self.depth {
            ctx.pop_opacity();
        }
    }
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

/// 两后端同场景渲染后比较中心像素与整体差异，返回 `(中心红通道差, 全局 max_diff)`。
fn compare(depth: usize) -> (i32, i32) {
    use y901_backend_win32::backend::font as wfont;
    use y901_backend_win32::window::Window;
    use y901_runtime::Options;

    let options = Options {
        title: "opacity parity".to_string(),
        width: 200,
        height: 160,
        theme: None,
        font: None,
    };

    // ---- A：D3D11 ----
    let mut win = Window::create(&options).expect("需要 D3D11 设备");
    let mut tree_a = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        depth,
    }));
    win.resize(&mut tree_a);
    tree_a.invalidate_all();
    win.request_snapshot();
    win.render(&mut tree_a);
    let (px_d3d, w, h) = win.take_snapshot().expect("快照失败");

    // ---- B：CPU 光栅器 ----
    let (cw, ch) = wfont::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = Box::new(wfont::TextEngine::new("Segoe UI", 15.0, cw, ch).expect("dwrite"));
    let mut soft = SoftBackend::new_headless(w, h, engine, cell).expect("soft init");
    let mut tree_b = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        depth,
    }));
    tree_b.layout(Size::new(w as f32, h as f32), &soft);
    soft.clear();
    tree_b.invalidate_all();
    tree_b.render(&mut soft);
    let px_soft = soft.pixels().to_vec();

    assert_eq!(px_d3d.len(), px_soft.len(), "两边像素缓存大小应相同");

    let total = (w * h) as usize;
    let mut max_diff = 0i32;
    let mut d3d_center = 0u8;
    let mut soft_center = 0u8;
    let (cx, cy) = (w / 2, h / 2);
    for i in 0..total {
        let a = i * 4;
        // BGRA：+2 是红通道
        let dr = (px_d3d[a + 2] as i32 - px_soft[a + 2] as i32).abs();
        let dg = (px_d3d[a + 1] as i32 - px_soft[a + 1] as i32).abs();
        let db = (px_d3d[a] as i32 - px_soft[a] as i32).abs();
        max_diff = max_diff.max(dr + dg + db);
        let (x, y) = ((i as u32) % w, (i as u32) / w);
        if x == cx && y == cy {
            d3d_center = px_d3d[a + 2];
            soft_center = px_soft[a + 2];
        }
    }
    eprintln!(
        "[opacity-parity] depth={depth} {w}x{h}: d3d_center_red={d3d_center} soft_center_red={soft_center} max_diff={max_diff}"
    );
    (
        (d3d_center as i32 - soft_center as i32).abs(),
        max_diff,
    )
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn opacity_halves_matches_between_d3d_and_soft() {
    let (center, worst) = compare(1);
    assert!(center <= 8, "opacity 0.5 中心红通道差 {center} 超过 8");
    assert!(worst <= 32, "整体 max_diff={worst} 超过 32");
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn nested_opacity_matches_between_d3d_and_soft() {
    // 0.5 × 0.5 = 0.25：若某侧用了覆盖而非乘积，这里会差出一大截
    let (center, worst) = compare(2);
    assert!(center <= 8, "嵌套 opacity 中心红通道差 {center} 超过 8");
    assert!(worst <= 32, "整体 max_diff={worst} 超过 32");
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn opacity_one_matches_baseline() {
    // depth=0：没有 push_opacity，两后端应基本一致（回归基线）
    let (center, worst) = compare(0);
    assert!(center <= 8, "无 opacity 时中心红通道差 {center} 超过 8");
    assert!(worst <= 32, "整体 max_diff={worst} 超过 32");
}
