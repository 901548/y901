//! X11 运行时：把「平台事件 → 框架 → 业务」串起来的地方（Linux）。
//!
//! 与 `Win32Runtime` 同构：建窗 → 拉事件 → 派发 → 重排 → 重绘 → 上屏。
//! 区别只在"窗口/事件/上屏"三件事：本文件用 `X11Window` + `SoftBackend` + `PutImage`。
//!
//! ## 为什么没有 GPU 路径
//!
//! X11 下我们没有 D3D11（也不打算手写 Vulkan/GL）。
//! 纯 CPU 光栅化器（`y901-render::SoftBackend`）在小窗口下完全够用，
//! 且与 Windows 的 `--force-soft` 路径是**同一份代码**——渲染结果天然一致。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_runtime::{translate, EventSource, Host, Options, PlatformRuntime, Presenter, RawEvent};
use y901_ui::{Size, UiEvent, UiTree};

use crate::window::X11Window;

/// X11 平台入口。
pub struct X11Runtime;

impl PlatformRuntime for X11Runtime {
    fn run(
        &mut self,
        mut host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
        options: Options,
    ) -> Result<(), String> {
        let display = std::env::var("DISPLAY").unwrap_or_default();
        if display.is_empty() {
            return Err("DISPLAY 未设置：请在 X 会话或 Xvfb 下运行（如 xvfb-run -a）".to_string());
        }

        let started = std::time::Instant::now();
        let mut window = X11Window::create(
            &display,
            options.width.clamp(1, u16::MAX as u32) as u16,
            options.height.clamp(1, u16::MAX as u32) as u16,
            &options.title,
        )?;

        // 字形引擎：Linux 上只有自研引擎（没有 DirectWrite）
        let font = options.font.clone().unwrap_or_default();
        let (cw, ch) = y901_render::cell_metrics_for(font.size);
        let cell = Size::new(cw, ch);
        let engine: Box<dyn y901_render::FontEngine> = Box::new(
            MyFontEngine::new(
                y901_render::load_font(&y901_render::resolve_font_path(&font.family))?,
                font.size,
                cell,
            )?,
        );
        let mut soft = SoftBackend::new(
            options.width.max(1),
            options.height.max(1),
            engine,
            cell,
            options.theme,
        )?;
        eprintln!(
            "[y901] X11 后端就绪：display={display} {}x{}，字形引擎=自研 TTF",
            options.width, options.height
        );

        host.attach_waker(window.waker());

        let mut tree = build();
        tree.layout(Size::new(soft.width() as f32, soft.height() as f32), &mut soft);

        let profile = std::env::var_os("Y901_PROFILE").is_some();
        let mut first_frame_done = false;
        let mut frames = 0u64;
        let mut stat_frames: u32 = 0;
        let mut stat_total_us: u128 = 0;
        let mut stat_max_us: u128 = 0;
        let mut stat_since = std::time::Instant::now();

        loop {
            let frame_start = std::time::Instant::now();
            // 100ms 兵底：即使没事件也会周期性醒来跑 Tick（PTY/AI 流式输出靠它）
            let wait_ms = std::env::var("Y901_IDLE_WAIT_MS")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(100);
            let raws = window.poll(wait_ms);

            let mut need_layout = false;
            let mut had_work = false;
            let viewport = Size::new(soft.width() as f32, soft.height() as f32);

            for raw in raws {
                match &raw {
                    RawEvent::Close => return Ok(()),
                    RawEvent::Resize { .. } => need_layout = true,
                    _ => {}
                }
                had_work = true;
                if let Some(ev) = translate(&raw, viewport) {
                    if host.event(&mut tree, &ev).is_handled() {
                        continue;
                    }
                    if tree.dispatch(&ev).needs_relayout() {
                        need_layout = true;
                    }
                }
            }

            for cmd in tree.drain_commands() {
                let _ = host.command(&mut tree, cmd);
            }
            host.idle(&mut tree);

            let external = tree.drain_external_invalidations();
            let structural = tree.take_layout_request();
            if need_layout || external > 0 || structural {
                let (w, h) = window.size();
                soft.resize(w as u32, h as u32);
                tree.layout(Size::new(w as f32, h as f32), &mut soft);
                tree.invalidate_all();
            }

            if let Some(theme) = host.take_theme_request() {
                if let Err(err) = soft.set_theme(theme) {
                    eprintln!("[y901] 切换主题失败: {err}");
                } else {
                    tree.invalidate_all();
                }
            }

            // 纯 CPU 路径：每帧清屏 + 整树重绘（与 Windows 的 --force-soft 一致）
            if !tree.dirty().is_empty() {
                tree.invalidate_all();
                soft.clear();
                tree.render(&mut soft);
                window.present(soft.pixels(), soft.width(), soft.height())?;
                frames += 1;
                if !first_frame_done {
                    first_frame_done = true;
                    let px = soft.pixels();
                    let non_bg = px
                        .chunks_exact(4)
                        .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 24)
                        .count();
                    let mut colors = std::collections::HashSet::new();
                    for p in px.chunks_exact(4) {
                        colors.insert((p[0], p[1], p[2]));
                    }
                    eprintln!(
                        "[y901] first frame ready in {:.1}ms：{}x{}，非背景像素 {} ({:.1}%)，独立颜色 {}种",
                        started.elapsed().as_secs_f64() * 1e3,
                        soft.width(),
                        soft.height(),
                        non_bg,
                        100.0 * non_bg as f64 / (soft.width() * soft.height()) as f64,
                        colors.len()
                    );
                }
            }

            if profile {
                let us = frame_start.elapsed().as_micros();
                stat_frames += 1;
                stat_total_us += us;
                stat_max_us = stat_max_us.max(us);
                if stat_since.elapsed().as_millis() >= 2000 {
                    let avg = stat_total_us / stat_frames.max(1) as u128;
                    eprintln!(
                        "[profile] frames={stat_frames} total_presented={frames} avg={avg}us max={stat_max_us}us"
                    );
                    stat_frames = 0;
                    stat_total_us = 0;
                    stat_max_us = 0;
                    stat_since = std::time::Instant::now();
                }
            }

            if !had_work && tree.dirty().is_empty() {
                // 没事可做 → 发一个 Tick（让 PTY/流式输出能推进，又不空转占 CPU）
                let _ = tree.dispatch(&UiEvent::Tick);
            }
        }
    }
}
