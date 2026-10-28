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
            // 本帧是否处理过事件（决定空闲时是否需要休眠，见循环末尾）
            let mut had_work = false;
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

                had_work = true;
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

            // 业务命令：Host 说没处理就丢弃（框架不猜语义）
            for cmd in tree.drain_commands() {
                let _ = host.command(&mut tree, cmd);
            }

            host.idle(&mut tree);

            // 后台线程的标脏请求：**必须在 layout 之前** drain（本帧只 drain 一次，
            // 避免同一帧多次 layout）。有请求时走完整 resize（内含 layout）——
            // `window.resize` 的代价只有一次尺寸比较，交换链在尺寸不变时会短路。
            let external = tree.drain_external_invalidations();
            // 结构性变化（add_node / insert_node / remove_node）必须重排：
            // Host 在 command() 里改树时，这一帧还没有任何 Relayout 事件。
            let structural = tree.take_layout_request();
            if need_layout || external > 0 || structural {
                window.resize(&mut tree);
            }

            // IME 候选窗跟随光标
            if let Some((x, y)) = host.ime_position(&mut tree) {
                window.set_ime_position(x, y);
            }

            // 运行时主题/字体切换：Host 表达意图，平台层执行。
            // 两者都只改渲染状态，**重绘/重排由这里负责**（后端不认识树）。
            if let Some(theme) = host.take_theme_request() {
                if let Err(err) = window.set_theme(theme) {
                    eprintln!("[y901] 切换主题失败: {err}");
                } else {
                    tree.invalidate_all();
                }
            }
            if let Some(font) = host.take_font_request() {
                match window.set_font(&font) {
                    Ok(()) => {
                        // cell 尺寸变了：先重排（用后端的新度量），再全量重绘
                        window.resize(&mut tree);
                        tree.invalidate_all();
                    }
                    Err(err) => eprintln!("[y901] 切换字体失败（保持旧字体）: {err}"),
                }
            }

            // 系统要求重画（遮挡/最小化恢复）：D3D 交换链内容不会被 Windows 恢复，
            // 而"有脏区才画"意味着此时通常没有脏区 → 必须显式请求一次整窗重绘。
            if window.take_repaint_request() {
                tree.invalidate_all();
            }

            if let Some(pos) = mouse {
                window.set_cursor(tree.cursor_at(pos));
            }

            window.render(&mut tree);

            // 空闲休眠：**没有事件、也没有画任何东西**时阻塞等消息。
            //
            // 之前主循环是无条件 `PeekMessage` 死循环：既没有消息也不渲染时仍然
            // 全速空转，吃满一个核 → 输入延迟、拖拽卡顿。
            // 不能无限等：PTY 输出走 channel 而不是 Windows 消息，必须周期性醒来。
            if !had_work && tree.dirty().is_empty() {
                let _ = window.wait_message(8);
            }
        }

        // 退出前把最后的 Tick 跑完，避免 PTY 数据丢失
        let _ = tree.dispatch(&UiEvent::Tick);
        Ok(())
    }
}
