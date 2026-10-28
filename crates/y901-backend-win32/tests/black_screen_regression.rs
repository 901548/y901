//! 黑屏回归：没有脏区时不得 Present 空帧；系统要求重画时必须整窗重绘。
//!
//! 真实的 `Window::render` 需要 D3D 设备（本机无 GPU），所以这里测两件可测的事：
//!
//! 1. 重绘请求标志的语义（wndproc 与 runtime 之间的契约）；
//! 2. `UiTree::render` 在脏区为空时**一次绘制都不产生**（这是窗口级早退的前提）。
//!
//! 窗口级早退本身用结构断言守住（见 `window_render_has_dirty_guard`）。

use std::cell::Cell;
use std::rc::Rc;

use y901_backend_win32::window::{request_full_redraw, take_full_redraw};
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiTree, Widget,
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

/// 统计绘制调用次数。
struct Counting {
    fills: Rc<Cell<u32>>,
    texts: Rc<Cell<u32>>,
}

impl TextMetrics for Counting {
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

impl RenderCtx for Counting {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {
        self.fills.set(self.fills.get() + 1);
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, t: &str, s: &TextStyle) -> f32 {
        self.texts.set(self.texts.get() + 1);
        self.measure_text(t, s).w
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

struct Box0;

impl Widget for Box0 {
    fn id(&self) -> &str {
        "box"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 50.0))
    }
    fn arrange(&mut self, _r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {}
    fn rect(&self) -> Rect {
        Rect::new(0.0, 0.0, 100.0, 50.0)
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect(), Color::WHITE);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn tree_render_draws_nothing_when_clean() {
    let mut tree = UiTree::new(Box::new(Box0));
    tree.layout(Size::new(400.0, 300.0), &Mono);

    let fills = Rc::new(Cell::new(0));
    let texts = Rc::new(Cell::new(0));

    // 第一帧：有脏区（首帧 full），必须画
    {
        let mut ctx = Counting {
            fills: fills.clone(),
            texts: texts.clone(),
        };
        tree.render(&mut ctx);
    }
    let after_first = fills.get();
    assert!(after_first > 0, "首帧必须绘制，实际 {after_first}");
    assert!(tree.dirty().is_empty(), "渲染后脏区应清空");

    // 第二帧：没有脏区 —— 一次绘制都不该发生（否则窗口会 Present 空帧）
    {
        let mut ctx = Counting {
            fills: fills.clone(),
            texts: texts.clone(),
        };
        tree.render(&mut ctx);
    }
    assert_eq!(
        fills.get(),
        after_first,
        "脏区为空时不得再产生任何绘制调用（黑屏 bug 的根因）"
    );
    assert_eq!(texts.get(), 0, "这个探针不画文字");
}

#[test]
fn repaint_request_is_one_shot() {
    // 先清掉可能残留的标记（其它测试/静态状态）
    let _ = take_full_redraw();
    assert!(!take_full_redraw(), "初始应无请求");

    request_full_redraw();
    assert!(take_full_redraw(), "置位后第一次取走必须为 true");
    assert!(!take_full_redraw(), "取走后再取必须为 false（一次性）");

    // 多次置位等价于一次
    request_full_redraw();
    request_full_redraw();
    assert!(take_full_redraw());
    assert!(!take_full_redraw());
}

#[test]
fn window_render_has_dirty_guard() {
    // 结构断言：Window::render 必须在 begin() 之前判断脏区为空并早退。
    // 这是"黑屏"的核心修复点——删掉它整个 bug 会回来。
    let src = include_str!("../src/window.rs");
    let start = src
        .find("pub fn render(&mut self, tree: &mut UiTree)")
        .expect("Window::render 必须存在");
    let body = &src[start..];
    let end = body[1..]
        .find("\n    pub ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];

    assert!(
        body.contains("tree.dirty().is_empty()"),
        "Window::render 必须检查脏区：\n{body}"
    );
    assert!(
        body.find("tree.dirty().is_empty()").unwrap() < body.find("backend.begin()").unwrap(),
        "脏区检查必须在 begin() 之前（否则已经清屏了）"
    );
    assert!(
        body.contains("return;"),
        "脏区为空时必须早退（不 Present）"
    );
}

#[test]
fn wm_paint_requests_full_redraw() {
    // 结构断言：WM_PAINT 分支必须置重绘请求，不能只 BeginPaint/EndPaint。
    let src = include_str!("../src/window.rs");
    let start = src.find("WM_PAINT => {").expect("WM_PAINT 分支");
    let body = &src[start..];
    let end = body.find("\n        WM_DESTROY").unwrap_or(body.len());
    let body = &body[..end];
    assert!(
        body.contains("request_full_redraw()"),
        "WM_PAINT 必须请求整窗重绘（遮挡/最小化恢复时 D3D 内容是未定义的）：\n{body}"
    );
}

#[test]
fn runtime_consumes_repaint_request_before_render() {
    // 结构断言：runtime 必须在 window.render 之前消费请求并 invalidate_all。
    let src = include_str!("../src/runtime.rs");
    let repaint = src.find("take_repaint_request()").expect("runtime 应消费重绘请求");
    let render = src.find("window.render(&mut tree)").expect("runtime 应调用 render");
    assert!(repaint < render, "必须在 render 之前处理重绘请求");
    assert!(
        src[repaint..render].contains("invalidate_all()"),
        "处理重绘请求时必须 invalidate_all"
    );
}
