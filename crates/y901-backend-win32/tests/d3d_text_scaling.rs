//! D3D11 文本缩放的像素断言。
//!
//! 三条：
//! 1. `s = 1.0` 与"不压栈"的结果逐像素一致（含**变异对照**）；
//! 2. `s = 0.5` 的墨量比 ≈ 0.25（±30%）；
//! 3. 同一场景下 D3D11 与 SoftBackend 的 MAD ≤ 0.05。
//!
//! 为什么需要它：D3D 的变换栈在 v0.1.4 之前**根本不存在**（`push_transform` 是空操作），
//! 文本 LOD 接进去之后如果只靠"代码看起来对"，就和之前"clippy 0/0 是伪影"是同一类风险。

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiEvent, UiTree, Widget,
};

const W: u32 = 240;
const H: u32 = 120;
/// 锚点固定在字形左上附近，缩放围绕它进行。
const ANCHOR: (f32, f32) = (20.0, 30.0);
const TEXT: &str = "A";

/// 一屏纯色背景 + 一个字形；`scale = None` 表示不压栈。
struct GlyphScene {
    rect: Rect,
    scale: Option<f32>,
}

impl GlyphScene {
    fn new(scale: Option<f32>) -> Self {
        Self {
            rect: Rect::ZERO,
            scale,
        }
    }
}

impl Widget for GlyphScene {
    fn id(&self) -> &str {
        "scene"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, c.max.h))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let r = self.rect;
        // 背景用纯色：墨量统计需要一个**确定**的基准色
        ctx.fill_rect(r, Color::rgba(0.05, 0.05, 0.07, 1.0));
        let style = TextStyle::fg(Color::rgba(0.95, 0.96, 1.0, 1.0));
        match self.scale {
            None => {
                ctx.draw_text(Pos::new(ANCHOR.0, ANCHOR.1), TEXT, &style);
            }
            Some(k) => {
                ctx.push_transform(Pos::new(ANCHOR.0, ANCHOR.1), k);
                ctx.draw_text(Pos::new(ANCHOR.0, ANCHOR.1), TEXT, &style);
                ctx.pop_transform();
            }
        }
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 渲染一帧并回读 BGRA 像素。
fn render_scene(scale: Option<f32>) -> Option<(Vec<u8>, u32, u32)> {
    let options = Options {
        title: "d3d text scaling probe".to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,
    };
    let mut window = Window::create(&options).ok()?;
    let mut tree = UiTree::new(Box::new(GlyphScene::new(scale)));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree); // 预热
    window.request_snapshot();
    tree.invalidate_all();
    window.render(&mut tree);
    window.take_snapshot()
}

/// 墨量：所有像素与背景色之差的绝对值之和（覆盖率积分的离散近似）。
fn ink_mass(px: &[u8], w: u32, h: u32) -> u64 {
    // 背景取右下角（那里只有清屏色）
    let i = ((h - 1) * w + (w - 1)) as usize * 4;
    let bg = (px[i + 2] as i32, px[i + 1] as i32, px[i] as i32); // BGRA → RGB
    let mut sum = 0u64;
    for y in 0..h {
        for x in 0..w {
            let k = ((y * w + x) as usize) * 4;
            let r = px[k + 2] as i32;
            let g = px[k + 1] as i32;
            let b = px[k] as i32;
            sum += (r - bg.0).unsigned_abs() as u64
                + (g - bg.1).unsigned_abs() as u64
                + (b - bg.2).unsigned_abs() as u64;
        }
    }
    sum
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[test]
#[ignore = "需要 D3D11：--ignored --nocapture"]
fn d3d_scale_one_is_pixel_identical_and_identity_is_observable() {
    let (base, w, h) = match render_scene(None) {
        Some(v) => v,
        None => {
            eprintln!("[d3d-scale] 跳过：无 D3D11 设备");
            return;
        }
    };
    let (one, w1, h1) = match render_scene(Some(1.0)) {
        Some(v) => v,
        None => return,
    };
    assert_eq!((w, h), (w1, h1), "两次回读尺寸应一致");

    let diff = base
        .iter()
        .zip(one.iter())
        .filter(|(a, b)| a != b)
        .count();
    eprintln!("[d3d-scale] s = 1.0 与不压栈：{diff} 字节不同（hash {:#x} vs {:#x}）", fnv(&base), fnv(&one));
    assert_eq!(diff, 0, "s = 1.0 必须等同于不压栈（变换栈恒等）");

    // **变异对照**：改一个像素后哈希必须不同，否则"逐像素比较"没有区分力
    let mut mutated = base.clone();
    mutated[0] ^= 0xFF;
    assert_ne!(
        fnv(&base),
        fnv(&mutated),
        "改一个像素后哈希应变化（说明比较有区分力）"
    );
}

#[test]
#[ignore = "需要 D3D11；已知偏差：s=0.5 墨量比实测 0.326，高于上界 0.325（0.3%），待决策"]
fn d3d_half_scale_ink_is_about_a_quarter() {
    let (full, w, h) = match render_scene(None) {
        Some(v) => v,
        None => {
            eprintln!("[d3d-scale] 跳过：无 D3D11 设备");
            return;
        }
    };
    let (half, _, _) = match render_scene(Some(0.5)) {
        Some(v) => v,
        None => return,
    };

    let m_full = ink_mass(&full, w, h);
    let m_half = ink_mass(&half, w, h);
    let ratio = m_half as f64 / m_full.max(1) as f64;
    eprintln!("[d3d-scale] s = 0.5 墨量比 = {ratio:.3}（{m_half}/{m_full}）");
    assert!(
        (0.175..=0.325).contains(&ratio),
        "s = 0.5 的墨量比应约为 0.25，实测 {ratio:.3}"
    );
    assert!(m_half < m_full, "缩小后墨量应更少");
}

#[test]
#[ignore = "需要 D3D11 + CPU 后端对照：--ignored --nocapture"]
fn d3d_matches_soft_backend_within_tolerance() {
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;

    let (d3d, w, h) = match render_scene(Some(0.5)) {
        Some(v) => v,
        None => {
            eprintln!("[d3d-scale] 跳过：无 D3D11 设备");
            return;
        }
    };

    // 同一场景跑 SoftBackend
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    let mut soft = SoftBackend::new_headless(w, h, Box::new(engine), Size::new(cw, ch))
        .expect("soft");
    let mut tree = UiTree::new(Box::new(GlyphScene::new(Some(0.5))));
    tree.layout(Size::new(w as f32, h as f32), &soft);
    soft.clear();
    tree.render(&mut soft);

    // 逐像素比：D3D 回读是 BGRA，SoftBackend 的 pixel() 是 0xRRGGBB
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in 0..h {
        for x in 0..w {
            let k = ((y * w + x) as usize) * 4;
            let dr = d3d[k + 2] as i32;
            let dg = d3d[k + 1] as i32;
            let db = d3d[k] as i32;
            let p = soft.pixel(x, y).expect("px");
            let sr = ((p >> 16) & 0xFF) as i32;
            let sg = ((p >> 8) & 0xFF) as i32;
            let sb = (p & 0xFF) as i32;
            sum += (dr - sr).unsigned_abs() as u64
                + (dg - sg).unsigned_abs() as u64
                + (db - sb).unsigned_abs() as u64;
            n += 3;
        }
    }
    let mad = sum as f64 / (255.0 * n as f64);
    eprintln!("[d3d-scale] D3D vs SoftBackend（同场景 s=0.5）MAD = {mad:.4}");

    // 同一场景在 CPU 上的墨量比：用来判断“比值偏高”是场景/字形特性还是 D3D 特有
    let mut soft_full = SoftBackend::new_headless(w, h, {
        let (cw2, ch2) = y901_render::cell_metrics_for(15.0);
        Box::new(
            MyFontEngine::new(
                y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
                15.0,
                Size::new(cw2, ch2),
            )
            .expect("engine"),
        )
    }, Size::new(cw, ch))
    .expect("soft full");
    let mut t2 = UiTree::new(Box::new(GlyphScene::new(None)));
    t2.layout(Size::new(w as f32, h as f32), &soft_full);
    soft_full.clear();
    t2.render(&mut soft_full);
    let mut mass_full = 0u64;
    for y in 0..h {
        for x in 0..w {
            let p = soft_full.pixel(x, y).expect("px");
            mass_full += (((p >> 16) & 0xFF) as i32 - ((0.05 * 255.0) as i32)).unsigned_abs() as u64
                + (((p >> 8) & 0xFF) as i32 - ((0.05 * 255.0) as i32)).unsigned_abs() as u64
                + ((p & 0xFF) as i32 - ((0.07 * 255.0) as i32)).unsigned_abs() as u64;
        }
    }
    let mut mass_half = 0u64;
    for y in 0..h {
        for x in 0..w {
            let p = soft.pixel(x, y).expect("px");
            mass_half += (((p >> 16) & 0xFF) as i32 - ((0.05 * 255.0) as i32)).unsigned_abs() as u64
                + (((p >> 8) & 0xFF) as i32 - ((0.05 * 255.0) as i32)).unsigned_abs() as u64
                + ((p & 0xFF) as i32 - ((0.07 * 255.0) as i32)).unsigned_abs() as u64;
        }
    }
    eprintln!(
        "[d3d-scale] 同场景 CPU 墨量比 = {:.3}（与 D3D 可直接对比）",
        mass_half as f64 / mass_full.max(1) as f64
    );
    assert!(mad <= 0.05, "两后端 MAD = {mad:.4} > 0.05");
}
