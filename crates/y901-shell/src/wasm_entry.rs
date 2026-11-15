//! wasm 入口：`y901_start()`（仅 wasm32 编译）。
//!
//! **为何在 `y901-shell` 而非 `y901-backend-web`**：shell 已按 wasm32 条件依赖 backend-web，
//! 反向依赖会构成循环，Cargo 直接拒绝。

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use y901_backend_web::bindings::HtmlCanvasElement;
use y901_backend_web::WasmRuntime;
use y901_backend_web::WasmState;
use y901_render::font_engine::MyFontEngine;
use y901_runtime::Host;
use y901_ui::{Command, Size};

/// 把 Rust panic 转成 `console.error`（**仅诊断用**）。
///
/// 为何需要：wasm 上 panic 被译成 `unreachable`，浏览器只报
/// `RuntimeError: unreachable`，**不给行号**。装上 hook 后 console 里会多一行
/// `[y901] PANIC: panicked at crates/...:LINE:COL`，定位才有依据。
pub fn set_panic_hook() {
    use std::sync::Once;
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            console_error(&format!("[y901] PANIC: {info}"));
        }));
    });
}

#[wasm_bindgen]
extern "C" {
    /// `console.error`（手写，不引入 web-sys）。
    #[wasm_bindgen(js_name = "console.error")]
    fn console_error(s: &str);

    /// `console.log`。
    #[wasm_bindgen(js_name = "console.log")]
    fn console_log(s: &str);
}

thread_local! {
    /// 当前页面的分享状态。
    ///
    /// 为何 `thread_local!` 而不是 `static Mutex`：wasm 单线程，`Rc` 不是 `Send`；
    /// `thread_local!` 既能存 `Rc`，也不需要锁。
    /// 局限：**一个页面一个实例**，与 `www/index.html` 的单 canvas 约定一致。
    static STATE: RefCell<Option<Rc<RefCell<WasmState>>>> = const { RefCell::new(None) };
}

/// 最小 host：只把命令吃掉，不做事（S1-b 只验证管线）。
struct MinimalHost;

impl Host for MinimalHost {
    fn idle(&mut self, _tree: &mut y901_ui::UiTree) {}

    fn command(&mut self, _tree: &mut y901_ui::UiTree, cmd: Command) -> bool {
        // **让命令可见**：之前这里无条件 `false`，点“保存”什么都不会发生，
        // 也看不出到底有没有命令到达。现在先打印，再按名字回应。
        let name = match &cmd {
            Command::App { name, .. } => name.as_str(),
            Command::Node { action, .. } => action.as_str(),
        };
        console_log(&format!("[y901] command: {name} ({cmd:?})"));
        match name {
            "save" => {
                console_log("[y901] 保存成功（demo host：仅日志，不做真实持久化）");
                true
            }
            "cancel" => {
                console_log("[y901] 已取消");
                true
            }
            _ => false, // 其他命令交回树处理
        }
    }
}

/// 页面入口：按 `y901-canvas` 约定取 canvas，建运行时并启动 rAF 循环。
///
/// 页面在 `www/index.html`（见 `docs/WASM.md` §3.2）。
#[wasm_bindgen]
pub fn y901_start() -> Result<(), JsValue> {
    set_panic_hook();

    let window = y901_backend_web::bindings::global_window()
        .ok_or_else(|| JsValue::from_str("Y901: 没有 window（不在浏览器里？）"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("Y901: 没有 document"))?;
    let canvas = document
        .get_element_by_id("y901-canvas")
        .ok_or_else(|| JsValue::from_str("Y901: 页面需要 <canvas id=\"y901-canvas\">"))?
        .unchecked_into::<HtmlCanvasElement>();

    let mut runtime =
        WasmRuntime::new(canvas).map_err(|e| JsValue::from_str(&format!("Y901: {e}")))?;
    // 先存下分享状态，供 `y901_font_loaded` 等入口使用。
    let state = Rc::clone(&runtime.state);
    STATE.with(|slot| *slot.borrow_mut() = Some(state));

    let host = Box::new(MinimalHost);
    let build = Box::new(|| crate::build_demo_page());
    runtime
        .run(host, build)
        .map_err(|e| JsValue::from_str(&format!("Y901: {e}")))
}

/// 字体就绪回调：**JS 侧 `fetch` 完成后把字节交过来**。
///
/// 为何这么分工：`fetch` 是 Promise，而 `wasm-bindgen-futures` 与本仓 `wasm-bindgen`
/// 版本不配对（`JsFuture::from(JsValue)` 不满足 trait bound）。把异步留在
/// JS（浏览器最擅长的地方），Rust 只同步接字节，**零异步依赖**。
#[wasm_bindgen]
pub fn y901_font_loaded(bytes: Vec<u8>) -> Result<(), JsValue> {
    // 与 `WasmFontEngine` 的格子保持一致：字号 16，cell 8x16。
    let cell = Size::new(8.0, 16.0);
    let bytes_len = bytes.len();
    let engine = match MyFontEngine::new(bytes, 16.0, cell) {
        Ok(e) => e,
        Err(e) => {
            console_error(&format!("[y901] 字体解析失败（{} 字节）：{}", bytes_len, e));
            return Err(JsValue::from_str(&format!("Y901: 字体解析失败: {e}")));
        }
    };

    let state = STATE
        .with(|slot| slot.borrow().clone())
        .ok_or_else(|| JsValue::from_str("Y901: y901_start 尚未调用"))?;
    // 先把真字体的度量打出来（诊断用，之后可删）。
    let real_cell = Size::new(engine.cell_width(), engine.cell_height());
    console_log(&format!(
        "[y901] new font metrics: cell_w={}, cell_h={} (旧格子 {}x{})",
        real_cell.w, real_cell.h, cell.w, cell.h
    ));

    let mut s = state
        .try_borrow_mut()
        .map_err(|_| JsValue::from_str("Y901: 状态被重入借用"))?;

    // **用真字体的格子尺寸**，而不是沿用零字形时代的 8x16。
    {
        let WasmState {
            canvas, soft, tree, ..
        } = &mut *s;
        soft.set_font(Box::new(engine), real_cell);
        // **必须重排**：布局仍按旧行高算的话，字形会上溢出父容器并被削顶。
        if let Some(tree) = tree.as_mut() {
            // **关键**：字体度量变了但视口没变，`layout()` 不会失效缓存 → 必须手动清。
            tree.clear_layout_cache();
            let (w, h) = (canvas.width(), canvas.height());
            tree.layout(Size::new(w as f32, h as f32), soft);
            tree.invalidate_all();
        }
    }
    Ok(())
}
