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

        // 把跨线程唤醒器交给 Host：后台线程（PTY/RPC）拿到数据时 wake()，
        // 主循环的阻塞等待会被立刻打断（否则要等下一次鼠标/键盘输入）。
        struct WindowWaker(crate::window::WakeHandle);
        impl y901_runtime::Waker for WindowWaker {
            fn wake(&self) {
                self.0.wake();
            }
        }
        host.attach_waker(std::sync::Arc::new(WindowWaker(window.wake_handle())));

        let mut tree = build();

        // 首帧布局：后端与文本度量都就绪后才能算
        window.resize(&mut tree);

        // 帧耗时打点（`Y901_PROFILE=1` 时每 2 秒打印一次）。
        // 只在显式开启时有一点开销，默认零成本。
        let profile = std::env::var_os("Y901_PROFILE").is_some();
        let mut stat_frames: u32 = 0;
        let mut stat_rendered: u32 = 0;
        let mut stat_full: u32 = 0;
        let mut stat_partial: u32 = 0;
        let mut stat_total_us: u128 = 0;
        let mut stat_max_us: u128 = 0;
        let mut stat_since = std::time::Instant::now();

        loop {
            let frame_start = std::time::Instant::now();
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

            // Host 请求真实 backbuffer 快照：先强制一帧全量重绘，再在 present
            // 之前 request；render 结束后回传给 Host（BGRA）。
            let snapshot_requested = host.take_snapshot_request();
            if snapshot_requested {
                tree.invalidate_all();
                window.request_snapshot();
            }

            let dirty_full = tree.dirty().is_full();
            let dirty_any = !tree.dirty().is_empty();
            window.render(&mut tree);
            if snapshot_requested {
                if let Some((pixels, w, h)) = window.take_snapshot() {
                    host.on_snapshot(&pixels, w, h);
                }
            }

            if profile {
                let us = frame_start.elapsed().as_micros();
                stat_frames += 1;
                stat_total_us += us;
                if us > stat_max_us {
                    stat_max_us = us;
                }
                if dirty_any {
                    stat_rendered += 1;
                    if dirty_full {
                        stat_full += 1;
                    } else {
                        stat_partial += 1;
                    }
                }
                if stat_since.elapsed().as_millis() >= 2000 {
                    let avg = if stat_frames > 0 {
                        stat_total_us / stat_frames as u128
                    } else {
                        0
                    };
                    eprintln!(
                        "[profile] frames={} rendered={} full={} partial={} avg={}us max={}us",
                        stat_frames, stat_rendered, stat_full, stat_partial, avg, stat_max_us
                    );
                    stat_frames = 0;
                    stat_rendered = 0;
                    stat_full = 0;
                    stat_partial = 0;
                    stat_total_us = 0;
                    stat_max_us = 0;
                    stat_since = std::time::Instant::now();
                }
            }

            // 空闲休眠：**没有事件、也没有画任何东西**时阻塞等消息。
            //
            // 之前主循环是无条件 `PeekMessage` 死循环：既没有消息也不渲染时仍然
            // 全速空转，吃满一个核 → 输入延迟、拖拽卡顿。
            // 不能无限等：PTY 输出走 channel 而不是 Windows 消息，必须周期性醒来。
            if !had_work && tree.dirty().is_empty() {
                // 默认 100ms 兜底：即使 Host 忘了接 `attach_waker`，
                // PTY 输出最多延迟 100ms 也会被 Tick 拉上来（不会卡死）。
                // `Y901_IDLE_WAIT_MS=0` → 真正无限阻塞（CPU 恒 0，但必须有 waker）。
                let wait_ms = std::env::var("Y901_IDLE_WAIT_MS")
                    .ok()
                    .and_then(|v| v.parse::<u32>().ok())
                    .unwrap_or(100);
                let ms = if wait_ms == 0 { u32::MAX } else { wait_ms };
                let _ = window.wait_message(ms);
            }
        }

        // 退出前把最后的 Tick 跑完，避免 PTY 数据丢失
        let _ = tree.dispatch(&UiEvent::Tick);
        Ok(())
    }
}
