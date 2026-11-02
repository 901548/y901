//! 文件拖放事件的路由：按坐标命中目标控件。

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use y901_ui::{
    Constraints, EventCtx, EventResult, FlexItem, Pos, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiEvent, UiTree, Widget,
};

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

/// 只关心"有没有收到文件拖放"的探针控件。
struct DropZone {
    hits: Rc<Cell<u32>>,
    last_names: Rc<std::cell::RefCell<Vec<String>>>,
    rect: Rect,
}

impl Widget for DropZone {
    fn id(&self) -> &str {
        "drop"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 100.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, e: &UiEvent) -> EventResult {
        match e {
            UiEvent::FileDrop { paths, .. } => {
                self.hits.set(self.hits.get() + 1);
                let mut names = self.last_names.borrow_mut();
                names.clear();
                names.extend(
                    paths
                        .iter()
                        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()),
                );
                EventResult::Handled
            }
            _ => EventResult::Ignored,
        }
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 探针收到的文件名列表。
type Names = Rc<std::cell::RefCell<Vec<String>>>;

fn tree() -> (UiTree, Rc<Cell<u32>>, Names) {
    let hits = Rc::new(Cell::new(0));
    let names = Rc::new(std::cell::RefCell::new(Vec::new()));
    let zone = DropZone {
        hits: hits.clone(),
        last_names: names.clone(),
        rect: Rect::ZERO,
    };
    // 放在一个列容器里（只占顶部 100px），这样才有"控件外"的区域
    let root = y901_ui::Flex::column("root").push(Box::new(zone));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    (tree, hits, names)
}

#[test]
fn file_drop_routes_to_control_under_cursor() {
    let (mut tree, hits, names) = tree();
    let r = tree.rect_of("drop").expect("drop 必须有矩形");
    assert!(r.h > 0.0);

    let ev = UiEvent::FileDrop {
        paths: vec![PathBuf::from(r"C:\a\one.txt"), PathBuf::from(r"C:\a\two.png")],
        pos: Pos::new(r.x + 10.0, r.y + 10.0),
    };
    let result = tree.dispatch(&ev);
    assert!(result.is_handled(), "拖放应被目标控件接管");
    assert_eq!(hits.get(), 1, "控件必须收到一次 FileDrop");
    assert_eq!(*names.borrow(), vec!["one.txt", "two.png"]);
}

#[test]
fn file_drop_outside_control_is_ignored() {
    let (mut tree, hits, _) = tree();
    let ev = UiEvent::FileDrop {
        paths: vec![PathBuf::from(r"C:\x\y.txt")],
        pos: Pos::new(390.0, 290.0), // 控件之外
    };
    let _ = tree.dispatch(&ev);
    assert_eq!(hits.get(), 0, "坐标落在控件外时不应派发");
}

#[test]
fn file_drop_marks_redraw_when_target_requests_it() {
    // 探针返回 Handled（不请求重绘）：脏区不该凭空出现
    let (mut tree, _, _) = tree();
    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty());

    let r = tree.rect_of("drop").expect("drop");
    let _ = tree.dispatch(&UiEvent::FileDrop {
        paths: vec![PathBuf::from("x")],
        pos: Pos::new(r.x + 1.0, r.y + 1.0),
    });
    assert!(
        tree.dirty().is_empty(),
        "控件返回 Handled 而不是 Redraw 时，框架不该标脏"
    );
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

/// 让未使用的导入不告警。
// 测试 API：测试内的类型见证/未用常量，仅测试代码引用
#[allow(dead_code)]
fn _unused(_f: FlexItem) {}
