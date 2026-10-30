//! 滚动残影：`ListView` 偏移变化必须把**整个视口**标脏。
//!
//! 背景：`ScrollView`/`ListView` 的偏移在**渲染/命中阶段**应用，
//! 子节点 rect 不变，所以 `UiTree` 的"布局前后 rect 对比"抓不到位移。
//! 若只标子节点，滚出去的旧内容会留在屏幕上（残影）。
//!
//! 本测试不依赖 GPU：检查滚动后脏区是否覆盖 ListView 的视口矩形。

use y901_ui::{
    Flex, FlexItem, Modifiers, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle, UiEvent, UiTree,
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

/// 只用来清空脏区的空渲染上下文。
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
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

fn build() -> UiTree {
    let items: Vec<String> = (0..100).map(|i| format!("row {i}")).collect();
    let list = ListView::new("list", 20.0)
        .items(items)
        .flex_item(FlexItem::height(200.0));
    let root = Flex::column("root").push(Box::new(list));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree
}

/// 脏区是否覆盖整个视口（整窗脏也算覆盖）。
fn covers_viewport(tree: &UiTree, viewport: Rect) -> bool {
    if tree.dirty().is_full() {
        return true;
    }
    match tree.dirty().bounds() {
        Some(b) => b.y <= viewport.y + 0.5 && b.bottom() >= viewport.bottom() - 0.5,
        None => false,
    }
}

#[test]
fn scroll_marks_whole_viewport() {
    let mut tree = build();
    let viewport = tree.rect_of("list").expect("list 必须存在");
    assert!(viewport.h > 0.0, "ListView 应有高度，实际 {viewport:?}");

    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty(), "前置：脏区已清空");

    // 在视口中心滚一格
    let pos = Pos::new(viewport.x + 10.0, viewport.y + 10.0);
    let r = tree.dispatch(&UiEvent::Wheel {
        delta: -60.0,
        pos,
        mods: Modifiers::default(),
    });
    assert!(r.needs_redraw(), "滚动必须要求重绘");

    // 滚动可能走两条路径：整窗脏（Relayout）或局部脏（Redraw）。
    // 不论哪条，都必须**覆盖整个视口**，否则滚出去的旧内容会留残影。
    assert!(
        covers_viewport(&tree, viewport),
        "滚动必须覆盖整个视口：viewport={viewport:?} full={} dirty={:?}",
        tree.dirty().is_full(),
        tree.dirty().bounds()
    );
}

#[test]
fn scroll_twice_keeps_viewport_dirty() {
    let mut tree = build();
    let viewport = tree.rect_of("list").expect("list 必须存在");
    let pos = Pos::new(viewport.x + 10.0, viewport.y + 10.0);

    for _ in 0..3 {
        tree.render(&mut Sink);
        let _ = tree.dispatch(&UiEvent::Wheel {
            delta: -60.0,
            pos,
            mods: Modifiers::default(),
        });
        assert!(
            covers_viewport(&tree, viewport),
            "第 n 次滚动也要覆盖整个视口：viewport={viewport:?} full={}",
            tree.dirty().is_full()
        );
    }
}

/// 未滚动（事件被忽略）时不得凭空标脏。
#[test]
fn no_scroll_no_dirty() {
    let mut tree = build();
    tree.render(&mut Sink);
    let viewport = tree.rect_of("list").expect("list 必须存在");
    // 已经在顶部，再往上滚 → 偏移不变
    let _ = tree.dispatch(&UiEvent::Wheel {
        delta: 60.0,
        pos: Pos::new(viewport.x + 10.0, viewport.y + 10.0),
        mods: Modifiers::default(),
    });
    assert!(
        tree.dirty().is_empty(),
        "偏移没变时不应标脏，实际 {:?}",
        tree.dirty().bounds()
    );
}
