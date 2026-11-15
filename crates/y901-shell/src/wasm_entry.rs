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

    fn command(&mut self, _tree: &mut y901_ui::UiTree, _cmd: Command) -> bool {
        false // 忽略所有命令：管线验证不依赖命令处理
    }
}

/// 页面入口：按 `y901-canvas` 约定取 canvas，建运行时并启动 rAF 循环。
///
/// 页面在 `www/index.html`（见 `docs/WASM.md` §3.2）。
#[wasm_bindgen]
pub fn y901_start() -> Result<(), JsValue> {
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
    let engine = MyFontEngine::new(bytes, 16.0, cell)
        .map_err(|e| JsValue::from_str(&format!("Y901: 字体解析失败: {e}")))?;

    let state = STATE
        .with(|slot| slot.borrow().clone())
        .ok_or_else(|| JsValue::from_str("Y901: y901_start 尚未调用"))?;
    let mut s = state
        .try_borrow_mut()
        .map_err(|_| JsValue::from_str("Y901: 状态被重入借用"))?;
    s.soft.set_font(Box::new(engine), cell);
    if let Some(tree) = s.tree.as_mut() {
        tree.invalidate_all();
    }
    Ok(())
}
