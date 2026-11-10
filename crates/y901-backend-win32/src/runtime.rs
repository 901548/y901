//! Win32 平台运行时。
//!
//! 平台特有的部分（窗口创建、D3D/GDI 提交、IME、光标、快照）都在 [`Window`] 里；
//! 事件循环本身与 X11 共用 `y901-backend-common::runtime::run_loop`。

use y901_runtime::{Host, Options, PlatformRuntime};
use y901_ui::UiTree;

use crate::window::Window;

/// Win32 平台运行时。
pub struct Win32Runtime;

impl PlatformRuntime for Win32Runtime {
    fn run(
        &mut self,
        mut host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
        options: Options,
    ) -> Result<(), String> {
        let mut window = Window::create(&options)?;

        // 把窗口的唤醒句柄交给 Host（`WindowWaker` 已提到模块级）。
        host.attach_waker(std::sync::Arc::new(WindowWaker(window.wake_handle())));

        let mut tree = build();

        // 首帧前的初始布局（按客户区尺寸）。
        window.resize(&mut tree);

        // 主循环：事件 → 命令 → idle → 重排 → 主题 → 平台收尾 → 渲染 → 快照 → 统计。
        y901_backend_common::runtime::run_loop(&mut window, &mut tree, host.as_mut())?;

        // 退出前补一次 Tick，让在跑的 PTY 会话有机会收尾。
        let _ = tree.dispatch(&y901_ui::UiEvent::Tick);
        Ok(())
    }

    /// 每窗口一线程。
    ///
    /// 为何并行可行：Win32 的消息队列**天生按线程隔离**（`GetMessage` 只取本线程消息），
    /// 所以每个线程跑自己的 `run_loop` 不需要任何分发器。
    ///
    /// `Window` / `Host` / `UiTree` **都在线程内创建** —— 它们都不是 `Send`，
    /// 不能跨线程传递（D3D11 设备有线程亲和性）。
    fn run_many(
        &mut self,
        options_list: Vec<Options>,
        make: y901_runtime::WindowFactory,
    ) -> Result<(), String> {
        // `FnMut` 不能直接被多个线程共享，用 `Arc<Mutex<>>` 包一层。
        let make = std::sync::Arc::new(std::sync::Mutex::new(make));
        let handles: Vec<(usize, std::thread::JoinHandle<Result<(), String>>)> = options_list
            .into_iter()
            .enumerate()
            .map(|(i, options)| {
                let make = std::sync::Arc::clone(&make);
                // 带上窗口序号，供 join 时拼错误信息
                let handle = std::thread::spawn(move || -> Result<(), String> {
                    let mut window = Window::create(&options)?;
                    let (mut host, mut tree) = {
                        let mut guard = make.lock().unwrap_or_else(|e| e.into_inner());
                        guard(i)
                    };
                    host.attach_waker(std::sync::Arc::new(WindowWaker(window.wake_handle())));
                    window.resize(&mut tree);
                    y901_backend_common::runtime::run_loop(&mut window, &mut tree, host.as_mut())?;
                    let _ = tree.dispatch(&y901_ui::UiEvent::Tick);
                    Ok(())
                });
                (i, handle)
            })
            .collect();

        // 错误带窗口序号：多窗口下"哪个窗口失败"是必要信息。
        let mut errors = Vec::new();
        for (i, h) in handles {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => errors.push(format!("窗口 {i}: {e}")),
                Err(_) => errors.push(format!("窗口 {i}: 建窗线程 panic")),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// 把 `WakeHandle` 适配成 `y901_runtime::Waker`（供 Host 打断 `wait_message`）。
///
/// 提到模块级：`run` 与 `run_many` 都要用（后者在线程内构造它）。
struct WindowWaker(crate::window::WakeHandle);

impl y901_runtime::Waker for WindowWaker {
    fn wake(&self) {
        self.0.wake();
    }
}
