//! Contract tests for `UiTree::tick_animations`.
//!
//! Why these exist: `tick_animations` previously had **no caller anywhere in the
//! workspace**, so any fade/scale animation's `progress` stayed at 0 forever.
//! `tree.rs` paints a full-rect overlay with alpha `(1 - progress)` on top of a
//! popup, so a stalled progress means the popup keeps a translucent film over
//! its content -- menus and panels looked "framed but empty".
//!
//! Contract: ticks advance animations; returns true while any animation is
//! active (the loop must keep producing frames), false once all are done.

use std::time::Duration;

use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, UiEvent,
    UiTree, Widget,
};

struct Leaf;

impl Widget for Leaf {
    fn id(&self) -> &str {
        "root"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(100.0, 100.0))
    }
    fn arrange(
        &mut self,
        _r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
    }
    fn rect(&self) -> Rect {
        Rect::ZERO
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn tick_without_animations_is_noop() {
    let mut tree = UiTree::new(Box::new(Leaf));
    assert!(
        !tree.tick_animations(Duration::from_millis(16)),
        "no active animations must return false so the loop may sleep"
    );
}

#[test]
fn repeated_tick_is_safe_when_idle() {
    let mut tree = UiTree::new(Box::new(Leaf));
    for _ in 0..5 {
        assert!(
            !tree.tick_animations(Duration::from_millis(16)),
            "idle tree must keep returning false"
        );
    }
}

#[test]
fn zero_dt_tick_is_safe() {
    let mut tree = UiTree::new(Box::new(Leaf));
    assert!(
        !tree.tick_animations(Duration::ZERO),
        "zero dt must not panic"
    );
}
