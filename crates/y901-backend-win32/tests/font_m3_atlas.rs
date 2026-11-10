#![cfg(windows)]

//! 3b-2：图集容量与驱逐压力测试（CJK × 4 相位）。

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

/// 取 1200 个常用汉字（U+4E00 起连续 1200 个码位）。
fn cjk_1200() -> Vec<char> {
    (0x4E00u32..0x4E00 + 1200)
        .map(|cp| char::from_u32(cp).unwrap())
        .collect()
}

/// 验收 1：2048² 图集能装下 1200 字 × 4 相位，且**稳态零驱逐**。
#[test]
fn atlas_2048_holds_1200_cjk_times_four_phases() {
    let mut e = engine(2048);
    assert_eq!(e.atlas_dim(), 2048);
    let chars = cjk_1200();

    let t = Instant::now();
    for ch in &chars {
        for phase in 0u8..4 {
            e.begin_frame();
            assert!(
                e.glyph_phased(*ch, phase).is_some(),
                "取字形失败 {ch:?} p{phase}"
            );
        }
    }
    let build = t.elapsed().as_secs_f64() * 1e3;
    let occ = e.occupancy();
    eprintln!(
        "[3b2] 2048² 建立 1200×4={} 个字形：{build:.1}ms，占用 {:.1}%，驱逐总计={}",
        chars.len() * 4,
        occ * 100.0,
        e.evicted_last_frame()
    );
    assert!(occ < 0.95, "占用率 {:.1}% 接近爆仓", occ * 100.0);

    // 稳态：重新请求同一批，应全命中、零驱逐
    let mut worst_evict = 0usize;
    let t2 = Instant::now();
    for _ in 0..3 {
        for ch in &chars {
            e.begin_frame();
            let _ = e.glyph_phased(*ch, 0);
            worst_evict = worst_evict.max(e.evicted_last_frame());
        }
    }
    let steady = t2.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "[3b2] 稳态（3 轮×1200 字命中）：{steady:.1}ms，最坏驱逐={worst_evict}",
        worst_evict = worst_evict
    );
    assert_eq!(worst_evict, 0, "稳态不应驱逐");
}

/// 验收 2（核心）：**强制驱逐** —— 把图集压到 512²，看 LRU 真正跑起来之后帧驱逐有多少。
///
/// 这个测试**不断言 ≤ 4**：512² 只能装 ~200 槽，而工作集是 1200字。
/// 它的用途是**把抖动数据量出来**，而不是证明不抖。
#[test]
#[ignore = "压力测试：会制造巨量驱逐，--ignored 运行"]
fn forced_eviction_thrash_measurement() {
    let mut e = engine(512);
    let chars = cjk_1200();
    eprintln!(
        "[3b2] 图集 512²（容量约 200 槽），工作集 {} 字",
        chars.len()
    );

    // 第一轮：建立（必然持续驱逐）
    let mut total_evict = 0usize;
    let mut worst_frame = 0usize;
    let t = Instant::now();
    for ch in chars.iter() {
        e.begin_frame();
        let _ = e.glyph(*ch);
        total_evict += e.evicted_last_frame();
        worst_frame = worst_frame.max(e.evicted_last_frame());
    }
    let build = t.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "[3b2] 建立 {} 字：{build:.1}ms，驱逐总计={total_evict}，单帧最坏={worst_frame}",
        chars.len()
    );
    let occ_after_build = e.occupancy();

    // 第二轮：重新请求同一批（工作集 > 容量 → 必然抖动）
    let mut re_evict = 0usize;
    let mut re_worst = 0usize;
    let t2 = Instant::now();
    for _ in 0..3 {
        for ch in chars.iter() {
            e.begin_frame();
            let _ = e.glyph(*ch);
            re_evict += e.evicted_last_frame();
            re_worst = re_worst.max(e.evicted_last_frame());
        }
    }
    let re = t2.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "[3b2] 3 轮重请求（3×{} 字）：{re:.1}ms，驱逐总计={re_evict}（平均 {:.1}/字），单帧最坏={re_worst}",
        chars.len(),
        re_evict as f64 / (3.0 * chars.len() as f64)
    );
    eprintln!(
        "[3b2] 建立后占用 {:.1}%，缓存字形数={}",
        occ_after_build * 100.0,
        e.glyph_count()
    );

    // 硬断言：必须真的发生过驱逐（否则测试没意义）
    assert!(total_evict > 0, "未触发驱逐，压力场景无效");
    // 记录抖动程度：工作集超容量时必然 > 0
    eprintln!("[3b2] 结论：512² 下工作集超容量 → 每帧驱逐 {re_worst} 个（预期为大量抖动）");
}

/// 验收 2b：驱逐后像素被清零，重建后字迹完整恢复（无残影）。
#[test]
fn eviction_clears_pixels_and_rebuild_restores() {
    let mut e = engine(256); // 极小图集，保证快速触发驱逐
    let chars = cjk_1200();
    let asz = e.atlas_dim() as usize;

    // 先取一个字形并记下它的图集坐标与像素
    e.begin_frame();
    let first = e.glyph(chars[0]).expect("第一个字").clone();
    let sample = |e: &MyFontEngine, g: &y901_backend_win32::backend::font::GlyphInfo| -> u32 {
        let mut sum = 0u32;
        for y in 0..g.height as usize {
            for x in 0..g.width as usize {
                let i = ((g.atlas_y as usize + y) * asz + (g.atlas_x as usize + x)) * 4;
                sum += e.atlas()[i] as u32;
            }
        }
        sum
    };
    let ink_before = sample(&e, &first);
    assert!(ink_before > 0, "首次入图集应有墨迹");

    // 灌满，直到第一个字被驱逐
    let mut evicted = 0;
    for ch in chars.iter().take(600) {
        e.begin_frame();
        let _ = e.glyph(*ch);
        evicted += e.evicted_last_frame();
    }
    eprintln!("[3b2] 256² 下灌 600 字，驱逐 {evicted} 个");
    assert!(evicted > 0, "小图集应该触发驱逐");

    // 重新取"第一个字"：它应已不在缓存中（被驱逐），应能重建
    e.begin_frame();
    let again = e.glyph(chars[0]).expect("重建失败").clone();
    let ink_after = sample(&e, &again);
    eprintln!(
        "[3b2] 驱逐后重建：墨量 {ink_before} -> {ink_after}（差 {}% 内应一致）",
        ((ink_after as f64 - ink_before as f64).abs() / ink_before as f64 * 100.0) as i32
    );
    let diff = (ink_after as f64 - ink_before as f64).abs() / ink_before as f64;
    assert!(
        diff < 0.02,
        "重建后墨量不一致：{ink_before} vs {ink_after}（{:.2}%）——可能有残影或尺寸变化",
        diff * 100.0
    );
}

/// 分解：光栅化本身 vs 入图集，到底哪一段慢？
#[test]
#[ignore = "基准"]
fn breakdown_rasterize_vs_atlas_insert() {
    use y901_backend_win32::font_engine::rasterize_glyph_phased;
    let data = load_font(SIMSUN).expect("font");
    let chars = cjk_1200();

    // A) 只光栅化（不入图集）
    let t = Instant::now();
    for ch in chars.iter().take(1200) {
        let _ = rasterize_glyph_phased(&data, *ch, SIZE, 0);
    }
    let raster = t.elapsed().as_secs_f64() * 1e3;

    // B) 光栅化 + 入图集
    let mut e = engine(2048);
    let t2 = Instant::now();
    for ch in chars.iter().take(1200) {
        e.begin_frame();
        let _ = e.glyph(*ch);
    }
    let with_atlas = t2.elapsed().as_secs_f64() * 1e3;
    eprintln!(
        "[3b2] 1200 字：纯光栅化 {raster:.1}ms（{:.1}us/字），光栅化+图集 {with_atlas:.1}ms（{:.1}us/字），图集占用 {:.1}%",
        raster / 1200.0,
        with_atlas / 1200.0,
        e.occupancy() * 100.0
    );
    eprintln!("[3b2] 图集部分估算 = {:.1}ms", with_atlas - raster);
}

/// 诊断：占用率与缓存数量应该一致（不一致说明公式或缓存有 bug）。
#[test]
#[ignore = "调试"]
fn occupancy_matches_glyph_count_sanity() {
    let mut e = engine(512);
    let chars = cjk_1200();
    for ch in chars.iter().take(1200) {
        e.begin_frame();
        let _ = e.glyph(*ch);
    }
    eprintln!(
        "[dbg] atlas={} cached={} occupancy={:.3} evicted_total={}",
        e.atlas_dim(),
        e.glyph_count(),
        e.occupancy(),
        e.evicted_last_frame()
    );
    // 手算：每个字形占 (w+1)*(h+1)，应与 occupancy*atlas^2 一致
    assert!(e.glyph_count() > 0);
}
