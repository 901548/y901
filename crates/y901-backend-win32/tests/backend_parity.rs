#![cfg(windows)]

//! 双后端一致性：同一棵树在 **D3D11** 与 **CPU `SoftBackend`** 上，墨迹必须落在同一像素行。
//!
//! 为什么单独一条：`GlyphInfo::bearing_y` 曾经在两个后端有不同语义（一个 y-up 底边、
//! 一个顶部下沉），结果是"软渲染与 GPU 渲染错一行"。这类 bug 的 MAD 可能仍然很小
//! （整体平移对平均值不敏感），所以这里除了 MAD 还**逐边断言墨迹包围盒**。
//!
//! 场景刻意用**混合字号**（默认 16 + 显式 18 + 显式 13）：这正是当年错行的现场——
//! 不同字号的 `ascent` 小数部分不同，基线取整若各自为政，行与行之间就会错开。
//!
//! 需要真实 GPU ⇒ `#[ignore]`；由 `scripts/hw-smoke.ps1` 与 `--include-ignored` 覆盖。

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, UiTree, Widget,
};

const W: u32 = 320;
const H: u32 = 160;

/// 三行文字，字号 16（默认）/ 18 / 13。
struct MixedText {
    rect: Rect,
}

impl Widget for MixedText {
    fn id(&self) -> &str {
        "mixed-text"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
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
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::rgba(0.05, 0.05, 0.07, 1.0));
        let fg = Color::rgba(0.95, 0.96, 1.0, 1.0);
        // 三行都用**同一段文字**，只有字号不同：字号差异是唯一变量。
        let base = TextStyle::fg(fg);
        ctx.draw_text(Pos::new(16.0, 16.0), "Hamburgefonstiv 123", &base);
        ctx.draw_text(
            Pos::new(16.0, 60.0),
            "Hamburgefonstiv 123",
            &TextStyle::fg(fg).size(18.0),
        );
        ctx.draw_text(
            Pos::new(16.0, 110.0),
            "Hamburgefonstiv 123",
            &TextStyle::fg(fg).size(13.0),
        );
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 让两个后端用**同一个字形引擎**，否则比的就不是"落点"而是"光栅化器"。
fn force_same_engine() {
    std::env::set_var("Y901_FONT", "self");
    std::env::set_var("Y901_TEST_DPI_SCALE", "1.0");
}

/// D3D11 渲染一帧并回读（BGRA）。
fn render_d3d() -> Option<(Vec<u8>, u32, u32)> {
    force_same_engine();
    let options = Options {
        title: "backend parity".to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).ok()?;
    let mut tree = UiTree::new(Box::new(MixedText { rect: Rect::ZERO }));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree); // 预热
    window.request_snapshot();
    tree.invalidate_all();
    window.render(&mut tree);
    window.take_snapshot()
}

/// CPU 后端渲染一帧。
fn render_soft() -> Option<y901_render::soft::SoftBackend> {
    use y901_render::MyFontEngine;
    force_same_engine();
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    // **必须用 D3D 侧同一个字体文件**：`default_test_font_path()` 指向的是随仓库的测试字体，
    // 与 win32 默认族（NSimSun → simsun.ttc）不是同一个 —— 两者的 hhea ascent 不同，
    // 直接比会得到 3px 的整行偏移，把"字体不同"误判成"后端错行"（踩过一次）。
    let family = y901_runtime::FontConfig::default().family;
    let path = y901_render::resolve_font_path(&family);
    let engine = MyFontEngine::new(
        y901_render::load_font(&path).expect("font"),
        16.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    let mut soft =
        y901_render::soft::SoftBackend::new_headless(W, H, Box::new(engine), Size::new(cw, ch))
            .expect("soft");
    let mut tree = UiTree::new(Box::new(MixedText { rect: Rect::ZERO }));
    tree.layout(Size::new(W as f32, H as f32), &soft);
    soft.clear();
    tree.render(&mut soft);
    Some(soft)
}

/// 墨迹统计：`(x0, y0, x1, y1, 墨量)`。背景色取左上角像素（场景里那里只有清屏色）。
fn ink_stats<F: Fn(u32, u32) -> (u8, u8, u8)>(w: u32, h: u32, rgb: F) -> (u32, u32, u32, u32, u64) {
    let bg = rgb(0, 0);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut mass = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = rgb(x, y);
            let d = (r as i32 - bg.0 as i32).abs()
                + (g as i32 - bg.1 as i32).abs()
                + (b as i32 - bg.2 as i32).abs();
            if d > 12 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                mass += d as u64;
            }
        }
    }
    (x0, y0, x1, y1, mass)
}

/// 逐像素平均绝对差（MAD），两侧都归一到 RGB。
fn mad<F: Fn(u32, u32) -> (u8, u8, u8), G: Fn(u32, u32) -> (u8, u8, u8)>(
    w: u32,
    h: u32,
    a: F,
    b: G,
) -> f64 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (ar, ag, ab) = a(x, y);
            let (br, bg, bb) = b(x, y);
            sum += (ar as i32 - br as i32).unsigned_abs() as u64
                + (ag as i32 - bg as i32).unsigned_abs() as u64
                + (ab as i32 - bb as i32).unsigned_abs() as u64;
            n += 3;
        }
    }
    sum as f64 / (255.0 * n as f64)
}

#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --test backend_parity -- --ignored --nocapture"]
fn d3d_and_soft_agree_on_ink_rows() {
    let Some((px, w, h)) = render_d3d() else {
        eprintln!("SKIP: 没有可用的 D3D11 设备");
        return;
    };
    let soft = render_soft().expect("soft 后端");
    assert_eq!(
        (w, h),
        (W, H),
        "回读尺寸应与 Options 一致（DPI 已固定 1.0）"
    );

    let d3d_rgb = |x: u32, y: u32| -> (u8, u8, u8) {
        let k = ((y * w + x) as usize) * 4;
        (px[k + 2], px[k + 1], px[k])
    };
    let soft_rgb = |x: u32, y: u32| -> (u8, u8, u8) {
        let p = soft.pixel(x, y).expect("soft 像素越界");
        (
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
        )
    };

    // 逐行墨迹带：错行时两个后端的行带会整体错开，直接看得出来
    let bands = |rgb: &dyn Fn(u32, u32) -> (u8, u8, u8)| -> Vec<(u32, u32)> {
        let bg = rgb(0, 0);
        let mut v = Vec::new();
        let mut start: Option<u32> = None;
        for y in 0..h {
            let mut any = false;
            for x in 0..w {
                let (r, g, b) = rgb(x, y);
                let d = (r as i32 - bg.0 as i32).abs()
                    + (g as i32 - bg.1 as i32).abs()
                    + (b as i32 - bg.2 as i32).abs();
                if d > 12 {
                    any = true;
                    break;
                }
            }
            match (any, start) {
                (true, None) => start = Some(y),
                (false, Some(s)) => {
                    v.push((s, y - 1));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            v.push((s, h - 1));
        }
        v
    };
    eprintln!("[parity] D3D 行带 = {:?}", bands(&d3d_rgb));
    eprintln!("[parity] CPU 行带 = {:?}", bands(&soft_rgb));

    let (ax0, ay0, ax1, ay1, amass) = ink_stats(w, h, d3d_rgb);
    let (bx0, by0, bx1, by1, bmass) = ink_stats(w, h, soft_rgb);
    eprintln!("[parity] D3D 墨迹包围盒 = ({ax0},{ay0})..({ax1},{ay1}) 墨量 {amass}");
    eprintln!("[parity] CPU 墨迹包围盒 = ({bx0},{by0})..({bx1},{by1}) 墨量 {bmass}");

    // 1) **逐边对齐**：这是"错一行"的直接判据。允许 1px 的覆盖率取整差。
    let near = |a: u32, b: u32| (a as i64 - b as i32 as i64).abs() <= 1;
    assert!(
        near(ay0, by0) && near(ay1, by1),
        "两个后端的**纵向**墨迹范围不一致：D3D y {ay0}..{ay1} vs CPU y {by0}..{by1}\
         —— 这正是 bearing_y/基线语义再次分叉的症状"
    );
    assert!(
        near(ax0, bx0) && near(ax1, bx1),
        "横向墨迹范围不一致：D3D x {ax0}..{ax1} vs CPU x {bx0}..{bx1}"
    );

    // 2) 墨量只做**数量级**校验。
    //
    // 为什么不能收紧：GPU 侧对覆盖率做了 `pow(cov, 1/2.2)` 的 gamma 校正
    // （见 `ps_main` 与 `docs/design/text-sharpness.md`），同一覆盖率在 D3D 上更亮，
    // 于是"总墨量"天然比 CPU 侧高约 1.5 倍。这是**已知且有意**的差异，
    // 把它写进阈值等于把 gamma 也钉死；而 gamma 本身是待定项。
    let ratio = amass as f64 / bmass.max(1) as f64;
    assert!(
        (0.4..=2.5).contains(&ratio),
        "墨量比 {ratio:.3} 超出 [0.4, 2.5]：D3D {amass} vs CPU {bmass}\
         （差这么远通常不是 gamma，而是字形/尺寸都不一致了）"
    );

    // 3) 逐像素差仍在容差内（不同覆盖率取整路径，不可能为 0）
    let m = mad(w, h, d3d_rgb, soft_rgb);
    eprintln!("[parity] MAD = {m:.4}（含 gamma 差异，故容差偏宽）");
    assert!(m <= 0.25, "两后端 MAD = {m:.4} > 0.25");
}
