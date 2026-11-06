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

        // 把窗口的唤醒句柄交给 Host：后台线程（PTY 等）可以借此打断 `wait_message`。
        struct WindowWaker(crate::window::WakeHandle);
        impl y901_runtime::Waker for WindowWaker {
            fn wake(&self) {
                self.0.wake();
            }
        }
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
}
