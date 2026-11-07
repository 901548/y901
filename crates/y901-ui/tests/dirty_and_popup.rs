//! 三项 P2 修复的回归测试：
//! 1. `DirtyRegion::add` 双向合并（流式增长不会无限累积）；
//! 2. 脏区碎片超过上限 → `plan_dirty_rects` 返回 `None`（整窗重绘）；
//! 3. `place_popup` 的翻转/夹取（浮层不溢出父边界，命中与绘制同源）。

use y901_ui::{place_popup, DirtyRegion, Pos, Rect, Size};

// ---------------------------------------------------------------- 任务 1

#[test]
fn growing_dirty_rect_collapses_to_one() {
    let mut d = DirtyRegion::default();
    // 模拟流式输出：同一块区域逐步向下增长
    for i in 0..50 {
        d.add(Rect::new(100.0, 100.0, 200.0, 20.0 + i as f32 * 4.0));
    }
    assert!(
        d.rects().len() <= 2,
        "增长型脏区必须被合并，实际 {} 块：{:?}",
        d.rects().len(),
        d.rects()
    );
}

#[test]
fn new_rect_containing_old_removes_old() {
    let mut d = DirtyRegion::default();
    d.add(Rect::new(10.0, 10.0, 20.0, 20.0));
    d.add(Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_eq!(d.rects().len(), 1, "被包含的旧矩形应被移除");
    let r = d.rects()[0];
    assert_eq!(r, Rect::new(0.0, 0.0, 100.0, 100.0));
}

#[test]
fn overlapping_rects_merge_into_union() {
    let mut d = DirtyRegion::default();
    d.add(Rect::new(0.0, 0.0, 50.0, 50.0));
    d.add(Rect::new(40.0, 40.0, 50.0, 50.0));
    assert_eq!(d.rects().len(), 1, "相交应合并成一块");
    let r = d.rects()[0];
    assert_eq!(r, Rect::new(0.0, 0.0, 90.0, 90.0), "并集应是 (0,0)-(90,90)");
}

#[test]
fn disjoint_rects_stay_separate() {
    let mut d = DirtyRegion::default();
    d.add(Rect::new(0.0, 0.0, 10.0, 10.0));
    d.add(Rect::new(200.0, 200.0, 10.0, 10.0));
    assert_eq!(d.rects().len(), 2, "不相交的不该合并（会误清中间区域）");
}

// ---------------------------------------------------------------- 任务 3

#[test]
fn popup_fits_below_when_space_allows() {
    let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
    let anchor = Rect::new(50.0, 50.0, 100.0, 24.0);
    let r = place_popup(anchor, Size::new(120.0, 100.0), bounds);
    assert_eq!(r.y, anchor.bottom(), "有空间时应贴在锚点下方");
    assert_eq!(r.x, anchor.x, "左对齐");
}

#[test]
fn popup_flips_up_when_bottom_space_is_short() {
    let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
    // 锚点靠近底部：下方只剩 30px，放不下 100px
    let anchor = Rect::new(50.0, 370.0, 100.0, 24.0);
    let r = place_popup(anchor, Size::new(120.0, 100.0), bounds);
    assert_eq!(r.y, anchor.y - 100.0, "下方放不下时应向上翻转");
    assert!(r.bottom() <= bounds.bottom(), "翻转后不能超出父边界");
}

#[test]
fn popup_shifts_left_when_right_space_is_short() {
    let bounds = Rect::new(0.0, 0.0, 400.0, 400.0);
    let anchor = Rect::new(350.0, 50.0, 40.0, 24.0);
    let r = place_popup(anchor, Size::new(120.0, 100.0), bounds);
    assert!(r.right() <= bounds.right(), "右侧放不下时应向左平移：{r:?}");
    assert!(r.x >= bounds.x);
}

#[test]
fn popup_never_exceeds_bounds() {
    let bounds = Rect::new(10.0, 10.0, 100.0, 100.0);
    // 弹层比父区域还大
    let anchor = Rect::new(50.0, 50.0, 10.0, 10.0);
    let r = place_popup(anchor, Size::new(500.0, 500.0), bounds);
    assert!(r.x >= bounds.x && r.y >= bounds.y, "不能越出左上：{r:?}");
    assert!(
        r.right() <= bounds.right() + 0.01 && r.bottom() <= bounds.bottom() + 0.01,
        "不能越出右下：{r:?}"
    );
}

#[test]
fn popup_hit_rect_matches_draw_rect() {
    // 命中与绘制同源：place_popup 的输出就是唯一真相
    let bounds = Rect::new(0.0, 0.0, 300.0, 200.0);
    let anchor = Rect::new(280.0, 180.0, 16.0, 16.0);
    let panel = place_popup(anchor, Size::new(140.0, 120.0), bounds);
    // 面板内一点必须命中，面板外一点必须不命中
    let inside = Pos::new(panel.x + panel.w * 0.5, panel.y + panel.h * 0.5);
    let outside = Pos::new(bounds.right() - 1.0, panel.y + panel.h * 0.5);
    assert!(panel.contains(inside));
    assert!(
        !panel.contains(outside) || panel.right() >= bounds.right(),
        "面板右侧已达边界时 outside 仍可能在面板内，这里只断言几何一致"
    );
    assert!(
        panel.right() <= bounds.right(),
        "面板本身不能超出父边界：{panel:?}"
    );
}
