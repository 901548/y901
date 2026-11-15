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

use wasm_bindgen::prelude::Closure;
use wasm_bindgen::{JsCast, JsValue};
use y901_render::soft::SoftBackend;
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{Host, RawEvent};
use y901_ui::{Size, UiTree};

pub mod bindings;
pub mod font;
pub mod font_engine;
pub mod frame;
pub mod input;
pub mod keys;
pub mod present;

pub use bindings::HtmlCanvasElement;
pub use font_engine::WasmFontEngine;
pub use frame::{frame_step, FrameStats};
pub use input::{KeyInput, MouseInput, WheelInput};
pub use keys::{button_from_dom, char_from_dom, key_from_dom, mods_from_event, wheel_delta_from_dom};
pub use present::Present;

/// 一个已注册的 DOM 监听器。
///
/// **为何要存起来**：`Closure` 一旦被 `drop`，JS 侧虽然还挂着监听器，
/// 但一触发就抛 `closure invoked after dropped`。所以闭包必须跟着
/// `WasmState` 活到页面结束。
///
/// **P0 阶段只定义不安装**：\u8fd9个结构会在 P1 里被真正填满\u3002
pub struct Listener {
    /// 挂在谁上（`window` 或 canvas）。
    #[allow(dead_code)] // P1 注册时使用
    pub target: bindings::EventTarget,
    /// 事件名（`"mousemove"` / `"keydown"` …）。
    #[allow(dead_code)] // P1 注册时使用
    pub kind: String,
    /// 回调本体（**持有它就是保存它**）。
    #[allow(dead_code)] // P1 注册时使用
    pub handler: Closure<dyn FnMut(wasm_bindgen::JsValue)>,
}

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
    ///
    /// **为何用 `Rc<RefCell<…>>` 而不是直接放 `VecDeque`**：DOM 回调只拿得到一个
    /// 共享句柄（它在 JS 侧被调用时，`WasmState` 正被本帧借用）。
    /// 让回调**只碰事件缓冲**、不碰整个 state，就不会撞上 `try_borrow_mut` 失败。
    pub events: Rc<RefCell<VecDeque<RawEvent>>>,
    /// **隐藏的 IME 承接 `input`**（为 `None` 时表示页面没有 `document.body`）。
    ///
    /// 存在理由：点击时要把焦点给它（而不是 canvas），输入法才会启动。
    pub ime_input: Option<wasm_bindgen::JsValue>,
    /// 已注册的 DOM 监听器（**持有它们就是保证它们生效**）。
    ///
    /// P0 阶段为空；P1 起向这里压入。
    pub listeners: Vec<Listener>,
    /// 上一帧的时间戳（毫秒）。
    pub last_frame_ms: f64,
    /// **当前这一帧的 rAF 回调**。
    ///
    /// 为何要存起来：`Closure` 一旦 drop，回调立即失效；
    /// 而 `Closure::forget()` 会**每帧泄漏一个**（内存无限增长）。
    /// 放在 `Rc<RefCell<Option<..>>>` 里：每帧先 `take` 出旧的并 drop，再存入新的。
    #[allow(clippy::type_complexity)] // 层数是"holder + 可选回调" 的必然结果，展开只会更难读
    pub raf: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>,
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
    /// 事件缓冲（DOM 回调往里写，`wait_events` 从里取）。
    pub events: &'a Rc<RefCell<VecDeque<RawEvent>>>,
}

impl WindowBackend for WasmWindow<'_> {
    /// **非阻塞**：wasm 不使用 `run_loop`，本方法只被 `frame_step` 之外的路径调用。
    /// 按约定**永远返回 `Some`**（`None` 在桌面表示"窗口被关闭"）。
    fn wait_events(&mut self, _wait_ms: u32) -> Option<Vec<RawEvent>> {
        // **浏览器不会阻塞等待**：节奏由 rAF 决定。
        // 有就全取，没有就空 —— 与桌面 `wait_events` 的"可阻塞"语义不同，
        // 这一差异已写进 `docs/WASM.md`。
        let drained: Vec<RawEvent> = self.events.borrow_mut().drain(..).collect();
        Some(drained)
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

        // **wasm 上一律全量重绘**。
        //
        // 为何：`UiTree::render` 不接受脏区参数，它读自己的 `self.dirty` 并
        // **跳过干净子树**。在**持久缓冲**型后端上，局部重绘已属证实有缺陷：
        // 悬停时被标脏的那一小块会覆盖上一帧，但父容器背景与裁剪
        // 并不会重画，于是出现"哪个组件被悬停、哪个就消失/留白边"。
        // 桌面端每帧重建图元列表，本来就是全量，因此从未暴露。
        //
        // `invalidate_all()` 把脏区置为 `full`，让 `render` 走全树；然后清屏。
        // 性能代价是每帧重画整树——在 wasm 上这是可接受的，因为
        // 上屏本来就是**整帧 `putImageData`**，省下的只是 CPU 重绘量。
        //
        // 底层缺陷（局部脏区在持久缓冲上不完整）仍在，这里是**绕开**而非修复；
        // 它需要单独一轮去定位 `UiTree` 的脏区重绘与父容器背景的配合。
        tree.invalidate_all();
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

impl WasmState {
    /// 从一个 canvas 构造：软后端按 canvas 尺寸分配像素缓冲，`Present` 取一次 2D 上下文。
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, String> {
        let (w, h) = (canvas.width(), canvas.height());
        let soft = SoftBackend::new(
            w,
            h,
            Box::new(crate::font_engine::WasmFontEngine::new()),
            Size::new(8.0, 16.0),
            None,
        )?;
        let cloned: HtmlCanvasElement = canvas.clone().into();
        let present = Present::new(cloned);
        Ok(Self {
            canvas,
            soft,
            present,
            tree: None,
            host: None,
            events: Rc::new(RefCell::new(VecDeque::new())),
            ime_input: None,
            listeners: Vec::new(),
            last_frame_ms: 0.0,
            raf: Rc::new(RefCell::new(None::<Closure<dyn FnMut(f64)>>)),
        })
    }
}

/// 浏览器端运行时。`run` 与 rAF 循环在 S1-b 实现。
pub struct WasmRuntime {
    /// 共享状态（rAF 回调通过 `Rc` 克隆持有）。
    pub state: Rc<RefCell<WasmState>>,
}

impl WasmRuntime {
    /// 从一个 canvas 构造运行时。
    pub fn new(canvas: HtmlCanvasElement) -> Result<Self, String> {
        Ok(Self {
            state: Rc::new(RefCell::new(WasmState::new(canvas)?)),
        })
    }

    /// 把 `host` 与建好的 `tree` 装进状态，准备出帧。
    ///
    /// 这里**只做装载**：rAF 循环的调度器留到下一次提交（见 `docs/WASM.md` S1-b 拆分）。
    pub fn install(&mut self, host: Box<dyn Host>, tree: UiTree) -> Result<(), String> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| "WasmRuntime 状态被重入借用".to_string())?;
        state.host = Some(host);
        state.tree = Some(tree);

        // **首次布局**：桌面在 `run_loop` 前有 `window.resize(&mut tree)`，
        // wasm 侧一直缺这一步 —— 组件 rect 全是 0，首帧 `dirty` 为空，
        // `render` 直接返回 `Skipped`，canvas 只剩页面 CSS 底色（看上去全黑）。
        //
        // 为何要解构：`tree.layout(..., &soft)` 同时可变借用 `tree` 与不可变借用 `soft`，
        // 两者同属一个结构体，只能分开借。
        {
            let WasmState {
                canvas, soft, tree, ..
            } = &mut *state;
            if let Some(tree) = tree.as_mut() {
                let (w, h) = (canvas.width(), canvas.height());
                tree.layout(Size::new(w as f32, h as f32), soft);
                // 布局完后必须标脏：否则首帧仍然会被判为 `Skipped`。
                tree.invalidate_all();
            }
        }

        // **P1：挂鼠标/滚轮监听器**。用 `listeners` 是否为空做幂等守卫：
        // 重复 `install()` 不会把同一事件注册两遍（否则每次移动会产生双倍事件）。
        install_listeners(&mut state)?;
        Ok(())
    }

    /// 装上 host/tree 并启动 rAF 循环；立即返回（不阻塞，见 `docs/WASM.md` D2）。
    pub fn run(
        &mut self,
        host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
    ) -> Result<(), String> {
        self.install(host, build())?;
        schedule_frame(Rc::clone(&self.state));
        Ok(())
    }

    /// 推一帧：把 `tree` / `host` 借出来，套上 `WasmWindow` 薄包装，交给 `frame_step`。
    ///
    /// **为什么单独暴露**：rAF 回调与测试都调它；`try_borrow_mut` 防同帧重入。
    pub fn tick(&mut self) -> Result<FrameStats, String> {
        let mut state = self
            .state
            .try_borrow_mut()
            .map_err(|_| "WasmRuntime 状态被重入借用".to_string())?;
        let WasmState {
            soft,
            present,
            canvas,
            tree,
            host,
            events,
            ..
        } = &mut *state;
        let tree = tree.as_mut().ok_or("tree 尚未安装")?;
        let host = host.as_mut().ok_or("host 尚未安装")?;
        let host: &mut dyn Host = host.as_mut();
        let mut window = WasmWindow {
            soft,
            present,
            canvas,
            events: &*events,
        };
        let evs = window.wait_events(0).unwrap_or_default();
        frame_step(
            &mut window,
            tree,
            host,
            &evs,
            std::time::Duration::from_millis(16),
        )
    }
}


/// **keydown 处理器。**
///
/// 抽成函数的理由：P3 起 keydown 要挂在**两个**目标上（canvas 与隐藏 input），
/// 而 `Closure` 不能 clone，所以必须能生成份本。
///
/// **一次按键只发一个事件**：可打印字符（无 ctrl/alt）只发 `Char`；否则发 `KeyDown`。
/// 两条都发会让 `TextInput` 插两次（`aaddmmiinn`）。
/// **keydown 处理器：只管控制键。**
///
/// **字符输入已经不在这里发了**——那一切交给隐藏 `<input>` 的 `input` 事件。
/// 原因：keydown 无法区分"用户真按了一个字符键"与"输入法提交后的幻影按键"，
/// 之前靠 `keyCode == 229` 与 60ms 时间窗口去猜，既会重复也会误吞。
/// `input` 事件由浏览器在"文本真正进入元素"时统一触发，天然不会重复。
///
/// 这里只剩两件事：把**非打印键**翻译成 `RawEvent::KeyDown`，以及阻止浏览器默认行为。
fn keydown_handler(
    events: Rc<RefCell<VecDeque<RawEvent>>>,
    composing: Rc<std::cell::Cell<bool>>,
) -> Closure<dyn FnMut(JsValue)> {
    Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
        let ke: &bindings::KeyboardEvent = val.unchecked_ref();
        // 组字期间不当按键：拼音字母不能影响控制键路径。
        if composing.get() || ke.is_composing() {
            return;
        }
        let code = ke.code();
        let key = ke.key();
        // 可打印字符**不在这里处理**：`input` 事件会负责它。
        if keys::char_from_dom(&key).is_some() && !ke.ctrl_key() && !ke.alt_key() {
            return;
        }
        let mods =
            keys::mods_from_event(ke.ctrl_key(), ke.shift_key(), ke.alt_key(), ke.meta_key());
        if let Some(k) = keys::key_from_dom(&code, &key) {
            events.borrow_mut().push_back(RawEvent::KeyDown { key: k, mods });
        }
        // 阻止浏览器默认：Tab 跳焦点、空格/方向键/翻页键滚页、Backspace 后退导航。
        if matches!(
            code.as_str(),
            "Tab" | "Space" | "Backspace" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight"
                | "Home" | "End" | "PageUp" | "PageDown"
        ) {
            let ev: &bindings::Event = val.unchecked_ref();
            ev.prevent_default();
        }
    })
}

/// **P1/P2：挂鼠标、滚轮与键盘监听器**。
///
/// 三条硬规则，都写进注释以免下次被"优化"掉：
///
/// 1. **闭包只捕获事件队列**，绝不碰 `WasmState`。事件可能在帧中途抵达，
///    那时 `WasmState` 正被 `&mut` 借用，闭包里碰它就会拿不到借而丢事件。
/// 2. **`wheel` 必须 `{ passive: false }`**：被动监听器里 `preventDefault()` 无效且控制台报警。
/// 3. **用 `unchecked_ref` 而不是 `dyn_into`**：监听器是按事件名注册的，类型已知，
///    再做一次 `instanceof` 只会让每次鼠标移动多一実 JS 往返。
fn install_listeners(state: &mut WasmState) -> Result<(), String> {
    // 幂等守卫：重复 `install()` 不能把同一事件注册两遍（否则一次移动产生两个事件）。
    if !state.listeners.is_empty() {
        return Ok(());
    }

    // canvas 默认不可聚焦；不设 `tabindex` 就永远收不到键盘事件。
    state.canvas.set_attribute("tabindex", "0");
    // 去掉焦点时浏览器给 canvas 画的**白色轮廓**（默认 `:focus-visible` 样式）。
    state.canvas.set_attribute("style", "outline: none;");

    // wasm-bindgen 的外部类型本身就是 `JsValue` 的包装；clippy 提示这里无需 `.into()`。
    let canvas_js: JsValue = state.canvas.clone();
    let events = Rc::clone(&state.events);
    // **IME 共享状态**（三处共用：keydown、input、composition）：
    //
    // - `composing`：正在拼音组字。组字期间的 `keydown` 与 `input` 都**不能**当成用户输入。
    let composing = Rc::new(std::cell::Cell::new(false));
    // **上一次已提交的串**：用来去重——某些浏览器在
    // `compositionend` 之后会把同一串再写回 `value` 并发一次 `input`。
    let last_committed: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    // **坐标换算所需：UI 树的 y 是"从底部往上"，DOM 的 `offsetY` 是"从顶部往下"。**
    // 公式在 `keys::to_ui_y`（带单测），这里只把画布高度抓一份。
    let canvas_h = state.canvas.height() as f32;

    // **P3：建一个隐藏 `input` 做输入法承接元素**。
    // 它是拼音组字事件的**唯一**来源；不建它，中文永远输不进去。
    // 它被放在屏幕外且 1x1 透明，看不见但可聚焦。
    let ime_input: Option<JsValue> = (|| {
        let window = bindings::global_window()?;
        let document = window.document()?;
        let body = document.body()?;
        let element = document.create_element("input");
        // 定位到屏幖外 + 完全透明；**不能用 `display:none`**（那样无法聚焦）。
        element.set_attribute(
            "style",
            "position:fixed;opacity:0;width:1px;height:1px;left:0;top:0;z-index:-1;border:0;padding:0;",
        );
        element.set_attribute("type", "text");
        element.set_attribute("autocomplete", "off");
        element.set_attribute("autocapitalize", "off");
        element.set_attribute("spellcheck", "false");
        let _ = body.append_child(&element);
        Some(JsValue::from(element))
    })();
    let mut listeners: Vec<Listener> = Vec::with_capacity(4);

    // ---- mousemove ----
    {
        let evs = Rc::clone(&events);
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let me: &bindings::MouseEvent = val.unchecked_ref();
            evs.borrow_mut().push_back(RawEvent::MouseMove {
                x: me.offset_x() as f32,
                y: keys::to_ui_y(canvas_h, me.offset_y() as f32),
            });
        });
        let target: bindings::EventTarget = canvas_js.clone().unchecked_into();
        target.add_event_listener("mousemove", &handler);
        listeners.push(Listener {
            target,
            kind: "mousemove".to_string(),
            handler,
        });
    }

    // ---- mousedown（顺手把焦点抢到 canvas）----
    {
        let evs = Rc::clone(&events);
        let ime_input_for_focus = ime_input.clone();
        let canvas_for_focus =
            Some(canvas_js.clone().unchecked_into::<bindings::HtmlCanvasElement>());
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let me: &bindings::MouseEvent = val.unchecked_ref();
            let Some(button) = keys::button_from_dom(me.button()) else {
                return; // 侧键（后退/前进）框架没有语义，丢弃
            };
            evs.borrow_mut().push_back(RawEvent::MouseDown {
                x: me.offset_x() as f32,
                y: keys::to_ui_y(canvas_h, me.offset_y() as f32),
                button,
            });
            // **关键：先 `preventDefault()` 再抢焦点**。
            // canvas 上有 `tabindex="0"`，浏览器在 `mousedown` 的**默认动作**里会把焦点移到
            // canvas；不阻止默认动作的话，我们 `input.focus()` 的结果在事件结束后被抢走。
            {
                let ev: &bindings::Event = val.unchecked_ref();
                ev.prevent_default();
            }
            // **把焦点给隐藏 input**（而不是 canvas），输入法才会启动；
            // 没有 input（极罕见：页面没有 body）时退回 canvas，至少保证英文可用。
            if let Some(input_js) = ime_input_for_focus.as_ref() {
                let input: &bindings::HtmlInputElement = input_js.unchecked_ref();
                input.focus();
            } else if let Some(canvas) = canvas_for_focus.as_ref() {
                canvas.focus();
            }
        });
        let target: bindings::EventTarget = canvas_js.clone().unchecked_into();
        target.add_event_listener("mousedown", &handler);
        listeners.push(Listener {
            target,
            kind: "mousedown".to_string(),
            handler,
        });
    }

    // ---- mouseup ----
    {
        let evs = Rc::clone(&events);
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let me: &bindings::MouseEvent = val.unchecked_ref();
            let Some(button) = keys::button_from_dom(me.button()) else {
                return;
            };
            evs.borrow_mut().push_back(RawEvent::MouseUp {
                x: me.offset_x() as f32,
                y: keys::to_ui_y(canvas_h, me.offset_y() as f32),
                button,
            });
        });
        let target: bindings::EventTarget = canvas_js.clone().unchecked_into();
        target.add_event_listener("mouseup", &handler);
        listeners.push(Listener {
            target,
            kind: "mouseup".to_string(),
            handler,
        });
    }

    // ---- wheel（必须非被动）----
    {
        let evs = Rc::clone(&events);
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let we: &bindings::WheelEvent = val.unchecked_ref();
            // WheelEvent 继承 MouseEvent：坐标与修饰键都能从同一个 JS 对象取。
            let me: &bindings::MouseEvent = val.unchecked_ref();
            let mods = keys::mods_from_event(
                me.ctrl_key(),
                me.shift_key(),
                me.alt_key(),
                me.meta_key(),
            );
            evs.borrow_mut().push_back(RawEvent::Wheel {
                x: me.offset_x() as f32,
                y: keys::to_ui_y(canvas_h, me.offset_y() as f32),
                delta: keys::wheel_delta_from_dom(we.delta_y()),
                mods,
            });
            // 阻止页面跟着滚（需配合下面的 passive: false）。
            let ev: &bindings::Event = val.unchecked_ref();
            ev.prevent_default();
        });
        let target: bindings::EventTarget = canvas_js.clone().unchecked_into();
        // **为何不传 `{ passive: false }`**：浏览器对 `wheel` 的"默认被动"仅适用于
        // `window` / `document` / `body`；挂在 **canvas 元素上的 wheel 监听器默认就是非被动**的，
        // `preventDefault()` 直接生效。若将来改挂在 `window` 上，就必须换回
        // `add_event_listener_with_options(..., { passive: false })`（绑定已就位）。
        target.add_event_listener("wheel", &handler);
        listeners.push(Listener {
            target,
            kind: "wheel".to_string(),
            handler,
        });
    }

    // ---- keydown（P2）----
    //
    // 框架的 `RawEvent` **没有 `KeyUp` 变体**（只有 `KeyDown` / `SystemKeyDown` / `Char` / `Ime`，
    // 见 `y901-runtime` 的 `RawEvent` 与 `translate()`），因此这里只挂 `keydown`。
    //
    // **两个目标都要挂**：焦点在 canvas 上时事件到 canvas，焦点在隐藏 input 上
    // （IME 模式）时到 input。两者不会同时收到，因此不会双发。
    {
        let mk = || keydown_handler(Rc::clone(&events), Rc::clone(&composing));
        let on_canvas = mk();
        let canvas_target: bindings::EventTarget = canvas_js.clone().unchecked_into();
        canvas_target.add_event_listener("keydown", &on_canvas);
        listeners.push(Listener {
            target: canvas_target,
            kind: "keydown".to_string(),
            handler: on_canvas,
        });
        if let Some(input_js) = ime_input.as_ref() {
            let on_input = mk();
            let input_target: bindings::EventTarget = input_js.clone().unchecked_into();
            input_target.add_event_listener("keydown", &on_input);
            listeners.push(Listener {
                target: input_target,
                kind: "keydown(ime)".to_string(),
                handler: on_input,
            });
        }
    }

    // ---- IME（P3）----
    //
    // `compositionstart/update/end` 全部来自那个隐藏 input：拼音组字只会发给
    // **可编辑元素**，canvas 永远收不到。
    if let Some(input_js) = ime_input.as_ref() {
        // ---- compositionstart：进入组字态，清空旧值 ----
        {
            let evs = Rc::clone(&events);
            let composing_for = Rc::clone(&composing);
            let input_for_clear = input_js.clone();
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
                composing_for.set(true);
                let el: &bindings::HtmlInputElement = input_for_clear.unchecked_ref();
                el.set_value("");
                evs.borrow_mut().push_back(RawEvent::Ime {
                    text: String::new(),
                    composition: true,
                });
            });
            let target: bindings::EventTarget = input_js.clone().unchecked_into();
            target.add_event_listener("compositionstart", &handler);
            listeners.push(Listener {
                target,
                kind: "compositionstart".to_string(),
                handler,
            });
        }

        // ---- compositionupdate：预编辑串（只画不提交）----
        {
            let evs = Rc::clone(&events);
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
                let ce: &bindings::CompositionEvent = val.unchecked_ref();
                evs.borrow_mut().push_back(RawEvent::Ime {
                    text: ce.data(),
                    composition: true,
                });
            });
            let target: bindings::EventTarget = input_js.clone().unchecked_into();
            target.add_event_listener("compositionupdate", &handler);
            listeners.push(Listener {
                target,
                kind: "compositionupdate".to_string(),
                handler,
            });
        }

        // ---- compositionend：**在这里提交**，并把已提交串记下来用于去重 ----
        //
        // 为何提交在这里而不是等 `input`：不同浏览器的事件顺序不同。
        // 若 `input` 先于本事件到达，那时还处于组字态（`composing == true`），
        // 会被忽略 → 提交丢失，界面上的预编辑串就停在那里。
        // 在本事件提交并**清空 value**，则无论顺序如何都只提交一次。
        //
        // 为何还要 `last_committed`：个别浏览器会在本事件**之后**把
        // 同一串再写回 `value` 并发 `input`，那一次必须被认出并丢弃。
        {
            let evs = Rc::clone(&events);
            let composing_for = Rc::clone(&composing);
            let last_for = Rc::clone(&last_committed);
            let input_for_clear = input_js.clone();
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
                let ce: &bindings::CompositionEvent = val.unchecked_ref();
                let text = ce.data();
                composing_for.set(false);
                let el: &bindings::HtmlInputElement = input_for_clear.unchecked_ref();
                el.set_value("");
                if !text.is_empty() {
                    *last_for.borrow_mut() = text.clone();
                    evs.borrow_mut().push_back(RawEvent::Ime {
                        text,
                        composition: false,
                    });
                }
            });
            let target: bindings::EventTarget = input_js.clone().unchecked_into();
            target.add_event_listener("compositionend", &handler);
            listeners.push(Listener {
                target,
                kind: "compositionend".to_string(),
                handler,
            });
        }

        // ---- input：**字符输入的唯一出口** ----
        //
        // 无论英文字符还是中文选词提交，浏览器都会在文本进入 input 后发这个事件。
        // 因此它天然不会重复、也不用猜幻影按键。
        {
            let evs = Rc::clone(&events);
            let composing_for = Rc::clone(&composing);
            let last_for = Rc::clone(&last_committed);
            let input_for_read = input_js.clone();
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
                let el: &bindings::HtmlInputElement = input_for_read.unchecked_ref();
                // 组字中：不读也不清（清了会丢掉正在组的字）。
                if composing_for.get() {
                    return;
                }
                let text = el.value();
                if text.is_empty() {
                    return;
                }
                el.set_value("");
                // **去重**：与刚在 `compositionend` 提交过的串相同 ⇒ 这是浏览器的回写，丢弃。
                if *last_for.borrow() == text {
                    return;
                }
                // 统一走 `Ime { composition: false }` → 运行时翻译成 `UiEvent::Text`，
                // 一次插入一串，无论长短。
                evs.borrow_mut().push_back(RawEvent::Ime {
                    text,
                    composition: false,
                });
            });
            let target: bindings::EventTarget = input_js.clone().unchecked_into();
            target.add_event_listener("input", &handler);
            listeners.push(Listener {
                target,
                kind: "input".to_string(),
                handler,
            });
        }
    }

    state.ime_input = ime_input;
    state.listeners = listeners;
    Ok(())
}

/// 调度下一帧：先 drop 旧闭包，再 wrap 新闭包并存起来。
///
/// **为何是自由函数**：`requestAnimationFrame` 的回调必须 `'static`，
/// 拿不到 `&mut WasmRuntime`；而循环里又要再次调度。
/// 把"调度"抽成只靠 `Rc<RefCell<WasmState>>` 的函数，递归就成立了。
///
/// **泄漏风险**：每帧 `take` 掉上一个 `Closure` 后才存入新的，
/// 因此**始终只有一个**存活的闭包，不会随帧数增长。
pub fn schedule_frame(state: Rc<RefCell<WasmState>>) {
    // 取出 holder（不持有 state 的借用，避开接下来的可变借用）
    let holder = {
        let Ok(s) = state.try_borrow() else {
            return; // 重入：本帧就不调度了，不 panic
        };
        Rc::clone(&s.raf)
    };
    // 先 drop 旧闭包（否则旧回调还持着一个 handler）
    if let Ok(mut slot) = holder.try_borrow_mut() {
        *slot = None;
    }

    let for_frame = Rc::clone(&state);
    let for_next = Rc::clone(&state);
    let cb = Closure::new(move |ts: f64| {
        // ---- 一帧 ----
        {
            let Ok(mut s) = for_frame.try_borrow_mut() else {
                return; // 重入保护：不 panic，丢掉这一帧
            };
            let WasmState {
                soft,
                present,
                canvas,
                tree,
                host,
                events,
                last_frame_ms,
                ..
            } = &mut *s;
            // dt 来自 rAF 时间戳（毫秒）；首帧默认 16ms。
            let dt_ms = if *last_frame_ms > 0.0 && ts > *last_frame_ms {
                (ts - *last_frame_ms).clamp(1.0, 100.0)
            } else {
                16.0
            };
            *last_frame_ms = ts;
            if let (Some(tree), Some(host)) = (tree.as_mut(), host.as_mut()) {
                let host: &mut dyn Host = host.as_mut();
                let mut window = WasmWindow {
                    soft,
                    present,
                    canvas,
                    events: &*events,
                };
                let evs = window.wait_events(0).unwrap_or_default();
                let _ = frame_step(
                    &mut window,
                    tree,
                    host,
                    &evs,
                    std::time::Duration::from_millis(dt_ms as u64),
                );
            }
        }
        // ---- 排下一帧（必须在上面的借用结束之后）----
        schedule_frame(Rc::clone(&for_next));
    });

    if let Some(win) = bindings::global_window() {
        win.request_animation_frame(&cb);
        if let Ok(mut slot) = holder.try_borrow_mut() {
            *slot = Some(cb);
        }
    }
}

impl y901_runtime::PlatformRuntime for WasmRuntime {
    /// 装 host/tree 并启动 rAF 循环；**立即返回 `Ok(())`**（浏览器没有可以阻塞的主循环）。
    fn run(
        &mut self,
        host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
        _options: y901_runtime::Options,
    ) -> Result<(), String> {
        WasmRuntime::run(self, host, build)
    }
}
