#![cfg(windows)]

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

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
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
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, 50.0))
    }
    fn arrange(
        &mut self,
        _r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
    }
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
fn window_render_has_dirty_guard() {
    // 结构性断言：防止"删掉脏区守卫"这类回归。
    // v0.1.12 起出图体在 `render_inner`（为基准拆出 sync 参数），
    // `render` 只剩委托一行。
    let src = include_str!("../src/window.rs");

    let start = src
        .find("fn render_inner(&mut self, tree: &mut UiTree")
        .expect("render_inner 定义未找到");
    let body = &src[start..];
    let end = body[1..]
        .find("\n    pub ")
        .map(|i| i + 1)
        .unwrap_or(body.len());
    let body = &body[..end];

    assert!(
        body.contains("tree.dirty().is_empty()"),
        "render_inner 必须检查脏区\n{body}"
    );
    assert!(
        body.find("tree.dirty().is_empty()").unwrap() < body.find("backend.begin()").unwrap(),
        "脏区检查必须在 begin() 之前"
    );
    // `render_inner` 现在返 `Result<(), String>`：脏区为空时的提前返回形式是 `return Ok(())`。
    // 断言意图不变（不 Present 就提前返回），只是跟随签名变化。
    assert!(
        body.contains("return Ok(())"),
        "脏区为空时提前返回（不 Present）：应为 return Ok(())
{body}"
    );
    // 两处都必须传 1：公开 `render`（打日志）与 `WindowBackend::render`（用 `?` 向上传播）。
    assert!(
        src.contains("self.render_inner(tree, 1)"),
        "Window::render 默认必须传 sync_interval = 1"
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
        body.contains("request_full_redraw(hwnd)"),
        "WM_PAINT 必须请求整窗重绘（遮挡/最小化恢复时 D3D 内容是未定义的）：\n{body}"
    );
}

#[test]
fn runtime_consumes_repaint_request_before_render() {
    // 语义：重绘请求必须在 render 之前被消费，否则这帧会用旧的脏区去画。
    // 重构后消费点在 Window::after_frame，顺序由公共循环保证：两个事实都验证。
    let window = include_str!("../src/window.rs");
    let fn_start = window
        .find("fn after_frame(")
        .expect("Window 必须实现 after_frame");
    let body = &window[fn_start..];
    let body_end = body.find("fn after_present(").unwrap_or(body.len());
    let body = &body[..body_end];

    let repaint = body
        .find("take_repaint_request()")
        .expect("after_frame 必须消费重绘请求");
    // 注意：同一函数里还有其他 `invalidate_all()`（字体、快照分支），
    // 所以不能拿“文件里第一个 invalidate”去比位置；必须看**重绘消费之后**的那段。
    assert!(
        body[repaint..].contains("tree.invalidate_all()"),
        "消费重绘请求后必须全量失效"
    );

    let common = include_str!("../../y901-runtime/src/runtime.rs");
    let hook = common
        .find("window.after_frame(tree, host)")
        .expect("run_loop 必须调 after_frame");
    let render = common
        .find("window.render(tree)?")
        .expect("run_loop 必须调 render");
    assert!(hook < render, "after_frame 必须在 render 之前");
}

/// **空 dirty 时 Win32 必须返回 `Skipped`**（不 Present）。
///
/// 为何不是行为测试：`Window::create` 需要真实 `HWND` + D3D 设备，
/// 而 `render_inner` 在没有设备时直接返回，建不出有意义的边界。
/// 因此这里钉住**结构**：skip 由 `render_inner` 实施，outcome 由包装层生成，
/// 且两者均以 `tree.dirty()` 为唯一依据。
#[test]
fn backend_render_reports_skipped_when_clean() {
    let src = include_str!("../src/window.rs");
    let start = src
        .find("fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String>")
        .expect("WindowBackend::render 实现未找到");
    let body = &src[start..];
    let end = body.find("fn set_theme(").unwrap_or(body.len());
    let body = &body[..end];

    assert!(
        body.contains("let had_dirty = !tree.dirty().is_empty();"),
        "outcome 必须基于调用前的脏区状态"
    );
    assert!(
        body.contains("self.render_inner(tree, 1)?"),
        "skip 与 Present 必须在 render_inner 内完成，且上屏失败要用 `?` 向上传播"
    );
    assert!(
        body.contains("RenderOutcome::Skipped") && body.contains("RenderOutcome::Presented"),
        "两种结果都必须出现"
    );
}
