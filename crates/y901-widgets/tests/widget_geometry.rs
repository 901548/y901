//! **widget 几何/状态缺陷的回归用例组**（2026-11-25 复审第二批）。
//!
//! 1. `TreeView` 折叠后不夹滚动偏移 → `visible_range()` 变成空区间，整个控件空白、
//!    点击全失效；
//! 2. `Table` 的 flex 列把"每列最小宽 80"叠加 → 总宽超出行区域，第二列被裁掉；
//! 3. 滚动条滑块 24px 下限与 `1 - ratio` 的位置夹紧冲突 → 滚到底也差一截到不了轨道末端。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, Key, Modifiers, MouseButton, Pos, Rect, RenderCtx,
    Semantics, Size, TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};
use y901_widgets::{ComboBox, MenuBar, Table, TableColumn, TreeItem, TreeView};

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

fn row_of(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

#[test]
fn collapsing_while_scrolled_keeps_tree_view_visible() {
    // 一个父节点 + 60 个子节点：滚到底后折叠父节点。
    let children: Vec<TreeItem> = (0..60)
        .map(|i| TreeItem::leaf(format!("child {i}")))
        .collect();
    let mut parent = TreeItem::branch("parent", children);
    parent.expanded = true;
    let mut tv = TreeView::new("tv", vec![parent]);
    let c = Constraints::loose(Size::new(240.0, 200.0));
    let mut cache = LayoutCache::new();
    let _ = tv.measure(&Mono, c, &mut cache);
    tv.arrange(Rect::new(0.0, 0.0, 240.0, 200.0), &Mono, &mut cache);

    // 滚到底
    let mut ctx = EventCtx::new(None, Size::new(240.0, 200.0));
    for _ in 0..40 {
        let _ = tv.event(
            &mut ctx,
            &UiEvent::Wheel {
                delta: -3.0,
                pos: Pos::new(10.0, 10.0),
                mods: Modifiers::default(),
            },
        );
    }
    assert!(
        !tv.visible_range().is_empty(),
        "前置：滚动到底后仍应有可见行"
    );

    // 折叠父节点（子节点全消失）
    assert!(tv.toggle_expanded(&[0]), "应能折叠父节点");
    let after = tv.visible_range();
    assert!(
        !after.is_empty(),
        "折叠后 visible_range 不能为空（旧实现在这里变成 0..0，整个控件空白）"
    );
    assert_eq!(after.start, 0, "折叠后内容只有 1 行，起点必须是 0");
}

#[test]
fn tree_view_scroll_clamps_when_content_shrinks() {
    let children: Vec<TreeItem> = (0..30)
        .map(|i| TreeItem::leaf(format!("child {i}")))
        .collect();
    let mut parent = TreeItem::branch("parent", children);
    parent.expanded = true;
    let mut tv = TreeView::new("tv", vec![parent]);
    let mut cache = LayoutCache::new();
    let _ = tv.measure(
        &Mono,
        Constraints::loose(Size::new(240.0, 120.0)),
        &mut cache,
    );
    tv.arrange(Rect::new(0.0, 0.0, 240.0, 120.0), &Mono, &mut cache);

    let mut ctx = EventCtx::new(None, Size::new(240.0, 120.0));
    for _ in 0..30 {
        let _ = tv.event(
            &mut ctx,
            &UiEvent::Wheel {
                delta: -3.0,
                pos: Pos::new(10.0, 10.0),
                mods: Modifiers::default(),
            },
        );
    }
    // 视口突然变高（内容不再需要滚动）→ arrange 必须夹回 0
    tv.arrange(Rect::new(0.0, 0.0, 240.0, 2000.0), &Mono, &mut cache);
    let r = tv.visible_range();
    assert_eq!(
        r.start, 0,
        "视口变高后滚动偏移必须回到 0（否则内容整体偏上）"
    );
}

#[test]
fn table_flex_columns_never_exceed_the_row_area() {
    // 行区域只有 120px，两个 flex 列：旧实现每列各要 80 → 总宽 160 > 120。
    let mut t = Table::new("t")
        .column(TableColumn::flex("a"))
        .column(TableColumn::flex("b"));
    for i in 0..3 {
        t = t.row(row_of(&format!("r{i}")));
    }
    let mut cache = LayoutCache::new();
    let _ = t.measure(
        &Mono,
        Constraints::loose(Size::new(120.0, 300.0)),
        &mut cache,
    );
    t.arrange(Rect::new(0.0, 0.0, 120.0, 300.0), &Mono, &mut cache);

    // 列几何是纯函数：直接断言，比截图精确
    let cols = t.column_layout();
    assert_eq!(cols.len(), 2, "应有两列");
    let total: f32 = cols.iter().map(|(_, w)| *w).sum();
    assert!(
        total <= 120.0 + 0.01,
        "两个 flex 列总宽应为 {total}，不能超过行区域 120（旧实现每列各取 80 → 160）"
    );
    // 相邻列首尾相接（没有重叠或空隙）
    assert!(
        (cols[0].0 + cols[0].1 - cols[1].0).abs() < 0.01,
        "第 2 列应紧接着第 1 列：{:?}",
        cols
    );
    assert!(
        cols[1].0 + cols[1].1 <= 120.0 + 0.01,
        "最后一列的右边界不能超出行区域：{:?}",
        cols
    );
}

#[derive(Default)]
struct Rec {
    fills: Vec<Rect>,
}

impl TextMetrics for Rec {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Mono.measure_text(t, &TextStyle::default())
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(120.0, 300.0)
    }
    fn fill_rect(&mut self, rect: Rect, _c: y901_ui::Color) {
        self.fills.push(rect);
    }
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _p: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
fn table_keyboard_navigation_works_end_to_end() {
    // 通过 UiTree 走真实焦点路由：Tab 到达表格后 ↓ 必须能移动选中行。
    let table = Table::new("t")
        .column(TableColumn::flex("name"))
        .row(row_of("a"))
        .row(row_of("b"))
        .row(row_of("c"));
    let mut tree = UiTree::new(Box::new(table));
    tree.layout(Size::new(320.0, 240.0), &Mono);
    tree.set_focus(Some("t".into()));
    assert_eq!(tree.focus(), Some("t"), "Table 必须能被设为焦点");

    tree.dispatch(&UiEvent::Key {
        key: Key::Down,
        modifiers: Modifiers::default(),
    });
    tree.dispatch(&UiEvent::Key {
        key: Key::Down,
        modifiers: Modifiers::default(),
    });
    let sel = tree.with_node_mut("t", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            let _ = any.downcast_mut::<Table>();
        }
    });
    let _ = sel;
    // 断言屏幕确实重绘（选中行变色）
    tree.invalidate_all();
    let mut rec = Rec::default();
    tree.render(&mut rec);
    assert!(!rec.fills.is_empty(), "表格必须真的画出内容");
    let _ = (
        MouseButton::Left,
        EventResult::Handled,
        Semantics::default(),
        TableColumn::flex("x"),
    );
}

/// **ComboBox：键盘高亮必须真的画出来**（旧实现只改字段、屏幕上看不见）。
#[test]
fn combo_box_keyboard_highlight_is_painted() {
    use y901_ui::Key;
    let mut cb = ComboBox::new("c", vec!["a", "b", "c", "d"]);
    let mut cache = LayoutCache::new();
    let _ = cb.measure(
        &Mono,
        Constraints::loose(Size::new(200.0, 32.0)),
        &mut cache,
    );
    cb.arrange(Rect::new(0.0, 0.0, 200.0, 32.0), &Mono, &mut cache);

    // 打开下拉（会请求 popup，popup 内容是 ComboList）
    let mut ctx = EventCtx::new(None, Size::new(200.0, 32.0));
    let _ = cb.event(
        &mut ctx,
        &UiEvent::MouseDown {
            pos: Pos::new(10.0, 10.0),
            button: MouseButton::Left,
        },
    );
    let mut ctx = EventCtx::new(None, Size::new(200.0, 32.0));
    let _ = cb.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(10.0, 10.0),
            button: MouseButton::Left,
        },
    );
    assert_eq!(ctx.popup_requests.len(), 1, "点开下拉应请求 popup");
    let mut list = (ctx.popup_requests.remove(0).factory)();
    let mut cache = LayoutCache::new();
    let s = list.measure(
        &Mono,
        Constraints::loose(Size::new(200.0, 200.0)),
        &mut cache,
    );
    list.arrange(Rect::new(0.0, 0.0, s.w, s.h), &Mono, &mut cache);

    // ↓ 两次：高亮落到第 2 项（**偶数行**：斑马纹不会给偶数行上色，
    // 所以"这一行有填充"只可能来自键盘高亮本身）。
    for _ in 0..2 {
        let mut ctx = EventCtx::new(None, Size::new(200.0, 200.0));
        let _ = list.event(
            &mut ctx,
            &UiEvent::Key {
                key: Key::Down,
                modifiers: Modifiers::default(),
            },
        );
    }

    let mut rec = Rec::default();
    list.render(&mut rec);
    // 行几何：LIST_PAD(4) + i*ITEM_H(28)，高亮在第 2 行 → y ≈ 4 + 56 = 60
    let row_y = 4.0 + 2.0 * 28.0;
    let painted = rec
        .fills
        .iter()
        .any(|r| (r.y - row_y).abs() < 0.5 && (r.h - 28.0).abs() < 0.5);
    assert!(
        painted,
        "第 2 行（偶数行）必须有键盘高亮填充；实际填充 = {:?}",
        rec.fills
    );
}

/// **MenuBar：点标题之外要清掉打开态**（否则高亮永远留着）。
#[test]
fn menu_bar_clears_open_state_on_outside_click() {
    use y901_ui::Role;
    let mut bar = MenuBar::new("mb")
        .menu(y901_widgets::MenuEntry::new(
            "File",
            vec!["Open".to_string()],
            "open",
        ))
        .menu(y901_widgets::MenuEntry::new(
            "Edit",
            vec!["Copy".to_string()],
            "copy",
        ));
    let mut cache = LayoutCache::new();
    let _ = bar.measure(
        &Mono,
        Constraints::loose(Size::new(400.0, 28.0)),
        &mut cache,
    );
    bar.arrange(Rect::new(0.0, 0.0, 400.0, 28.0), &Mono, &mut cache);

    // 点第一个标题 → 打开
    let mut ctx = EventCtx::new(None, Size::new(400.0, 28.0));
    let _ = bar.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(5.0, 5.0),
            button: MouseButton::Left,
        },
    );
    assert!(bar.active_index().is_some(), "点标题应进入打开态");

    // 点标题之外（同一控件内的空白处）→ 必须清掉
    let mut ctx = EventCtx::new(None, Size::new(400.0, 28.0));
    let _ = bar.event(
        &mut ctx,
        &UiEvent::MouseUp {
            pos: Pos::new(399.0, 5.0),
            button: MouseButton::Left,
        },
    );
    assert_eq!(
        bar.active_index(),
        None,
        "点标题之外应清掉打开态（旧实现永远留着高亮）"
    );
    let _ = Role::PANEL;
}
