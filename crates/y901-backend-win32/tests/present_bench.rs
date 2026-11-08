//! Present 节拍基准（sync=0 vs sync=1）。
//!
//! **本基准测的是窗口模式下的 Present 节拍**：DWM 合成器会将帧节奏限制在 ~1/刷新率，
//! `sync_interval` 改不动它（实测 p50 差仅 18µs）。要测**纯提交耗时**需要独占全屏或 GPU timestamp，均暂缓。
//!
//! ```text
//! cargo test -p y901-backend-win32 --release --test present_bench -- --ignored --nocapture
//! ```

use std::time::Instant;

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    UiEvent, UiTree, Widget,
};

struct Scene {
    rect: Rect,
}

impl Widget for Scene {
    fn id(&self) -> &str {
        "scene"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
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
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let r = self.rect;
        ctx.fill_rect(r, Color::rgba(0.05, 0.05, 0.07, 1.0));
        ctx.fill_round_rect(
            Rect::new(8.0, 8.0, 120.0, 40.0),
            Color::rgba(0.2, 0.5, 0.9, 1.0),
            6.0,
        );
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn report(tag: &str, mut s: Vec<f64>) -> f64 {
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg = s.iter().sum::<f64>() / s.len().max(1) as f64;
    eprintln!(
        "[present] {tag}: avg={avg:.1}us p50={:.1}us p95={:.1}us max={:.1}us",
        pct(&s, 0.5),
        pct(&s, 0.95),
        pct(&s, 1.0)
    );
    pct(&s, 0.5)
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn present_bench_sync0_vs_sync1() {
    let options = Options {
        title: "present bench".to_string(),
        width: 800,
        height: 600,
        theme: None,
        font: None,
    
            fullscreen: false,
        };
    let mut window = Window::create(&options).expect("需要 D3D11");
    let mut tree = UiTree::new(Box::new(Scene { rect: Rect::ZERO }));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree);

    const N: usize = 100;
    let mut a = Vec::with_capacity(N);
    for _ in 0..N {
        tree.invalidate_all();
        let t = Instant::now();
        window.render_with_sync(&mut tree, 0);
        a.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let mut b = Vec::with_capacity(N);
    for _ in 0..N {
        tree.invalidate_all();
        let t = Instant::now();
        window.render(&mut tree);
        b.push(t.elapsed().as_secs_f64() * 1e6);
    }

    let p50_a = report("A′ 窗口+sync=0", a);
    let p50_b = report("B  窗口+sync=1", b);
    eprintln!("[present] p50 差值（B-A′）= {:.1}us", p50_b - p50_a);
}
