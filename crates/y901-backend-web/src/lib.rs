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

/// 光标 CSS 是否需要重写：值相同、且优先级已经是 `important` 时跳过。
///
/// **为什么重要**：鼠标在同一个控件上移动时，`cursor_at` 的结果逐帧不变；
/// 每次移动都 `setProperty` 会让浏览器样式系统反复失效（实测 60 帧内有 25 次重复写）。
/// 这里只做判断，真正的读取/写入留在调用点（纯函数便于单测）。
fn cursor_needs_write(current: &str, priority: &str, target: &str) -> bool {
    current != target || priority != "important"
}

impl WasmState {
    /// **彻底拆除这个 runtime**：取消帧、卸监听器、移除隐藏 input、释放树。
    ///
    /// 为何需要：单页里热重启 / 测试重载时，若不拆，
    /// ① 已排队的 rAF 会在 `Closure` 被 drop 后跑一次 → `closure invoked after dropped`；
    /// ② 监听器 Closure 不释放，旧 runtime 的回调继续往旧队列推事件；
    /// ③ 隐藏 `input` 每次都往 `body` 里多塞一个。
    ///
    /// 调用时机：由页面 / JS 侧显式调 `y901_dispose()`；也可在重新
    /// `y901_start()` 之前调一次。
    pub fn dispose(&mut self) {
        // 1) 取消尚未执行的 rAF，并丢掉存储的 Closure。
        if let Some(win) = bindings::global_window() {
            let h = self.raf_handle.replace(0);
            if h != 0 {
                win.cancel_animation_frame(h);
            }
        }
        if let Ok(mut slot) = self.raf.try_borrow_mut() {
            *slot = None;
        }
        // 2) 卸掉所有 DOM 监听器（Closure 被 drop 时 wasm-bindgen 会摘除 JS 监听）。
        self.listeners.clear();
        // 3) 把隐藏 input 从 DOM 移除，避免页面上积累。
        if let Some(input_js) = self.ime_input.take() {
            let el: &bindings::Element = input_js.unchecked_ref();
            el.remove();
        }
        // 4) 释放树与 host（业务层资源随之释放），清空事件队列。
        self.tree = None;
        self.host = None;
        self.events.borrow_mut().clear();
    }

    /// **处理回调里只能标记、不能直接做的两件事**：窗口尺寸变化、鼠标光标形状。
    ///
    /// 两者都需要 `tree`（命中测试 / 重排），而 DOM 回调拿不到 `WasmState`。
    /// 因此回调只置标志，真正的动作在每帧开头做。
    fn apply_pending(&mut self) {
        // ---- 窗口尺寸（P4）----
        if self.pending_resize.replace(false) {
            let (w, h) = (
                self.canvas.client_width().max(1),
                self.canvas.client_height().max(1),
            );
            if w != self.canvas.width() || h != self.canvas.height() {
                self.canvas.set_width(w);
                self.canvas.set_height(h);
                self.soft.resize(w, h);
                // **尺寸变了 ⇒ 布局缓存必须作废**（与换字体同理）。
                {
                    let WasmState { soft, tree, .. } = self;
                    if let Some(tree) = tree.as_mut() {
                        tree.clear_layout_cache();
                        tree.layout(Size::new(w as f32, h as f32), soft);
                        tree.invalidate_all();
                    }
                }
                // **同步 Y 翻转所用的高度**（否则拉伸后鼠标全错）。
                self.canvas_h.set(h as f32);
                self.events
                    .borrow_mut()
                    .push_back(RawEvent::Resize { w, h });
            }
        }

        // ---- 鼠标光标形状（P4）----
        if let Some((x, y)) = self.pending_cursor.take() {
            if let Some(tree) = self.tree.as_ref() {
                let css = match tree.cursor_at(y901_ui::Pos::new(x, y)) {
                    y901_ui::Cursor::Pointer => "pointer",
                    y901_ui::Cursor::Text => "text",
                    y901_ui::Cursor::ResizeHorizontal => "ew-resize",
                    y901_ui::Cursor::ResizeVertical => "ns-resize",
                    y901_ui::Cursor::Grab => "grab",
                    y901_ui::Cursor::Default => "default",
                };
                // **只改单个属性**：`setAttribute("style", ...)` 会整条覆盖，
                // 把页面 CSS 的 `width/height` 也抹掉（resize 就此失效）。
                // **`important`**：防止页面里任何 `canvas { cursor: … }` 规则把它盖掉。
                let style = self.canvas.style();
                let current = style.get_property_value("cursor");
                let priority = style.get_property_priority("cursor");
                if cursor_needs_write(&current, &priority, css) {
                    style.set_property_with_priority("cursor", css, "important");
                }
            }
        }
    }
}

/// 一个已注册的 DOM 监听器。
///
/// **为何要存起来**：`Closure` 一旦被 `drop`，JS 侧虽然还挂着监听器，
/// 但一触发就抛 `closure invoked after dropped`。所以闭包必须跟着
/// `WasmState` 活到页面结束。
///
/// **P0 阶段只定义不安装**：\u8fd9个结构会在 P1 里被真正填满\u3002
pub struct Listener {
    /// 挂在谁上（`window` 或 canvas）。
    pub target: bindings::EventTarget,
    /// 事件名（`"mousemove"` / `"keydown"` …）。
    pub kind: String,
    /// 回调本体（**持有它就是保存它**）。
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
    /// **待处理的窗口尺寸变化标志**。
    ///
    /// 为何不在 `resize` 回调里直接改：回调拿不到 `WasmState`（只捕获共享句柄），
    /// 而改尺寸要同时动 `canvas` / `soft` / `tree`，只能在帧循环里做。
    pub pending_resize: Rc<std::cell::Cell<bool>>,
    /// **当前画布高度**（用于 DOM 坐标→UI 坐标的 Y 翻转）。
    ///
    /// **为何不在回调里直接读 `canvas.height()`**：回调拿不到 `WasmState`；
    /// 而这个值**必须随 resize 变化** —— 旧代码用安装时的快照，
    /// 窗口拉伸后 Y 翻转就彻底错位（"只有端口偶尔生效"就是这个原因）。
    pub canvas_h: Rc<std::cell::Cell<f32>>,
    /// **待处理的光标位置**（UI 坐标），同理：命中测试需要 `tree`。
    pub pending_cursor: Rc<std::cell::Cell<Option<(f32, f32)>>>,
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
    /// **已排队的 rAF 句柄**（0 = 没有），`dispose()` 靠它取消尚未执行的帧。
    pub raf_handle: Rc<std::cell::Cell<i32>>,
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

        // **极简重写：只有一条路——清屏 → 整树重绘 → 整帧上屏**。
        //
        // 为何废弃局部重绘 + 局部上传：为每帧省下大约 10ms，
        // 却引入了脏区截取/边界取整/阈值回退/ImageData 复用/
        // 两条路径坐标一致性……整套只会出 bug 的代码。
        // release 下 800×600 全帧渲染+上屏约 15ms，完全能稳定 60fps。
        let (vw, vh) = self.viewport();
        self.soft.clear();
        // **必须整树重绘**：上一行已把整张缓冲洗成清屏色，
        // 而 `UiTree::render` 在脏区不是 `full` 时**只画脏区**。
        // 两者相乘的结果就是那个著名的故障：
        // **"整屏变黑，只剩下刚点击的那个输入框"**——
        // 因为那一帧的脏区恰好就是输入框的矩形。
        // 这不是 clip 泄漏（裁剪区已在渲染末尾恢复），而是"清了却没画"。
        tree.invalidate_all();
        tree.render(self.soft);
        self.present.present(self.soft.pixels(), vw, vh);
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
        let mut soft = SoftBackend::new(
            w,
            h,
            Box::new(crate::font_engine::WasmFontEngine::new()),
            Size::new(8.0, 16.0),
            None,
        )?;
        // **比例字体（微软雅黑等）按真实字宽推进**：
        // 桌面与 GPU 路径不受影响（默认仍为格宽布局）。
        soft.set_proportional_text(true);
        let cloned: HtmlCanvasElement = canvas.clone().into();
        let present = Present::new(cloned);
        Ok(Self {
            canvas,
            soft,
            present,
            tree: None,
            host: None,
            events: Rc::new(RefCell::new(VecDeque::new())),
            canvas_h: Rc::new(std::cell::Cell::new(0.0)),
            pending_resize: Rc::new(std::cell::Cell::new(false)),
            pending_cursor: Rc::new(std::cell::Cell::new(None)),
            ime_input: None,
            listeners: Vec::new(),
            last_frame_ms: 0.0,
            raf: Rc::new(RefCell::new(None::<Closure<dyn FnMut(f64)>>)),
            raf_handle: Rc::new(std::cell::Cell::new(0)),
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
        state.apply_pending();
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
/// **window 级统一 keydown 处理器**（重构核心）。
///
/// 旧实现把 `keydown` 同时挂在 canvas 与隐藏 input 上，并用自己的
/// `Cell<bool> composing` 挡住组字期按键：
/// - 焦点在两个元素之间流转时，事件只会到其中一个；
/// - 浏览器不保证发 `compositionend`，标志一卡死就把**所有按键永久吞掉**。
///
/// 现在只有**一个**监听器，挂在 `window` 上（事件必然经过），靠两件事实判断：
/// ① `document.activeElement` 是不是我们自己的元素（canvas 或隐藏 input）；
/// ② 这次事件本身是不是组字产生的（`event.isComposing`）。
/// 判断逻辑在纯函数 `keys::should_send_keydown` 里（带单测），这里只做取值与投递。
fn keydown_handler(events: Rc<RefCell<VecDeque<RawEvent>>>) -> Closure<dyn FnMut(JsValue)> {
    Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
        let ke: &bindings::KeyboardEvent = val.unchecked_ref();
        // **极简接线**：不看焦点在哪个元素上，只看两件事——
        // ① 这次事件是不是输入法组字产生的（`isComposing`）；
        // ② 它是不是可打印字符（那些由 `input` 事件提交）。
        let code = ke.code();
        let key = ke.key();
        // **只看事实**：这次事件是不是组字产生的。不看任何自己维护的标志。
        if !keys::should_send_keydown(
            ke.is_composing(),
            &code,
            keys::char_from_dom(&key).is_some(),
            ke.ctrl_key(),
            ke.alt_key(),
        ) {
            return;
        }
        let mods =
            keys::mods_from_event(ke.ctrl_key(), ke.shift_key(), ke.alt_key(), ke.meta_key());
        if let Some(k) = keys::key_from_dom(&code, &key) {
            events.borrow_mut().push_back(RawEvent::KeyDown { key: k, mods });
        }
        // 阻止浏览器默认：Tab 跳焦点、空格滚动、Backspace 返回上一页、
        // 方向键/翻页键滚动页面。**Backspace 尤其重要**：不阻止就会跳转。
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

/// 隐藏 IME `<input>` 的固定 id（统一监听靠它判断焦点）。
pub const IME_ID: &str = "y901-ime";

/// 焦点是否在指定 id 的元素上。
fn active_is(id: &str) -> bool {
    active_element_id().as_deref() == Some(id)
}

/// `window` 的 `EventTarget` 视图（回调里首选它挂监听）。
fn global_window_target() -> bindings::EventTarget {
    let win_js: JsValue = bindings::global_window().map(JsValue::from).unwrap_or(JsValue::NULL);
    win_js.unchecked_into()
}

/// 当前焦点元素的 `id`（`None` = 无焦点）。
fn active_element_id() -> Option<String> {
    let window = bindings::global_window()?;
    let document = window.document()?;
    let el = document.active_element()?;
    Some(el.id())
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
    // **不再写整条 `style`**：那会把页面 CSS 里的 `width/height` 一并抹掉。
    // `outline: none` 现在写在 `www/index.html` 的 `<style>` 里。

    // wasm-bindgen 的外部类型本身就是 `JsValue` 的包装；clippy 提示这里无需 `.into()`。
    let canvas_js: JsValue = state.canvas.clone();
    let events = Rc::clone(&state.events);
    // **IME 共享状态**（三处共用：keydown、input、composition）：
    //
    // - `composing`：正在拼音组字。组字期间的 `keydown` 与 `input` 都**不能**当成用户输入。
    // **用户是否交互过**（点过 canvas）。用于避免"页面刚打开就抢焦点"。
    let interacted_cell = Rc::new(std::cell::Cell::new(false));
    // **blur 时焦点是否在我们的隐藏 input 上**。
    let input_had_focus_cell = Rc::new(std::cell::Cell::new(false));
    // **组字标志已彻底删除**：改用 `event.isComposing`（事实），不再自己记状态。
    // **上一次 `compositionend` 提交的串**（ASCII 与中文的交叉去重用）。
    let last_committed: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    // **坐标换算所需：UI 树的 y 是"从底部往上"，DOM 的 `offsetY` 是"从顶部往下"。**
    // 公式在 `keys::to_ui_y`（带单测），这里只把画布高度抓一份。
    // **实时高度**：回调读它做 Y 翻转，resize 后由 `apply_pending` 更新。
    let canvas_h = Rc::clone(&state.canvas_h);
    canvas_h.set(state.canvas.height() as f32);

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
        // **一个固定 id**：统一监听时用 `document.activeElement.id` 判断焦点在谁身上。
        element.set_attribute("id", IME_ID);
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
        let cursor_for = Rc::clone(&state.pending_cursor);
        let canvas_h = Rc::clone(&canvas_h);
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let me: &bindings::MouseEvent = val.unchecked_ref();
            let (mx, my) = (
                me.offset_x() as f32,
                keys::to_ui_y(canvas_h.get(), me.offset_y() as f32),
            );
            // **记下待处理的光标位置**：命中测试需要 `tree`，回调里拿不到。
            cursor_for.set(Some((mx, my)));
            evs.borrow_mut().push_back(RawEvent::MouseMove { x: mx, y: my });
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
        let canvas_h = Rc::clone(&canvas_h);
        let ime_input_for_focus = ime_input.clone();
        let interacted_for = Rc::clone(&interacted_cell);
        let input_focus_for = Rc::clone(&input_had_focus_cell);
        let canvas_for_focus =
            Some(canvas_js.clone().unchecked_into::<bindings::HtmlCanvasElement>());
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let me: &bindings::MouseEvent = val.unchecked_ref();
            let Some(button) = keys::button_from_dom(me.button()) else {
                return; // 侧键（后退/前进）框架没有语义，丢弃
            };
            evs.borrow_mut().push_back(RawEvent::MouseDown {
                x: me.offset_x() as f32,
                y: keys::to_ui_y(canvas_h.get(), me.offset_y() as f32),
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
            interacted_for.set(true);
            if let Some(input_js) = ime_input_for_focus.as_ref() {
                let input: &bindings::HtmlInputElement = input_js.unchecked_ref();
                input.focus();
                input_focus_for.set(true);
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
        let canvas_h = Rc::clone(&canvas_h);
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
            let me: &bindings::MouseEvent = val.unchecked_ref();
            let Some(button) = keys::button_from_dom(me.button()) else {
                return;
            };
            evs.borrow_mut().push_back(RawEvent::MouseUp {
                x: me.offset_x() as f32,
                y: keys::to_ui_y(canvas_h.get(), me.offset_y() as f32),
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
                y: keys::to_ui_y(canvas_h.get(), me.offset_y() as f32),
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

    // ---- window \u7ea7\u952e\u76d8\u4e0e\u6587\u672c\u76d1\u542c\uff08**\u91cd\u6784**\uff09----
    //
    // \u5168\u90e8\u6302\u5728 **window** \u4e0a\uff1a\u65e0\u8bba\u7126\u70b9\u5728 canvas \u8fd8\u662f\u9690\u85cf input\uff0c\u4e8b\u4ef6\u90fd\u4f1a\u7ecf\u8fc7 window\u3002
    // \u5904\u7406\u5668\u53ea\u770b\u4e24\u4ef6\u4e8b\uff1a`document.activeElement` \u662f\u4e0d\u662f\u6211\u4eec\u81ea\u5df1\u7684\u5143\u7d20\uff0c
    // \u4ee5\u53ca\u4e8b\u4ef6\u81ea\u5df1\u7684 `isComposing`\u3002**\u4e0d\u518d\u7ef4\u62a4\u7ec4\u5b57\u6807\u5fd7**\u2014\u2014
    // \u90a3\u4e2a\u6807\u5fd7\u4e00\u65e6\u56e0\u4e3a\u6d4f\u89c8\u5668\u4e0d\u53d1 `compositionend` \u800c\u5361\u6b7b\uff0c\u5c31\u4f1a\u628a\u6240\u6709\u6309\u952e\u6c38\u4e45\u541e\u6389\u3002

    // ---- keydown\uff1a\u53ea\u53d1\u63a7\u5236\u952e\uff08\u5b57\u7b26\u7531 `input` \u63d0\u4ea4\uff09----
    if bindings::global_window().is_some() {
        let handler = keydown_handler(Rc::clone(&events));
        let target: bindings::EventTarget = global_window_target();
        target.add_event_listener("keydown", &handler);
        listeners.push(Listener {
            target,
            kind: "keydown(window)".to_string(),
            handler,
        });
    }

    // ---- composition\uff08\u4e0e keydown \u4e00\u6837\u6302 window\uff09----
    if let Some(input_js) = ime_input.as_ref() {
        // compositionstart\uff1a\u8fdb\u5165\u7ec4\u5b57\uff0c\u6e05\u7a7a\u65e7\u503c\uff0c\u7ed9\u754c\u9762\u4e00\u4e2a\u7a7a\u7684\u9884\u7f16\u8f91\u4e32
        {
            let evs = Rc::clone(&events);
            let input_for_clear = input_js.clone();
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
                if !active_is(IME_ID) {
                    return; // \u7ec4\u5b57\u53d1\u5728\u522b\u7684\u8f93\u5165\u6846\u4e0a\uff1a\u4e0d\u5f52\u6211\u4eec
                }
                let el: &bindings::HtmlInputElement = input_for_clear.unchecked_ref();
                el.set_value("");
                evs.borrow_mut().push_back(RawEvent::Ime {
                    text: String::new(),
                    composition: true,
                });
            });
            let target: bindings::EventTarget =
                global_window_target();
            target.add_event_listener("compositionstart", &handler);
            listeners.push(Listener {
                target,
                kind: "compositionstart(window)".to_string(),
                handler,
            });
        }
        // compositionupdate\uff1a\u53ea\u753b\u4e0d\u63d0\u4ea4
        {
            let evs = Rc::clone(&events);
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
                if !active_is(IME_ID) {
                    return;
                }
                let ce: &bindings::CompositionEvent = val.unchecked_ref();
                evs.borrow_mut().push_back(RawEvent::Ime {
                    text: ce.data(),
                    composition: true,
                });
            });
            let target: bindings::EventTarget =
                global_window_target();
            target.add_event_listener("compositionupdate", &handler);
            listeners.push(Listener {
                target,
                kind: "compositionupdate(window)".to_string(),
                handler,
            });
        }
        // compositionend\uff1a**\u552f\u4e00\u63d0\u4ea4\u70b9**\uff0c\u8bb0\u4e0b\u5df2\u63d0\u4ea4\u4e32\u4f9b input \u53bb\u91cd
        {
            let evs = Rc::clone(&events);
            let last_for = Rc::clone(&last_committed);
            let input_for_clear = input_js.clone();
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
                let ce: &bindings::CompositionEvent = val.unchecked_ref();
                let text = ce.data();
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
            let target: bindings::EventTarget =
                global_window_target();
            target.add_event_listener("compositionend", &handler);
            listeners.push(Listener {
                target,
                kind: "compositionend(window)".to_string(),
                handler,
            });
        }

        // ---- input：**ASCII 字符的唯一通路** ----
        //
        // 极简接线：只看事件自己的 `isComposing`。
        // 组字中的 `input` 是预编辑（不能当正文插入），组字结束由
        // `compositionend` 提交；非组字的 `input` 就是真实输入。
        {
            let evs = Rc::clone(&events);
            let input_for_read = input_js.clone();
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |val: JsValue| {
                let ev: &bindings::Event = val.unchecked_ref();
                if ev.is_composing() {
                    return;
                }
                let el: &bindings::HtmlInputElement = input_for_read.unchecked_ref();
                let text = el.value();
                if text.is_empty() {
                    return;
                }
                el.set_value("");
                evs.borrow_mut().push_back(RawEvent::Ime {
                    text,
                    composition: false,
                });
            });
            let target: bindings::EventTarget = global_window_target();
            target.add_event_listener("input", &handler);
            listeners.push(Listener {
                target,
                kind: "input(window)".to_string(),
                handler,
            });
        }


        // ---- blur\uff1a\u9690\u85cf input \u79bb\u7126\u2192\u6e05\u6389\u53bb\u91cd\u6807\u8bb0 ----
        {
            let last_for = Rc::clone(&last_committed);
            let handler = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
                last_for.borrow_mut().clear();
            });
            let target: bindings::EventTarget = input_js.clone().unchecked_into();
            target.add_event_listener("blur", &handler);
            listeners.push(Listener {
                target,
                kind: "blur(ime)".to_string(),
                handler,
            });
        }
    }


    // ---- window resize（P4）----
    //
    // 挂在 **window** 上而不是 canvas：画布自身尺寸不变时，是窗口变化才会引起
    // `clientWidth/Height` 变化（页面 CSS 里 canvas 是 100%。）
    if let Some(win) = bindings::global_window() {
        let pending = Rc::clone(&state.pending_resize);
        let handler = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
            pending.set(true);
        });
        let win_js: JsValue = win.into();
        let target: bindings::EventTarget = win_js.unchecked_into();
        target.add_event_listener("resize", &handler);
        listeners.push(Listener {
            target,
            kind: "resize".to_string(),
            handler,
        });
    }

    // ---- window focus / blur：回来时把焦点还给隐藏 input ----
    //
    // 场景：用户切到别的标签页再回来，焦点丢了 → 键盘不响应。
    // 但**不能无条件抢**：若用户正在地址栏打字，抢焦点会打断他。
    // 判定用纯函数 `keys::should_refocus`（带单测）。
    if let Some(win) = bindings::global_window() {
        let input_js = ime_input.clone();
        let interacted_for = Rc::clone(&interacted_cell);
        let had_focus_for = Rc::clone(&input_had_focus_cell);
        let on_blur = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
            // blur 时记下"刚刚焦点还在我们这里吗"，供随后的 focus 判定用。
            had_focus_for.set(interacted_for.get());
        });
        let win_js: JsValue = win.clone();
        let win_target: bindings::EventTarget = win_js.unchecked_into();
        win_target.add_event_listener("blur", &on_blur);
        listeners.push(Listener {
            target: win_target,
            kind: "blur".to_string(),
            handler: on_blur,
        });

        let interacted_for = Rc::clone(&interacted_cell);
        let had_focus_for = Rc::clone(&input_had_focus_cell);
        let on_focus = Closure::<dyn FnMut(JsValue)>::new(move |_val: JsValue| {
            if keys::should_refocus(interacted_for.get(), had_focus_for.get()) {
                if let Some(input_js) = input_js.as_ref() {
                    let input: &bindings::HtmlInputElement = input_js.unchecked_ref();
                    input.focus();
                }
            }
        });
        let win_js: JsValue = win.into();
        let win_target: bindings::EventTarget = win_js.unchecked_into();
        win_target.add_event_listener("focus", &on_focus);
        listeners.push(Listener {
            target: win_target,
            kind: "focus".to_string(),
            handler: on_focus,
        });
    }

    // **首帧先同步一次尺寸**：HTML 里 canvas 属性是 800x600，而 CSS 尺寸可能不同。
    state.pending_resize.set(true);
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
        (Rc::clone(&s.raf), Rc::clone(&s.raf_handle))
    };
    let (holder, state_raf_handle) = holder;
    // 先 drop 旧闭包（否则旧回调还持着一个 handler）
    if let Ok(mut slot) = holder.try_borrow_mut() {
        *slot = None;
    }

    // **弱引用破环**：state → raf holder → closure；若 closure 再强引用 state 就是一个环，
    // runtime 永远无法释放。Weak 后，外部所有 Rc drop 时 state 立即释放，
    // 下一次 upgrade() 返回 None，循环自然停下。
    let for_frame = Rc::downgrade(&state);
    let for_next = Rc::downgrade(&state);
    let raf_handle = Rc::clone(&state_raf_handle);
    let cb = Closure::new(move |ts: f64| {
        // ---- 一帧 ----
        {
            let Some(frame_state) = for_frame.upgrade() else {
                return; // runtime 已销毁：不再跑任何帧
            };
            let Ok(mut s) = frame_state.try_borrow_mut() else {
                return; // 重入保护：不 panic，丢掉这一帧
            };
            // 先处理"回调里只能标记"的两件事：窗口尺寸变化与鼠标光标形状。
            s.apply_pending();
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
        if let Some(next_state) = for_next.upgrade() {
            schedule_frame(next_state);
        }
    });

    if let Some(win) = bindings::global_window() {
        let h = win.request_animation_frame(&cb);
        raf_handle.set(h);
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

#[cfg(test)]
mod cursor_write_tests {
    use super::cursor_needs_write;

    /// 值没变、优先级已是 `important`：**不写**（这就是冗余写入的消除点）。
    #[test]
    fn same_value_and_priority_is_skipped() {
        assert!(!cursor_needs_write("text", "important", "text"));
        assert!(!cursor_needs_write("pointer", "important", "pointer"));
    }

    /// 值变了必须写。
    #[test]
    fn changed_value_is_written() {
        assert!(cursor_needs_write("default", "important", "text"));
        assert!(cursor_needs_write("", "", "pointer"));
    }

    /// 值相同但优先级被别的规则清成普通级时仍要补 `important`，
    /// 否则页面里 `canvas { cursor: … }` 会盖掉控件的形状。
    #[test]
    fn missing_important_priority_is_restored() {
        assert!(cursor_needs_write("text", "", "text"));
        assert!(cursor_needs_write("text", "user", "text"));
    }
}
