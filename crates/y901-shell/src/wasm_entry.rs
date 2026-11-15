//! wasm 入口：`y901_start()`（仅 wasm32 编译）。
//!
//! **为何在 `y901-shell` 而非 `y901-backend-web`**：shell 已按 wasm32 条件依赖 backend-web，
//! 反向依赖会构成循环，Cargo 直接拒绝。

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use y901_backend_web::bindings::HtmlCanvasElement;
use y901_backend_web::WasmRuntime;
use y901_runtime::Host;
use y901_ui::Command;

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
    let host = Box::new(MinimalHost);
    let build = Box::new(|| crate::build_demo_page());
    runtime
        .run(host, build)
        .map_err(|e| JsValue::from_str(&format!("Y901: {e}")))
}
