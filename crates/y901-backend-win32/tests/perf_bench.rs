//! 后端基准（需 GPU）：字形图集 Skyline 分配、实例构建与提交。
//!
//! ```text
//! cargo test -p y901-backend-win32 --release --test perf_bench -- --ignored --nocapture
//! ```

use std::time::Instant;

use y901_backend_win32::window::Window;
use y901_render::atlas::SkylinePacker;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::new(8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_atlas_5000_glyphs() {
    // 中英混排字形尺寸：ASCII 约 8x16，CJK 约 16x16（含 1px padding）
    const N: usize = 5000;
    let mut packer = SkylinePacker::new(1024);

    let mut samples = Vec::with_capacity(N);
    let mut placed_area: u64 = 0;
    let mut ok = 0usize;
    let t_all = Instant::now();
    for i in 0..N {
        let (w, h) = if i % 2 == 0 { (8, 16) } else { (16, 16) };
        let t = Instant::now();
        let got = packer.insert(w, h);
        samples.push(t.elapsed().as_secs_f64() * 1e9);
        if got.is_some() {
            ok += 1;
            placed_area += (w * h) as u64;
        }
    }
    let total = t_all.elapsed().as_secs_f64() * 1e6;
    let mut sorted = samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let used_h = packer.used_height() as u64;
    let capacity = 1024u64 * used_h;
    eprintln!(
        "[atlas] {N} glyphs into 1024^2: ok={ok} total={total:.0}us avg={:.0}ns p99={:.0}ns max={:.0}ns",
        samples.iter().sum::<f64>() / samples.len() as f64,
        pct(&sorted, 0.99),
        sorted.last().copied().unwrap_or(0.0)
    );
    eprintln!(
        "[atlas] used_height={used_h} occupied_capacity={capacity} placed_area={placed_area} \
         fragmentation={:.1}%",
        100.0 * (1.0 - placed_area as f64 / capacity.max(1) as f64)
    );
}

/// 10k 个不同颜色/坐标的实心矩形：拆成 CPU 构建与 GPU 提交两段计时。
struct ManyRects {
    rect: Rect,
    count: usize,
}

impl Widget for ManyRects {
    fn id(&self) -> &str {
        "many"
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
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let cols = 100usize;
        for i in 0..self.count {
            let x = self.rect.x + (i % cols) as f32 * 4.0;
            let y = self.rect.y + (i / cols) as f32 * 4.0;
            let c = Color::rgba(
                (i % 251) as f32 / 251.0,
                (i % 241) as f32 / 241.0,
                (i % 239) as f32 / 239.0,
                1.0,
            );
            ctx.fill_rect(Rect::new(x, y, 3.0, 3.0), c);
        }
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

struct Counter {
    fills: usize,
}

impl TextMetrics for Counter {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::ZERO
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Counter {
    fn size(&self) -> Size {
        Size::new(800.0, 600.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {
        self.fills += 1;
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_instance_submission_10k() {
    const N: usize = 10_000;
    let mut tree = UiTree::new(Box::new(ManyRects {
        rect: Rect::ZERO,
        count: N,
    }));
    tree.layout(Size::new(800.0, 600.0), &Mono);

    // (a) 纯 CPU：构建实例流（不提交）
    let mut cpu_samples = Vec::new();
    for _ in 0..20 {
        tree.invalidate_all();
        let mut c = Counter { fills: 0 };
        let t = Instant::now();
        tree.render(&mut c);
        cpu_samples.push(t.elapsed().as_secs_f64() * 1e6);
        assert_eq!(c.fills, N);
    }
    let mut sorted = cpu_samples.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "[inst] CPU build {} fill_rect: avg={:.0}us p99={:.0}us",
        N,
        cpu_samples.iter().sum::<f64>() / cpu_samples.len() as f64,
        pct(&sorted, 0.99)
    );

    // (b) 含 GPU：真实窗口渲染（Map + DrawInstanced + Present）
    let options = Options {
        title: "y901 perf".to_string(),
        width: 800,
        height: 600,
        theme: None,
        font: None,
    
            fullscreen: false,
        };
    let mut window = Window::create(&options).expect("需要 D3D11");
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree); // 预热

    let mut gpu_samples = Vec::new();
    for _ in 0..20 {
        tree.invalidate_all();
        let t = Instant::now();
        window.render(&mut tree);
        gpu_samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    let mut gsorted = gpu_samples.clone();
    gsorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "[inst] full frame (CPU+Map+Draw+Present) {} instances: avg={:.0}us p99={:.0}us",
        N,
        gpu_samples.iter().sum::<f64>() / gpu_samples.len() as f64,
        pct(&gsorted, 0.99)
    );
    eprintln!(
        "[inst] 提交部分估算 ≈ {:.0}us / frame",
        gpu_samples.iter().sum::<f64>() / gpu_samples.len() as f64
            - cpu_samples.iter().sum::<f64>() / cpu_samples.len() as f64
    );
}

/// 基线：一个空帧（只清屏 + blit + present）的耗时。
///
/// 用来把“实例提交”与“交换链/blit 固定开销”分开。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn bench_empty_frame_baseline() {
    let mut tree = UiTree::new(Box::new(ManyRects {
        rect: Rect::ZERO,
        count: 0,
    }));
    let options = Options {
        title: "y901 perf baseline".to_string(),
        width: 800,
        height: 600,
        theme: None,
        font: None,
    
            fullscreen: false,
        };
    let mut window = Window::create(&options).expect("需要 D3D11");
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree);

    let mut samples = Vec::new();
    for _ in 0..20 {
        tree.invalidate_all();
        let t = Instant::now();
        window.render(&mut tree);
        samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "[inst] empty frame baseline: avg={:.0}us p99={:.0}us",
        samples.iter().sum::<f64>() / samples.len() as f64,
        pct(&samples, 0.99)
    );
}
