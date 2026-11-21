//! X11 运行时：把「平台事件 → 框架 → 业务」串起来的地方（Linux）。
//!
//! 与 `Win32Runtime` 同构：建窗 → 拉事件 → 派发 → 重排 → 重绘 → 上屏。
//! 主循环已抽到 `y901-runtime::runtime::run_loop`；本文件只负责
//! **窗口创建 + 字形引擎/光栅器初始化 + 窗口句柄接进循环**。
//!
//! ## DPI 与运行期缩放
//!
//! 启动时按 `XSettings → Xft.dpi → 单一 XRandR 输出物理 DPI → 屏幕毫米` 的优先级选缩放
//! （见 [`crate::window::dpi_scale_with_xsettings_and_outputs`]），并用**物理**尺寸/字号建软后端；
//! 同时把"按新缩放重建软后端"的配方交给窗口（[`crate::window::X11Window::set_rescale_factory`]）。
//! 桌面改缩放后窗口会在下一次 `wait_events` 里自己发现并重建 —— 不需要重启应用。
//! 多显示器时全局缩放不猜屏：按窗口位置查每显示器 DPI 用 [`crate::randr::dpi_at`]。
//!
//! ## 为什么没有 GPU 路径
//!
//! X11 下我们没有 D3D11（也不打算手写 Vulkan/GL）。
//! 纯 CPU 光栅化器（`y901-render::SoftBackend`）在小窗口下完全够用，
//! 且与 Windows 的 `--force-soft` 路径是**同一份代码**——渲染结果天然一致。
//!
//! 光栅器现在住在 [`X11Window`] 里（`soft` 字段），`WindowBackend::relayout/render`
//! 直接驱动它；本文件不再持有第二份软后端。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_runtime::runtime::run_loop;
use y901_runtime::{EventSource, Host, Options, PlatformRuntime};
use y901_ui::Size;
use y901_ui::UiTree;

use crate::window::X11Window;

/// X11 平台入口。
pub struct X11Runtime;

/// 按给定缩放与物理尺寸造一个软后端（启动与"运行期缩放"共用同一条路径）。
///
/// 为什么抽成函数：运行期缩放要**重建**整套东西（字体引擎 + 缓冲 + DPI 注入），
/// 若与启动路径各写一份，迟早会出现"启动时字号乘了 scale、缩放后没乘"这类偏差。
fn build_soft(
    width: u32,
    height: u32,
    dpi_scale: f32,
    font: &y901_runtime::FontConfig,
    theme: Option<y901_ui::theme::Theme>,
) -> Result<SoftBackend, String> {
    let physical_font = (font.size * dpi_scale).max(1.0);
    let (cw, ch) = y901_render::cell_metrics_for(physical_font);
    let cell = Size::new(cw, ch);
    let engine: Box<dyn y901_render::FontEngine> = Box::new(MyFontEngine::new(
        y901_render::load_font(&y901_render::resolve_font_path(&font.family))?,
        physical_font,
        cell,
    )?);
    let mut soft = SoftBackend::new(width.max(1), height.max(1), engine, cell, theme)?;
    // 把缩放注入软后端：`TextMetrics::scale()` 与 per-text 字号都靠它。
    soft.set_dpi_scale(dpi_scale);
    Ok(soft)
}

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

        // **DPI 缩放**：先连一次服务器，读 **XSettings**（桌面设置服务）→ 根窗口
        // `Xft.dpi`（Xresources）→ XRandR 活动输出物理 DPI → 屏幕毫米尺寸，按优先级推 `scale`。
        // 为什么先探再建窗：软后端要用**物理**尺寸分配缓冲、字形引擎要用**物理**字号；
        // 这两者都得在 `X11Window::create` 之前就定下来（它只接收已建好的 soft）。
        let dpi_scale = match crate::conn::Connection::connect(&display) {
            Ok(mut conn) => {
                let info = conn.info().clone();
                let xsettings = crate::window::read_xsettings(&mut conn, 0);
                let xft_dpi = crate::window::read_xft_dpi(&mut conn, info.root);
                // 只是**查询**：没有 XRandR / 老版本 / 无活动输出都返回空，不影响旧路径。
                let outputs = conn.active_output_dpis().unwrap_or_default();
                if let Some(x) = xsettings {
                    eprintln!("[y901] XSettings：{x:?}");
                }
                if !outputs.is_empty() {
                    eprintln!(
                        "[y901] XRandR 活动输出 {} 个（物理 DPI 仅作单屏回退；多屏按位置查询见 randr::dpi_at）",
                        outputs.len()
                    );
                }
                crate::window::dpi_scale_with_xsettings_and_outputs(
                    &info,
                    xsettings,
                    xft_dpi,
                    Some(&outputs),
                )
            }
            Err(e) => {
                eprintln!("[y901] 读取 X11 屏幕信息失败（{e}），DPI 按 1.0 处理");
                1.0
            }
        };
        let width = ((options.width as f32 * dpi_scale).round() as u32).clamp(1, u16::MAX as u32);
        let height = ((options.height as f32 * dpi_scale).round() as u32).clamp(1, u16::MAX as u32);

        // 字形引擎：Linux 上只有自研引擎（没有 DirectWrite）。
        // 字号同样要乘 `dpi_scale`：引擎按**物理像素**光栅化，逻辑字号由 UI 层消费。
        let font = options.font.clone().unwrap_or_default();
        let soft = build_soft(width, height, dpi_scale, &font, options.theme)?;

        let mut window = X11Window::create(crate::window::CreateArgs {
            display: &display,
            width: width as u16,
            height: height as u16,
            title: &options.title,
            soft,
            fullscreen: options.fullscreen,
        })?;

        // **运行期跟随桌面缩放**：把"按新缩放建一份软后端"的配方交给窗口。
        // 不注入时窗口根本不查设置（默认零开销）；注入后每秒最多查一次。
        {
            let logical = (options.width as f32, options.height as f32);
            let font = font.clone();
            let theme = options.theme;
            window.set_rescale_factory(
                dpi_scale,
                Box::new(move |scale: f32| {
                    let w = ((logical.0 * scale).round() as u32).clamp(1, u16::MAX as u32);
                    let h = ((logical.1 * scale).round() as u32).clamp(1, u16::MAX as u32);
                    build_soft(w, h, scale, &font, theme)
                }),
            );
        }
        eprintln!(
            "[y901] X11 后端就绪：display={display} 逻辑 {}x{}（物理 {width}x{height}，scale={dpi_scale:.2}），字形引擎=自研 TTF",
            options.width, options.height
        );

        // **IBus 输入法（M1）**：连接失败就降级为 `NoIme`——
        // 按键一律放行，与“没有 IME”完全一致（不崩、不吞键）。
        match crate::ime::ibus::IbusIme::connect("libpinyin") {
            Ok(ibus_ime) => {
                eprintln!(
                    "[y901] IBus 输入法已连接：context={} engine=libpinyin",
                    ibus_ime.context_path()
                );
                window.set_ime(Box::new(ibus_ime));
            }
            Err(e) => {
                eprintln!("[y901] IBus 不可用（{e}），IME 已关闭：英文输入行为不变");
            }
        }

        host.attach_waker(window.waker());

        // 建树：按光栅器的真实尺寸排版（字体 cell 决定行列数）
        let mut tree = build();
        let (sw, sh) = (window.soft().width(), window.soft().height());
        tree.layout(Size::new(sw as f32, sh as f32), window.soft_mut());

        let code = run_loop(&mut window, &mut tree, host.as_mut());

        if let Ok(pixels) = code.as_ref().map(|_| window.soft().pixels()) {
            // 首帧统计：只在退出时打一次，避免每帧算一遍（run_loop 已打过耗时）
            let non_bg = pixels
                .chunks_exact(4)
                .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 24)
                .count();
            let mut colors = std::collections::HashSet::new();
            for p in pixels.chunks_exact(4) {
                colors.insert((p[0], p[1], p[2]));
            }
            eprintln!(
                "[y901] X11 会话结束：总耗时 {:.1}ms，{}x{}，非背景像素 {} ({:.1}%)，独立颜色 {}种",
                started.elapsed().as_secs_f64() * 1e3,
                sw,
                sh,
                non_bg,
                100.0 * non_bg as f64 / (sw.max(1) * sh.max(1)) as f64,
                colors.len()
            );
        }

        code
    }

    /// 每窗口一线程。
    ///
    /// 为何并行可行：**每个 `X11Window` 自带一条 `Connection`**，
    /// 事件队列与全屏状态已按窗口隔离（多窗口第 1 步已验证）。
    ///
    /// `Window` / `Host` / `UiTree` **都在线程内创建** —— 它们都不是 `Send`。
    /// `DISPLAY` 是进程级环境变量（只读），各线程各自读一次即可。
    fn run_many(
        &mut self,
        options_list: Vec<Options>,
        make: y901_runtime::WindowFactory,
    ) -> Result<(), String> {
        let make = std::sync::Arc::new(std::sync::Mutex::new(make));
        let handles: Vec<(usize, std::thread::JoinHandle<Result<(), String>>)> = options_list
            .into_iter()
            .enumerate()
            .map(|(i, options)| {
                let make = std::sync::Arc::clone(&make);
                // 带上窗口序号，供 join 时拼错误信息
                let handle = std::thread::spawn(move || -> Result<(), String> {
                    let (host, tree) = {
                        let mut guard = make.lock().unwrap_or_else(|e| e.into_inner());
                        guard(i)
                    };
                    // `X11Runtime` 是零字节结构体：每线程自建一个即可。
                    let mut rt = X11Runtime;
                    rt.run(host, Box::new(move || tree), options)
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
