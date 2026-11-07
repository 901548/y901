//! 脏区调度：**常数标定** + **策略代价矩阵**（纯测量，不改生产代码）。
//!
//! 这个文件不改变任何生产行为：它只调用 `SoftBackend` 的公开 API 计时，
//! 并用纯函数实现四种调度策略来比较代价。
//! 安全网：生产路径完全未动，像素哈希基线不受影响。
//!
//! 运行：
//! ```text
//! cargo test -p y901-render --release --test dirty_strategy_bench -- --nocapture
//! ```

use std::time::Instant;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Rect, RenderCtx, Size};

const W: u32 = 1920;
const H: u32 = 1080;

// ---------------------------------------------------------------------------
// 标定
// ---------------------------------------------------------------------------

fn backend(w: u32, h: u32) -> SoftBackend {
    // 复用与 image_mip_quality 相同的构造方式：字体只用于文本，填充路径不需要真实字形。
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        y901_ui::Size::new(cw, ch),
    )
    .expect("engine");
    SoftBackend::new_headless(w, h, Box::new(engine), Size::new(cw, ch)).expect("soft")
}

/// `c_pixel`：满屏 `fill_rect` 的**每像素**耗时。
///
/// 方法：`W×H` 满屏填充 N 次，取总耗时 / (N·W·H)。
/// 注意它包含带宽与调用开销，**不是**理论上的 ALU 每像素成本。
#[test]
fn calibrate_c_pixel() {
    let mut s = backend(W, H);
    let full = Rect::new(0.0, 0.0, W as f32, H as f32);
    let c = Color::rgba(0.3, 0.6, 0.9, 1.0);

    for _ in 0..5 {
        s.fill_rect(full, c); // 预热
    }
    const N: usize = 200;
    let t = Instant::now();
    for _ in 0..N {
        s.fill_rect(full, c);
    }
    let ns = t.elapsed().as_secs_f64() * 1e9 / N as f64;
    let c_pixel = ns / (W as f64 * H as f64);
    eprintln!(
        "[calib] 满屏 fill_rect: {:.1} us/帧 ({} px) => c_pixel = {:.4} ns/px",
        ns / 1000.0,
        W * H,
        c_pixel
    );
    assert!(c_pixel > 0.0, "c_pixel 必须为正");
    // 帧填充不可能快于 0.01 ns/px（100 GB/s 量级），也不可能慢于 100 ns/px。
    assert!(
        (0.001..100.0).contains(&c_pixel),
        "c_pixel={c_pixel} 超出合理范围，标定方法可能错了"
    );
}

/// `c_switch`：一次 scissor（clip 压栈/出栈）的耗时。
///
/// 方法：同一个 clip 反复 push/pop N 次，取平均。
/// 这只测 **CPU 侧状态开销**；GPU 侧的状态切换无法在这里体现（SoftBackend 是纯 CPU）。
#[test]
fn calibrate_c_switch() {
    let mut s = backend(W, H);
    let r = Rect::new(10.0, 10.0, 100.0, 100.0);
    for _ in 0..100 {
        s.push_clip(r);
        s.pop_clip();
    }
    const N: usize = 100_000;
    let t = Instant::now();
    for _ in 0..N {
        s.push_clip(r);
        s.pop_clip();
    }
    let ns = t.elapsed().as_secs_f64() * 1e9 / N as f64;
    eprintln!(
        "[calib] scissor push+pop: {ns:.1} ns/次 => c_switch = {:.4} us",
        ns / 1000.0
    );
    assert!(ns > 0.0, "c_switch 必须为正");
    assert!(ns < 100_000.0, "c_switch={ns}ns 太大，测量方法可能有问题");
}

// ---------------------------------------------------------------------------
// 四种策略（纯函数，与生产代码无关）
// ---------------------------------------------------------------------------

fn area(r: Rect) -> f64 {
    (r.w.max(0.0) * r.h.max(0.0)) as f64
}

fn bbox(dirty: &[Rect]) -> Option<Rect> {
    let mut it = dirty.iter();
    let first = *it.next()?;
    let mut b = first;
    for r in it {
        let x0 = b.x.min(r.x);
        let y0 = b.y.min(r.y);
        let x1 = (b.x + b.w).max(r.x + r.w);
        let y1 = (b.y + b.h).max(r.y + r.h);
        b = Rect::new(x0, y0, x1 - x0, y1 - y0);
    }
    Some(b)
}

fn bbox_strategy(dirty: &[Rect]) -> Vec<Rect> {
    bbox(dirty).into_iter().collect()
}

fn per_rect_strategy(dirty: &[Rect]) -> Vec<Rect> {
    dirty.to_vec()
}

/// 当前生产策略的**近似**：贪心地“能并就并”，上限 16，超限退化为包围盒。
fn current_strategy(dirty: &[Rect]) -> Vec<Rect> {
    const MAX: usize = 16;
    let mut out: Vec<Rect> = Vec::new();
    for &r in dirty {
        if out.len() >= MAX {
            return bbox_strategy(dirty);
        }
        let mut merged = false;
        for slot in out.iter_mut() {
            let b = *slot;
            let sep_x = b.x + b.w < r.x || r.x + r.w < b.x;
            let sep_y = b.y + b.h < r.y || r.y + r.h < b.y;
            if !(sep_x || sep_y) {
                *slot = bbox(&[b, r]).expect("non-empty");
                merged = true;
                break;
            }
        }
        if !merged {
            out.push(r);
        }
    }
    out
}

/// 带状（banded）：按 y 分带，每带取所有脏块的 x 并集（一个矩形近似）。
fn banded_strategy(dirty: &[Rect]) -> Vec<Rect> {
    // 以所有脏块的 y 边界切带；每带内取“覆盖该带的脏块”的 x 跨度。
    let mut ys: Vec<f32> = dirty.iter().flat_map(|r| [r.y, r.y + r.h]).collect();
    ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ys.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    let mut out = Vec::new();
    for w in ys.windows(2) {
        let (y0, y1) = (w[0], w[1]);
        if y1 - y0 < 0.01 {
            continue;
        }
        let mut x0 = f32::INFINITY;
        let mut x1 = f32::NEG_INFINITY;
        for r in dirty {
            if r.y < y1 && r.y + r.h > y0 {
                x0 = x0.min(r.x);
                x1 = x1.max(r.x + r.w);
            }
        }
        if x0.is_finite() && x1 > x0 {
            out.push(Rect::new(x0, y0, x1 - x0, y1 - y0));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 五种负载（对应设计文档 §4.1）
// ---------------------------------------------------------------------------

fn wl_ai_stream() -> Vec<Rect> {
    // 文本逐行追加：同一 x，y 递增，宽度不同（20%~100%）
    (0..6)
        .map(|i| Rect::new(10.0, 10.0 + i as f32 * 20.0, 400.0 + i as f32 * 60.0, 20.0))
        .collect()
}

fn wl_terminal_scroll() -> Vec<Rect> {
    // 整屏上移：两行全宽带
    vec![
        Rect::new(0.0, 0.0, W as f32, 20.0),
        Rect::new(0.0, H as f32 - 20.0, W as f32, 20.0),
    ]
}

fn wl_multi_widget() -> Vec<Rect> {
    // 8 个散落控件 120×40
    let pts = [
        (60.0, 80.0),
        (400.0, 300.0),
        (900.0, 150.0),
        (1500.0, 700.0),
        (300.0, 900.0),
        (1200.0, 200.0),
        (700.0, 600.0),
        (1700.0, 950.0),
    ];
    pts.iter()
        .map(|&(x, y)| Rect::new(x, y, 120.0, 40.0))
        .collect()
}

fn wl_corner_touch() -> Vec<Rect> {
    // 设计文档 §2.1 的病态构造：两条细条在角上相贴
    vec![
        Rect::new(0.0, 0.0, 1000.0, 1.0),
        Rect::new(0.0, 1.0, 1.0, 1000.0),
    ]
}

fn wl_cursor_blink() -> Vec<Rect> {
    vec![Rect::new(800.0, 500.0, 8.0, 16.0)]
}

// ---------------------------------------------------------------------------
// 矩阵
// ---------------------------------------------------------------------------

struct Row {
    name: &'static str,
    dirty_area: f64,
    per_rect_k: usize,
    bbox_area: f64,
    cur_k: usize,
    cur_area: f64,
    band_k: usize,
    band_area: f64,
}

fn row(name: &'static str, dirty: Vec<Rect>) -> Row {
    let dirty_area: f64 = dirty.iter().map(|&r| area(r)).sum();
    let b = bbox_strategy(&dirty);
    let cur = current_strategy(&dirty);
    let band = banded_strategy(&dirty);
    Row {
        name,
        dirty_area,
        per_rect_k: per_rect_strategy(&dirty).len(),
        bbox_area: b.iter().map(|&r| area(r)).sum(),
        cur_k: cur.len(),
        cur_area: cur.iter().map(|&r| area(r)).sum(),
        band_k: band.len(),
        band_area: band.iter().map(|&r| area(r)).sum(),
    }
}

#[test]
fn strategy_matrix() {
    // 标定值：从 calibrate_* 的实测数量级取，允许被环境覆盖（CI 上差异很大）。
    // c_switch 用 100 ns（CPU push/pop 量级），c_pixel 用 0.05 ns/px（CPU 填充量级）。
    // **这两个数是量级参考**，真正使用时以 calibrate_* 的输出为准。
    let c_switch_us = 0.1_f64;
    let c_pixel_ns = 0.05_f64;
    let cost = |k: usize, a: f64| -> f64 { k as f64 * c_switch_us * 1000.0 + a * c_pixel_ns }; // ns

    let rows = vec![
        row("AI 流式", wl_ai_stream()),
        row("终端滚屏", wl_terminal_scroll()),
        row("多控件", wl_multi_widget()),
        row("角落相贴", wl_corner_touch()),
        row("光标闪烁", wl_cursor_blink()),
    ];

    eprintln!(
        "{:<10} {:>10} | {:>5} {:>12} | {:>4} {:>12} | {:>4} {:>12} | {:>5} {:>12}",
        "负载",
        "脏面积",
        "k",
        "bbox(ns)",
        "k",
        "current(ns)",
        "k",
        "banded(ns)",
        "k",
        "per-rect(ns)"
    );
    for r in &rows {
        eprintln!(
            "{:<10} {:>10.0} | {:>5} {:>12.0} | {:>4} {:>12.0} | {:>4} {:>12.0} | {:>5} {:>12.0}",
            r.name,
            r.dirty_area,
            1,
            cost(1, r.bbox_area),
            r.cur_k,
            cost(r.cur_k, r.cur_area),
            r.band_k,
            cost(r.band_k, r.band_area),
            r.per_rect_k,
            cost(r.per_rect_k, r.dirty_area)
        );
    }

    // ---- 可失败断言（不是只打印） ----
    let corner = rows.iter().find(|r| r.name == "角落相贴").expect("row");
    let blowup = corner.cur_area / corner.dirty_area;
    eprintln!("[H2] 角落相贴：current 过度覆盖 = {blowup:.1}x");
    assert!(
        blowup > 100.0,
        "设计文档断言该形状应有数百倍过度覆盖，实测只有 {blowup:.1}x —— 病态例子或策略实现有误"
    );

    let cursor = rows.iter().find(|r| r.name == "光标闪烁").expect("row");
    assert_eq!(cursor.cur_k, 1, "单块脏区不需要合并，k 应为 1");
    assert_eq!(cursor.per_rect_k, 1);
}
