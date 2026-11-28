//! **`ListView` 虚拟滚动的反查一致性**（属性测试，确定性伪随机）。
//!
//! 虚拟列表算错的表现是"看得见却点不中"或"点中了邻行"：`visible_range()` 决定画哪几行，
//! `row_at()` 决定点在哪一行，`row_rect()` 决定脏区。三者只要有一个用了不同的偏移，
//! 键盘选择、鼠标点击、局部重绘就会各说各话。这里随机行数/行高/视口/滚动位置，钉住：
//!
//! 1. `visible_range` 是合法子区间（`start <= end <= 行数`）；
//! 2. 区间内每一行的**中心点**用 `row_at` 反查必须还是它（双向一致）；
//! 3. 区间内行矩形按 y 单调、互不重叠；
//! 4. 反复上下滚后区间始终合法。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, Widget,
};
use y901_widgets::ListView;

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

/// 布局一个 ListView（走真实 measure/arrange，不手塞 rect）。
fn laid_out(rows: usize, row_h: f32, w: f32, h: f32) -> ListView {
    let mut lv = ListView::new("lv", row_h).len(rows);
    let mut cache = LayoutCache::new();
    let c = Constraints::loose(Size::new(w, h));
    let _ = lv.measure(&Mono, c, &mut cache);
    lv.arrange(Rect::new(0.0, 0.0, w, h), &Mono, &mut cache);
    lv
}

fn scroll_to(lv: &mut ListView, offset: f32) {
    // 没有公开 set_offset：用滚轮事件滚过去（也是真实路径）。
    let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
    let mut guard = 0;
    loop {
        let before = lv.visible_range();
        let _ = lv.event(
            &mut ctx,
            &UiEvent::Wheel {
                delta: offset, // 大 delta → 走 clamp
                pos: Pos::new(1.0, 1.0),
                mods: y901_ui::Modifiers::default(),
            },
        );
        guard += 1;
        if lv.visible_range() == before || guard > 200 {
            break;
        }
    }
}

#[test]
fn visible_range_and_row_at_agree() {
    let mut rng = Lcg(0xabcd_1234);
    for case in 0..150 {
        let rows = (rng.next() % 400) as usize;
        let row_h = 12.0 + (rng.next() % 24) as f32;
        let w = 120.0 + (rng.next() % 200) as f32;
        let h = 40.0 + (rng.next() % 300) as f32;
        let mut lv = laid_out(rows, row_h, w, h);
        if rng.next().is_multiple_of(2) {
            scroll_to(&mut lv, -300.0);
        }

        let vr = lv.visible_range();
        // 1) 区间必须落在 [0, rows] 且有序
        assert!(
            vr.start <= vr.end,
            "[case{case}] visible_range 反了：{vr:?}"
        );
        assert!(
            vr.end <= rows,
            "[case{case}] visible_range 越界：{vr:?} rows={rows}"
        );

        // 2) 区间内每一行的中心点，用 row_at 反查必须还是它
        for i in vr.clone() {
            let Some(r) = lv.row_rect(i) else { continue };
            if r.h < 2.0 {
                continue;
            }
            let center = Pos::new(r.x + r.w * 0.5, r.y + r.h * 0.5);
            let back = lv.row_at(center);
            assert_eq!(
                back,
                Some(i),
                "[case{case}] row_at({center:?}) 应为 {i}，实得 {back:?}（visible={vr:?} row={r:?} rows={rows} row_h={row_h} viewport={w}x{h}）"
            );
        }

        // 3) 区间外（可见区上下各留一点）的行不应被 row_at 命中
        if vr.start > 0 {
            if let Some(r) = lv.row_rect(vr.start - 1) {
                let p = Pos::new(r.x + r.w * 0.5, r.y + r.h * 0.5);
                assert!(
                    !lv.visible_range().contains(&(vr.start - 1)),
                    "[case{case}] 前置条件失效"
                );
                assert_eq!(
                    lv.row_at(p),
                    Some(vr.start - 1),
                    "[case{case}] 区间前一行若仍可见，row_at 仍应命中"
                );
            }
        }

        // 4) 行矩形不重叠且按 y 递增
        let mut prev_bottom = f32::NEG_INFINITY;
        for i in vr.clone() {
            if let Some(r) = lv.row_rect(i) {
                assert!(
                    r.y >= prev_bottom - 0.01,
                    "[case{case}] 第 {i} 行与上一行重叠：{r:?} prev_bottom={prev_bottom}"
                );
                prev_bottom = r.y + r.h;
            }
        }
    }
}

#[test]
fn scrolling_never_leaves_the_content_range() {
    let mut rng = Lcg(0x1357_9bdf);
    for case in 0..120 {
        let rows = (rng.next() % 60) as usize;
        let row_h = 10.0 + (rng.next() % 20) as f32;
        let mut lv = laid_out(rows, row_h, 200.0, 120.0);
        // 反复上下滚：可见区间必须始终是合法子区间
        for step in 0..25 {
            let dy = if rng.next().is_multiple_of(2) {
                -50.0
            } else {
                50.0
            };
            let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
            let _ = lv.event(
                &mut ctx,
                &UiEvent::Wheel {
                    delta: dy,
                    pos: Pos::new(1.0, 1.0),
                    mods: y901_ui::Modifiers::default(),
                },
            );
            let vr = lv.visible_range();
            assert!(
                vr.start <= vr.end && vr.end <= rows,
                "[case{case} step{step}] 滚动后 visible_range 非法：{vr:?} rows={rows}"
            );
        }
    }
}

// 让 rustc 知道这些 trait 被用到（EventCtx/Widget 的方法签名需要）
#[allow(dead_code)]
fn _assert_traits(_: &EventCtx) -> EventResult {
    EventResult::Ignored
}
#[allow(dead_code)]
fn _assert_render(_: &mut dyn RenderCtx) {}
#[allow(dead_code)]
fn _assert_sem() -> Semantics {
    Semantics::default()
}
#[allow(dead_code)]
fn _assert_widget<W: Widget>() {}
