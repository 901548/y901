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
