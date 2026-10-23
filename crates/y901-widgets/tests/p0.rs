//! P0 四项：自动订阅 / insert_node / 独立 Scrollbar / 列表选中。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Constraints, Flex, FlexItem, Pos, Rect, Signal, Size, TextStyle, TextMetrics, UiEvent, UiTree,
    Widget,
};
use y901_widgets::{ListView, Scrollbar, TextLabel};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

// ---------- P0-1：bind 即自动订阅 ----------

#[test]
fn bound_label_subscribes_on_mount_without_manual_call() {
    let title = Signal::new("a".to_string());
    let label = TextLabel::new("title", "a").bind(title.clone());

    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 挂载（内部会调 on_mount → 自动 subscribe）
    assert!(tree.add_node("root", Box::new(label)));
    assert_eq!(title.subscriber_count(), 1, "挂载应自动订阅");

    // 业务代码只改数据，不做任何订阅/标脏
    title.set("b".to_string());
    assert_eq!(tree.drain_invalidation(), 1, "应产生一条失效");

    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.snapshot_json().to_string().contains('b'));
}

#[test]
fn unmounted_label_unsubscribes_automatically() {
    let title = Signal::new("x".to_string());
    let label = TextLabel::new("title", "x").bind(title.clone());

    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.add_node("root", Box::new(label));
    assert_eq!(title.subscriber_count(), 1);

    tree.remove_node("title");
    assert_eq!(title.subscriber_count(), 0, "卸载应自动退订");

    title.set("y".to_string());
    assert_eq!(tree.drain_invalidation(), 0);
}

// ---------- P0-2：insert_node ----------

struct Fixed {
    id: String,
    rect: Rect,
}

impl Widget for Fixed {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _c: &mut y901_ui::layout::LayoutCache) -> Size {
        c.constrain(Size::new(c.max.w.min(100.0), 20.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

fn fixed(id: &str) -> Box<dyn Widget> {
    Box::new(Fixed {
        id: id.to_string(),
        rect: Rect::ZERO,
    })
}

fn order(tree: &UiTree) -> Vec<String> {
    ["a", "b", "c"]
        .iter()
        .filter_map(|id| tree.rect_of(id).map(|r| (*id, r.y)))
        .map(|(id, _)| id.to_string())
        .collect()
}

#[test]
fn insert_node_places_child_at_index() {
    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(fixed("a")).push(fixed("c")),
    ));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 插到中间（index=1）
    assert!(tree.insert_node("root", 1, fixed("b")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let a = tree.rect_of("a").unwrap();
    let b = tree.rect_of("b").unwrap();
    let c = tree.rect_of("c").unwrap();
    assert!(a.y < b.y && b.y < c.y, "顺序应为 a < b < c，实际 {a:?} {b:?} {c:?}");
}

#[test]
fn insert_node_clamps_out_of_range_index() {
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(fixed("a"))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    // 越界 → 追加到末尾
    assert!(tree.insert_node("root", 99, fixed("z")));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let a = tree.rect_of("a").unwrap();
    let z = tree.rect_of("z").unwrap();
    assert!(z.y > a.y, "越界插入应落到末尾");
}

// ---------- P0-3：独立 Scrollbar ----------

#[test]
fn standalone_scrollbar_reports_new_offset() {
    let seen = Rc::new(Cell::new(-1.0f32));
    let cb = seen.clone();

    let mut bar = Scrollbar::new("bar", "list")
        .tracker(1000.0, 100.0, 0.0)
        .on_change(move |offset| cb.set(offset));

    // 手动赋几何（独立使用时不进布局也可以）
    let mut cache = y901_ui::layout::LayoutCache::new();
    bar.measure(&Mono, Constraints::loose(Size::new(10.0, 100.0)), &mut cache);
    bar.arrange(Rect::new(0.0, 0.0, 10.0, 100.0), &Mono, &mut cache);

    let mut ctx = y901_ui::EventCtx::new(None, Size::new(400.0, 300.0));

    // 按在轨道中部并拖动
    bar.event(
        &mut ctx,
        &UiEvent::MouseDown {
            pos: Pos::new(5.0, 50.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    bar.event(&mut ctx, &UiEvent::MouseMove { pos: Pos::new(5.0, 80.0) });

    let got = seen.get();
    assert!(got > 0.0, "回调应收到新 offset，实际 {got}");
    assert!(got <= 900.0, "offset 不应超过内容高 - 视口高，实际 {got}");
    assert!((got - bar.offset()).abs() < 0.01, "回调值与内部状态一致");
}

// ---------- P0-4：ListView 选中 ----------

#[test]
fn click_selects_row_and_fires_callback() {
    let picked = Rc::new(Cell::new(usize::MAX));
    let cb = picked.clone();

    let items: Vec<String> = (0..20).map(|i| format!("row {i}")).collect();
    let list = ListView::new("list", 20.0)
        .items(items)
        .on_row_selected(move |i| cb.set(i));

    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(
        list.flex_item(FlexItem::fill()),
    ))));
    tree.layout(Size::new(300.0, 200.0), &Mono);

    // 第 5 行：y = 5*20 + 10
    tree.dispatch_to(
        "list",
        &UiEvent::MouseDown {
            pos: Pos::new(20.0, 110.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    tree.layout(Size::new(300.0, 200.0), &Mono);

    assert_eq!(picked.get(), 5, "回调应收到第 5 行");
    let snap = tree.snapshot_json().to_string();
    assert!(snap.contains("\"selected\":5"), "语义树应暴露选中行：{snap}");
}
