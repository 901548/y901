#![cfg(windows)]

//! 纯 CPU 后端的光栅化验证（无窗口、无 GPU）。
//!
//! 这里只验证"数学"：像素能不能按预期落在应该的地方。
//! 与 D3D11 的逐像素对拍在 `soft_vs_d3d`里（需 GPU）。

use y901_backend_win32::soft::SoftBackend;
use y901_ui::{Color, Pos, Rect, RenderCtx, TextStyle};

fn soft() -> SoftBackend {
    headless(200, 120)
}

/// 造一个测试用的软后端（用自研 TTF 引擎，不依赖 DirectWrite）。
fn headless(w: u32, h: u32) -> SoftBackend {
    let (cw, ch) = y901_backend_win32::backend::font::cell_metrics_for(15.0);
    let cell = y901_ui::Size::new(cw, ch);
    let engine = y901_render::MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(w, h, Box::new(engine), cell).expect("init")
}

#[test]
fn fill_rect_interior_is_exact() {
    let mut b = soft();
    b.clear();
    b.fill_rect(
        Rect::new(10.0, 10.0, 100.0, 40.0),
        Color::rgba(1.0, 0.0, 0.0, 1.0),
    );

    let inside = b.pixel(50, 30).expect("像素存在");
    assert_eq!(
        inside, 0xFFFF_0000,
        "纯红应是 0xFFFF0000，实际 {inside:#010X}"
    );

    let outside = b.pixel(5, 5).expect("像素存在");
    assert_ne!(outside, 0xFFFF_0000, "矩形外不能被填充");
}

#[test]
fn alpha_blend_is_source_over() {
    let mut b = soft();
    // 背景涂白
    b.clear();
    b.fill_rect(
        Rect::new(0.0, 0.0, 200.0, 120.0),
        Color::rgba(1.0, 1.0, 1.0, 1.0),
    );
    // 50% 红
    b.fill_rect(
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Color::rgba(1.0, 0.0, 0.0, 0.5),
    );

    let p = b.pixel(50, 50).expect("像素");
    let r = (p >> 16) & 0xff;
    let g = (p >> 8) & 0xff;
    let bl = p & 0xff;
    // src.a = 0.5 → dst = 0.5*红 + 0.5*白 ≈ (255,127,127)
    assert!((r as i32 - 255).abs() <= 1, "红通道 {r}");
    assert!((g as i32 - 127).abs() <= 2, "绿通道 {g} 应约 127");
    assert!((bl as i32 - 127).abs() <= 2, "蓝通道 {bl} 应约 127");
}

#[test]
fn round_rect_cuts_corners_but_fills_center() {
    let mut b = soft();
    b.clear();
    let r = Rect::new(20.0, 20.0, 100.0, 80.0);
    b.fill_round_rect(r, Color::rgba(0.2, 0.8, 0.2, 1.0), 16.0);

    // 中心完全不透明
    let center = b.pixel(70, 60).expect("中心");
    assert_eq!(center >> 24, 255, "中心应不透明");

    // 左上角（圆角外）应未被填充：与背景相同
    let corner = b.pixel(21, 21).expect("角");
    let bg = b.pixel(199, 119).expect("背景");
    assert_eq!(corner, bg, "圆角外侧不应被涂色");

    // 圆角内侧（中心往左上一点）应已被填
    let inside_corner = b.pixel(30, 30).expect("in");
    assert_ne!(inside_corner, bg, "圆角内侧应被填充");
}

#[test]
fn gradient_endpoints_and_midpoint() {
    let mut b = soft();
    b.clear();
    let r = Rect::new(0.0, 0.0, 100.0, 20.0);
    b.fill_gradient_rect_dir(
        r,
        Color::rgba(0.0, 0.0, 0.0, 1.0),
        Color::rgba(1.0, 1.0, 1.0, 1.0),
        (1.0, 0.0),
    );

    let left = b.pixel(0, 10).expect("l");
    let right = b.pixel(99, 10).expect("r");
    let mid = b.pixel(50, 10).expect("m");

    let lr = (left >> 16) & 0xff;
    let rr = (right >> 16) & 0xff;
    let mr = (mid >> 16) & 0xff;
    assert!(lr <= 3, "左端应接近黑，实际 {lr}");
    assert!(rr >= 250, "右端应接近白，实际 {rr}");
    assert!((110..=150).contains(&mr), "中点应约 127，实际 {mr}");
}

#[test]
fn clip_culls_outside_area() {
    let mut b = soft();
    b.clear();
    let bg = b.pixel(150, 100).expect("bg");
    b.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
    b.fill_rect(
        Rect::new(0.0, 0.0, 200.0, 120.0),
        Color::rgba(1.0, 0.0, 0.0, 1.0),
    );
    b.pop_clip();

    assert_eq!(b.pixel(10, 10).expect("in"), 0xFFFF_0000, "裁剪内应被填");
    assert_eq!(b.pixel(150, 100).expect("out"), bg, "裁剪外不能被填");
}

#[test]
fn frame_hash_is_deterministic() {
    let mut a = soft();
    let mut c = soft();
    for b in [&mut a, &mut c] {
        b.clear();
        b.fill_rect(
            Rect::new(5.0, 5.0, 50.0, 30.0),
            Color::rgba(0.3, 0.6, 0.9, 1.0),
        );
        b.fill_round_rect(
            Rect::new(60.0, 40.0, 80.0, 50.0),
            Color::rgba(0.9, 0.4, 0.1, 0.8),
            10.0,
        );
        b.fill_gradient_rect_dir(
            Rect::new(0.0, 100.0, 200.0, 20.0),
            Color::rgba(0.0, 0.0, 0.0, 1.0),
            Color::rgba(0.2, 0.8, 1.0, 1.0),
            (0.0, 1.0),
        );
    }
    assert_eq!(
        a.frame_hash(),
        c.frame_hash(),
        "相同绘制序列必须得到相同像素"
    );
}

#[test]
fn text_draws_visible_pixels() {
    let mut b = soft();
    b.clear();
    let before = b.frame_hash();
    let w = b.draw_text(
        Pos::new(8.0, 8.0),
        "Hello Y901",
        &TextStyle::fg(Color::rgba(1.0, 1.0, 1.0, 1.0)),
    );
    assert!(w > 0.0, "字幅应大于 0，实际 {w}");
    assert_ne!(before, b.frame_hash(), "文字必须改变像素");

    // 统计非背景像素数
    let bg = b.pixel(199, 119).expect("bg");
    let mut bright = 0;
    for y in 0..40 {
        for x in 0..200 {
            if b.pixel(x, y) != Some(bg) {
                bright += 1;
            }
        }
    }
    assert!(bright >= 30, "字形像素太少：{bright}（字形引擎可能未就绪）");
}

#[test]
fn resize_rebuilds_buffer_and_reports_change() {
    let mut b = soft();
    assert!(b.resize(320, 240), "尺寸变了应报告 true");
    assert!(!b.resize(320, 240), "尺寸没变应报告 false");
    assert_eq!(b.size(), y901_ui::Size::new(320.0, 240.0));
    assert_eq!(
        b.pixel(319, 239).map(|p| p >> 24),
        Some(0),
        "新缓冲应为全 0"
    );
}

/// 吐吐量：1280x800 下各种图元的 CPU 耗时（决定 60fps 是否可行）。
#[test]
#[ignore = "基准：--ignored --nocapture"]
fn soft_fill_rate_1280x800() {
    use std::time::Instant;
    let mut b = headless(1280, 800);
    let full = Rect::new(0.0, 0.0, 1280.0, 800.0);
    let color = Color::rgba(0.2, 0.4, 0.6, 1.0);

    // 1) 整屏实心矩形
    let mut best = f64::MAX;
    for _ in 0..10 {
        b.clear();
        let t = Instant::now();
        b.fill_rect(full, color);
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!("[soft] fill_rect 1280x800: {best:.2} ms");

    // 2) 整屏渐变
    let mut best2 = f64::MAX;
    for _ in 0..10 {
        b.clear();
        let t = Instant::now();
        b.fill_gradient_rect_dir(
            full,
            Color::rgba(0.05, 0.05, 0.07, 1.0),
            Color::rgba(0.2, 0.3, 0.5, 1.0),
            (0.0, 1.0),
        );
        best2 = best2.min(t.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!("[soft] gradient 1280x800: {best2:.2} ms");

    // 3) SDF 圆角（400x300）
    let rr = Rect::new(100.0, 100.0, 400.0, 300.0);
    let mut best3 = f64::MAX;
    for _ in 0..10 {
        b.clear();
        let t = Instant::now();
        b.fill_round_rect(rr, color, 16.0);
        best3 = best3.min(t.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!("[soft] round_rect 400x300 (SDF): {best3:.2} ms");

    // 4) 整屏文字（80 列 × 40 行 ASCII）：文本光栅是 GUI 的另一条主路径
    let style = TextStyle::fg(color);
    let line = "The quick brown fox jumps over 0123456789 lazy dog";
    let mut best4 = f64::MAX;
    for _ in 0..10 {
        b.clear();
        let t = Instant::now();
        for row in 0..40 {
            b.draw_text(Pos::new(8.0, 8.0 + row as f32 * 18.0), line, &style);
        }
        best4 = best4.min(t.elapsed().as_secs_f64() * 1e3);
    }
    eprintln!("[soft] text 40x55 chars 1280x800: {best4:.2} ms");
    assert!(best4 < 50.0, "文字太慢，需降级");
    assert!(best < 50.0 && best2 < 50.0 && best3 < 50.0, "太慢，需降级");
}

/// **无损验证**：同一棵树、同一尺寸，分别用 D3D11 与纯 CPU 渲染，
/// 逐像素对比（允许抗锯差异）。
///
/// 这是 Step 1 的关键证据：如果两条完全不同的实现能画出几乎一致的画面，
/// 说明软光栅化的数学（预乘 alpha / SDF / 渐变）是对的。
#[test]
#[ignore = "需要真实 GPU（对照组用 D3D11）"]
fn soft_matches_d3d_within_antialiasing_tolerance() {
    use y901_backend_win32::window::Window;
    use y901_runtime::Options;
    use y901_ui::{Color, Rect, RenderCtx, Size, TextMetrics, UiTree};

    struct Scene {
        rect: Rect,
        /// 是否画"半透明叠实心"。半透明在两个后端上存在已知差异（见报告）。
        translucent: bool,
    }
    impl y901_ui::Widget for Scene {
        fn id(&self) -> &str {
            "scene"
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
            // 实心（不透明）
            ctx.fill_rect(
                Rect::new(r.x + 10.0, r.y + 10.0, 120.0, 40.0),
                Color::rgba(0.8, 0.2, 0.1, 1.0),
            );
            // 半透明叠在实心上
            if self.translucent {
                ctx.fill_rect(
                    Rect::new(r.x + 80.0, r.y + 30.0, 80.0, 40.0),
                    Color::rgba(0.2, 0.8, 0.3, 0.5),
                );
            }
            // 圆角
            ctx.fill_round_rect(
                Rect::new(r.x + 10.0, r.y + 70.0, 140.0, 50.0),
                Color::rgba(0.2, 0.4, 0.9, 1.0),
                12.0,
            );
            // 渐变（线性，向下）
            ctx.fill_gradient_rect_dir(
                Rect::new(r.x + 160.0, r.y + 10.0, 100.0, 110.0),
                Color::rgba(0.05, 0.05, 0.07, 1.0),
                Color::rgba(0.30, 0.45, 0.70, 1.0),
                (0.0, 1.0),
            );
        }
        fn semantics(&self) -> y901_ui::Semantics {
            y901_ui::Semantics::default()
        }
    }

    let options = Options {
        title: "soft vs d3d".to_string(),
        width: 320,
        height: 200,
        theme: None,
        font: None,

        fullscreen: false,
    };

    // **两个场景都跑**：不半透明（回归基线）+ 半透明（预乘回归）。
    // 历史教训：旧版只跑 translucent=false，于是 D3D 预乘 bug（半透明偏亮）
    // 在 CI 里永远是绿的。现在两个场景都断言。
    for translucent in [false, true] {
        // A）D3D11
        let mut win = Window::create(&options).expect("需要 D3D11");
        let mut tree_a = UiTree::new(Box::new(Scene {
            rect: Rect::ZERO,
            translucent,
        }));
        win.resize(&mut tree_a);
        tree_a.invalidate_all();
        win.request_snapshot();
        win.render(&mut tree_a);
        let (px_d3d, w, h) = win.take_snapshot().expect("快照");
        let _hwnd = win.hwnd();

        // B）纯 CPU（同一窗口句柄、同一尺寸）
        let (cw, ch) = y901_backend_win32::backend::font::cell_metrics_for(15.0);
        let cell = Size::new(cw, ch);
        let engine = Box::new(
            y901_backend_win32::backend::font::TextEngine::new("Segoe UI", 15.0, cw, ch)
                .expect("dwrite"),
        );
        let mut soft = SoftBackend::new(w, h, engine, cell, None).expect("soft init");
        let mut tree_b = UiTree::new(Box::new(Scene {
            rect: Rect::ZERO,
            translucent,
        }));
        tree_b.layout(Size::new(w as f32, h as f32), &soft);
        soft.clear();
        tree_b.invalidate_all();
        tree_b.render(&mut soft);
        let px_soft = soft.pixels().to_vec();

        assert_eq!(px_d3d.len(), px_soft.len(), "两边尺寸应一致");

        // 逐像素比较（BGRA，只比 RGB）
        let total = (w * h) as usize;
        let mut exact = 0usize;
        let mut within = 0usize; // 容差 <= 8
        let mut max_diff = 0i32;
        let mut worst = (0u32, 0u32);
        for i in 0..total {
            let (a, b) = (i * 4, i * 4);
            let d = (px_d3d[a] as i32 - px_soft[b] as i32).abs()
                + (px_d3d[a + 1] as i32 - px_soft[b + 1] as i32).abs()
                + (px_d3d[a + 2] as i32 - px_soft[b + 2] as i32).abs();
            if d == 0 {
                exact += 1;
            }
            if d <= 24 {
                within += 1;
            }
            if d > max_diff {
                max_diff = d;
                worst = ((i as u32) % w, (i as u32) / w);
            }
        }
        // 诊断：打印几个关键点的两边像素
        for (x, y) in [(80u32, 30u32), (50, 20), (100, 90), (200, 60)] {
            let i = ((y * w + x) * 4) as usize;
            eprintln!(
                "[soft-vs-d3d] ({x},{y}) d3d=({},{},{}) soft=({},{},{})",
                px_d3d[i],
                px_d3d[i + 1],
                px_d3d[i + 2],
                px_soft[i],
                px_soft[i + 1],
                px_soft[i + 2]
            );
        }
        let exact_pct = 100.0 * exact as f64 / total as f64;
        let within_pct = 100.0 * within as f64 / total as f64;
        eprintln!(
        "[soft-vs-d3d] {w}x{h}: exact={exact_pct:.1}%  within(|d|<=24)={within_pct:.1}%  max_diff={max_diff} at {worst:?}"
    );
        // 半透明场景：预乘修复前 max_diff 达 165（半透明块内偏亮），
        // 修复后为 2（仅剩抗锯齿边缘）。阈值取 8：给浮点与舍入余量，
        // 但远小于 165 —— 能分辨“偏亮”与“正确”。
        if translucent {
            assert!(
                max_diff <= 8,
                "半透明场景 max_diff={max_diff} 超过 8（预乘修复前为 165）"
            );
        }
        assert!(
            within_pct >= 99.0,
            "纯 CPU 与 D3D11 的差异过大：只有 {within_pct:.1}% 像素在容差内（max_diff={max_diff}）"
        );
    }
}

/// 半透明叠加：CPU 必须与 **解析解**一致（source-over）。
///
/// 为什么单独测而不拿 D3D 对比：实测发现两个后端在半透明实心上**不一致**
/// （D3D 给出的绿更亮）。CPU 这边与公式完全吻合，所以**差异在 D3D 侧**，
/// 属于另一个需要单独查的问题（已记入报告）。这里把 CPU 的正确性锁死。
#[test]
fn translucent_overlay_matches_analytic_source_over() {
    use y901_ui::{Color, Rect, RenderCtx};

    let mut b = headless(64, 64);
    b.clear();
    b.fill_rect(
        Rect::new(0.0, 0.0, 64.0, 64.0),
        Color::rgba(0.8, 0.2, 0.1, 1.0),
    );
    b.fill_rect(
        Rect::new(0.0, 0.0, 64.0, 64.0),
        Color::rgba(0.2, 0.8, 0.3, 0.5),
    );

    let p = b.pixel(32, 32).expect("像素");
    let (bb, gg, rr) = (p & 0xff, (p >> 8) & 0xff, (p >> 16) & 0xff);
    // src-over: 0.5*(0.2,0.8,0.3) + 0.5*(0.8,0.2,0.1) = (0.5,0.5,0.2) → (128,128,51)
    assert!((rr as i32 - 128).abs() <= 2, "R={rr} 应约 128");
    assert!((gg as i32 - 128).abs() <= 2, "G={gg} 应约 128");
    assert!((bb as i32 - 51).abs() <= 2, "B={bb} 应约 51");
}
