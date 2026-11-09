//! X11 运行时：把「平台事件 → 框架 → 业务」串起来的地方（Linux）。
//!
//! 与 `Win32Runtime` 同构：建窗 → 拉事件 → 派发 → 重排 → 重绘 → 上屏。
//! 主循环已抽到 `y901-backend-common::runtime::run_loop`；本文件只负责
//! **窗口创建 + 字形引擎/光栅器初始化 + 窗口句柄接进循环**。
//!
//! ## 为什么没有 GPU 路径
//!
//! X11 下我们没有 D3D11（也不打算手写 Vulkan/GL）。
//! 纯 CPU 光栅化器（`y901-render::SoftBackend`）在小窗口下完全够用，
//! 且与 Windows 的 `--force-soft` 路径是**同一份代码**——渲染结果天然一致。
//!
//! 光栅器现在住在 [`X11Window`] 里（`soft` 字段），`WindowBackend::relayout/render`
//! 直接驱动它；本文件不再持有第二份软后端。

use y901_backend_common::runtime::run_loop;
use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_runtime::{EventSource, Host, Options, PlatformRuntime};
use y901_ui::Size;
use y901_ui::UiTree;

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
        let width = options.width.clamp(1, u16::MAX as u32);
        let height = options.height.clamp(1, u16::MAX as u32);

        // 字形引擎：Linux 上只有自研引擎（没有 DirectWrite）
        let font = options.font.clone().unwrap_or_default();
        let (cw, ch) = y901_render::cell_metrics_for(font.size);
        let cell = Size::new(cw, ch);
        let engine: Box<dyn y901_render::FontEngine> = Box::new(MyFontEngine::new(
            y901_render::load_font(&y901_render::resolve_font_path(&font.family))?,
            font.size,
            cell,
        )?);
        let soft = SoftBackend::new(width.max(1), height.max(1), engine, cell, options.theme)?;

        let mut window = X11Window::create(crate::window::CreateArgs {
            display: &display,
            width: width as u16,
            height: height as u16,
            title: &options.title,
            soft,
            fullscreen: options.fullscreen,
        })?;
        eprintln!(
            "[y901] X11 后端就绪：display={display} {}x{}，字形引擎=自研 TTF",
            options.width, options.height
        );

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
}
