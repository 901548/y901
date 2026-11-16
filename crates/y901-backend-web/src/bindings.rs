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

    /// `requestAnimationFrame(cb)`。
    ///
    /// **返回句柄必须保存**：想取消已排队的帧就靠它（`cancelAnimationFrame`）。
    /// 否则 runtime 销毁后，那个回调仍会被调用一次，
    /// 而它持有的 `Closure` 已被 drop → 抛 `closure invoked after dropped`。
    #[wasm_bindgen(method, js_name = "requestAnimationFrame")]
    pub fn request_animation_frame(this: &Window, cb: &Closure<dyn FnMut(f64)>) -> i32;

    /// `cancelAnimationFrame(handle)`。
    #[wasm_bindgen(method, js_name = "cancelAnimationFrame")]
    pub fn cancel_animation_frame(this: &Window, handle: i32);

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

    /// `canvas.clientWidth`（CSS 像素）。
    #[wasm_bindgen(method, getter, js_name = "clientWidth")]
    pub fn client_width(this: &HtmlCanvasElement) -> u32;

    /// `canvas.clientHeight`（CSS 像素）。
    #[wasm_bindgen(method, getter, js_name = "clientHeight")]
    pub fn client_height(this: &HtmlCanvasElement) -> u32;

    /// 设置 `tabindex`。
    ///
    /// **收键盘事件的前提**：`<canvas>` 默认不可聚焦，不设 `tabindex` 就永远收不到
    /// `keydown`。这是 S2 里最容易漏的一条（见 `docs/WASM.md` S2 笔记）。
    #[wasm_bindgen(method, js_name = "setAttribute")]
    pub fn set_attribute(this: &HtmlCanvasElement, name: &str, value: &str);

    /// `canvas.getAttribute(name)`——调试用：看内联 `style` 到底被写成了什么。
    #[wasm_bindgen(method, js_name = "getAttribute")]
    pub fn get_attribute(this: &HtmlCanvasElement, name: &str) -> Option<String>;

    /// `canvas.style`（`CSSStyleDeclaration`）。
    ///
    /// **为何用它而不是 `setAttribute("style", ...)`**：后者会**整条替换**内联
    /// 样式，把页面 CSS 里的 `width/height` 一并抹掉（这正是上一轮 resize
    /// 失效的原因）；`setProperty` 只改单个属性。
    #[wasm_bindgen(method, getter, structural, js_name = "style")]
    pub fn style(this: &HtmlCanvasElement) -> CssStyleDeclaration;
}

/// 元素的内联样式表。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = "CSSStyleDeclaration")]
    pub type CssStyleDeclaration;

    /// `style.setProperty(name, value)`。
    #[wasm_bindgen(method, js_name = "setProperty")]
    pub fn set_property(this: &CssStyleDeclaration, name: &str, value: &str);

    /// `style.setProperty(name, value, priority)`。
    ///
    /// `priority = "important"` 用来盖过页面 CSS 里可能存在的
    /// 全局 `canvas { cursor: ... }` 防守规则。
    #[wasm_bindgen(method, js_name = "setProperty")]
    pub fn set_property_with_priority(
        this: &CssStyleDeclaration,
        name: &str,
        value: &str,
        priority: &str,
    );

    /// `style.getPropertyValue(name)`。
    #[wasm_bindgen(method, js_name = "getPropertyValue")]
    pub fn get_property_value(this: &CssStyleDeclaration, name: &str) -> String;

    /// `style.getPropertyPriority(name)`。
    #[wasm_bindgen(method, js_name = "getPropertyPriority")]
    pub fn get_property_priority(this: &CssStyleDeclaration, name: &str) -> String;

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

    /// `KeyboardEvent.keyCode`（已废弃但仍普遍可用）。
    ///
    /// 用途：输入法组字相关的按键事件常报 `229`，这是识别
    /// "这个 keydown 不是真实字符输入"的又一道保险。
    #[wasm_bindgen(method, getter, js_name = "keyCode")]
    pub fn key_code(this: &KeyboardEvent) -> u32;
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

    /// `element.remove()`——从它的父节点上摘掉。
    ///
    /// 用作 `dispose()`：隐藏 `input` 是我们机动插进 `body` 的，
    /// 不清理就会在页面上积累（热重启多次就堆多个）。
    #[wasm_bindgen(method, structural, js_name = "remove")]
    pub fn remove(this: &Element);

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


/// `Date.now()`——毫秒时间戳。
///
/// 用途：区分"输入法提交后紧跟的幻影 keydown"与"真实按键"。
/// 为何用这个而不用 `performance.now()`：两者都行，`Date.now` 更普通，
/// 且不需要 `performance` 对象。
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = Date, js_name = now)]
    pub fn date_now() -> f64;
}