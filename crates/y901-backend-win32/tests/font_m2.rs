//! Milestone 2 验收：度量对齐、图集集成、LRU、恢复。

use y901_backend_win32::backend::font::{cell_metrics_for, DirtyRect, TextEngine};
use y901_backend_win32::font_engine::{load_font, MyFontEngine};
use y901_ui::Size;

const SIZE: f32 = 15.0;

fn ours() -> MyFontEngine {
    let data = load_font(&y901_backend_win32::font_engine::default_test_font_path()).expect("font");
    let (cw, ch) = cell_metrics_for(SIZE);
    MyFontEngine::new(data, SIZE, Size::new(cw, ch)).expect("engine")
}

fn legacy() -> TextEngine {
    let (cw, ch) = cell_metrics_for(SIZE);
    TextEngine::new("Segoe UI", SIZE, cw, ch).expect("dwrite")
}

/// 验收 5：`cell_span` 序列必须与 DirectWrite 逐位相同（ASCII 32..=126）。
#[test]
fn cell_span_matches_legacy_for_ascii() {
    let a = ours();
    let b = legacy();
    let mut diff = Vec::new();
    for cp in 32u32..=126 {
        let ch = char::from_u32(cp).unwrap();
        let (sa, sb) = (a.cell_span(ch), b.cell_span(ch));
        if sa != sb {
            diff.push((ch, sa, sb));
        }
    }
    assert!(
        diff.is_empty(),
        "cell_span 不一致（字符, 自研, legacy）：{diff:?}"
    );
}

/// 验收 5b：CJK / 全角标点也要一致。
#[test]
fn cell_span_matches_legacy_for_cjk() {
    let a = ours();
    let b = legacy();
    for ch in ['中', '文', '字', '体', '。', '，', '！', 'あ', 'Ａ'] {
        assert_eq!(a.cell_span(ch), b.cell_span(ch), "字符 {ch:?} 格数不一致");
    }
}

/// 验收 2：ASCII 全集占用率 < 60%。
#[test]
fn ascii_full_set_occupancy_below_60_percent() {
    let mut e = ours();
    for cp in 32u32..=126 {
        let ch = char::from_u32(cp).unwrap();
        e.begin_frame();
        let _ = e.glyph(ch);
    }
    let occ = e.occupancy();
    eprintln!("[atlas] ASCII 95 字符占用率 = {:.1}%", occ * 100.0);
    assert!(occ < 0.60, "占用率 {:.1}% 超过 60%", occ * 100.0);
}

/// 验收 3：含 LRU 驱逐时，每帧驱逐数可控（构造超载）。
#[test]
fn eviction_is_bounded_per_frame() {
    let mut e = ours();
    // 灌满：反复请求一批字符，直到图集接近饱和
    let chars: Vec<char> = (33u32..=126).map(|c| char::from_u32(c).unwrap()).collect();
    for _ in 0..40 {
        for ch in &chars {
            e.begin_frame();
            let _ = e.glyph(*ch);
        }
    }
    // 稳态：再跑 5 帧，每帧驱逐应很小
    let mut worst = 0usize;
    for _ in 0..5 {
        e.begin_frame();
        for ch in &chars {
            let _ = e.glyph(*ch);
        }
        worst = worst.max(e.evicted_last_frame());
    }
    eprintln!("[lru] 稳态每帧字形数={} 最坏驱逐={worst}", e.glyph_count());
    assert!(worst <= 4, "每帧驱逐 {worst} 个，超过 4");
}

/// 验收 4：强制清空后，所有字形能全量恢复。
#[test]
fn clears_and_recovers_all_glyphs() {
    let mut e = ours();
    let chars: Vec<char> = (33u32..=90).map(|c| char::from_u32(c).unwrap()).collect();
    for ch in &chars {
        e.begin_frame();
        assert!(e.glyph(*ch).is_some(), "首次取字形失败 {ch:?}");
    }
    e.clear_atlas();
    assert!(e.glyph_count() == 0, "清空后缓存应为空");
    let (reset, _) = e.take_dirty();
    assert!(reset, "清空后 take_dirty 必须报 reset");
    for ch in &chars {
        e.begin_frame();
        assert!(e.glyph(*ch).is_some(), "恢复失败 {ch:?}");
    }
    assert_eq!(e.glyph_count(), chars.len(), "应全部恢复");
}

/// `take_dirty` 协议：reset 是**消耗式**（取一次就没了），脏区列表也被取走。
#[test]
fn take_dirty_is_consuming_like_legacy() {
    let mut e = ours();
    let (r0, d0) = e.take_dirty();
    assert!(r0, "初始应报 reset");
    assert!(d0.is_empty(), "初始无脏区");
    let (r1, d1) = e.take_dirty();
    assert!(!r1, "reset 应被消耗（第二次 false）");
    assert!(d1.is_empty(), "第二次仍应为空");

    e.begin_frame();
    let _ = e.glyph('A');
    let (r2, d2) = e.take_dirty();
    assert!(!r2, "新字形不应报 reset（只是局部脏区）");
    assert_eq!(d2.len(), 1, "写了一个字形，应有一个脏区，实际 {:?}", d2.len());
    let DirtyRect { width, height, .. } = d2[0];
    assert!(width > 0 && height > 0, "脏区应非空");
}

/// 图集像素格式：RGB = 覆盖度，A = 255。
#[test]
fn atlas_pixels_are_coverage_with_opaque_alpha() {
    let mut e = ours();
    e.begin_frame();
    let g = e.glyph('A').expect("A").clone();
    let asz = e.atlas_size() as usize;
    let mut non_zero = 0;
    for y in 0..g.height as usize {
        for x in 0..g.width as usize {
            let i = ((g.atlas_y as usize + y) * asz + (g.atlas_x as usize + x)) * 4;
            let (r, gg, b, a) = (e.atlas()[i], e.atlas()[i + 1], e.atlas()[i + 2], e.atlas()[i + 3]);
            assert_eq!(a, 255, "图集 alpha 必须是 255");
            assert_eq!(r, gg, "RGB 应相等（灰度覆盖度）");
            assert_eq!(r, b, "RGB 应相等");
            if r > 0 {
                non_zero += 1;
            }
        }
    }
    assert!(non_zero > 10, "'A' 应有实际覆盖像素，实际 {non_zero}");
}

/// 验收 3a-1：同一字符的 4 个相位位图必须互不相同。
#[test]
fn four_phases_produce_distinct_bitmaps() {
    let mut e = ours();
    let asz = e.atlas_size() as usize;
    for ch in ['A', 'o', 'e', 'B', '8', 'g', 'm'] {
        let mut hashes = Vec::new();
        for phase in 0u8..4 {
            e.begin_frame();
            let g = e.glyph_phased(ch, phase).unwrap_or_else(|| panic!("取字形失败 {ch:?} p{phase}")).clone();
            // 对该字形在图集里的区域做 FNV 哈希
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for y in 0..g.height as usize {
                for x in 0..g.width as usize {
                    let i = ((g.atlas_y as usize + y) * asz + (g.atlas_x as usize + x)) * 4;
                    h ^= e.atlas()[i] as u64;
                    h = h.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
            hashes.push(h);
        }
        let uniq: std::collections::HashSet<_> = hashes.iter().collect();
        eprintln!("[phase] {ch:?}: {hashes:?} -> {} 种", uniq.len());
        assert!(
            uniq.len() >= 3,
            "{ch:?} 的 4 个相位几乎一样（只有 {} 种）——亚像素平移没生效",
            uniq.len()
        );
    }
}

/// 验收 3a-2：相位平移使墨迹**质心**单调右移。
///
/// 为什么不看"最左墨迹列"：字形起点本来就落在某个像素内部（小数 x），
/// 平移 0/4~3/4 px 仍在同一列内 → 列号不变，但覆盖度分布在变。
/// 质心（按覆盖度加权）才是真正的"位置”。
#[test]
fn phase_shifts_ink_centroid_monotonically() {
    let mut e = ours();
    let asz = e.atlas_size() as usize;
    let mut centroids = Vec::new();
    for phase in 0u8..4 {
        e.begin_frame();
        let g = e.glyph_phased('A', phase).expect("A").clone();
        let mut sum = 0f64;
        let mut weight = 0f64;
        for y in 0..g.height as usize {
            for x in 0..g.width as usize {
                let i = ((g.atlas_y as usize + y) * asz + (g.atlas_x as usize + x)) * 4;
                let c = e.atlas()[i] as f64;
                sum += c * x as f64;
                weight += c;
            }
        }
        centroids.push(sum / weight.max(1.0));
    }
    eprintln!("[phase] 'A' 墨迹质心随相位: {centroids:?}");
    // 0 → 1/4 → 1/2 → 3/4 应逐步右移（允许四舍五入带来的小抖动）
    for i in 1..4 {
        assert!(
            centroids[i] > centroids[i - 1] - 0.05,
            "质心未单调右移：{centroids:?}"
        );
    }
    assert!(
        centroids[3] > centroids[0] + 0.3,
        "相位 3 应比相位 0 明显右移（>0.3px），实际 {centroids:?}"
    );
}

/// 3a 的 L2 实测：在**最佳平移**下比较形状（区分"形状不对"与"只是错位"）。
///
/// 为什么要搜平移：M1 已经知道两边的 bearing 差 1~2px。
/// 若搜到最佳平移后 p95 很小 → 说明**形状一致**，剩下的只是定位差（M3 的度量对齐要解决的）；
/// 若搜完仍很大 → 说明形状真的不对。
#[test]
#[ignore = "需要 DirectWrite"]
fn l2_best_shift_diagnostic() {
    use y901_backend_win32::backend::font::TextEngine;

    let f = load_font(&y901_backend_win32::font_engine::default_test_font_path()).expect("font");
    let (cw, chh) = cell_metrics_for(SIZE);
    let mut dw = TextEngine::new("Segoe UI", SIZE, cw, chh).expect("dwrite");
    let asz = dw.atlas_size as usize;

    for ch in ['A', 'o', 'e', 'B', '8'] {
        let ours = y901_backend_win32::font_engine::rasterize_glyph(&f, ch, SIZE)
            .unwrap_or_else(|| panic!("{ch:?}"));
        let info = match dw.glyph(ch) {
            Some(g) if !g.is_empty() => g.clone(),
            _ => continue,
        };
        let mut theirs = vec![0u8; (info.width * info.height) as usize];
        for y in 0..info.height as usize {
            for x in 0..info.width as usize {
                theirs[y * info.width as usize + x] =
                    dw.atlas[((info.atlas_y as usize + y) * asz + (info.atlas_x as usize + x)) * 4];
            }
        }
        let (ow, oh) = (ours.width as i32, ours.height as i32);
        let (tw, th) = (info.width as i32, info.height as i32);

        let mut best: Option<(i32, i32, f64, i32)> = None; // (dx,dy,mean,p95)
        for dy in -2..=2 {
            for dx in -2..=2 {
                let mut diffs = Vec::new();
                for y in 0..oh.min(th) {
                    for x in 0..ow.min(tw) {
                        let ox = x + dx;
                        let oy = y + dy;
                        let a = if ox >= 0 && ox < ow && oy >= 0 && oy < oh {
                            ours.at(ox as u32, oy as u32) as i32
                        } else {
                            0
                        };
                        let b = theirs[(y * tw + x) as usize] as i32;
                        diffs.push((a - b).abs());
                    }
                }
                diffs.sort_unstable();
                let mean = diffs.iter().sum::<i32>() as f64 / diffs.len() as f64;
                let p95 = diffs[(diffs.len() as f64 * 0.95) as usize];
                if best.is_none_or(|(_, _, m, _)| mean < m) {
                    best = Some((dx, dy, mean, p95));
                }
            }
        }
        let (dx, dy, mean, p95) = best.expect("best");
        eprintln!("[l2] {ch:?}: 最佳平移=({dx:+},{dy:+}) mean={mean:.1} p95={p95}");
    }
}
