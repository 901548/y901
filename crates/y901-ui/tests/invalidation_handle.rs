//! `InvalidationHandle`：后台线程请求标脏。
//!
//! 这些测试全部走真实路径（`UiTree::invalidation_handle` → 后台线程 →
//! `UiTree::drain_external_invalidations`），不直接构造内部状态。

use std::sync::mpsc;
use std::time::Duration;

use y901_ui::{
    Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle,
    UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::new(8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

struct Leaf {
    id: &'static str,
    rect: Rect,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 20.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

fn tree_with_two_leaves() -> UiTree {
    let root = y901_ui::layout::Flex::column("root")
        .push(Box::new(Leaf {
            id: "a",
            rect: Rect::ZERO,
        }))
        .push(Box::new(Leaf {
            id: "b",
            rect: Rect::ZERO,
        }));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    // 首帧之后清空脏区，模拟"稳定帧"
    let mut sink = Sink;
    tree.render(&mut sink);
    tree
}

struct Sink;

impl TextMetrics for Sink {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::ZERO
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Sink {
    fn size(&self) -> Size {
        Size::new(200.0, 100.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
fn invalidation_handle_is_send_sync() {
    // 编译期断言（与 invalidate.rs 里的 const 断言同义，这里再固化一次运行时可见性）
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<y901_ui::InvalidationHandle>();

    // 句柄必须能真的移动进线程
    let tree = tree_with_two_leaves();
    let handle = tree.invalidation_handle();
    let join = std::thread::spawn(move || {
        handle.invalidate("a");
    });
    join.join().expect("后台线程不应 panic");
}

#[test]
fn background_invalidate_marks_dirty_on_next_frame() {
    let mut tree = tree_with_two_leaves();
    // 先清空脏区：渲染一次（dirty 空时 render 直接返回，这里用 invalidate_all 制造确定性）
    tree.invalidate_all();
    let mut sink = Sink;
    tree.render(&mut sink);
    assert!(tree.dirty().is_empty(), "前置：脏区已清空");

    let handle = tree.invalidation_handle();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        handle.invalidate("a");
        tx.send(()).expect("发送完成信号");
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("后台线程应在 2s 内完成");

    let n = tree.drain_external_invalidations();
    assert_eq!(n, 1, "应有 1 条后台请求");
    assert!(!tree.dirty().is_empty(), "drain 之后必须有脏区");
}

#[test]
fn background_invalidate_all_marks_full_dirty() {
    let mut tree = tree_with_two_leaves();
    let handle = tree.invalidation_handle();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        handle.invalidate_all();
        tx.send(()).expect("发送完成信号");
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("后台线程完成");

    let n = tree.drain_external_invalidations();
    assert_eq!(n, 1);
    assert!(tree.dirty().is_full(), "invalidate_all 必须是整窗脏区");
}

#[test]
fn multiple_background_invalidations_merge() {
    let mut tree = tree_with_two_leaves();
    let handle = tree.invalidation_handle();

    let mut joins = Vec::new();
    for _ in 0..4 {
        let h = handle.clone();
        joins.push(std::thread::spawn(move || {
            h.invalidate("a");
            h.invalidate("b");
        }));
    }
    joins.push({
        let h = handle.clone();
        std::thread::spawn(move || h.invalidate_all())
    });
    for j in joins {
        j.join().expect("后台线程不应 panic");
    }

    let n = tree.drain_external_invalidations();
    assert!(n >= 9, "5 个线程共提交 9 条请求，实际 {n}");
    assert!(tree.dirty().is_full(), "含 All 时必须合并成整窗");
}

#[test]
fn woken_flag_clears_after_drain() {
    let mut tree = tree_with_two_leaves();
    let handle = tree.invalidation_handle();
    handle.invalidate("a");

    assert!(handle.has_pending(), "push 之后应有待处理标记");
    let first = tree.drain_external_invalidations();
    assert_eq!(first, 1);
    assert!(!handle.has_pending(), "drain 之后标记必须清零");

    let second = tree.drain_external_invalidations();
    assert_eq!(second, 0, "第二次 drain 必须为空");
}

#[test]
fn handle_after_tree_dropped_is_harmless() {
    // 句柄不持有树：树销毁后后台仍可 push，不 panic、不悬垂。
    let handle = {
        let tree = tree_with_two_leaves();
        tree.invalidation_handle()
    };
    handle.invalidate("ghost");
    assert!(handle.has_pending(), "孤儿句柄仍可接收请求");
    let drained = handle.drain();
    assert_eq!(drained.len(), 1);
}
// ---------- 审计 #7：单个节点的后台标脏不得升级成整窗 ----------

/// `One(id)` 只标**该节点**的矩形，绝不能变成整窗重绘。
///
/// 旧实现把 `One` 和 `All` 都升级成 `DirtyRegion::full()`（`whole_window` 是死变量），
/// 于是后台线程每来一条数据就是一次整窗重绘 + 整帧纹理上传。
#[test]
fn single_node_invalidation_does_not_become_a_whole_window_repaint() {
    let mut tree = tree_with_two_leaves();
    tree.invalidate_all();
    let mut sink = Sink;
    tree.render(&mut sink);
    assert!(tree.dirty().is_empty(), "前置：脏区已清空");

    let rect_a = tree.rect_of("a").expect("节点 a 已布局");
    assert!(!rect_a.is_empty(), "前置：a 有非空矩形");

    tree.invalidation_handle().invalidate("a");
    assert_eq!(tree.drain_external_invalidations(), 1);

    assert!(
        !tree.dirty().is_full(),
        "单个节点的后台标脏不得升级成整窗（审计 #7 的原始症状）"
    );
    let rects = tree.dirty().rects().to_vec();
    assert!(!rects.is_empty(), "该节点仍必须被标脏");
    // 允许墨迹外扩，所以只要求"不超出节点矩形太多"：用包含关系反向检查。
    let union = rects.iter().fold(rects[0], |acc, r| acc.union(*r));
    assert!(
        union.w <= rect_a.w + 64.0 && union.h <= rect_a.h + 64.0,
        "脏区应围绕该节点（允许墨迹外扩），实际 {union:?} vs 节点 {rect_a:?}"
    );
}

/// `One(id)` 必须请求本帧重排：后台改的是数据，尺寸可能变了。
#[test]
fn single_node_invalidation_requests_a_relayout() {
    let mut tree = tree_with_two_leaves();
    // 清掉建树时的"需要布局"标志。
    let _ = tree.take_layout_request();
    tree.invalidation_handle().invalidate("b");
    assert_eq!(tree.drain_external_invalidations(), 1);
    assert!(
        tree.take_layout_request(),
        "后台标脏后必须请求重排，否则尺寸变化要等下一次别的事件才生效"
    );
}

/// 同帧里既有 `One` 又有 `All`：合并成整窗（取最强，不能漏块）。
#[test]
fn all_wins_over_one_in_the_same_batch() {
    let mut tree = tree_with_two_leaves();
    let h = tree.invalidation_handle();
    h.invalidate("a");
    h.invalidate_all();
    assert_eq!(tree.drain_external_invalidations(), 2);
    assert!(tree.dirty().is_full(), "含 All 时必须整窗");
}

/// 不存在的 id：不标脏、不请求重排（与 `UiTree::invalidate` 的静默忽略一致）。
#[test]
fn unknown_id_is_ignored_without_forcing_a_repaint() {
    let mut tree = tree_with_two_leaves();
    tree.invalidate_all();
    let mut sink = Sink;
    tree.render(&mut sink);
    let _ = tree.take_layout_request();

    tree.invalidation_handle().invalidate("ghost");
    let n = tree.drain_external_invalidations();
    assert_eq!(n, 1, "请求本身要被计数（调用方靠它判断'有没有后台活'）");
    assert!(
        tree.dirty().is_empty(),
        "不存在的节点不该产生脏区（否则每次僵尸请求都要重绘）"
    );
}
