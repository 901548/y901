//! Win32 运行时：唯一把「平台事件 → 框架 → 业务」串起来的地方。

use y901_runtime::{translate, Host, Options, PlatformRuntime, RawEvent};
use y901_ui::{Size, UiEvent, UiTree};

use crate::window::Window;

/// Win32 平台入口。
pub struct Win32Runtime;

impl PlatformRuntime for Win32Runtime {
    fn run(
        mut host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
        options: Options,
    ) -> Result<(), String> {
        let mut window = Window::create(&options)?;
        let mut tree = build();

        // 首帧布局：后端与文本度量都就绪后才能算
        window.resize(&mut tree);

        loop {
            if !window.pump() {
                break;
            }

            let (w, h) = window.viewport();
            let viewport = Size::new(w as f32, h as f32);
            let mut need_layout = false;
            let mut mouse = None;

            for raw in window.drain_events() {
                match &raw {
                    RawEvent::Close => return Ok(()),
                    RawEvent::SetCursor { x, y } => {
                        mouse = Some(y901_ui::Pos::new(*x, *y));
                        continue;
                    }
                    RawEvent::Resize { .. } => {
                        need_layout = true;
                    }
                    _ => {}
                }

                if let Some(ev) = translate(&raw, viewport) {
                    // Host 先处理：返回 Handled 表示它接管了这条事件
                    if host.event(&mut tree, &ev).is_handled() {
                        continue;
                    }
                    if tree.dispatch(&ev).needs_relayout() {
                        need_layout = true;
                    }
                }
            }

            if need_layout {
                window.resize(&mut tree);
            }

            // 业务命令：Host 说没处理就丢弃（框架不猜语义）
            for cmd in tree.drain_commands() {
                let _ = host.command(&mut tree, cmd);
            }

            host.idle(&mut tree);

            // IME 候选窗跟随光标
            if let Some((x, y)) = host.ime_position(&mut tree) {
                window.set_ime_position(x, y);
            }

            if let Some(pos) = mouse {
                window.set_cursor(tree.cursor_at(pos));
            }

            window.render(&mut tree);
        }

        // 退出前把最后的 Tick 跑完，避免 PTY 数据丢失
        let _ = tree.dispatch(&UiEvent::Tick);
        Ok(())
    }
}
