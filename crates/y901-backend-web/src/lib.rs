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
use y901_render::soft::SoftBackend;
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{Host, RawEvent};
use y901_ui::{RenderCtx, Size, UiTree};

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

        // [CORNER-TEST] 四角颜色探针：**唯一目的是确认浏览器里跑的版本与 Y 轴方向**。
        // 左上=红、右上=绿、左下=蓝、右下=黄。确诊后立即删除。
        {
            let (cw, chh) = (
                self.soft.width() as f32,
                self.soft.height() as f32,
            );
            let d = 40.0;
            self.soft.fill_rect(
                y901_ui::Rect::new(0.0, 0.0, d, d),
                y901_ui::Color::rgba(1.0, 0.0, 0.0, 1.0),
            ); // 红 —— 缓冲左上
            self.soft.fill_rect(
                y901_ui::Rect::new(cw - d, 0.0, d, d),
                y901_ui::Color::rgba(0.0, 1.0, 0.0, 1.0),
            ); // 绿 —— 缓冲右上
            self.soft.fill_rect(
                y901_ui::Rect::new(0.0, chh - d, d, d),
                y901_ui::Color::rgba(0.0, 0.0, 1.0, 1.0),
            ); // 蓝 —— 缓冲左下
            self.soft.fill_rect(
                y901_ui::Rect::new(cw - d, chh - d, d, d),
                y901_ui::Color::rgba(1.0, 1.0, 0.0, 1.0),
            ); // 黄 —— 缓冲右下
        }

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
            events: VecDeque::new(),
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
        let events: Vec<RawEvent> = events.drain(..).collect();
        let mut window = WasmWindow {
            soft,
            present,
            canvas,
        };
        frame_step(&mut window, tree, host, &events, std::time::Duration::from_millis(16))
    }
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
                let evs: Vec<RawEvent> = events.drain(..).collect();
                let mut window = WasmWindow {
                    soft,
                    present,
                    canvas,
                };
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
