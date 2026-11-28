//! **`TreeView` 虚拟化的不变量**（属性测试，确定性伪随机）。
//!
//! 树比列表多一个可变因素：**展开/折叠会改变总行数**，而滚动偏移是像素值 ——
//! 折掉一大片之后偏移可能指向内容之外（审计 #34 记的正是这类问题：滚到底再折叠父节点，
//! 可见区间变空、整控件只剩背景、点击全失效）。这里随机展开/折叠 + 滚动，钉住：
//!
//! 1. `visible_range` 始终合法（`end <= visible_len`）；
//! 2. 视口内的行能用 `row_at` 反查回自己；
//! 3. 行矩形按 y 单调不重叠；
//! 4. **滚到底后把视口拉高，偏移必须夹回 0**（否则内容整体偏上留白）。

use y901_ui::layout::LayoutCache;
use y901_ui::Widget;
use y901_ui::{Constraints, EventCtx, Modifiers, Pos, Rect, Size, TextMetrics, TextStyle, UiEvent};
use y901_widgets::tree_view::TreeItem;
use y901_widgets::TreeView;

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

fn tree(branches: usize, kids: usize) -> TreeView {
    let roots: Vec<TreeItem> = (0..branches)
        .map(|b| {
            TreeItem::branch(
                format!("branch{b}"),
                (0..kids)
                    .map(|k| TreeItem::leaf(format!("leaf{b}-{k}")))
                    .collect(),
            )
        })
        .collect();
    TreeView::new("tv", roots)
}

fn arrange(tv: &mut TreeView, w: f32, h: f32) {
    let mut cache = LayoutCache::new();
    let c = Constraints::loose(Size::new(w, h));
    let _ = Widget::measure(tv, &Mono, c, &mut cache);
    Widget::arrange(tv, Rect::new(0.0, 0.0, w, h), &Mono, &mut cache);
}

fn wheel(tv: &mut TreeView, delta: f32) {
    let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
    let _ = Widget::event(
        tv,
        &mut ctx,
        &UiEvent::Wheel {
            delta,
            pos: Pos::new(5.0, 5.0),
            mods: Modifiers::default(),
        },
    );
}

fn check(tv: &TreeView, ctx: &str) {
    let len = tv.visible_len();
    let vr = tv.visible_range();
    assert!(vr.start <= vr.end, "[{ctx}] visible_range 反了：{vr:?}");
    assert!(
        vr.end <= len,
        "[{ctx}] visible_range 越界：{vr:?} len={len}"
    );
    // 可见行里，落在视口内的行必须能被 row_at 反查回自己
    let rect = Widget::rect(tv);
    for i in vr.clone() {
        let Some(r) = tv.row_rect(i) else { continue };
        let vis = r.intersect(rect);
        if vis.h < 2.0 {
            continue;
        }
        let p = Pos::new(r.x + r.w * 0.5, vis.y + vis.h * 0.5);
        assert_eq!(
            tv.row_at(p),
            Some(i),
            "[{ctx}] row_at({p:?}) 应为 {i}，实得 {:?}（row={r:?} view={rect:?} vr={vr:?}）",
            tv.row_at(p)
        );
    }
    // 行矩形按 y 单调不重叠
    let mut prev_bottom = f32::NEG_INFINITY;
    for i in vr.clone() {
        if let Some(r) = tv.row_rect(i) {
            assert!(
                r.y >= prev_bottom - 0.01,
                "[{ctx}] 第 {i} 行与上一行重叠：{r:?} prev_bottom={prev_bottom}"
            );
            prev_bottom = r.y + r.h;
        }
    }
}

#[test]
fn tree_view_range_and_hit_test_stay_consistent() {
    let mut rng = Lcg(0x07ee_5eed);
    for case in 0..120 {
        let branches = 1 + (rng.next() % 6) as usize;
        let kids = 1 + (rng.next() % 8) as usize;
        let mut tv = tree(branches, kids);
        let h = 60.0 + (rng.next() % 200) as f32;
        arrange(&mut tv, 240.0, h);
        // 随机展开/折叠
        for _ in 0..8 {
            let b = (rng.next() as usize) % branches;
            let _ = tv.toggle_expanded(&[b]);
            let _ = tv.toggle_expanded(&[b]);
            tv.rebuild();
            arrange(&mut tv, 240.0, h);
            check(&tv, &format!("case{case} toggle{b}"));
        }
        // 随机滚动
        for step in 0..10 {
            wheel(
                &mut tv,
                if rng.next().is_multiple_of(2) {
                    3.0
                } else {
                    -3.0
                },
            );
            check(&tv, &format!("case{case} step{step}"));
        }
    }
}

#[test]
fn growing_the_viewport_clamps_the_scroll_offset() {
    // 回归：滚到底后把视口拉高，内容不能整体偏上留白（审计 #34 的同族问题）。
    let mut tv = tree(6, 8);
    for b in 0..6 {
        let _ = tv.toggle_expanded(&[b]);
    }
    tv.rebuild();
    arrange(&mut tv, 240.0, 80.0);
    for _ in 0..40 {
        wheel(&mut tv, -5.0); // 一路滚到底
    }
    let vr_bottom = tv.visible_range();
    assert!(
        vr_bottom.end == tv.visible_len(),
        "前置：应已滚到底（{}..{} / {}）",
        vr_bottom.start,
        vr_bottom.end,
        tv.visible_len()
    );

    // 视口拉高到能装下全部内容
    arrange(&mut tv, 240.0, 2000.0);
    let first = tv.row_rect(0).expect("第 0 行");
    assert!(
        (first.y - 0.0).abs() < 0.01,
        "视口拉高后滚动偏移必须被夹回 0（否则内容整体偏上）：row0.y={}",
        first.y
    );
    check(&tv, "after-grow");
}
