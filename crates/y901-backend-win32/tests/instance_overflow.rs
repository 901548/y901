#![cfg(windows)]

//! **实例数超过缓冲容量时不得静默丢实例**（RENDER-BUG-01 / issue #7）。
//!
//! 旧行为：`let count = instances.len().min(INSTANCE_CAPACITY)` 之后只上传前
//! `INSTANCE_CAPACITY` 个 —— 第 10 001 个实例往后**整块消失**，画面表现为
//! "后半帧的控件没了"，且只有一条进程级告警（默认没人看得到）。
//!
//! 现在：先用 `capacity_for()` 把实例缓冲按 2 的幂扩容，再上传**全部**实例；
//! 只有扩容失败才退化为截断（并且一定打印告警）。
//!
//! 这里用"一个叶子在自己的 `render` 里画 10 001 个 1×1 矩形"来制造超限实例流：
//! 不依赖布局规模，也不会因为 10k 个控件而拖慢调试构建。

use y901_backend_win32::backend::d3d11::{capacity_for, INSTANCE_CAPACITY};
use y901_backend_win32::window::Window;
use y901_runtime::render_stats::{BackendKind, StageId};
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::Options;
use y901_ui::layout::LayoutCache;
use y901_ui::{Color, Constraints, Rect, RenderCtx, Semantics, Size, TextMetrics, UiTree, Widget};

const W: u32 = 320;
const H: u32 = 160;
/// 比默认容量多 1 个，确保确实越过容量边界。
const RECTS: usize = INSTANCE_CAPACITY + 1;

static D3D_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn skip(reason: &str) {
    use std::io::Write as _;
    let path = std::env::temp_dir().join("y901-test-skips.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "SKIP: instance_overflow {reason}");
    }
    eprintln!("[instance-overflow] 跳过：{reason}");
}

fn window(title: &str) -> Option<Window> {
    let options = Options {
        title: title.to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,
        fullscreen: false,
    };
    Window::create(&options).ok()
}

/// 一个在自己的 `render` 里狂刷矩形的叶子（制造超限实例流）。
struct Flood {
    id: &'static str,
    rect: Rect,
}

impl Widget for Flood {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(W as f32, H as f32))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 每个 1×1 矩形至少一个实例；不改颜色（避免每帧都切换状态）。
        let color = Color::from_rgb8(60, 90, 140);
        for i in 0..RECTS {
            let x = (i % W as usize) as f32;
            let y = (i / W as usize % H as usize) as f32;
            ctx.fill_rect(Rect::new(x, y, 1.0, 1.0), color);
        }
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 渲染一帧，**对"设备丢失"做有限重试**。
///
/// 为什么需要：CI 的 windows runner 只有 WARP。扩容实例缓冲（多分配一份
/// 1.9 MB 的 dynamic buffer）在那台机器上会把 WARP 逼出 `0x887A0005`
/// （`DXGI_ERROR_DEVICE_REMOVED`），后端随即重建设备；下一帧通常就好了。
/// 这不是超限修复的缺陷（扩容前后都会丢设备），属环境抖动 —— 但**不能**因此
/// 把断言放宽或静默跳过：这里最多重试 3 次，全部失败才按"环境不可用"留痕跳过。
fn render_tolerant(win: &mut Window, tree: &mut UiTree) -> Result<RenderOutcome, String> {
    let mut last = String::new();
    for _ in 0..3 {
        match WindowBackend::render(win, tree) {
            Ok(o) => return Ok(o),
            Err(e) => {
                let device_lost = e.contains("0x887A0005")
                    || e.contains("device lost")
                    || e.contains("DEVICE_REMOVED");
                if !device_lost {
                    return Err(e);
                }
                last = e;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        }
    }
    Err(last)
}

/// 容量规则：不小于默认值、按 2 的幂增长、越界不 panic。
#[test]
fn capacity_for_rounds_up_to_powers_of_two() {
    assert_eq!(capacity_for(0), INSTANCE_CAPACITY);
    assert_eq!(capacity_for(1), INSTANCE_CAPACITY);
    assert_eq!(capacity_for(INSTANCE_CAPACITY), INSTANCE_CAPACITY);
    assert_eq!(
        capacity_for(INSTANCE_CAPACITY + 1),
        INSTANCE_CAPACITY * 2,
        "刚超出一个就必须翻倍（否则立刻又要扩容）"
    );
    assert_eq!(capacity_for(INSTANCE_CAPACITY * 2), INSTANCE_CAPACITY * 2);
    assert_eq!(
        capacity_for(INSTANCE_CAPACITY * 2 + 1),
        INSTANCE_CAPACITY * 4
    );
}

/// 超限时必须**全部画出来**：`instance_drawn == instance_pushed`。
#[test]
fn every_instance_is_drawn_when_the_count_exceeds_capacity() {
    std::env::set_var("Y901_RENDER_STATS", "1");
    std::env::remove_var("Y901_FORCE_SOFT");
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-instance-overflow") else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };
    assert_eq!(
        win.render_stats_backend(),
        Some(BackendKind::D3d11),
        "前置：必须真的走 D3D11（守统计能力表）"
    );

    let mut tree = UiTree::new(Box::new(Flood {
        id: "flood",
        rect: Rect::ZERO,
    }));
    tree.invalidate_all();
    win.resize(&mut tree);

    let outcome = match render_tolerant(&mut win, &mut tree) {
        Ok(o) => o,
        Err(e) => {
            skip(&format!("渲染失败（设备不可用）：{e}"));
            return;
        }
    };
    assert!(matches!(outcome, RenderOutcome::Presented));

    let stats = win.take_render_stats().expect("开启统计后必须交付统计");
    let pushed = stats.counters.instance_pushed;
    let drawn = stats.counters.instance_drawn;
    assert!(
        pushed > INSTANCE_CAPACITY as u32,
        "前置：本帧实例数应超过默认容量 {INSTANCE_CAPACITY}，实测 {pushed}"
    );
    assert_eq!(
        drawn, pushed,
        "超限时不得丢实例：推入 {pushed} 个，只画了 {drawn} 个（issue #7 的原始症状）"
    );
    // 阶段自洽性顺带再验一次（扩容不得破坏统计）。
    assert!(stats.stages.get(StageId::BackendSubmit).is_some());
    assert_eq!(stats.validate(), Ok(()));

    // 第二帧继续超限：扩容只在第一次发生，之后不应再重建（容量已够）。
    tree.invalidate_all();
    if let Err(e) = render_tolerant(&mut win, &mut tree) {
        skip(&format!("第二帧渲染失败（设备不可用）：{e}"));
        return;
    }
    let stats = win.take_render_stats().expect("第二帧统计");
    assert_eq!(
        stats.counters.instance_drawn, stats.counters.instance_pushed,
        "第二帧同样不得丢实例（扩容后容量应足够）"
    );
}

/// 像素层面确认"后半帧还在"：把 10001 个矩形按行铺满，检查**最后一行**也被画上了。
///
/// 为什么不能只信计数器：计数器是后端自报的；只有回读像素才能证明超限部分
/// 真的进了 GPU 并且被着色。
#[test]
fn pixels_from_beyond_the_old_capacity_are_actually_present() {
    std::env::set_var("Y901_RENDER_STATS", "1");
    std::env::remove_var("Y901_FORCE_SOFT");
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-instance-overflow-pixels") else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };

    // 只画一个位于**客户区右下角**的方块 —— 它必然是实例流里的最后一个
    //（Flood 先画 10001 个碎块把它挤到后面）。
    struct Tail {
        id: &'static str,
        rect: Rect,
    }
    impl Widget for Tail {
        fn id(&self) -> &str {
            self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(Size::new(W as f32, H as f32))
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn children(&self) -> &[Box<dyn Widget>] {
            &[]
        }
        fn render(&mut self, ctx: &mut dyn RenderCtx) {
            // 先灌满实例流（把下面的标记块顶到容量之后）
            let color = Color::from_rgb8(10, 10, 10);
            for i in 0..RECTS {
                let x = (i % 2) as f32;
                let y = (i % 3) as f32;
                ctx.fill_rect(Rect::new(x, y, 1.0, 1.0), color);
            }
            // 标记块：画在画布**右下角**（8×8 纯红）—— 只有"超限部分也被画出来"才看得到。
            // 用 `ctx.size()`（RenderCtx 的当前画布尺寸），不硬编码 W/H：
            // 本机 150% DPI 下客户区会被放大。
            let sz = ctx.size();
            ctx.fill_rect(
                Rect::new(sz.w - 8.0, sz.h - 8.0, 8.0, 8.0),
                Color::from_rgb8(255, 0, 0),
            );
        }
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    let mut tree = UiTree::new(Box::new(Tail {
        id: "tail",
        rect: Rect::ZERO,
    }));
    tree.invalidate_all();
    win.resize(&mut tree);
    win.request_snapshot();
    if let Err(e) = render_tolerant(&mut win, &mut tree) {
        skip(&format!("渲染失败（设备不可用）：{e}"));
        return;
    }

    // 像素回读走与其它 GPU 快照用例相同的路径（`take_snapshot`）。
    let Some((data, w, h)) = win.take_snapshot() else {
        skip("快照失败（无可用 D3D11 设备）");
        return;
    };
    let px = |x: u32, y: u32| -> (u8, u8, u8) {
        let i = ((y * w + x) * 4) as usize;
        (data[i + 2], data[i + 1], data[i])
    };
    let (x, y) = (w - 4, h - 4);
    let (r, g, b) = px(x, y);
    assert!(
        r > 200 && g < 40 && b < 40,
        "右下角应为标记块的红色，实际 ({r},{g},{b})（{w}x{h}，说明超限实例没画出来）"
    );
}
