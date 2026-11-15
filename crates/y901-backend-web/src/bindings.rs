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

    /// 全局 `window` 对象。
    ///
    /// **必须是全局对象而非函数**：`window` 是对象，`js_name = "window"` 写在 `fn` 上会生成
    /// `window()` 调用，浏览器报 `TypeError: window is not a function`。
    /// 用 `thread_local_v2`（wasm-bindgen 0.2.129 的推荐写法，`static` 已废弃）。
    #[wasm_bindgen(thread_local_v2, js_name = "window")]
    static GLOBAL_WINDOW: Window;

    #[wasm_bindgen(method, getter)]
    pub fn document(this: &Window) -> Option<Document>;
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

    /// `getContext("2d")`。
    ///
    /// **`kind` 必须传**：`getContext` 在 JS 侧要 1 个参数，漏了就报
    /// `TypeError: 1 argument required, but only 0 present`。
    #[wasm_bindgen(method, js_name = "getContext")]
    pub fn get_context(this: &HtmlCanvasElement, kind: &str) -> Option<CanvasRenderingContext2d>;

    /// 设置 `tabindex`。
    ///
    /// **收键盘事件的前提**：`<canvas>` 默认不可聚焦，不设 `tabindex` 就永远收不到
    /// `keydown`。这是 S2 里最容易漏的一条（见 `docs/WASM.md` S2 笔记）。
    #[wasm_bindgen(method, js_name = "setAttribute")]
    pub fn set_attribute(this: &HtmlCanvasElement, name: &str, value: &str);

    /// `canvas.focus()`\uff1a\u70b9\u51fb\u540e\u628a\u952e\u76d8\u7126\u70b9\u62a2\u5230 canvas \u4e0a\u3002
    #[wasm_bindgen(method)]
    pub fn focus(this: &HtmlCanvasElement);
}

/// 2D 绘图上下文。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "CanvasRenderingContext2D")]
    pub type CanvasRenderingContext2d;

    /// 整幅上屏（见 `docs/WASM.md` §3.2）。
    #[wasm_bindgen(method, js_name = "putImageData")]
    pub fn put_image_data(this: &CanvasRenderingContext2d, data: &ImageData, dx: f64, dy: f64);

    /// `createImageData(w, h)`：建一张空图像（可直接写入像素）。
    #[wasm_bindgen(method, js_name = "createImageData")]
    pub fn create_image_data(this: &CanvasRenderingContext2d, w: u32, h: u32) -> ImageData;
}

/// `ImageData`。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "ImageData")]
    pub type ImageData;

    /// `new ImageData(new Uint8ClampedArray(w*h*4), w, h)`。
    #[wasm_bindgen(constructor)]
    pub fn new(pixels: &JsValue, width: u32, height: u32) -> ImageData;

    /// 图像的像素数组（`Uint8ClampedArray`）。
    #[wasm_bindgen(method, getter)]
    pub fn data(this: &ImageData) -> Uint8ClampedArray;
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

    /// `KeyboardEvent.ctrlKey`。
    #[wasm_bindgen(method, getter, js_name = "ctrlKey")]
    pub fn ctrl_key(this: &KeyboardEvent) -> bool;
    /// `KeyboardEvent.shiftKey`。
    #[wasm_bindgen(method, getter, js_name = "shiftKey")]
    pub fn shift_key(this: &KeyboardEvent) -> bool;
    /// `KeyboardEvent.altKey`。
    #[wasm_bindgen(method, getter, js_name = "altKey")]
    pub fn alt_key(this: &KeyboardEvent) -> bool;
    /// `KeyboardEvent.metaKey`（\u2318 / Win\uff09\u3002\u53ea\u53d6\u4e0d\u7528\uff1a\u6846\u67b6\u7684 `Modifiers` \u6ca1\u6709 meta \u4f4d\u3002
    #[wasm_bindgen(method, getter, js_name = "metaKey")]
    pub fn meta_key(this: &KeyboardEvent) -> bool;

    /// `KeyboardEvent.isComposing`：输入法正在组字。
    ///
    /// 组字期间的 `keydown` **不能**当普通按键处理，否则拼音字母会被当成
    /// "要输入的字符"插进输入框。
    #[wasm_bindgen(method, getter, js_name = "isComposing")]
    pub fn is_composing(this: &KeyboardEvent) -> bool;
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

    /// `MouseEvent.offsetX`\u2014\u2014**\u76f8\u5bf9\u4e8e\u4e8b\u4ef6\u76ee\u6807\uff08canvas\uff09\u7684\u5750\u6807**\u3002
    ///
    /// \u4e3a\u4f55\u7528 `offset*` \u800c\u4e0d\u7528 `client*` \u51cf `getBoundingClientRect()`\uff1a
    /// canvas \u6ca1\u6709 CSS \u7f29\u653e\uff08\u5e03\u5c40\u5c3a\u5bf8 = \u5c5e\u6027\u5c3a\u5bf8\uff09\uff0c`offset*` \u5df2\u7ecf\u662f
    /// canvas \u5185\u90e8\u7684\u843d\u70b9\uff0c**\u4e0d\u53d7\u9875\u9762\u6eda\u52a8\u4e0e\u9876\u90e8\u504f\u79fb\u5f71\u54cd**\uff0c\u4e5f\u5c11\u4e00\u6b21 DOM \u8c03\u7528\u3002
    #[wasm_bindgen(method, getter, js_name = "offsetX")]
    pub fn offset_x(this: &MouseEvent) -> f64;
    /// `MouseEvent.offsetY`\u3002
    #[wasm_bindgen(method, getter, js_name = "offsetY")]
    pub fn offset_y(this: &MouseEvent) -> f64;

    /// `MouseEvent.ctrlKey`\u3002
    #[wasm_bindgen(method, getter, js_name = "ctrlKey")]
    pub fn ctrl_key(this: &MouseEvent) -> bool;
    /// `MouseEvent.shiftKey`\u3002
    #[wasm_bindgen(method, getter, js_name = "shiftKey")]
    pub fn shift_key(this: &MouseEvent) -> bool;
    /// `MouseEvent.altKey`\u3002
    #[wasm_bindgen(method, getter, js_name = "altKey")]
    pub fn alt_key(this: &MouseEvent) -> bool;
    /// `MouseEvent.metaKey`\u3002
    #[wasm_bindgen(method, getter, js_name = "metaKey")]
    pub fn meta_key(this: &MouseEvent) -> bool;
}

/// 滚轮事件。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "WheelEvent")]
    pub type WheelEvent;

    #[wasm_bindgen(method, getter, js_name = "deltaY")]
    pub fn delta_y(this: &WheelEvent) -> f64;
}

/// 事件监听。S2 接线用。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "EventTarget")]
    pub type EventTarget;

    /// `addEventListener(type, listener)`\u2014\u2014\u4e0d\u5e26 options \u7684\u7b80\u5199\u5f62\u5f0f\u3002
    #[wasm_bindgen(method, js_name = "addEventListener")]
    pub fn add_event_listener(this: &EventTarget, kind: &str, cb: &Closure<dyn FnMut(JsValue)>);

    /// `addEventListener(type, listener, options)`\u3002
    ///
    /// \u4e3a\u4f55\u9700\u8981\u5b83\uff1a\u90e8\u5206\u4e8b\u4ef6\uff08\u5c24\u5176 `wheel`\uff09\u5728\u67d0\u4e9b\u76ee\u6807\u4e0a\u9ed8\u8ba4**\u88ab\u52a8**\uff0c
    /// \u88ab\u52a8\u76d1\u542c\u5668\u91cc\u8c03 `preventDefault()` \u65e0\u6548\u4e14\u63a7\u5236\u53f0\u62a5\u8b66\u3002
    /// \u4f20 `{ passive: false }` \u624d\u80fd\u771f\u6b63\u963b\u6b62\u9875\u9762\u6eda\u52a8\u3002
    #[wasm_bindgen(method, js_name = "addEventListener")]
    pub fn add_event_listener_with_options(
        this: &EventTarget,
        kind: &str,
        cb: &Closure<dyn FnMut(JsValue)>,
        options: &JsValue,
    );
}

/// DOM 事件基类\uff08\u7528\u6765\u8c03 `preventDefault`\uff09\u3002
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "Event")]
    pub type Event;

    /// \u963b\u6b62\u6d4f\u89c8\u5668\u9ed8\u8ba4\u884c\u4e3a\uff08\u9875\u9762\u6eda\u52a8\u3001Tab \u8df3\u7126\u7b49\uff09\u3002
    #[wasm_bindgen(method, js_name = "preventDefault")]
    pub fn prevent_default(this: &Event);
}

/// \u6784\u9020 JS \u666e\u901a\u5bf9\u8c61\uff0c\u7528\u6765\u4f20 `addEventListener` \u7684 options\u3002
///
/// **\u4e3a\u4ec0\u4e48\u8981\u81ea\u5df1\u9020\u5bf9\u8c61**\uff1a\u672c crate \u4e0d\u4f9d\u8d56 `js_sys`\uff08\u79bb\u7ebf\u73af\u5883\u4e0b\u7d22\u5f15\u4e0d\u53ef\u7528\uff09\uff0c
/// \u800c `{ passive: false }` \u65e0\u6cd5\u7528\u5355\u4e2a\u5e03\u5c14\u53c2\u6570\u8868\u8fbe\u3002\u7528 `indexing_setter`
/// \u5199\u5165\u5c5e\u6027\uff0c\u751f\u6210\u7684 JS \u5c31\u662f `opts["passive"] = false`\u3002
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "Object")]
    pub type JsObject;

    /// `new Object()`\u3002
    #[wasm_bindgen(constructor)]
    pub fn new() -> JsObject;

    /// `obj[key] = value`\u3002
    #[wasm_bindgen(method, structural, indexing_setter)]
    pub fn set_prop(this: &JsObject, key: &str, value: &JsValue);
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

    /// `data[i] = v`。**用 `indexing_setter`**（JS 没有 `setIndex` 方法，那会静默失败）。
    #[wasm_bindgen(method, structural, indexing_setter)]
    pub fn set_index(this: &Uint8ClampedArray, index: u32, value: u8);
}
/// `Document`——用来按 id 取 canvas（页面约定见 `docs/WASM.md` §3.2）。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "Document")]
    pub type Document;

    #[wasm_bindgen(method, js_name = "getElementById")]
    pub fn get_element_by_id(this: &Document, id: &str) -> Option<Element>;

    /// `document.body`。
    #[wasm_bindgen(method, getter)]
    pub fn body(this: &Document) -> Option<Element>;

    /// `document.createElement(tag)`。
    #[wasm_bindgen(method, js_name = "createElement")]
    pub fn create_element(this: &Document, tag: &str) -> Element;

    /// `document.activeElement`——**调试用**：确认焦点到底在谁身上。
    #[wasm_bindgen(method, getter, js_name = "activeElement")]
    pub fn active_element(this: &Document) -> Option<Element>;
}

/// DOM `Element`（取到后用 `unchecked_into` 转成 `HtmlCanvasElement`）。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "Element")]
    pub type Element;

    /// `element.setAttribute(name, value)`。
    #[wasm_bindgen(method, js_name = "setAttribute")]
    pub fn set_attribute(this: &Element, name: &str, value: &str);

    /// `parent.appendChild(child)`。
    ///
    /// 用 `structural`：参数声明为 `&Element`，但实际传的是 `HtmlInputElement`；
    /// 不加 `structural` 的话 wasm-bindgen 会在 JS 侧做 `instanceof Element` 检查。
    #[wasm_bindgen(method, structural, js_name = "appendChild")]
    pub fn append_child(this: &Element, child: &Element) -> Element;

    /// `element.tagName`（大写，如 `"CANVAS"` / `"INPUT"`）。
    #[wasm_bindgen(method, getter, js_name = "tagName")]
    pub fn tag_name(this: &Element) -> String;

    /// `element.id`。
    #[wasm_bindgen(method, getter)]
    pub fn id(this: &Element) -> String;
}

/// `HTMLInputElement`——**隐藏的 IME 承接元素**。
///
/// 为何要它：浏览器的输入法（拼音）只作用在**可编辑元素**上。canvas
/// 收不到 `composition*` 事件，所以在页面里放一个 1px、透明的 input，把它设为焦点，
/// 拼音组字就发到它身上，我们再转成 `RawEvent::Ime`。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "HTMLInputElement")]
    pub type HtmlInputElement;

    /// `input.value`。
    #[wasm_bindgen(method, getter)]
    pub fn value(this: &HtmlInputElement) -> String;

    /// `input.value = v`。
    #[wasm_bindgen(method, setter)]
    pub fn set_value(this: &HtmlInputElement, v: &str);

    /// `input.focus()`——**唤醒输入法的关键一步**。
    #[wasm_bindgen(method)]
    pub fn focus(this: &HtmlInputElement);
}

/// `CompositionEvent`：输入法组字事件。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "CompositionEvent")]
    pub type CompositionEvent;

    /// `event.data`：当前组字串（`compositionend` 时是最终提交串）。
    #[wasm_bindgen(method, getter)]
    pub fn data(this: &CompositionEvent) -> String;
}

/// 取全局 `window`（调用方便用）。
pub fn global_window() -> Option<Window> {
    // thread_local_v2 的访问器把 `&Window` 交给回调；拷一份 `JsValue` 再转回 `Window`。
    GLOBAL_WINDOW.with(|w| {
        let js: wasm_bindgen::JsValue = wasm_bindgen::JsValue::from(w);
        Some(<Window as wasm_bindgen::JsCast>::unchecked_from_js(js))
    })
}

/// `console.log`。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "console.log")]
    pub fn console_log(s: &str);
}
