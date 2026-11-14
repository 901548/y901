//! 动画基础设施：缓动函数 + 时间轴 + 打断语义。
//!
//! ## 为什么要独立一层而不是各组件各写一套
//!
//! 每个组件自己缓存"开始时间 + 初始值 + 目标值"会带来三个共通 bug：
//! 1. **打断时跳变**：动画中途改目标，旧动画不死 → 两个动画同时写同一个属性；
//! 2. **完成回调丢失**：动画到 1.0 之后没有人告诉上层"结束了"；
//! 3. **空转占 CPU**：没动画时仍然每帧 Tick。
//!
//! 这三条都在本模块内解决，组件只需"要一个 0..1 的进度"。

use std::time::Duration;

/// 缓动曲线。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Easing {
    /// 匀速（适合不应该有"惯性"的场景，如进度条）
    #[default]
    Linear,
    /// 加速进入（t²）—— 适合"离开/隐藏"
    EaseInQuad,
    /// 减速停下（1-(1-t)²）—— 适合"进入/展开"
    EaseOutQuad,
    /// 两头慢中间快（标准过渡曲线）
    EaseInOutQuad,
    /// 加速进入（t³）
    EaseInCubic,
    /// 减速停下（1-(1-t)³）—— 弹性感最强的常用曲线
    EaseOutCubic,
    /// 两头慢中间快（三次）
    EaseInOutCubic,
}

impl Easing {
    /// 把线性进度 `t ∈ [0,1]` 映射成缓动后的进度。
    ///
    /// 输入：`t`【超出 [0,1] 会被裁剪】。输出：同样在 `[0,1]`。
    /// 副作用：无。
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseInQuad => t * t,
            Easing::EaseOutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - 2.0 * (1.0 - t) * (1.0 - t)
                }
            }
            Easing::EaseInCubic => t * t * t,
            Easing::EaseOutCubic => {
                let u = 1.0 - t;
                1.0 - u * u * u
            }
            Easing::EaseInOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let u = -2.0 * t + 2.0;
                    1.0 - u * u * u / 2.0
                }
            }
        }
    }
}

/// 一个可打断的补间动画。
///
/// 语义：从 `from` 到 `to`，在 `duration` 内走完；`value()` 返回**已缓动**的值
/// （不是 0..1 进度，而是 `from..to` 区间内的值）。
#[derive(Clone, Debug)]
pub struct Animate {
    /// 起始值
    from: f32,
    /// 目标值
    to: f32,
    /// 总时长（`Duration::ZERO` = 瞬间完成）
    duration: Duration,
    /// 已经过的时间
    elapsed: Duration,
    /// 缓动曲线
    easing: Easing,
    /// 是否已完成
    done: bool,
}

impl Animate {
    /// 创建一个动画。
    pub fn new(from: f32, to: f32, duration: Duration, easing: Easing) -> Self {
        Self {
            from,
            to,
            duration,
            elapsed: Duration::ZERO,
            easing,
            done: duration.is_zero(),
        }
    }

    /// 推进时间。
    ///
    /// 返回：`true` = 还在进行（需要继续 Tick）；`false` = 已完成。
    /// 副作用：累加 `elapsed`，到达时长后置 `done`。
    ///
    /// 为什么完成后再 Tick 仍返回 `false`：调用方（动画列表）靠这个信号删除项，
    /// 否则每帧都会重新触发一次"完成回调"。
    pub fn tick(&mut self, dt: Duration) -> bool {
        if self.done {
            return false;
        }
        self.elapsed = (self.elapsed + dt).min(self.duration);
        if self.elapsed >= self.duration {
            self.done = true;
            return false;
        }
        true
    }

    /// 当前值（`from`..`to`，已缓动）。
    pub fn value(&self) -> f32 {
        if self.duration.is_zero() {
            return self.to;
        }
        let t = self.elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let e = self.easing.apply(t);
        self.from + (self.to - self.from) * e
    }

    /// 归一化进度 0..1（已缓动）。
    pub fn progress(&self) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        self.easing
            .apply(self.elapsed.as_secs_f32() / self.duration.as_secs_f32())
    }

    /// 是否已完成。
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// 目标值（用于"被新动画打断时从当前值接续"）。
    pub fn target(&self) -> f32 {
        self.to
    }

    /// 从当前值重新开始一个新动画（**打断**）。
    ///
    /// 返回：新动画（`from = 当前值`）。
    /// 为什么从当前值而不是从旧动画的 `from`：
    /// 否则中途改目标会产生**回跳**（先跳回起点再走），看起来像抖动。
    pub fn retarget(&self, to: f32, duration: Duration, easing: Easing) -> Self {
        Self::new(self.value(), to, duration, easing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_ctx_start_animation_pushes_request() {
        use crate::widget::EventCtx;
        let mut ctx = EventCtx::new(None, crate::geometry::Size::new(100.0, 100.0));
        let key = key_of("probe");
        assert!(ctx.anim_requests.is_empty());
        ctx.start_animation(
            key,
            Animate::new(
                0.0,
                1.0,
                std::time::Duration::from_millis(120),
                Easing::EaseOutCubic,
            ),
        );
        assert_eq!(ctx.anim_requests.len(), 1);
        assert_eq!(ctx.anim_requests[0].0, key);
    }

    #[test]
    fn start_animation_marks_redraw_and_is_drainable() {
        use crate::widget::EventCtx;
        let mut ctx = EventCtx::new(None, crate::geometry::Size::new(10.0, 10.0));
        assert!(!ctx.redraw, "初始不应要求重绘");
        ctx.start_animation(
            key_of("x"),
            Animate::new(
                0.0,
                1.0,
                std::time::Duration::from_millis(80),
                Easing::Linear,
            ),
        );
        assert!(ctx.redraw, "请求动画应顺带标记重绘");
        // 与 UiTree 的消费方式一致：take 后队列应为空
        let drained: Vec<(u64, Animate)> = std::mem::take(&mut ctx.anim_requests);
        assert_eq!(drained.len(), 1);
        assert!(ctx.anim_requests.is_empty());
    }

    #[test]
    fn linear_easing_is_identity() {
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            assert!((Easing::Linear.apply(t) - t).abs() < 1e-6);
        }
    }

    #[test]
    fn easing_endpoints_and_monotonicity() {
        let all = [
            Easing::Linear,
            Easing::EaseInQuad,
            Easing::EaseOutQuad,
            Easing::EaseInOutQuad,
            Easing::EaseInCubic,
            Easing::EaseOutCubic,
            Easing::EaseInOutCubic,
        ];
        for e in all {
            assert!((e.apply(0.0) - 0.0).abs() < 1e-6, "{e:?} 起点应为 0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?} 终点应为 1");
            // 单调递增（不允许回头，否则动画会看起来像抖动）
            let mut prev = -1.0;
            for i in 0..=100 {
                let v = e.apply(i as f32 / 100.0);
                assert!(v >= prev - 1e-6, "{e:?} 在 t={} 处非单调", i as f32 / 100.0);
                prev = v;
            }
        }
    }

    #[test]
    fn ease_in_starts_slow_and_out_starts_fast() {
        // 加速曲线在前半段应低于线性，减速曲线应高于线性
        assert!(Easing::EaseInCubic.apply(0.25) < 0.25);
        assert!(Easing::EaseOutCubic.apply(0.25) > 0.25);
        // 中间点对称曲线应过 0.5
        assert!((Easing::EaseInOutCubic.apply(0.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn animate_interpolates_and_finishes() {
        let mut a = Animate::new(0.0, 100.0, Duration::from_millis(100), Easing::Linear);
        assert_eq!(a.value(), 0.0);
        assert!(a.tick(Duration::from_millis(50)));
        assert!(
            (a.value() - 50.0).abs() < 0.5,
            "一半时应约为 50，实际 {}",
            a.value()
        );
        assert!(!a.tick(Duration::from_millis(60)), "超时后应完成");
        assert!(a.is_done());
        assert!((a.value() - 100.0).abs() < 1e-3, "完成后应为终值");
    }

    #[test]
    fn ticking_a_finished_animation_is_a_no_op() {
        // 防止"每帧都重新触发完成回调"
        let mut a = Animate::new(0.0, 1.0, Duration::from_millis(10), Easing::Linear);
        assert!(!a.tick(Duration::from_millis(20)));
        assert!(!a.tick(Duration::from_millis(20)));
        assert!(!a.tick(Duration::from_millis(20)));
        assert!(a.is_done());
    }

    #[test]
    fn retarget_continues_from_current_value_without_jumping_back() {
        let mut a = Animate::new(0.0, 100.0, Duration::from_millis(100), Easing::Linear);
        a.tick(Duration::from_millis(40));
        let mid = a.value();
        assert!((mid - 40.0).abs() < 1.0);

        // 中途改目标 → 新动画应从 40 开始，而不是从 0
        let b = a.retarget(0.0, Duration::from_millis(100), Easing::Linear);
        assert!(
            (b.value() - mid).abs() < 1e-3,
            "打断后应从当前值接续，实际 {} vs {}",
            b.value(),
            mid
        );
        assert_eq!(b.target(), 0.0);
    }

    #[test]
    fn zero_duration_completes_immediately() {
        let a = Animate::new(5.0, 9.0, Duration::ZERO, Easing::Linear);
        assert!(a.is_done(), "零时长应立即完成");
        assert_eq!(a.value(), 9.0);
        assert_eq!(a.progress(), 1.0);
    }

    #[test]
    fn progress_tracks_easing_not_raw_time() {
        let mut a = Animate::new(0.0, 1.0, Duration::from_millis(100), Easing::EaseOutCubic);
        a.tick(Duration::from_millis(50));
        let p = a.progress();
        assert!(p > 0.5, "减速曲线一半时应过半（实际 {p}）");
        assert!(p < 1.0);
    }
}

/// 把一个字符串 id 散列成动画 key（FNV-1a）。
///
/// 为什么要有它：控件在 `render` 里只知道自己的 id，
/// 而动画表需要稳定的 `u64` key。两边用同一个函数才不会对不上。
pub fn key_of(id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 弹出层开启动画的时长（毫秒）。
pub const POPUP_OPEN_MS: u64 = 150;

/// 一帧的动画快照（key → 进度）。
///
/// 为什么用小型 Vec 而不是 HashMap：同时活动的动画通常只有 0~3 个，
/// 线性查找比散列快、且不需要每帧分配哈希表。
#[derive(Clone, Debug, Default)]
pub struct AnimSnapshot {
    /// 各 key 的当前进度
    entries: Vec<(u64, f32)>,
}

impl AnimSnapshot {
    /// 从"已完成度" 列表构造。
    pub fn new(entries: Vec<(u64, f32)>) -> Self {
        Self { entries }
    }

    /// 查询某个 key 的进度（不存在 → `None`）。
    pub fn get(&self, key: u64) -> Option<f32> {
        self.entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
