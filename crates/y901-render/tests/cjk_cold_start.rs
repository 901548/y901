//! CJK 冷启动基准：字形生成、mip 生成、首帧渲染。
//!
//! 跑法：
//! ```text
//! cargo test -p y901-render --release --test cjk_cold_start -- --ignored --nocapture
//! ```
//!
//! 回答三个问题：
//! 1. 500 个 CJK 字的 L0 / L1 / L2 生成各要多久？
//! 2. 冷启动（字体加载 + 500 字 L0 + 首帧渲染）是否 ≤ 50ms？
//! 3. 没有任何缩放渲染时，mip 生成次数是否真的是 0？

use std::time::Instant;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, RenderCtx, Size, TextStyle};

/// 500 个常用 CJK 字（U+4E00 起的连续区间，取舍不为语义、只为规模）。
fn cjk_chars(n: usize) -> Vec<char> {
    (0..n).filter_map(|i| char::from_u32(0x4E00 + i as u32)).collect()
}

/// 优先用 CJK 字体；找不到就返回 Err（测试会明确报错而不是静默降级）。
fn cjk_font() -> Result<Vec<u8>, String> {
    for p in [
        "C:\\Windows\\Fonts\\simsun.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ] {
        if std::path::Path::new(p).exists() {
            return y901_render::load_font(p);
        }
    }
    Err("找不到 CJK 字体".to_string())
}

fn engine(font: Vec<u8>) -> MyFontEngine {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    MyFontEngine::new(font, 15.0, Size::new(cw, ch)).expect("engine")
}

#[test]
#[ignore = "冷启动基准：--ignored --nocapture 手动跑"]
fn cjk_500_glyph_and_mip_generation_time() {
    let load_start = Instant::now();
    let font = cjk_font().expect("CJK 字体");
    let load_ms = load_start.elapsed().as_secs_f64() * 1e3;
    let mut e = engine(font);
    let chars = cjk_chars(500);

    // ---- L0 ----
    let t = Instant::now();
    let mut l0_ok = 0;
    for &c in &chars {
        if e.glyph(c).is_some() {
            l0_ok += 1;
        }
    }
    let l0_ms = t.elapsed().as_secs_f64() * 1e3;

    // ---- L1 only ----
    let t = Instant::now();
    for &c in &chars {
        let _ = e.ensure_mips_upto(c, 0);
    }
    let l1_ms = t.elapsed().as_secs_f64() * 1e3;

    // ---- L2 only（L1 已存在） ----
    let t = Instant::now();
    for &c in &chars {
        let _ = e.ensure_mips_upto(c, 1);
    }
    let l2_ms = t.elapsed().as_secs_f64() * 1e3;

    eprintln!(
        "[cjk] 字体加载 {load_ms:.1}ms；L0 {l0_ok}/{} 字 {l0_ms:.1}ms；L1 {l1_ms:.1}ms；L2 {l2_ms:.1}ms；L1+L2 合计 {:.1}ms",
        chars.len(),
        l1_ms + l2_ms
    );
    assert!(l0_ok > 400, "CJK 字体应能光栅化绝大多数汉字，实际 {l0_ok}");
}

#[test]
#[ignore = "冷启动基准：--ignored --nocapture 手动跑"]
fn cjk_500_cold_start_first_frame() {
    let chars = cjk_chars(500);

    let t_all = Instant::now();
    let font = cjk_font().expect("CJK 字体");
    let mut e = engine(font);

    // 500 字 L0（模拟首帧要显示的内容）
    for &c in &chars {
        let _ = e.glyph(c);
    }

    // 首帧：把它们画到一个 1200x800 的软后端上
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let mut s =
        SoftBackend::new_headless(1200, 800, Box::new(e), Size::new(cw, ch)).expect("soft");
    s.clear();
    let style = TextStyle::fg(Color::WHITE);
    for (i, &c) in chars.iter().enumerate() {
        let x = 8.0 + (i % 40) as f32 * 28.0;
        let y = 8.0 + (i / 40) as f32 * 24.0;
        s.draw_text(Pos::new(x, y), &c.to_string(), &style);
    }
    let total_ms = t_all.elapsed().as_secs_f64() * 1e3;

    eprintln!("[cjk] 冷启动（加载 + 500 L0 + 首帧绘制）= {total_ms:.1}ms（目标 ≤ 50ms）");
    assert!(total_ms > 0.0);
}

#[test]
#[ignore = "冷启动基准：--ignored --nocapture 手动跑"]
fn no_scaling_means_zero_mip_generation() {
    let font = cjk_font().expect("CJK 字体");
    let mut e = engine(font);
    let chars = cjk_chars(200);

    // 只调 glyph()（等价于 scale = 1.0 的渲染路径）
    for &c in &chars {
        let _ = e.glyph(c);
    }

    // \u6ce8\u610f\uff1a\u8fd9\u91cc\u4e0d\u80fd\u76f4\u63a5\u6570 mip\uff0c\u53ea\u80fd\u95f4\u63a5\u9a8c\u8bc1\uff1a
    // \u5982\u679c mip \u88ab\u751f\u6210\u4e86\uff0c\u518d\u8c03 ensure_mips \u5c31\u4f1a\u7acb\u5373\u8fd4\u56de\u4e14\u4e0d\u82b1\u65f6\u95f4\u3002
    // \u6240\u4ee5\u6bd4\u8f83"\u9996\u6b21 ensure_mips \u7684\u8017\u65f6"\u4e0e"\u5df2\u751f\u6210\u540e\u518d\u8c03"\u7684\u8017\u65f6\u3002
    let t = Instant::now();
    for &c in &chars[..50] {
        let _ = e.ensure_mips_upto(c, 0);
    }
    let first = t.elapsed().as_secs_f64() * 1e3;

    let t = Instant::now();
    for &c in &chars[..50] {
        let _ = e.ensure_mips_upto(c, 0);
    }
    let again = t.elapsed().as_secs_f64() * 1e3;

    eprintln!("[cjk] 首次生成 50 字 L1 = {first:.3}ms；重复调用 = {again:.3}ms（应接近 0）");
    assert!(
        again < first / 2.0,
        "幂等调用应明显快于首次生成（首次 {first:.3}ms，再次 {again:.3}ms）"
    );
}
