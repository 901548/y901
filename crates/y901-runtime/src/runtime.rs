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

#[path = "profile.rs"]
mod profile;

use web_time::Instant;

use crate::{translate, Host, RawEvent};
use y901_ui::{Size, UiEvent, UiTree};

use crate::window::{RenderOutcome, WindowBackend};

/// 空闲时的默认等待上限（毫秒）。
///
/// 与两个后端原先各自读取 `Y901_IDLE_WAIT_MS` 的行为一致：
/// 默认 100ms，`0` 表示"不限时等待、由 waker 唤醒"。
///
/// **两个实现细节都是 bug 修出来的**：
/// 1. 解析结果用 `OnceLock` 缓存：这个函数每帧至少调两次（一处在 `run_loop` 的
///    idle 分支、一处在 `wait_message` 的超时），而 `env::var` 每次都要拿全局锁 + 分配；
/// 2. 文档说 `0` = 不限时等待，但把 0 直接交给 `MsgWaitForMultipleObjectsEx` /
///    `poll` 是**立即返回** —— 于是"0 = 阻塞到 waker"变成"0 = 100% CPU 空转"。
///    这里把 0 映射成一个足够大的上限，真正的唤醒仍由 waker 负责
///    （调用方还会再与 `tick_due_ms` 取 min，所以不会错过 Tick）。
fn idle_wait_ms() -> u32 {
    static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| {
        match std::env::var("Y901_IDLE_WAIT_MS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            // 30s：远大于任何合理的唤醒间隔，又不会让"没有 waker 的实现"卡死退出。
            Some(0) => 30_000,
            Some(ms) => ms,
            None => 100,
        }
    })
}

/// Remaining portion of a 16 ms busy-frame budget, rounded down to wait granularity.
fn busy_wait_ms(frame_elapsed: std::time::Duration) -> u32 {
    std::time::Duration::from_millis(16)
        .saturating_sub(frame_elapsed)
        .as_millis() as u32
}

fn elapsed_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis() as u64
}

/// 一帧的观测结果。
pub struct FrameStats {
    /// 本帧是否有事件或布局工作。
    pub had_work: bool,
    /// 本帧是否重新布局。
    pub need_layout: bool,
    /// 本帧是否真正上屏。
    pub presented: bool,
    /// 事件里出现了 `RawEvent::Close`（桌面关窗）。
    pub close_requested: bool,
}

/// 消化一批"已到手"的事件并推进一帧：**不阻塞、不计时**。
///
/// 覆盖：事件消化 → 命令/idle → 动画推进 → 重排 → 主题 → `after_frame` →
/// `render` → `after_present`。
///
/// - `dt`：上一帧到本帧的时间步，**由调用方提供**（桌面用 `Instant`，wasm 用 rAF 时间戳）。
/// - `close_requested`：以前 `RawEvent::Close` 会直接 `return`，现在改成标记，由调用方决定是否停止。
/// - 首帧日志与 profile 统计**不进来**，它们属于调用方的职责。
pub fn frame_step<W: WindowBackend>(
    window: &mut W,
    tree: &mut UiTree,
    host: &mut dyn Host,
    events: &[RawEvent],
    dt: std::time::Duration,
) -> Result<FrameStats, String> {
    let (w, h) = window.viewport();
    let viewport = Size::new(w as f32, h as f32);
    let mut need_layout = false;
    let mut had_work = false;
    let mut close_requested = false;

    // ---- 1. 事件 ----
    for raw in events {
        if matches!(raw, RawEvent::Close) {
            close_requested = true;
            continue;
        }
        if matches!(raw, RawEvent::Resize { .. }) {
            need_layout = true;
        }
        had_work = true;
        if let Some(ev) = translate(raw, viewport) {
            // Host 先处理；已处理则不再往下分发。
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

    // ---- 2.5 动画推进（每帧必须调）----
    if tree.tick_animations(dt) {
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

    // ---- 5. 后端收尾（IME / 光标 / 快照 / 字体）----
    window.after_frame(tree, host);

    // ---- 6. 渲染 ----
    let outcome = window.render(tree)?;
    window.after_present(tree, host);

    Ok(FrameStats {
        had_work,
        need_layout,
        presented: matches!(outcome, RenderOutcome::Presented),
        close_requested,
    })
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
    // **零开销**：未开启时为 `None`，下面只有一次 `if let`。
    let mut profiler: Option<profile::FrameProfile> = if profile {
        Some(profile::FrameProfile::new())
    } else {
        None
    };
    let mut first_frame_done = false;
    // 上一帧是否有工作：有工作时不得等待，否则 Win32 会在每帧都“睡”一次。
    let mut was_busy = false;
    // 上一帧的时间点：既供动画 `dt`，也用于**帧预算补偿**。
    //
    // 忙档的等待 = `16ms − (loop_start − last_tick)`，也就是"上一帧工作已经花掉多少，
    // 这一帧只补剩下的差额"。为什么不再需要"猜后端是否自节流"：
    //   - D3D11 的 `Present(1)` 会阻塞到 vsync，上一帧工作耗时本身就 ≈ 16ms，
    //     差额自然趋近 0 —— 不需要额外的自节流标志；
    //   - 软后端（GDI）工作只要几 ms，差额补到约 16ms —— 不会空转烧 CPU。
    // 旧实现在这里维护 `last_frame_ms` 但**从未更新**（恒 0），于是每帧固定睡 16ms，
    // 把帧周期拉到 21~24ms（≈40fps）；上一轮审查发现后改为现在的差值口径。
    let mut last_tick = Instant::now();
    // 最近一次发出 `UiEvent::Tick` 的时间（供 Tick 节拍）。
    let mut last_tick_at = Instant::now();

    loop {
        let loop_start = Instant::now();
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
            // 若本帧已经花了该花的时间（布局 + 绘制 + 呈现），就只补足差额。
            // 否则"固定 16ms 睡 + 5~8ms 工作"会把帧周期拉到 21~24ms（≈40fps）。
            busy_wait_ms(loop_start.saturating_duration_since(last_tick))
        } else {
            idle_ms.min(tick_due_ms)
        };
        let Some(raws) = window.wait_events(wait_ms) else {
            return Ok(());
        };
        let tick_now = Instant::now();
        let dt = tick_now.saturating_duration_since(last_tick);
        // Do not count event-wait time against the work budget or present_ms.
        let work_start = Instant::now();
        // ---- 1..6. 一帧的通用部分（与 wasm 共用）----
        let stats = frame_step(window, tree, host, &raws, dt)?;
        let (had_work, need_layout, presented) =
            (stats.had_work, stats.need_layout, stats.presented);
        if stats.close_requested {
            return Ok(());
        }
        last_tick = tick_now;
        if !first_frame_done {
            first_frame_done = true;
            eprintln!(
                "[y901] first frame ready in {:.1}ms",
                started.elapsed().as_secs_f64() * 1e3
            );
        }

        // ---- 7. 统计 ----
        // Measure frame work after render and after_present (excluding event wait)
        // for the profiler's present_ms field.
        let frame_elapsed = work_start.elapsed();
        if let Some(p) = profiler.as_mut() {
            p.record(
                frame_elapsed.as_micros() as u64,
                profile::FrameMeta {
                    had_work,
                    need_layout,
                    presented,
                    present_ms: elapsed_ms(frame_elapsed),
                },
            );
        }

        was_busy = had_work || presented;

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

    #[test]
    fn busy_wait_uses_remaining_frame_budget() {
        use super::busy_wait_ms;
        use std::time::Duration;

        assert_eq!(busy_wait_ms(Duration::from_millis(5)), 11);
        assert_eq!(busy_wait_ms(Duration::from_millis(16)), 0);
        assert_eq!(busy_wait_ms(Duration::from_millis(20)), 0);
    }

    #[test]
    fn elapsed_ms_reports_the_supplied_frame_duration() {
        use super::elapsed_ms;
        use std::time::Duration;

        assert_eq!(elapsed_ms(Duration::from_millis(23)), 23);
        assert_eq!(elapsed_ms(Duration::from_micros(999)), 0);
    }
}
