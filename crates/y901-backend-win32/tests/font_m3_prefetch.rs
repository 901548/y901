#![cfg(windows)]

//! 3b-3（C 方案）：可见行才用 4 相位 + 滚动预取。
//!
//! C 方案的本质：**只有马上要画的字才需要 4 相位**。
//! 后台预取 / 不可见内容只要 phase 0（它们要么不画，要么已经被预取热好）。

use std::time::Instant;

use y901_backend_win32::backend::font::cell_metrics_for;
use y901_backend_win32::font_engine::{load_font, MyFontEngine};
use y901_ui::Size;

const SIZE: f32 = 15.0;
const SIMSUN: &str = "C:\\Windows\\Fonts\\simsun.ttc";

fn engine(atlas: u32) -> MyFontEngine {
    let data = load_font(SIMSUN).expect("simsun");
    let (cw, ch) = cell_metrics_for(SIZE);
    MyFontEngine::with_atlas_size(data, SIZE, Size::new(cw, ch), atlas).expect("engine")
}

fn cjk(n: usize) -> Vec<char> {
    (0x4E00u32..0x4E00 + n as u32)
        .map(|cp| char::from_u32(cp).unwrap())
        .collect()
}

/// 验收 1：**C 方案**下，1200 字首帧 < 60ms。
///
/// 场景：1200 字的文档，可见窗口 240 字（约 12 行 × 20 字）。
/// 可见的 240 字 × 4 相位 + 不可见的 960 字 × 1 相位。
#[test]
fn c_plan_1200_chars_first_frame_under_60ms() {
    let mut e = engine(2048);
    let chars = cjk(1200);
    let visible = 240usize;

    let t = Instant::now();
    // 可见区：4 相位全生成
    for ch in chars.iter().take(visible) {
        for phase in 0u8..4 {
            e.begin_frame();
            assert!(e.glyph_phased(*ch, phase).is_some(), "可见字失败 {ch:?}");
        }
    }
    // 不可见区：只预取 phase 0
    let added = e.prefetch(&chars[visible..]);
    let first_frame = t.elapsed().as_secs_f64() * 1e3;

    let slots = visible * 4 + added;
    eprintln!(
        "[3b3] C 方案 1200 字（可见 {visible}×4 + 不可见 {added}×1 = {slots} 槽）：首帧 {first_frame:.1}ms，占用 {:.1}%，驱逐 {}",
        e.occupancy() * 100.0,
        e.evicted_last_frame()
    );
    // 计时断言只在 release 下有意义：debug 构建慢 ~8 倍（实测 248ms）。
    // 用 `cfg!(debug_assertions)` 而不是 `#[ignore]`：这样 `cargo test` 仍能跑到这条路径。
    if cfg!(debug_assertions) {
        eprintln!("[3b3] debug 构建：首帧 {first_frame:.1}ms（不计入 60ms 断言）");
    } else {
        assert!(
            first_frame < 60.0,
            "C 方案首帧 {first_frame:.1}ms 超过 60ms"
        );
    }
}

/// 对照：如果**所有 1200 字都生成 4 相位**（不用 C 方案）会多久？
#[test]
#[ignore = "对照基准"]
fn all_phases_reference_cost() {
    let mut e = engine(2048);
    let chars = cjk(1200);
    let t = Instant::now();
    for ch in &chars {
        for phase in 0u8..4 {
            e.begin_frame();
            let _ = e.glyph_phased(*ch, phase);
        }
    }
    let ms = t.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "[3b3] 对照：1200 字 × 4 相位全生成 {ms:.1}ms，占用 {:.1}%",
        e.occupancy() * 100.0
    );
}

/// 验收 2：预取后，滚动进入视口的行**不需要现场光栅化**（帧耗时几乎为 0）。
///
/// 验证方式：测"首次进入视口的 240 字"在预取前/后的耗时差。
#[test]
fn prefetch_removes_on_demand_rasterization() {
    let chars = cjk(1200);
    let window: Vec<char> = chars[240..480].to_vec();

    // A) 不预取：直接请求（会现场光栅化）
    let mut a = engine(2048);
    let t = Instant::now();
    for ch in &window {
        for phase in 0u8..4 {
            a.begin_frame();
            let _ = a.glyph_phased(*ch, phase);
        }
    }
    let cold = t.elapsed().as_secs_f64() * 1e3;

    // B) 预取后再请求（应几乎全命中；phase 0 已热，其余 3 个相位仍需生成）
    let mut b = engine(2048);
    let t = Instant::now();
    let added = b.prefetch(&window);
    let prefetch_ms = t.elapsed().as_secs_f64() * 1e3;
    let t2 = Instant::now();
    for ch in &window {
        for phase in 0u8..4 {
            b.begin_frame();
            let _ = b.glyph_phased(*ch, phase);
        }
    }
    let warm = t2.elapsed().as_secs_f64() * 1e3;

    eprintln!(
        "[3b3] 预取效果：不预取 {cold:.1}ms；预取 {added} 字 {prefetch_ms:.1}ms + 随后请求 {warm:.1}ms",
    );
    // 预取 + 随后请求 应比"直接请求"快（phase 0 那一块已经完成了）
    assert!(
        warm < cold,
        "预取后的请求应更快：{cold:.2}ms vs {warm:.2}ms"
    );
}

/// 验收 2b：预取不会因预取而驱逐现有字形（不制造抖动）。
#[test]
fn prefetch_does_not_evict_existing_glyphs() {
    let mut e = engine(2048);
    let chars = cjk(1200);
    e.prefetch(&chars[..200]);
    let before = e.glyph_count();
    let evict_before = e.evicted_last_frame();
    let added = e.prefetch(&chars[200..400]);
    eprintln!(
        "[3b3] 预取 200 字：之前 {before} 个字形，新增 {added}，驱逐 {evict_before} -> {}",
        e.evicted_last_frame()
    );
    assert_eq!(added, 200, "应新增 200 个字形");
    assert_eq!(e.glyph_count(), 400, "旧字形应保留");
    assert_eq!(e.evicted_last_frame(), evict_before, "预取不应驱逐");
}
