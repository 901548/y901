//! Y901 浏览器（wasm32）后端。
//!
//! 设计见 `docs/WASM.md`。**不依赖 `web-sys`**：需要的外部符号手写在 [`bindings`]，
//! 避免 `web-sys` 的编译时间膨胀（决策 D6）。
//!
//! 阶段：**S1-a**。已有：手写绑定、`Present` 上屏、`WasmWindow` 的 `WindowBackend` 实现。
//! 未做（S1-b）：`WasmRuntime::run` + rAF 循环 + `create_runtime()` 的 wasm 分支。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use y901_render::soft::SoftBackend;
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{Host, RawEvent};
use y901_ui::{Size, UiTree};

pub mod bindings;
pub mod font;
pub mod frame;
pub mod input;
pub mod keys;
pub mod present;

pub use bindings::HtmlCanvasElement;
pub use frame::{frame_step, FrameStats};
pub use input::{KeyInput, MouseInput, WheelInput};
pub use present::Present;

/// 浏览器端的全部可变状态。
///
/// 为什么要 `Rc<RefCell<…>>`：`requestAnimationFrame` 的回调必须 `'static`，
/// 而 `tree` / `host` 都在 `run` 的栈上（见 `docs/WASM.md` 决策 D2 与生命周期模型 A）。
///
/// **构造留到 S1-b**：`SoftBackend::new` / `new_headless` 都需要一个
/// `Box<dyn FontEngine>`，浏览器端还没有字体引擎实现。
pub struct WasmState {
    /// canvas 句柄。
    pub canvas: HtmlCanvasElement,
    /// 软渲染器（产像素）。
    pub soft: SoftBackend,
    /// 上屏器。
    pub present: Present,
    /// UI 树（`run` 时注入）。
    pub tree: Option<UiTree>,
    /// 业务层 host（`run` 时注入）。
    pub host: Option<Box<dyn Host>>,
    /// 本帧已排队的原始事件。
    pub events: VecDeque<RawEvent>,
    /// 上一帧的时间戳（毫秒）。
    pub last_frame_ms: f64,
}

/// `WindowBackend` 的**薄包装**：只借 `soft` / `present` / `canvas`。
///
/// 为什么不让 `WasmRuntime` 直接实现 `WindowBackend`：那样 `frame_step(window, tree, host, …)`
/// 会同时可变借用"整块 state"和"state 里的 tree"，借用检查器不允许。
/// 这一层包装把 `tree` / `host` 排除在外，于是三者可以并存（零 `take`、零签名改动）。
pub struct WasmWindow<'a> {
    pub soft: &'a mut SoftBackend,
    pub present: &'a mut Present,
    pub canvas: &'a HtmlCanvasElement,
}

impl WindowBackend for WasmWindow<'_> {
    /// **非阻塞**：wasm 不使用 `run_loop`，本方法只被 `frame_step` 之外的路径调用。
    /// 按约定**永远返回 `Some`**（`None` 在桌面表示"窗口被关闭"）。
    fn wait_events(&mut self, _wait_ms: u32) -> Option<Vec<RawEvent>> {
        Some(Vec::new())
    }

    fn viewport(&self) -> (u32, u32) {
        (self.canvas.width(), self.canvas.height())
    }

    fn relayout(&mut self, tree: &mut UiTree) {
        let (w, h) = self.viewport();
        tree.layout(Size::new(w as f32, h as f32), self.soft);
    }

    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String> {
        if tree.dirty().is_empty() {
            return Ok(RenderOutcome::Skipped);
        }
        self.soft.clear();
        tree.render(self.soft);
        let (w, h) = self.viewport();
        let pixels = self.soft.pixels().to_vec();
        self.present.present(&pixels, w, h);
        Ok(RenderOutcome::Presented)
    }

    fn set_theme(&mut self, theme: y901_ui::theme::Theme) -> Result<(), String> {
        self.soft.set_theme(theme)
    }
}

/// 浏览器端运行时。`run` 与 rAF 循环在 S1-b 实现。
pub struct WasmRuntime {
    /// 共享状态（rAF 回调通过 `Rc` 克隆持有）。
    pub state: Rc<RefCell<WasmState>>,
}