//! Y901 浏览器（wasm32）后端。
//!
//! 设计见 `docs/WASM.md`。**不依赖 `web-sys`**：需要的几十个外部符号
//! 手写在 [`bindings`]，避免 `web-sys` 的编译时间膨胀。
//!
//! 现状：**S1 骨架**。模块签名已定下来，函数体为 `todo!()`。

pub mod bindings;
pub mod font;
pub mod frame;
pub mod input;
pub mod keys;
pub mod present;

pub use frame::{frame_step, FrameStats};
pub use input::{KeyInput, MouseInput, WheelInput};
pub use present::Present;

/// 浏览器端运行时。
///
/// 与桌面的差异：没有阻塞的 `wait_events`，帧由 `requestAnimationFrame` 逐次推进
/// （见 `docs/WASM.md` 决策 D2）。
pub struct WasmRuntime {
    /// canvas 句柄（`HtmlCanvasElement`）。
    pub canvas: bindings::HtmlCanvasElement,
    /// 呈现器（`putImageData`）。
    pub present: Present,
    /// 本帧已排队的原始事件。
    pub events: Vec<y901_runtime::RawEvent>,
    /// 上一次 rAF 的时间戳（毫秒）。
    pub last_frame_ms: f64,
}

impl WasmRuntime {
    /// 用一个 canvas 构造。真正的初始化在 S1 实现。
    pub fn new(canvas: bindings::HtmlCanvasElement) -> Self {
        // `#[wasm_bindgen]` 类型不是 `Copy`：先克隆一份给 `Present`，再保留自己的句柄。
        let cloned: bindings::HtmlCanvasElement = canvas.clone().into();
        let present = Present::new(cloned);
        Self {
            present,
            canvas,
            events: Vec::new(),
            last_frame_ms: 0.0,
        }
    }
}