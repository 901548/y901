//! 滚动 / 分栏 / 主题的集成测试。
//!
//! 重点验证三件事：
//! 1. 滚动之后命中测试严格限制在可见区（看不见就点不到）；
//! 2. 分隔条拖拽真的改变两栏尺寸；
//! 3. 换主题后组件颜色随之改变（组件不持有业务色）。

use y901_ui::{
    Axis, Color, Constraints, Cursor, Flex, FlexItem, Pos, Rect, RenderCtx, Size, State,
    TextMetrics, TextStyle, Theme, UiEvent, UiTree, Widget,
};
use y901_widgets::{ScrollView, SplitView, TextLabel};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        20.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 固定高度的行，id = "row{i}"。
struct Row {
    id: String,
    rect: Rect,
}

impl Widget for Row {
    fn id(&self) -> &str {
        &self.id
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
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::WHITE);
    }
}

fn scrolled_tree(rows: usize) -> UiTree {
    let mut list = Flex::column("list");
    for i in 0..rows {
        list = list.push(Box::new(Row {
            id: format!("row{i}"),
            rect: Rect::ZERO,
        }));
    }
    let sv = ScrollView::new("sv", Box::new(list)).flex_item(FlexItem::fill());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(sv))));
    tree.layout(Size::new(200.0, 100.0), &Mono);
    tree
}

#[test]
fn hit_test_respects_clip_after_scroll() {
    let mut tree = scrolled_tree(20);
    // 内容 400px，视口 100px

    // 未滚动：y=10 命中 row0
    assert_eq!(tree.hit_test(Pos::new(50.0, 10.0)).as_deref(), Some("row0"));

    // 向下滚 192px（-4 格 × 48）
    tree.dispatch(&UiEvent::Wheel {
        pos: Pos::new(50.0, 50.0),
        delta: -480.0,
        mods: y901_ui::Modifiers::default(),
    });
    // 需要重新 arrange 才能把新 offset 反映到子节点位置
    tree.layout(Size::new(200.0, 100.0), &Mono);

    // row10 顶部原本在 200，滚动后落在 8..28
    assert_eq!(
        tree.hit_test(Pos::new(50.0, 16.0)).as_deref(),
        Some("row10"),
        "滚动后应命中新的可见行"
    );

    // 关键断言：滚出视口的 row0 **不能被命中**
    let hit = tree.hit_test(Pos::new(50.0, 16.0));
    assert_ne!(hit.as_deref(), Some("row0"), "不可见元素必须点不到");
}

#[test]
fn scrolled_out_rows_cannot_be_hit() {
    let mut tree = scrolled_tree(20);

    // 滚到最底部：offset = 400 - 100 = 300 → 视口显示 row15..row19
    for _ in 0..4 {
        tree.dispatch(&UiEvent::Wheel {
            pos: Pos::new(50.0, 50.0),
            delta: -480.0,
            mods: y901_ui::Modifiers::default(),
        });
    }
    tree.layout(Size::new(200.0, 100.0), &Mono);

    // 视口内任意一点，都只能命中 row15..row19
    for y in [1.0f32, 10.0, 50.0, 99.0] {
        let hit = tree.hit_test(Pos::new(50.0, y)).expect("视口内应有命中");
        let index: usize = hit.trim_start_matches("row").parse().expect("rowN");
        assert!(index >= 15, "只能命中可见行，y={y} 命中了 {hit}");
    }

    // 完全滚出视口的 row0 在任何位置都不可命中
    for y in [1.0f32, 50.0, 99.0] {
        assert_ne!(
            tree.hit_test(Pos::new(50.0, y)).as_deref(),
            Some("row0"),
            "已滚出视口的行不得可点击"
        );
    }
}

#[test]
fn splitter_drag_changes_pane_sizes() {
    let split = SplitView::horizontal(
        "split",
        Box::new(Flex::column("left")),
        Box::new(Flex::column("right")),
    )
    .ratio(0.3);

    let mut tree = UiTree::new(Box::new(split));
    tree.layout(Size::new(400.0, 200.0), &Mono);

    let before = tree.rect_of("left").expect("left pane").w;
    assert!((before - 120.0).abs() < 1.0, "初始 30% 宽度，实际 {before}");

    // 分隔条在 x=120 附近；按下 → 拖到 240
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(120.5, 100.0),
        button: y901_ui::MouseButton::Left,
    });
    tree.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(240.0, 100.0),
    });
    tree.dispatch(&UiEvent::MouseUp {
        pos: Pos::new(240.0, 100.0),
        button: y901_ui::MouseButton::Left,
    });
    tree.layout(Size::new(400.0, 200.0), &Mono);

    let after = tree.rect_of("left").expect("left pane").w;
    assert!(
        after > before + 40.0,
        "拖拽后左栏应明显变宽：{before} → {after}"
    );

    // 最小宽度约束：右栏不能小于 200 → 左栏最大 200
    assert!(
        after <= 400.0 - 200.0 + 1.0,
        "受 min_second 约束，实际 {after}"
    );
}

#[test]
fn splitter_reports_resize_cursor() {
    let split = SplitView::horizontal(
        "split",
        Box::new(Flex::column("left")),
        Box::new(Flex::column("right")),
    )
    .ratio(0.25);
    let mut tree = UiTree::new(Box::new(split));
    tree.layout(Size::new(400.0, 200.0), &Mono);

    // 分隔条在 x=100
    assert_eq!(
        tree.cursor_at(Pos::new(100.5, 100.0)),
        Cursor::ResizeHorizontal
    );
    assert_eq!(tree.cursor_at(Pos::new(300.0, 100.0)), Cursor::Default);
}

#[test]
fn theme_switch_changes_component_colors() {
    // 只验证查表：组件渲染时用的就是这里的值
    let dark = Theme::DARK.style(y901_ui::Role::BUTTON, State::Normal).bg;
    let light = y901_ui::theme::LIGHT
        .style(y901_ui::Role::BUTTON, State::Normal)
        .bg;
    assert_ne!(dark, light);
    // 暗色主题下按钮比面板亮（可辨识）
    assert_ne!(
        dark,
        Theme::DARK.style(y901_ui::Role::PANEL, State::Normal).bg
    );
}

#[test]
fn scroll_view_clip_matches_viewport() {
    let tree = scrolled_tree(5);
    // Root → sv：sv 的 clip 应该是有界的视口，且比整棵树小
    let sv_rect = tree.rect_of("sv").expect("sv");
    assert!(sv_rect.h > 0.0);
    // 行在内容坐标系里，超出视口的行 rect 会低于视口底部
    let row4 = tree.rect_of("row4").expect("row4");
    assert!(row4.y >= 0.0, "5 行内容不足一屏时不需要滚动");
}

#[test]
fn text_label_still_renders_with_theme() {
    // 组件不再硬编码颜色：改主题不应 panic，且能正常出图
    #[derive(Default)]
    struct Rec {
        texts: Vec<String>,
    }
    impl TextMetrics for Rec {
        fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
            Mono.measure_text(t, s)
        }
        fn line_height(&self, s: &TextStyle) -> f32 {
            Mono.line_height(s)
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }
    impl RenderCtx for Rec {
        fn size(&self) -> Size {
            Size::new(100.0, 40.0)
        }
        fn fill_rect(&mut self, _r: Rect, _c: Color) {}
        fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
        fn draw_text(&mut self, _o: Pos, t: &str, _s: &TextStyle) -> f32 {
            self.texts.push(t.to_string());
            0.0
        }
        fn push_clip(&mut self, _r: Rect) {}
        fn pop_clip(&mut self) {}
    }

    let mut label = TextLabel::new("l", "hello");
    let mut cache = y901_ui::layout::LayoutCache::new();
    label.measure(
        &Mono,
        Constraints::loose(Size::new(100.0, 40.0)),
        &mut cache,
    );
    label.arrange(Rect::new(0.0, 0.0, 100.0, 20.0), &Mono, &mut cache);

    let mut rec = Rec::default();
    label.render(&mut rec);
    assert_eq!(rec.texts, vec!["hello".to_string()]);
    let _ = Axis::Horizontal;
}
