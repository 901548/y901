//! 帧时直方图（`Y901_PROFILE` 开关）。
//!
//! **为何单独一个模块**：之前只有 `avg` 与 `max` 两个数，看不出"
//! 尖刺是偏态还是一两次偶然"。这里保留最近 4096 帧的采样，
//! 每 2 秒给出 p50/p95/p99/max，并在 max 那一帧上附带归因信息。
//!
//! **零开销**：`run_loop` 里只有 `Option<FrameProfile>` 的一次 `if let`；
//! 环境变量未设时 `profiler == None`，**不分配、不计时、不计数**。
//!
//! **口径**：`us` 是**循环总耗时**（含空闲等待）。用 `presented` 字段区分空闲帧与呈现帧；
//! 空闲帧的 `us` 反映的是**等待时长**，不是渲染耗时。

use web_time::Instant;

/// 一帧的归因信息（只在 profile 开启时构造）。
///
/// **口径变更（2026-11-19）**：`us` 与 `present_ms` 现在只统计**本帧工作耗时**
/// （事件分发 → 布局 → 渲染 → after_present），不含 `wait_events` 的等待。
/// 因此本文件输出的 p50/p95 是**忙时延**，不能再与旧版本里
/// "循环周期（含等待、≈16ms）"的数字直接对比。
///
/// `present_ms` 曾经恒为 0（`last_frame_ms` 从未赋值），现在记录同一段工作耗时。
#[derive(Clone, Copy, Default)]
pub struct FrameMeta {
    /// 本帧是否有事件/布局工作。
    pub had_work: bool,
    /// 本帧是否重新布局。
    pub need_layout: bool,
    /// 本帧是否真正上屏（`Presented`）。
    pub presented: bool,
    /// 本帧实际耗时（毫秒）。
    pub present_ms: u64,
}

/// 环形缓冲 + 分位数报告。
pub struct FrameProfile {
    samples: Vec<u32>,
    next: usize,
    filled: bool,
    /// 最大值及其归因（只保留当前 2 秒窗口内的）。
    max_us: u32,
    max_meta: FrameMeta,
    window_frames: u32,
    last_report: Instant,
}

impl FrameProfile {
    /// 新建（预分配 4096 项，仅 profile 开启时一次）。
    pub fn new() -> Self {
        Self {
            samples: vec![0; 4096],
            next: 0,
            filled: false,
            max_us: 0,
            max_meta: FrameMeta::default(),
            window_frames: 0,
            last_report: Instant::now(),
        }
    }

    /// 记一帧；每 2 秒输出一次分布。
    pub fn record(&mut self, us: u64, meta: FrameMeta) {
        let us = us.min(u32::MAX as u64) as u32;
        self.samples[self.next] = us;
        self.next = (self.next + 1) % self.samples.len();
        if self.next == 0 {
            self.filled = true;
        }
        self.window_frames += 1;
        if us > self.max_us {
            self.max_us = us;
            self.max_meta = meta;
        }
        if self.last_report.elapsed().as_millis() >= 2000 {
            self.report();
        }
    }

    /// 输出 p50/p95/p99/max + max 帧归因，然后清空窗口计数。
    fn report(&mut self) {
        let n = if self.filled {
            self.samples.len()
        } else {
            self.next
        };
        if n == 0 {
            self.last_report = Instant::now();
            return;
        }
        let mut v: Vec<u32> = self.samples[..n].to_vec();
        v.sort_unstable();
        let pick = |q: f64| -> u32 {
            let idx = ((n as f64 - 1.0) * q).round() as usize;
            v[idx.min(n - 1)]
        };
        let m = self.max_meta;
        eprintln!(
            "[profile] n={} frames/2s={} p50={}us p95={}us p99={}us max={}us | max-frame: had_work={} need_layout={} presented={} frame_ms={}",
            n,
            self.window_frames,
            pick(0.50),
            pick(0.95),
            pick(0.99),
            self.max_us,
            m.had_work,
            m.need_layout,
            m.presented,
            m.present_ms,
        );
        self.window_frames = 0;
        self.max_us = 0;
        self.max_meta = FrameMeta::default();
        self.last_report = Instant::now();
    }
}
