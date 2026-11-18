#![cfg(windows)]

//! 3c 最终交付证据：`--font=self` 下的像素统计 + PNG 基准。
//!
//! 山不能拈我们就拍屏幕（无法抓真实窗口），所以用**回读像素 + 统计 + PNG**：
//! 数字可以做回归基准，比截图更硬。
//!
//! **2026-11-12 扩展**：场景末尾新增一段**框架控件布局**（`Flex` + 三个
//! `TextLabel`，字号 16/18/13，`align::Center`），让"布局 + per-text 字号"
//! 也进入像素基线 —— 此前门禁只守字形光栅化。

use std::time::Instant;

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Align, Color, Constraints, EventCtx, EventResult, Flex, Pos, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};
use y901_widgets::TextLabel;

/// 门禁场景的**布局段**：一个横向 `Flex`，三个不同字号的标签。
///
/// 为什么是这三个字号：16 = 引擎默认（`TextStyle.size = None` 路径），
/// 18 与 13 = 显式 `Some(px)` 路径 —— 三者同时出现，能同时守住
/// "默认字号不变" 与 "per-text 字号差异可见"。
///
/// 为什么用 `align::Center`：交叉轴对齐会改变子项的 **y 偏移**，
/// 让"布局是否真的生效"变成**像素可见**的差异（而非仅尺寸差异）。
fn gate_bar() -> Flex {
    Flex::row("gate-bar")
        .align(Align::Center)
        .gap(6.0)
        .push(Box::new(TextLabel::new("gate-t16", "M3 字号 16")))
        .push(Box::new(TextLabel::new("gate-t18", "T18").size(18.0)))
        .push(Box::new(TextLabel::new("gate-t13", "s13").size(13.0)))
}

/// 一个固定场景：中英文混排文本 + 3 个 SDF 圆角按钮 + 一条渐变。
///
/// 末尾另有一段**框架控件布局**（`bar`），见 [`gate_bar`]。
struct Scene {
    rect: Rect,
    /// 布局段（`Flex` 子树）。`render` 里逐个绘制其子节点。
    bar: Flex,
}

impl Widget for Scene {
    fn id(&self) -> &str {
        "scene"
    }
    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 让布局段参与度量（返回值不变 —— 场景始终占满）。
        let _ = self.bar.measure(m, c, cache);
        c.constrain(Size::new(c.max.w, c.max.h))
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        // 布局段放在下方空白带（原绘制占 y+0..140，背景条到 y+140）。
        let bar_rect = Rect::new(r.x + 12.0, r.y + 146.0, (r.w - 24.0).max(0.0), 40.0);
        self.bar.arrange(bar_rect, m, cache);
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let r = self.rect;
        // 渐变顶栏
        ctx.fill_gradient_rect_dir(
            Rect::new(r.x, r.y, r.w, 40.0),
            Color::rgba(0.06, 0.08, 0.14, 1.0),
            Color::rgba(0.20, 0.32, 0.60, 1.0),
            (0.0, 1.0),
        );
        // 中英文混排
        let white = TextStyle::fg(Color::rgba(0.95, 0.96, 1.0, 1.0));
        ctx.draw_text(
            Pos::new(r.x + 12.0, r.y + 8.0),
            "Y901 字体引擎 M3 ready",
            &white,
        );
        let dim = TextStyle::fg(Color::rgba(0.62, 0.66, 0.74, 1.0));
        ctx.draw_text(
            Pos::new(r.x + 12.0, r.y + 48.0),
            "自研 TTF：轮廓展平 + 扫描线 + 面积累积 + 4 相位",
            &dim,
        );
        // 3 个 SDF 圆角按钮
        for (i, col) in [
            Color::rgba(0.24, 0.55, 0.95, 1.0),
            Color::rgba(0.20, 0.72, 0.45, 1.0),
            Color::rgba(0.92, 0.44, 0.18, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            ctx.fill_round_rect(
                Rect::new(r.x + 12.0 + i as f32 * 108.0, r.y + 76.0, 96.0, 30.0),
                col,
                8.0,
            );
        }
        // 一律背景
        ctx.fill_rect(
            Rect::new(r.x, r.y + 120.0, r.w, 20.0),
            Color::rgba(0.10, 0.10, 0.12, 1.0),
        );
        // **布局段**：`Flex::render` 只画自身背景（默认 None），子节点需逐个绘制。
        if let Some(kids) = self.bar.children_mut() {
            for kid in kids.iter_mut() {
                kid.render(ctx);
            }
        }
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 环境变量与**进程全局**状态一样：`cargo test` 默认并发跑用例，
/// 两个测试同时 `set_var` 同一个键会互相覆盖（一个正在用，另一个已删掉）。
/// 用锁 + **恢复原值**，而不是直接 remove。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 在锁保护下设置环境变量，并在 drop 时恢复原值（含"原本不存在"这种情况）。
struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            _lock: lock,
            key,
            prev,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.prev.take() {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn crc32(data: &[u8]) -> u32 {
    let mut c: u32 = 0xFFFF_FFFF;
    for b in data {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for x in data {
        a = (a + *x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// 写**无压缩** PNG（零依赖：zlib 用 stored blocks）。
///
/// 为什么不引入 png 包：一个基准图像就为它加一个依赖不值得；
/// stored block 只多几十行，输出也能被任意看图工具打开。
fn write_png(path: &str, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let chunk = |out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let mut crc_in = Vec::with_capacity(4 + data.len());
        crc_in.extend_from_slice(tag);
        crc_in.extend_from_slice(data);
        out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
    };

    // IHDR：宽、高、位深 8、真彩色 RGBA、无压缩、不隔行
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);

    // 构造扫描行（每行前一个 filter=0 字节）
    // 注意：D3D 回读是 **BGRA**，PNG 要的是 **RGBA**，必须交换 R/B。
    // （我第一版直接兆写，图片颜色全错了——拿 System.Drawing 抽样才发现）。
    let mut raw = Vec::with_capacity(((w * 4 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0u8);
        let row = (y * w * 4) as usize;
        for x in 0..w as usize {
            let i = row + x * 4;
            raw.push(rgba[i + 2]); // R ← 原 B
            raw.push(rgba[i + 1]); // G
            raw.push(rgba[i]); // B ← 原 R
            raw.push(rgba[i + 3]); // A
        }
    }

    // zlib：2 字节头 + stored blocks + adler32
    let mut z = vec![0x78, 0x01];
    let mut pos = 0usize;
    while pos < raw.len() {
        let n = ((raw.len() - pos).min(65535)) as u16;
        z.push(if pos + n as usize >= raw.len() { 1 } else { 0 });
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(&raw[pos..pos + n as usize]);
        pos += n as usize;
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);

    std::fs::write(path, &out).map_err(|e| format!("{e}"))
}

#[test]
#[ignore = "需要 GPU（回读像素）"]
fn pixel_statistics_and_png_baseline() {
    // 强制自研引擎（需在创建窗口前设）
    let _env = EnvGuard::set("Y901_FONT", "self");

    let options = Options {
        title: "y901 3c evidence".to_string(),
        width: 400,
        height: 220,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let t0 = Instant::now();
    // 注意：`Options.width/height` 是**窗口尺寸**（含边框与标题栏），
    // 而下面回读的 `w`/`h` 是 `GetClientRect` 的**客户区**尺寸。
    // 实测：400x220 的窗口 → 378x164 的客户区（水平边框约 22px、垂直约 56px）。
    // 边框尺寸随 DPI / 主题 / DWM 状态变化，所以像素哈希**不能当跨会话常量**：
    // 项目早期观测到 382x173 / 0xF38B9C1AB8A877E2，本轮观测到 378x164 / 0xF4C92C36969FEC48。
    // 门禁脚本 `scripts/hash-gate.ps1` 改为"同会话前后对比 + 记录客户区尺寸"，不再硬编码常量。
    let mut window = Window::create(&options).expect("需要 D3D11");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        bar: gate_bar(),
    }));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.request_snapshot();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");
    let first_frame = t0.elapsed().as_secs_f64() * 1e3;

    // 统计
    let total = (w * h) as usize;
    let mut non_bg = 0usize;
    let mut bright = 0usize;
    let mut colors = std::collections::HashSet::new();
    // 背景取右下角（场景里那里只有清屏色）
    let bg_i = (((h - 2) * w + 2) * 4) as usize;
    let bg = [px[bg_i], px[bg_i + 1], px[bg_i + 2]];
    let mut min_l = 255i32;
    let mut max_l = 0i32;
    for i in (0..(w * h) as usize).step_by(4) {
        let (b, g, r) = (px[i * 4] as i32, px[i * 4 + 1] as i32, px[i * 4 + 2] as i32);
        if (b - bg[0] as i32).abs() + (g - bg[1] as i32).abs() + (r - bg[2] as i32).abs() > 12 {
            non_bg += 1;
        }
        colors.insert((px[i * 4], px[i * 4 + 1], px[i * 4 + 2]));
        let luma = (r * 299 + g * 587 + b * 114) / 1000;
        if luma > 200 {
            bright += 1;
        }
        min_l = min_l.min(luma);
        max_l = max_l.max(luma);
    }
    let hash = fnv(&px);
    // 用绝对路径：测试的 cwd 是 crate 目录（crates/y901-backend-win32），不是仓库根
    let png_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/evidence-self-font.png"
    );
    write_png(png_path, &px, w, h).expect("写 PNG 失败");

    eprintln!("[3c] 场景 {w}x{h}，首帧 {first_frame:.1}ms");
    eprintln!(
        "[3c] 非背景像素 {non_bg} / {total} = {:.1}%",
        100.0 * non_bg as f64 / total as f64
    );
    eprintln!("[3c] 独立颜色数 {} 种（渐变/抗锯证据）", colors.len());
    eprintln!(
        "[3c] 亮像素（luma>200）{bright} 个 = {:.2}%（文字抗锯）",
        100.0 * bright as f64 / total as f64
    );
    eprintln!("[3c] 亮度范围 {min_l}..{max_l}");
    eprintln!("[3c] 像素 FNV 哈希 = 0x{hash:016X}");
    eprintln!(
        "[3c] PNG 已保存：{png_path}（{} 字节）",
        std::fs::metadata(png_path).map(|m| m.len()).unwrap_or(0)
    );

    assert!(
        non_bg as f64 / total as f64 > 0.05,
        "非背景像素过少，可能没画出来"
    );
    assert!(
        colors.len() > 50,
        "独立颜色太少：{}（渐变可能未生效）",
        colors.len()
    );
    assert!(bright > 0, "没有亮像素：文字可能没画出来");
}

/// 诊断：回读缓冲是不是上下翻转的？通过"哪一行最亮"判断文字在上还是在下。
#[test]
#[ignore = "调试"]
fn probe_orientation_and_channels() {
    let _env = EnvGuard::set("Y901_FONT", "self");
    let options = Options {
        title: "probe".to_string(),
        width: 400,
        height: 220,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("d3d");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        bar: gate_bar(),
    }));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.request_snapshot();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("snap");

    // 每行的亮像素数（luma>180）—— 文字行会突出
    let mut rows: Vec<(u32, usize)> = Vec::new();
    for y in 0..h {
        let mut n = 0;
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let (b, g, r) = (px[i] as i32, px[i + 1] as i32, px[i + 2] as i32);
            let luma = (r * 299 + g * 587 + b * 114) / 1000;
            if luma > 180 {
                n += 1;
            }
        }
        if n > 0 {
            rows.push((y, n));
        }
    }
    eprintln!(
        "[probe] 有亮像素的行（前 8）: {:?}",
        &rows[..rows.len().min(8)]
    );
    eprintln!(
        "[probe] 有亮像素的行（后 8）: {:?}",
        &rows[rows.len().saturating_sub(8)..]
    );

    // 顶栏渐变：第 3 行 vs 第 30 行 的 B 分量
    for y in [3u32, 20, 35] {
        let i = ((y * w + 5) * 4) as usize;
        eprintln!(
            "[probe] y={y} x=5 原始字节 = [{}, {}, {}, {}]",
            px[i],
            px[i + 1],
            px[i + 2],
            px[i + 3]
        );
    }
}
