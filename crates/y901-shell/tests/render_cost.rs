//! 演示页整窗渲染的**量级**观测：区分"真热点"与"未优化构建"。
//!
//! 用法（默认不跑，避免 CI 抖动）：
//!
//! ```text
//! cargo test -p y901-shell --test render_cost -- --ignored --nocapture
//! cargo test --release -p y901-shell --test render_cost -- --ignored --nocapture
//! ```
//!
//! 为什么要分 debug / release 各跑一次：浏览器里那次 `render=183ms` 是用
//! `wasm-pack --dev`（未优化）构建测的。这里的 debug 数字对应"未优化"，
//! release 对应"优化后"。若两者差一个数量级，就说明 183ms 主要来自构建档位，
//! 而不是某个光栅化热点——否则修复方向会完全错。

use std::time::Instant;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_shell::build_demo_page;
use y901_ui::{Size, UiTree};

const VW: u32 = 800;
const VH: u32 = 600;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        16.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    SoftBackend::new_headless(VW, VH, Box::new(engine), Size::new(cw, ch)).expect("soft")
}

#[test]
#[ignore = "性能观测：手动 --ignored --nocapture 运行"]
fn demo_page_full_render_cost() {
    let mut tree: UiTree = build_demo_page();
    let mut back = soft();
    tree.layout(Size::new(VW as f32, VH as f32), &back);

    // 预热：字形图集第一次拼贴要栅格化，冷启动数字不代表稳态。
    for _ in 0..3 {
        tree.invalidate_all();
        tree.render(&mut back);
    }

    let rounds = 20;
    let mut samples = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        tree.invalidate_all();
        let t0 = Instant::now();
        tree.render(&mut back);
        samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = samples[samples.len() / 2];
    let p95 = samples[(samples.len() as f64 * 0.95) as usize];
    let worst = samples[samples.len() - 1];

    eprintln!(
        "[render-cost] 800x600 演示页整窗: median={median:.1}ms p95={p95:.1}ms max={worst:.1}ms \
         ({} 轮, {} 字节缓冲)",
        rounds,
        back.pixels().len()
    );

    // 只断言"能跑完"，不设性能门禁（机器差异太大）。
    assert!(median.is_finite());
}

/// 单次全屏 `fill_rect` 的成本：这是"整窗重绘"里最贵的那一类调用。
#[test]
#[ignore = "性能观测：手动 --ignored --nocapture 运行"]
fn full_screen_fill_cost() {
    use y901_ui::{Color, Rect, RenderCtx};

    let mut back = soft();
    let rounds = 50;
    let mut samples = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let t0 = Instant::now();
        back.fill_rect(
            Rect::new(0.0, 0.0, VW as f32, VH as f32),
            Color::rgba(0.02, 0.02, 0.03, 1.0),
        );
        samples.push(t0.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    eprintln!(
        "[fill-cost] 800x600 不透明整屏填充: median={:.3}ms max={:.3}ms",
        samples[samples.len() / 2],
        samples[samples.len() - 1]
    );
    assert!(samples[0].is_finite());
}
