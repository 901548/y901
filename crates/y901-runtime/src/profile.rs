//! \u5e27\u65f6\u76f4\u65b9\u56fe\uff08`Y901_PROFILE` \u5f00\u5173\uff09\u3002
//!
//! **\u4e3a\u4f55\u5355\u72ec\u4e00\u4e2a\u6a21\u5757**\uff1a\u4e4b\u524d\u53ea\u6709 `avg` \u4e0e `max` \u4e24\u4e2a\u6570\uff0c\u770b\u4e0d\u51fa"
//! \u5c16\u523a\u662f\u504f\u6001\u8fd8\u662f\u4e00\u4e24\u6b21\u5076\u7136"\u3002\u8fd9\u91cc\u4fdd\u7559\u6700\u8fd1 4096 \u5e27\u7684\u91c7\u6837\uff0c
//! \u6bcf 2 \u79d2\u7ed9\u51fa p50/p95/p99/max\uff0c\u5e76\u5728 max \u90a3\u4e00\u5e27\u4e0a\u9644\u5e26\u5f52\u56e0\u4fe1\u606f\u3002
//!
//! **\u96f6\u5f00\u9500**\uff1a`run_loop` \u91cc\u53ea\u6709 `Option<FrameProfile>` \u7684\u4e00\u6b21 `if let`\uff1b
//! \u73af\u5883\u53d8\u91cf\u672a\u8bbe\u65f6 `profiler == None`\uff0c**\u4e0d\u5206\u914d\u3001\u4e0d\u8ba1\u65f6\u3001\u4e0d\u8ba1\u6570**\u3002
//!
//! **口径**：`us` 是**循环总耗时**（含空闲等待）。用 `presented` 字段区分空闲帧与呈现帧；
//! 空闲帧的 `us` 反映的是**等待时长**，不是渲染耗时。

use std::time::Instant;

/// \u4e00\u5e27\u7684\u5f52\u56e0\u4fe1\u606f\uff08\u53ea\u5728 profile \u5f00\u542f\u65f6\u6784\u9020\uff09\u3002
#[derive(Clone, Copy, Default)]
pub struct FrameMeta {
    /// \u672c\u5e27\u662f\u5426\u6709\u4e8b\u4ef6/\u5e03\u5c40\u5de5\u4f5c\u3002
    pub had_work: bool,
    /// \u672c\u5e27\u662f\u5426\u91cd\u65b0\u5e03\u5c40\u3002
    pub need_layout: bool,
    /// \u672c\u5e27\u662f\u5426\u771f\u6b63\u4e0a\u5c4f\uff08`Presented`\uff09\u3002
    pub presented: bool,
    /// \u672c\u5e27\u5b9e\u9645\u8017\u65f6\uff08\u6beb\u79d2\uff09\u3002
    pub present_ms: u64,
}

/// \u73af\u5f62\u7f13\u51b2 + \u5206\u4f4d\u6570\u62a5\u544a\u3002
pub struct FrameProfile {
    samples: Vec<u32>,
    next: usize,
    filled: bool,
    /// \u6700\u5927\u503c\u53ca\u5176\u5f52\u56e0\uff08\u53ea\u4fdd\u7559\u5f53\u524d 2 \u79d2\u7a97\u53e3\u5185\u7684\uff09\u3002
    max_us: u32,
    max_meta: FrameMeta,
    window_frames: u32,
    last_report: Instant,
}

impl FrameProfile {
    /// \u65b0\u5efa\uff08\u9884\u5206\u914d 4096 \u9879\uff0c\u4ec5 profile \u5f00\u542f\u65f6\u4e00\u6b21\uff09\u3002
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

    /// \u8bb0\u4e00\u5e27\uff1b\u6bcf 2 \u79d2\u8f93\u51fa\u4e00\u6b21\u5206\u5e03\u3002
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

    /// \u8f93\u51fa p50/p95/p99/max + max \u5e27\u5f52\u56e0\uff0c\u7136\u540e\u6e05\u7a7a\u7a97\u53e3\u8ba1\u6570\u3002
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
