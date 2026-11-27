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
        // **末尾标记块**：刻意排在 10001 个碎块之后，因此它落在"旧容量 10000"
        // 之外。只有扩容成功、超限实例真的进了 GPU，右下角才会是纯红。
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

/// 渲染一帧，**对"设备丢失"做有限重试**。
///
/// 为什么需要：CI 的 windows runner 只有 WARP。扩容实例缓冲（多分配一份
/// 1.9 MB 的 dynamic buffer）在那台机器上会把 WARP 逼出 `0x887A0005`
/// （`DXGI_ERROR_DEVICE_REMOVED`），后端随即重建设备；下一帧通常就好了。
/// 这不是超限修复的缺陷（扩容前后都会丢设备），属环境抖动 —— 但**不能**因此
/// 把断言放宽或静默跳过：这里最多重试 3 次，全部失败才按"环境不可用"留痕跳过。
fn render_tolerant(win: &mut Window, tree: &mut UiTree) -> Result<RenderOutcome, String> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        // **重试前必须重新标脏**：上一轮 `render` 已经消费掉脏区（`UiTree::render`
        // 会清空它），不补标的话重试只会得到 `Skipped`，看起来像"没渲染"。
        if attempt > 0 {
            tree.invalidate_all();
        }
        match WindowBackend::render(win, tree) {
            // 重试时脏区又被清空（理论上不会）：再走一轮，而不是把 Skipped 当成功。
            Ok(RenderOutcome::Skipped) if attempt > 0 => continue,
            Ok(o) => return Ok(o),
            Err(e) => {
                let device_lost = e.contains("0x887A0005")
                    || e.contains("device lost")
                    || e.contains("DEVICE_REMOVED");
                if !device_lost {
                    return Err(e);
                }
                last_err = e;
                std::thread::sleep(std::time::Duration::from_millis(80));
            }
        }
    }
    Err(last_err)
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

/// **超限时全部实例都要画出来**（计数器 + 像素两条证据，同一个窗口内依次做）。
///
/// 为什么合并成一个用例：本仓库约定"同一进程只允许一个 D3D 窗口活着"
///（见 `grid_image_gpu.rs` / `font_3c_pixels.rs`）。两个 GPU 用例默认并行跑会同时
/// 建两个设备/交换链，在 CI 的 WARP 上实测 `STATUS_HEAP_CORRUPTION`；即使加互斥锁，
/// 第二个窗口的创建仍会在 WARP 上卡住（线程间共享 D3D 运行时状态）。
/// 串行地在**同一个窗口**里做两段检查，既满足约定也不牺牲覆盖面。
///
/// **默认 `#[ignore]`**：CI 的 windows runner 没有 GPU，`Window::create` 直接失败，
/// 本用例只会 SKIP；真正有意义的执行需要一台有 D3D11 设备的机器
/// （`cargo test -p y901-backend-win32 --test instance_overflow -- --ignored`）。
///
/// **2026-11-27 更新**：WARP 上原本会 `STATUS_HEAP_CORRUPTION`（0xc0000374）的那个
/// 崩溃已经定位并修好 —— 根因不是 WARP，是 `rebuild()` 之后"容量记在 `Gfx`、
/// 缓冲在 `Pipeline`"两者不同步，`copy_nonoverlapping` 写过了映射区尾部。
/// 现在容量与缓冲由 `Pipeline` 独占（见 `d3d11.rs` 里 `Pipeline::instance_capacity`
/// 与 `device_object_lifetime_tests`）。WARP 上复跑不再崩溃，只是因为本机 WARP 的
/// `Present` 本身就返回 `0x887A0005`（100 个实例也一样），所以这里仍会走 SKIP。
#[test]
#[ignore = "需要可用的 D3D11 设备（CI 无 GPU）；本地 --ignored 跑"]
fn instance_overflow_draws_everything_and_paints_the_tail() {
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    std::env::set_var("Y901_RENDER_STATS", "1");
    std::env::remove_var("Y901_FORCE_SOFT");
    let Some(mut win) = window("y901-instance-overflow") else {
        skip("无 D3D11 设备（Window::create 失败）");
        return;
    };
    assert_eq!(
        win.render_stats_backend(),
        Some(BackendKind::D3d11),
        "前置：必须真的走 D3D11（守统计能力表）"
    );

    // ---- 第一段：计数器证明"推入多少就画多少" ----
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
    let (pushed, drawn) = (
        stats.counters.instance_pushed,
        stats.counters.instance_drawn,
    );
    assert!(
        pushed > INSTANCE_CAPACITY as u32,
        "前置：本帧实例数应超过默认容量 {INSTANCE_CAPACITY}，实测 {pushed}"
    );
    assert_eq!(
        drawn, pushed,
        "超限时不得丢实例：推入 {pushed} 个，只画了 {drawn} 个（issue #7 的原始症状）"
    );
    assert!(stats.stages.get(StageId::BackendSubmit).is_some());
    assert_eq!(stats.validate(), Ok(()));

    // ---- 第二段：像素证明"超限那部分真的进了 GPU" ----
    // 标记块被故意排在实例流末尾（10001 个碎块之后），只有扩容成功才看得见。
    tree = UiTree::new(Box::new(Flood {
        id: "flood2",
        rect: Rect::ZERO,
    }));
    tree.invalidate_all();
    win.resize(&mut tree);
    win.request_snapshot();
    if let Err(e) = render_tolerant(&mut win, &mut tree) {
        skip(&format!("第二段渲染失败（设备不可用）：{e}"));
        return;
    };
    // 第二段同样要求不丢实例（扩容后容量已够，不该再扩容）。
    let stats2 = win.take_render_stats().expect("第二段统计");
    assert_eq!(
        stats2.counters.instance_drawn, stats2.counters.instance_pushed,
        "第二段同样不得丢实例"
    );

    let Some((data, w, h)) = win.take_snapshot() else {
        skip("快照失败（无可用 D3D11 设备）");
        return;
    };
    let px = |x: u32, y: u32| -> (u8, u8, u8) {
        let i = ((y * w + x) * 4) as usize;
        (data[i + 2], data[i + 1], data[i])
    };
    let (r, g, b) = px(w - 4, h - 4);
    assert!(
        r > 200 && g < 40 && b < 40,
        "右下角应为标记块的红色，实际 ({r},{g},{b})（{w}x{h}，说明超限实例没画出来）"
    );
}
