//! 审计回归：证伪"裁剪已生效""挂载已触发"等声明。
//!
//! 这些测试的价值不在于覆盖新功能，而在于**如果代码被回滚，它们会立刻失败**。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, FlexItem, Pos, Rect, RenderCtx, Size,
    TextMetrics, TextStyle, UiTree, Widget,
};

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

/// 记录 push_clip / pop_clip / 绘制次数的后端。
#[derive(Default)]
struct ClipRecorder {
    clips: Vec<Rect>,
    depth: usize,
    max_depth: usize,
    paints: usize,
}

impl TextMetrics for ClipRecorder {
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

impl RenderCtx for ClipRecorder {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {
        self.paints += 1;
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        self.paints += 1;
        0.0
    }
    fn push_clip(&mut self, r: Rect) {
        self.clips.push(r);
        self.depth += 1;
        self.max_depth = self.max_depth.max(self.depth);
    }
    fn pop_clip(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}

struct Box3 {
    id: String,
    rect: Rect,
    paints: Rc<RefCell<usize>>,
}

impl Widget for Box3 {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 40.0))
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
        *self.paints.borrow_mut() += 1;
        ctx.fill_rect(self.rect, Color::WHITE);
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default().exposed(true)
    }
}

/// 1. 渲染时框架必须为 ScrollView 的视口压裁剪栈。
#[test]
fn render_pushes_clip_for_scrolling_container() {
    let paints = Rc::new(RefCell::new(0usize));
    let mut column = Flex::column("content");
    for i in 0..10 {
        column = column.push(Box::new(Box3 {
            id: format!("row{i}"),
            rect: Rect::ZERO,
            paints: paints.clone(),
        }));
    }

    let sv = y901_widgets::ScrollView::new("sv", Box::new(column)).flex_item(FlexItem::fill());
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(sv))));
    tree.layout(Size::new(300.0, 120.0), &Mono);

    let mut rec = ClipRecorder::default();
    tree.render(&mut rec);

    assert!(
        !rec.clips.is_empty(),
        "渲染必须为滚动容器压裁剪栈（否则子节点会画到视口外）"
    );
    let inner = rec.clips[0];
    assert!(
        (inner.w - 290.0).abs() < 1.0,
        "裁剪宽度应扣除滚动条：{inner:?}"
    );
    assert!(
        (inner.h - 120.0).abs() < 1.0,
        "裁剪高度应等于视口：{inner:?}"
    );
    assert_eq!(
        rec.max_depth, 1,
        "ScrollView 只应压一层裁剪，实际 {}",
        rec.max_depth
    );
    assert_eq!(rec.depth, 0, "push/pop 必须配平，实际残留 {}", rec.depth);
}

/// 2. 关闭的浮层既不能画、也不能被命中。
#[test]
fn closed_overlay_is_neither_rendered_nor_hittable() {
    let menu = y901_widgets::Menu::new("menu").items(vec!["A".to_string()]);
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    assert!(tree.rect_of("menu").is_none(), "关闭时不应进索引");

    let mut rec = ClipRecorder::default();
    tree.render(&mut rec);
    assert_eq!(rec.paints, 0, "关闭的浮层不应产生任何绘制");

    // 打开后可见
    let mut menu = y901_widgets::Menu::new("menu").items(vec!["A".to_string()]);
    menu.open();
    let mut tree = UiTree::new(Box::new(Flex::column("root").push(Box::new(menu))));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    assert!(tree.rect_of("menu").is_some(), "打开后应进索引");
}

/// 3. 默认封禁：普通 Widget 的 semantics 不会被 AI 看到。
#[test]
fn default_semantics_are_not_exposed() {
    assert!(
        !y901_ui::Semantics::default().exposed_to_ai,
        "Semantics::default() 必须默认封禁"
    );
}

/// 4. 初始树递归挂载：on_mount 会被调用。
#[test]
fn initial_tree_triggers_on_mount_recursively() {
    struct Counter {
        id: String,
        log: Rc<RefCell<Vec<String>>>,
    }
    impl Widget for Counter {
        fn id(&self) -> &str {
            &self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(10.0, 10.0))
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
        fn render(&mut self, _c: &mut dyn RenderCtx) {}
        fn on_mount(&mut self, _ctx: &mut y901_ui::MountCtx) {
            self.log.borrow_mut().push(self.id.clone());
        }
        fn semantics(&self) -> y901_ui::Semantics {
            y901_ui::Semantics::default()
        }
    }

    let log = Rc::new(RefCell::new(Vec::new()));
    let root = Flex::column("root").push(Box::new(Counter {
        id: "a".into(),
        log: log.clone(),
    }));
    let _tree = UiTree::new(Box::new(root));

    assert!(
        log.borrow().contains(&"a".to_string()),
        "构造 UiTree 时必须递归触发 on_mount，实际 {:?}",
        log.borrow()
    );
}

/// 5. ListView 事件顺序：点滚动条不得选中行。
#[test]
fn listview_bar_branch_precedes_row_click() {
    use y901_widgets::ListView;

    let picked = Rc::new(RefCell::new(None));
    let p = picked.clone();
    let items: Vec<String> = (0..100).map(|i| format!("r{i}")).collect();
    let list =
        ListView::from_strings("list", items).on_row_selected(move |i| *p.borrow_mut() = Some(i));

    let mut tree = UiTree::new(Box::new(
        Flex::column("root").push(Box::new(list.flex_item(FlexItem::fill()))),
    ));
    tree.layout(Size::new(300.0, 200.0), &Mono);

    tree.dispatch_to(
        "list",
        &y901_ui::UiEvent::MouseDown {
            pos: Pos::new(296.0, 100.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    assert!(picked.borrow().is_none(), "点滚动条不应触发行选中");
}
