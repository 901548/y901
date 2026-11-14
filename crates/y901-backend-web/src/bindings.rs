//! 手写的 DOM 绑定（**不用 `web-sys`**，见 `docs/WASM.md` 决策 D6）。
//!
//! 这里只声明我们真正用到的外部符号。`#[wasm_bindgen]` 会在
//! 编译时生成 JS 桥接，名字对不上会在运行时静默失败——
//! 因此 S1 会配一条"绑定 golden test"断言 JS 侧名字（见 `docs/WASM.md` §5）。

use wasm_bindgen::prelude::*;

/// `window` 对象。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "window")]
    pub type Window;

    #[wasm_bindgen(method, js_name = "requestAnimationFrame")]
    pub fn request_animation_frame(this: &Window, cb: &Closure<dyn FnMut(f64)>);
}

/// `HtmlCanvasElement`。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "HtmlCanvasElement")]
    pub type HtmlCanvasElement;

    #[wasm_bindgen(method, getter)]
    pub fn width(this: &HtmlCanvasElement) -> u32;
    #[wasm_bindgen(method, setter)]
    pub fn set_width(this: &HtmlCanvasElement, v: u32);
    #[wasm_bindgen(method, getter)]
    pub fn height(this: &HtmlCanvasElement) -> u32;
    #[wasm_bindgen(method, setter)]
    pub fn set_height(this: &HtmlCanvasElement, v: u32);

    /// `getContext("2d")`；非 2d 上下文返回 `None`。
    #[wasm_bindgen(method, js_name = "getContext")]
    pub fn get_context_2d(this: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d>;
}

/// 2D 绘图上下文。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "CanvasRenderingContext2D")]
    pub type CanvasRenderingContext2d;

    /// 整幅上屏（见 `docs/WASM.md` §3.2）。
    #[wasm_bindgen(method, js_name = "putImageData")]
    pub fn put_image_data(this: &CanvasRenderingContext2d, data: &ImageData, dx: f64, dy: f64);
}

/// `ImageData`。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "ImageData")]
    pub type ImageData;

    /// `new ImageData(new Uint8ClampedArray(w*h*4), w, h)`。
    #[wasm_bindgen(constructor)]
    pub fn new(pixels: &JsValue, width: u32, height: u32) -> ImageData;
}

/// 键盘事件。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "KeyboardEvent")]
    pub type KeyboardEvent;

    #[wasm_bindgen(method, getter)]
    pub fn code(this: &KeyboardEvent) -> String;
    #[wasm_bindgen(method, getter)]
    pub fn key(this: &KeyboardEvent) -> String;
}

/// 鼠标事件。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "MouseEvent")]
    pub type MouseEvent;

    #[wasm_bindgen(method, getter, js_name = "clientX")]
    pub fn client_x(this: &MouseEvent) -> f64;
    #[wasm_bindgen(method, getter, js_name = "clientY")]
    pub fn client_y(this: &MouseEvent) -> f64;
    #[wasm_bindgen(method, getter)]
    pub fn button(this: &MouseEvent) -> i16;
}

/// 滚轮事件。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "WheelEvent")]
    pub type WheelEvent;

    #[wasm_bindgen(method, getter, js_name = "deltaY")]
    pub fn delta_y(this: &WheelEvent) -> f64;
}

/// 事件监听。S1 接线时会用到。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "EventTarget")]
    pub type EventTarget;

    #[wasm_bindgen(method, js_name = "addEventListener")]
    pub fn add_event_listener(this: &EventTarget, kind: &str, cb: &Closure<dyn FnMut(JsValue)>);
}
/// `Uint8ClampedArray`——`ImageData` 的像素入参。
///
/// **手写而不用 `js_sys`**：`js-sys` 在离线环境解析不到，而我们只需要"建一个长度 + 整块写入"两个操作（见 `docs/WASM.md` D6）。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "Uint8ClampedArray")]
    pub type Uint8ClampedArray;

    #[wasm_bindgen(constructor)]
    pub fn new(length: u32) -> Uint8ClampedArray;

    #[wasm_bindgen(method, js_name = "set")]
    pub fn set_from(this: &Uint8ClampedArray, src: &[u8], offset: u32);
}