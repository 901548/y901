//! `UiTree` 动画注册表的集成测试。
//!
//! 核心行为：key 打断、完成回调与清理、并发多动画、
//! 以及"没动画时 `tick_animations` 返回 false（主循环可以休眠）"。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use y901_ui::animation::{Animate, Easing};
use y901_ui::UiTree;


/// 最小控件：只占位，不画东西。用于把动画注册表弄进一个真实的 `UiTree`。
struct Leaf {
    id: String,
    rect: y901_ui::Rect,
}

impl Leaf {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            rect: y901_ui::Rect::ZERO,
        }
    }
}

impl y901_ui::Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn y901_ui::TextMetrics,
        c: y901_ui::Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> y901_ui::Size {
        c.constrain(y901_ui::Size::new(10.0, 10.0))
    }
    fn arrange(
        &mut self,
        rect: y901_ui::Rect,
        _m: &dyn y901_ui::TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }
    fn rect(&self) -> y901_ui::Rect {
        self.rect
    }
    fn event(
        &mut self,
        _c: &mut y901_ui::EventCtx,
        _e: &y901_ui::UiEvent,
    ) -> y901_ui::EventResult {
        y901_ui::EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

fn tree() -> UiTree {
    UiTree::new(Box::new(Leaf::new("root")))
}

#[test]
fn animate_then_tick_runs_to_completion_and_clears() {
    let mut t = tree();
    t.animate(1, Animate::new(0.0, 10.0, Duration::from_millis(100), Easing::Linear), None);

    assert!(t.tick_animations(Duration::from_millis(40)), "还在跑应返回 true");
    assert_eq!(t.active_animation_count(), 1);
    let v = t.animation_value(1).expect("应有值");
    assert!((v - 4.0).abs() < 0.5, "四成时应约为 4，实际 {v}");

    assert!(!t.tick_animations(Duration::from_millis(80)), "跑完后应返回 false");
    assert_eq!(t.active_animation_count(), 0, "完成后应被移除");
    assert!(t.animation_value(1).is_none());
}

#[test]
fn same_key_interrupts_the_old_animation() {
    let mut t = tree();
    t.animate(7, Animate::new(0.0, 100.0, Duration::from_millis(100), Easing::Linear), None);
    t.tick_animations(Duration::from_millis(50));

    // 同一个 key 再设一次 → 旧动画必须消失（不能两个同时写同一属性）
    t.animate(7, Animate::new(0.0, 0.0, Duration::from_millis(100), Easing::Linear), None);
    assert_eq!(t.active_animation_count(), 1, "同 key 应只保留一个动画");
}

#[test]
fn different_keys_run_concurrently() {
    let mut t = tree();
    t.animate(1, Animate::new(0.0, 1.0, Duration::from_millis(100), Easing::Linear), None);
    t.animate(2, Animate::new(0.0, 1.0, Duration::from_millis(200), Easing::Linear), None);
    assert_eq!(t.active_animation_count(), 2, "不同 key 应并存");

    // 100ms 后：key 1 完成，key 2 还在跑
    t.tick_animations(Duration::from_millis(100));
    assert_eq!(t.active_animation_count(), 1);
    assert!(t.animation_value(2).is_some(), "key 2 应仍在跑");
}

#[test]
fn completion_callback_runs_exactly_once() {
    let mut t = tree();
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    t.animate(3, Animate::new(0.0, 1.0, Duration::from_millis(10), Easing::Linear), None);
    // 通过直接修改第一个动画来挂回调（正式 API 见下一步）
    t.set_animation_on_complete(3, Box::new(move || {
        h.fetch_add(1, Ordering::SeqCst);
    }));

    t.tick_animations(Duration::from_millis(20));
    t.tick_animations(Duration::from_millis(20));
    t.tick_animations(Duration::from_millis(20));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "完成回调应只跑一次");
}

#[test]
fn no_animations_means_no_extra_ticks() {
    let mut t = tree();
    // 没有任何动画时，tick 应立即返回 false → 主循环不需要为动画提速
    assert!(!t.tick_animations(Duration::from_millis(16)));
    assert_eq!(t.active_animation_count(), 0);
}
