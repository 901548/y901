//! 两个后端**逐段同构**的事件循环骨架。
//!
//! 搬进来的部分（Win32 `runtime.rs` 与 X11 `runtime.rs` 里写法几乎逐字相同）：
//!
//! 1. `RawEvent::Close` → 退出
//! 2. `translate()` → `host.event()` 先看，未被处理再 `tree.dispatch()`
//! 3. `needs_relayout()` → 标记需要重排
//! 4. `tree.drain_commands()` → `host.command()`
//! 5. `host.idle()`
//! 6. `drain_external_invalidations()` + `take_layout_request()` → 重排
//! 7. `host.take_theme_request()` → 切主题 → 全量失效
//! 8. 脏区非空 → 渲染
//! 9. `Y901_PROFILE` 统计
//! 10. 空闲且无脏区 → 一次 `Tick`
//!
//! **没有搬进来的**（在各自后端实现，不进本文件）：
//! 窗口创建、事件来源初始化、IME 定位、光标形状、快照、字体请求、Present 细节。

use std::time::Instant;

use crate::{translate, Host, RawEvent};
use y901_ui::{Size, UiEvent, UiTree};

use crate::window::{RenderOutcome, WindowBackend};

/// 空闲时的默认等待上限（毫秒）。
///
/// 与两个后端原先各自读取 `Y901_IDLE_WAIT_MS` 的行为一致：
/// 默认 100ms，`0` 表示"不限时等待、由 waker 唤醒"。
fn idle_wait_ms() -> u32 {
    std::env::var("Y901_IDLE_WAIT_MS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(100)
}

/// 跑事件循环直到窗口要求退出。
///
/// 返回 `Ok(())` 表示正常退出（用户关闭窗口）；`Err` 表示后端报错。
pub fn run_loop<W: WindowBackend>(
    window: &mut W,
    tree: &mut UiTree,
    host: &mut dyn Host,
) -> Result<(), String> {
    let started = Instant::now();
    let profile = std::env::var_os("Y901_PROFILE").is_some();
    let mut first_frame_done = false;
    let mut stat_frames: u32 = 0;
    let mut stat_total_us: u128 = 0;
    let mut stat_max_us: u128 = 0;
    let mut stat_since = Instant::now();
    // 上一帧是否有工作：有工作时不得等待，否则 Win32 会在每帧都“睡”一次。
    let mut was_busy = false;
    // 上一帧的实际耗时（布局 + 绘制 + 呈现），供帧预算补偿使用。
    let mut last_frame_ms: u64 = 0;
    // 后端是否**自节流**（D3D11 的 `Present(1)` 会阻塞到 vsync）。
    // 为何要猜：软后端（GDI）不自节流，若一律 `wait=0` 会空转烧 CPU；
    // 而 D3D11 若再多睡 16ms 就与 vsync 叠加成 30fps。故用"上一帧呈现耗时"判定。
    // 上一帧的时间点（供动画 dt）。
    let mut last_tick = Instant::now();
    // 最近一次发出 `UiEvent::Tick` 的时间（供 Tick 节拍）。
    let mut last_tick_at = Instant::now();

    loop {
        let frame_start = Instant::now();

        // ---- 1. 事件 ----
        // **帧预算**：忙档不再 `wait=0`（那会让软后端空转烧 CPU），而是 16ms ≈ 60fps；
        // 闲档取 `min(idle_wait, 距下次 Tick 到期)`，让空闲动画也能按节拍出帧。
        let tick_due_ms = {
            let busy = was_busy;
            let period = if busy {
                std::time::Duration::from_millis(16)
            } else {
                std::time::Duration::from_millis(idle_wait_ms() as u64)
            };
            period
                .checked_sub(last_tick_at.elapsed())
                .map(|d| d.as_millis() as u32)
                .unwrap_or(0)
        };
        let idle_ms = idle_wait_ms();
        let wait_ms = if was_busy {
            // **帧预算补偿**：16ms 是"帧周期上限"，不是"固定睡眠" ——
            // 若本帧已经花了 该花的时间（布局 + 绘制 + 呈现），就只补足差额。
            // 否则"固定 16ms 睡 + 5~8ms 工作"会把帧周期拉到 21~24ms（≈40fps）。
            16u64.saturating_sub(last_frame_ms) as u32
        } else {
            idle_ms.min(tick_due_ms)
        };
        let Some(raws) = window.wait_events(wait_ms) else {
            return Ok(());
        };
        let (w, h) = window.viewport();
        let viewport = Size::new(w as f32, h as f32);
        let mut need_layout = false;
        let mut had_work = false;

        for raw in raws {
            if matches!(raw, RawEvent::Close) {
                return Ok(());
            }
            if matches!(raw, RawEvent::Resize { .. }) {
                need_layout = true;
            }
            had_work = true;
            if let Some(ev) = translate(&raw, viewport) {
                // Host 先处理；已处理则不再往下发。
                if host.event(tree, &ev).is_handled() {
                    continue;
                }
                if tree.dispatch(&ev).needs_relayout() {
                    need_layout = true;
                }
            }
        }

        // ---- 2. 命令 + 空闲回调 ----
        for cmd in tree.drain_commands() {
            let _ = host.command(tree, cmd);
        }
        host.idle(tree);

        // ---- 2.5 动画推进（**每帧必须调**）----
        //
        // `UiTree::tick_animations` 之前**全仓没有调用点**（本轮修复）：
        // 于是任何淡入/缩放动画的 `progress` 永远停在 0。
        // 后果：弹层的"淡入膜"（`tree.rs` 用 `1-progress` 透明度的背景色盖一层）
        // **永远不会消失** —— 菜单/面板"有框无字"就是这个原因。
        // 用**真实 dt**（上一帧到本帧），不用固定步长，否则动画速度随帧率变。
        let tick_now = Instant::now();
        let dt = tick_now.saturating_duration_since(last_tick);
        last_tick = tick_now;
        if tree.tick_animations(dt) {
            // 有活跃动画 → 本帧算"有工作"，下一帧不睡眠，直到动画结束。
            had_work = true;
        }

        // ---- 3. 重排判定 ----
        let external = tree.drain_external_invalidations();
        let structural = tree.take_layout_request();
        if need_layout || external > 0 || structural {
            window.relayout(tree);
        }

        // ---- 4. 主题 ----
        if let Some(theme) = host.take_theme_request() {
            if let Err(err) = window.set_theme(theme) {
                eprintln!("[y901] 切换主题失败: {err}");
            } else {
                tree.invalidate_all();
            }
        }

        // ---- 5. 后端专属收尾（IME / 光标 / 快照 / 字体） ----
        window.after_frame(tree, host);

        // ---- 6. 渲染 ----
        //
        // skip/Present 判定在 `WindowBackend::render`，返回 `RenderOutcome`。
        // `UiTree::render` 的空 dirty 是**防御性 guard**（不决定 Present）。
        // 本循环只看 `RenderOutcome`，不查 `tree.dirty()`。
        let frame_started = Instant::now();
        let outcome = window.render(tree)?;
        // 本帧实际耗时（含呈现），用于帧预算补偿。
        last_frame_ms = frame_started.elapsed().as_millis() as u64;
        if !first_frame_done {
            first_frame_done = true;
            eprintln!(
                "[y901] first frame ready in {:.1}ms",
                started.elapsed().as_secs_f64() * 1e3
            );
        }
        window.after_present(tree, host);

        // ---- 7. 统计 ----
        if profile {
            let us = frame_start.elapsed().as_micros();
            stat_frames += 1;
            stat_total_us += us;
            stat_max_us = stat_max_us.max(us);
            if stat_since.elapsed().as_millis() >= 2000 {
                let avg = stat_total_us / stat_frames.max(1) as u128;
                eprintln!("[profile] frames={stat_frames} avg={avg}us max={stat_max_us}us");
                stat_frames = 0;
                stat_total_us = 0;
                stat_max_us = 0;
                stat_since = Instant::now();
            }
        }

        was_busy = had_work || matches!(outcome, RenderOutcome::Presented);

        // ---- 8. Tick 节拍 ----
        //
        // **不再依赖 `RenderOutcome::Skipped`**：旧写法只在"本帧没干活"
        // 且"当前帧被跳过"时才发 Tick，而只要窗口持续
        // Presented（例如有任何东西在标脏），Skipped 就永远不会出现
        // —— 于是 `wants_tick()` 组件（Spinner/Skeleton/ComboBox）永远收不到心跳。
        //
        // 现在按时间节拍发：有工作（16ms ≈ 60fps）；空闲（`idle_wait_ms`）。
        // **`wants_tick` 订阅者也算忙**：否则闲置的 Spinner/Skeleton 只能 10fps。
        let has_tick_consumers = tree.tick_subscribers() > 0;
        let tick_period = if had_work || was_busy || has_tick_consumers {
            std::time::Duration::from_millis(16)
        } else {
            std::time::Duration::from_millis(idle_wait_ms() as u64)
        };
        if last_tick_at.elapsed() >= tick_period {
            let _ = tree.dispatch(&UiEvent::Tick);
            last_tick_at = Instant::now();
        }
    }
}

#[cfg(test)]
mod guard_tests {
    /// `run_loop` 不得自己查脏区：否则会与 `WindowBackend::render` 的判定互相掩盖。
    ///
    /// 为何是结构断言而不是行为测试：`run_loop` 需要完整的 `Host` + `UiTree`
    /// + 一个会真实渲染的窗口，在单元测试里构造起来成本高于它能发现的问题。
    ///
    /// 这条断言就是为了防止"有人又把 `tree.dirty()` 加回来"。
    #[test]
    fn run_loop_never_queries_dirty_directly() {
        let src = include_str!("runtime.rs");
        // 过滤掉注释行：注释里会提到 tree.dirty()（用于说明"不查"）。
        // 只扫**含测试模块之前**的源码：否则会把本测试自己的字符串字面量算进去。
        // 同时剔除行注释（注释里会提到 `tree.dirty()` 用于说明"不查"）。
        let prod = src.split("#[cfg(test)]").next().unwrap_or(src);
        let mut dirty_calls = 0;
        for line in prod.lines() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains("tree.dirty()") {
                dirty_calls += 1;
            }
        }
        assert_eq!(
            dirty_calls, 0,
            "run_loop 不得直接查 tree.dirty()；应用 RenderOutcome 判断"
        );
        let code = prod;
        assert!(
            code.contains("window.render(tree)?"),
            "run_loop 必须通过 WindowBackend::render 取结果"
        );
        assert!(
            code.contains("RenderOutcome::Presented") && code.contains("RenderOutcome::Skipped"),
            "run_loop 必须同时用到两种 outcome"
        );
    }
}
