//! **`ScrollView` 的不变量：内容位移必须与 `offset()` 同帧一致**（属性测试）。
//!
//! 滚动容器有个隐蔽的时序陷阱：`offset` 决定内容摆在哪，但 `offset` 本身也会在
//! `arrange` 里变（钳制、粘底）。凡是"摆完内容再改 offset"，都会让同一帧里
//! **子节点位置与 offset() 各说各话** —— 画面滞后一帧再跳。
//!
//! 这正是 2026-11-28 抓到的真 bug：流式追加时粘底把 offset 推到新底部，
//! 但子节点已按旧 offset 摆好，最后一段落在视口外，下一帧才跳上来。
//! 修法是把"粘底"挪到 `kid.arrange()` 之前定稿。

use std::cell::Cell;
use std::rc::Rc;
use y901_ui::layout::LayoutCache;
use y901_ui::{Constraints, Rect, Size, TextMetrics, TextStyle, Widget};
use y901_widgets::ScrollView;

struct Mono;
impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 高度可变的叶子（模拟"追加了一行日志"）。
struct Grow {
    id: &'static str,
    h: Rc<Cell<f32>>,
    rect: Rect,
}
impl Widget for Grow {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        Size::new(c.max.w, self.h.get())
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

fn frame(sv: &mut ScrollView, w: f32, h: f32) -> Rect {
    let mut cache = LayoutCache::new();
    let c = Constraints::loose(Size::new(w, h));
    let _ = Widget::measure(sv, &Mono, c, &mut cache);
    let viewport = Rect::new(0.0, 0.0, w, h);
    Widget::arrange(sv, viewport, &Mono, &mut cache);
    viewport
}

#[test]
fn child_is_positioned_with_the_final_offset_in_the_same_frame() {
    let h = Rc::new(Cell::new(400.0));
    let mut sv = ScrollView::new(
        "sv",
        Box::new(Grow {
            id: "content",
            h: h.clone(),
            rect: Rect::ZERO,
        }),
    );
    sv.set_stick_to_bottom(true);

    // 首帧：内容高于视口 → 出现滚动条
    let viewport = frame(&mut sv, 200.0, 100.0);
    assert!(sv.offset() >= 0.0);

    // 滚到底（粘底已开）
    for _ in 0..10 {
        sv.scroll_to_end();
        let _vp = frame(&mut sv, 200.0, 100.0);
    }
    let offset_before = sv.offset();
    assert!(
        offset_before > 0.0,
        "前置：应已能滚动（offset={offset_before}）"
    );

    // 追加内容 → 粘底应把偏移推到新的底部
    h.set(460.0);
    let _vp = frame(&mut sv, 200.0, 100.0);
    let offset_after = sv.offset();
    assert!(
        offset_after > offset_before,
        "粘底：内容变高后 offset 应变大（{offset_before} → {offset_after}）"
    );

    // **核心不变量**：子节点本帧的位置必须与 offset 一致
    let kid = &sv.children()[0];
    let kid_y = kid.rect().y;
    assert!(
        (kid_y - (viewport.y - offset_after)).abs() < 0.01,
        "子节点位置与 offset 不同帧：kid.y={kid_y}，应为 viewport.y - offset = {}（offset={offset_after}）\
         —— 说明本帧是先摆内容、后改 offset，画面会滞后一帧再跳",
        viewport.y - offset_after
    );
}
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// 随机"内容变高/变矮 + 滚动"下，每一帧都必须满足
/// `子节点 y == viewport.y - offset()`（位置与偏移同帧一致）。
#[test]
fn offset_and_child_position_never_disagree() {
    let mut rng = Lcg(0x5c_2011);
    for case in 0..120 {
        let h = Rc::new(Cell::new(300.0));
        let mut sv = ScrollView::new(
            "sv",
            Box::new(Grow {
                id: "content",
                h: h.clone(),
                rect: Rect::ZERO,
            }),
        );
        if rng.next().is_multiple_of(2) {
            sv.set_stick_to_bottom(true);
        }
        let vh = 80.0 + (rng.next() % 120) as f32;
        let vw = 160.0;

        let mut cur = 300.0f32;
        for step in 0..20 {
            match rng.next() % 4 {
                0 => cur += 40.0,                  // 追加
                1 => cur = (cur - 60.0).max(20.0), // 变矮
                2 => {
                    sv.scroll_to_end();
                }
                _ => {
                    let mut ctx = y901_ui::EventCtx::new(None, Size::new(400.0, 400.0));
                    let _ = Widget::event(
                        &mut sv,
                        &mut ctx,
                        &y901_ui::UiEvent::Wheel {
                            delta: if rng.next().is_multiple_of(2) {
                                2.0
                            } else {
                                -2.0
                            },
                            pos: y901_ui::Pos::new(5.0, 5.0),
                            mods: y901_ui::Modifiers::default(),
                        },
                    );
                }
            }
            h.set(cur);
            let viewport = frame(&mut sv, vw, vh);
            let kid_y = sv.children()[0].rect().y;
            let expect = viewport.y - sv.offset();
            assert!(
                (kid_y - expect).abs() < 0.01,
                "[case{case} step{step}] 子节点 y={kid_y} 与 offset 不一致（应为 {expect}，offset={} content_h={cur} vh={vh}）",
                sv.offset()
            );
            // 偏移必须始终在合法范围
            assert!(sv.offset() >= 0.0, "[case{case}] offset 为负");
        }
    }
}
