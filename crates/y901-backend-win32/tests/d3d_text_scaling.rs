#![cfg(windows)]

//! D3D11 文本缩放的像素断言（三条）。
//!
//! 1. `s = 1.0` 与"不压栈"逐像素一致（含**变异对照**）；
//! 2. `s = 0.5` 的**墨量比**与**像素个数比**都落在区间内（双口径，与 CPU 侧同定义）；
//! 3. 同一场景下 D3D11 与 SoftBackend 的 MAD ≤ 0.05。
//!
//! 背景色的取法：统一用常量 `BG`（与场景里 `fill_rect` 的颜色一致）。
//! 早期版本在 CPU 对照里硬编码了一个**不匹配**的背景色，算出 0.745 这种不可能值 ——
//! 那是"检查代码的 bug"，不是渲染问题；现在两个口径共用同一份 `stats()`。

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, UiTree, Widget,
};

const W: u32 = 240;
const H: u32 = 120;
const ANCHOR: (f32, f32) = (20.0, 30.0);
const TEXT: &str = "A";
/// 背景色：`0.05 × 255 = 12.75 → 13`，`0.07 × 255 = 17.85 → 18`。
const BG: (u8, u8, u8) = (13, 13, 18);

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
        let r = self.rect;
        // 纯色背景：墨量与像素个数都需要一个确定基准
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
fn render_d3d(scale: Option<f32>) -> Option<(Vec<u8>, u32, u32)> {
    let options = Options {
        title: "d3d text scaling probe".to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,

        fullscreen: false,
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

/// 同一场景在 SoftBackend 上渲染。
fn render_soft(scale: Option<f32>) -> Option<y901_render::soft::SoftBackend> {
    use y901_render::MyFontEngine;
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    let mut soft =
        y901_render::soft::SoftBackend::new_headless(W, H, Box::new(engine), Size::new(cw, ch))
            .expect("soft");
    let mut tree = UiTree::new(Box::new(GlyphScene::new(scale)));
    tree.layout(Size::new(W as f32, H as f32), &soft);
    soft.clear();
    tree.render(&mut soft);
    Some(soft)
}

/// 双口径统计：`(墨量, 非背景像素个数)`。`rgb(x, y)` 返回该点 RGB。
fn stats<F: Fn(u32, u32) -> (u8, u8, u8)>(w: u32, h: u32, rgb: F) -> (u64, u64) {
    let mut mass = 0u64;
    let mut count = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = rgb(x, y);
            let d = (r as i32 - BG.0 as i32).unsigned_abs()
                + (g as i32 - BG.1 as i32).unsigned_abs()
                + (b as i32 - BG.2 as i32).unsigned_abs();
            mass += d as u64;
            if d > 0 {
                count += 1;
            }
        }
    }
    (mass, count)
}

fn stats_d3d(px: &[u8], w: u32, h: u32) -> (u64, u64) {
    stats(w, h, |x, y| {
        let k = ((y * w + x) as usize) * 4;
        (px[k + 2], px[k + 1], px[k])
    })
}

fn stats_soft(soft: &y901_render::soft::SoftBackend, w: u32, h: u32) -> (u64, u64) {
    stats(w, h, |x, y| {
        let p = soft.pixel(x, y).expect("px");
        (
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
        )
    })
}

/// **把 DPI 固定为 1.0**。
///
/// 这些用例断言的是"像素数 / 墨量比 / 双后端 MAD"，全部依赖**客户区尺寸 == W×H**。
/// `Options.width/height` 自 `docs/design/dpi-policy.md` 起是 **DIP**：
/// 150% 缩放下 240×120 会变成 360×180 的客户区，而 CPU 对照用的 `SoftBackend`
/// 是按 `W×H` 建的 —— 尺寸对不上，`soft.pixel(x, y)` 直接返回 `None`（曾经的报错就在这里）。
///
/// 文本缩放的**比例**与 DPI 无关，固定 1.0 既保持断言含义，又让结果可复现。
fn force_dpi_one() {
    std::env::set_var("Y901_TEST_DPI_SCALE", "1.0");
}

/// 串行化所有 D3D 探针测试。
///
/// 为什么需要：每个测试都 `Window::create` 一个真实窗口；并行时会争抢 D3D 设备与前台窗口，
/// 实测出现过一次假红（单独跑全绿）。
static D3D_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 记录一次 "skip"。
///
/// 为什么不用 `println!`：`cargo test` 默认**捕获** stdout，门禁跑不带 `--nocapture`，
/// 所以那行字根本到不了门禁眼前。写一个标记文件可以跨进程、不依赖 stdout 抓取。
fn record_skip(reason: &str) {
    eprintln!("SKIP: {reason}");
    let path = skip_log_path();
    // 开写失败必须 panic：否则 gate 看不到 SKIP，会把"没跑"当成"没有 skip"。
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("SKIP log write failed: {e}"));
    {
        use std::io::Write as _;
        writeln!(f, "SKIP: {reason}")
            .expect("SKIP log write failed -- gate will not see this skip");
    }
}

/// skip 记录文件路径（门禁会先删除它、跑完再数行数）。
fn skip_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("y901-test-skips.txt")
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
fn d3d_scale_one_is_pixel_identical_and_identity_is_observable() {
    force_dpi_one();
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let (base, w, h) = match render_d3d(None) {
        Some(v) => v,
        None => {
            record_skip("d3d_text_scaling: no D3D11 device");
            return;
        }
    };
    let (one, w1, h1) = match render_d3d(Some(1.0)) {
        Some(v) => v,
        None => {
            record_skip("d3d_text_scaling: no D3D11 device");
            return;
        }
    };
    assert_eq!((w, h), (w1, h1), "两次回读尺寸应一致");

    let diff = base.iter().zip(one.iter()).filter(|(a, b)| a != b).count();
    eprintln!(
        "[d3d-scale] s = 1.0 与不压栈：{diff} 字节不同（hash {:#x} vs {:#x}）",
        fnv(&base),
        fnv(&one)
    );
    assert_eq!(diff, 0, "s = 1.0 必须等同于不压栈（变换栈恒等）");

    // 变异对照：改一个像素后哈希必须不同，否则"逐像素比较"没有区分力
    let mut mutated = base.clone();
    mutated[0] ^= 0xFF;
    assert_ne!(
        fnv(&base),
        fnv(&mutated),
        "改一个像素后哈希应变化（说明比较有区分力）"
    );
}

#[test]
fn d3d_half_scale_ink_ratios_are_sane() {
    force_dpi_one();
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let (full, w, h) = match render_d3d(None) {
        Some(v) => v,
        None => {
            record_skip("d3d_text_scaling: no D3D11 device");
            return;
        }
    };
    let (half, _, _) = match render_d3d(Some(0.5)) {
        Some(v) => v,
        None => {
            record_skip("d3d_text_scaling: no D3D11 device");
            return;
        }
    };
    let (m_full, c_full) = stats_d3d(&full, w, h);
    let (m_half, c_half) = stats_d3d(&half, w, h);
    eprintln!("[d3d-scale] 回读尺寸 {w}x{h}");
    let mass_ratio = m_half as f64 / m_full.max(1) as f64;
    let count_ratio = c_half as f64 / c_full.max(1) as f64;

    // 同一场景在 CPU 上的两个口径：判断"比值偏高"是字形特性还是 D3D 特有
    if let (Some(a), Some(b)) = (render_soft(None), render_soft(Some(0.5))) {
        let (am, ac) = stats_soft(&a, w, h);
        let (bm, bc) = stats_soft(&b, w, h);
        eprintln!(
            "[d3d-scale] 同场景 CPU：墨量比 {:.3}，像素个数比 {:.3}",
            bm as f64 / am.max(1) as f64,
            bc as f64 / ac.max(1) as f64
        );
    }

    eprintln!(
        "[d3d-scale] D3D  s=0.5：墨量比 {mass_ratio:.3}（{m_half}/{m_full}），像素个数比 {count_ratio:.3}（{c_half}/{c_full}）"
    );
    // 口径说明：GPU 侧对覆盖率做了 `pow(cov, 1/2.2)` 的 gamma 校正
    // （见 `ps_main` 与 `docs/design/text-sharpness.md`），同一覆盖率在 D3D 上更亮，
    // 于是"墨量/像素个数"这类**按亮度阈值统计**的比值天然比 CPU 侧高（实测约 1.8×）。
    // 区间因此按"物理上应当缩小"而非"恰好 1/4"来定：只需落在同数量级、
    // 且明显小于 1（缩小后必然更少）。精确的 1:1 契约由 `backend_parity` 用像素对齐守护。
    assert!(
        (0.12..=0.45).contains(&mass_ratio),
        "墨量比 {mass_ratio:.3} 超出 0.12..=0.45（缩小应落在四分之一量级）"
    );
    assert!(
        (0.15..=0.50).contains(&count_ratio),
        "像素个数比 {count_ratio:.3} 超出 0.15..=0.50"
    );
    assert!(m_half < m_full && c_half < c_full, "缩小后两个口径都应更少");
}

#[test]
fn d3d_matches_soft_backend_within_tolerance() {
    force_dpi_one();
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let (d3d, w, h) = match render_d3d(Some(0.5)) {
        Some(v) => v,
        None => {
            record_skip("d3d_text_scaling: no D3D11 device");
            return;
        }
    };
    let soft = match render_soft(Some(0.5)) {
        Some(s) => s,
        None => {
            record_skip("d3d_text_scaling: no D3D11 device");
            return;
        }
    };

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
    assert!(mad <= 0.05, "两后端 MAD = {mad:.4} > 0.05");
}
