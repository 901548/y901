//! mip 链的**存储成本**与**打包器碎片率**测量（第 2d 步）。
//!
//! 跑法：
//! ```text
//! cargo test -p y901-render --release --test mip_storage -- --nocapture
//! ```

use y901_render::MyFontEngine;
use y901_ui::Size;

fn engine(px: f32) -> MyFontEngine {
    let (cw, ch) = y901_render::cell_metrics_for(px);
    MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        px,
        Size::new(cw, ch),
    )
    .expect("engine")
}

#[test]
fn latin_mip_storage_cost_and_fragmentation() {
    let mut e = engine(15.0);

    let chars: Vec<char> = ('A'..='Z').chain('a'..='z').chain('0'..='9').collect();

    let mut area_l0 = 0u64;
    let mut area_l1 = 0u64;
    let mut area_l2 = 0u64;
    let mut with_l1 = 0usize;
    let mut with_l2 = 0usize;
    // 碎片率代理指标：已用区域的包围盒 vs 实际打包面积
    let (mut min_x, mut min_y) = (u32::MAX, u32::MAX);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    let mut track = |x: u32, y: u32, w: u32, h: u32| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + w);
        max_y = max_y.max(y + h);
    };

    for &ch in &chars {
        // 先取字形（进缓存），mip 是**懒生成**的：得显式要求才会付存储成本
        let _ = e.glyph(ch);
        let _ = e.ensure_mips(ch);
        let g = match e.glyph(ch) {
            Some(g) => g,
            None => continue,
        };
        // 注意：这里只能拿到 L0 的宽高与 mip 尺寸，因为像素已写进图集
        area_l0 += (g.width as u64 + 1) * (g.height as u64 + 1);
        track(g.atlas_x, g.atlas_y, g.width, g.height);
        if let Some(m) = g.mips[0] {
            area_l1 += (m.width as u64 + 1) * (m.height as u64 + 1);
            with_l1 += 1;
            track(m.atlas_x, m.atlas_y, m.width, m.height);
        }
        if let Some(m) = g.mips[1] {
            area_l2 += (m.width as u64 + 1) * (m.height as u64 + 1);
            with_l2 += 1;
            track(m.atlas_x, m.atlas_y, m.width, m.height);
        }
    }

    let total = area_l0 + area_l1 + area_l2;
    let ratio = total as f64 / area_l0 as f64;
    let ratio_l1_only = (area_l0 + area_l1) as f64 / area_l0 as f64;

    eprintln!(
        "[mip-storage] 字形 {} 个：L0={} 像素面积，L1={} （{} 个字形生成），L2={} （{} 个）",
        chars.len(), area_l0, area_l1, with_l1, area_l2, with_l2
    );
    eprintln!(
        "[mip-storage] 总占用 / L0 = {:.3}×（红线 1.6×）；若只保留 L1 = {:.3}×",
        ratio, ratio_l1_only
    );

    // 拉美尔坐标：实际占用 vs 理论下界
    let atlas_area = (e.atlas_dim() as f64).powi(2);
    let packed = total as f64;
    eprintln!(
        "[mip-storage] 图集 {}x{} = {:.0} 像素，已打包 {:.0} 像素，占用率 {:.2}%",
        e.atlas_dim(),
        e.atlas_dim(),
        atlas_area,
        packed,
        100.0 * packed / atlas_area
    );

    // 碎片率（代理指标）= 1 - 已用面积 / 包围盒面积。
    // 为什么不直接读打包器：`SkylinePacker` 未对外暴露空洞统计，
    // 而包围盒比值能用现有字段算出，且对"碎片是否严重"足够敏感。
    let bbox = (max_x - min_x) as u64 * (max_y - min_y) as u64;
    let frag = 1.0 - total as f64 / bbox as f64;
    eprintln!(
        "[mip-storage] 已用包围盒 {}x{} = {} 像素；碎片率（代理）{:.1}%",
        max_x - min_x,
        max_y - min_y,
        bbox,
        100.0 * frag
    );

    assert_eq!(with_l1, chars.len(), "每个字形都应生成 L1");
    assert!(ratio > 1.0);
}
