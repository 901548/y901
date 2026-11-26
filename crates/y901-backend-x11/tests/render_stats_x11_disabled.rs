#![cfg(target_os = "linux")]

//! **统计关闭时的零成本契约**（RenderStats A 阶段第 4 步的反面）。
//!
//! `render_stats::enabled()` 是进程级 `OnceLock`，一个进程只有一种模式，所以
//! "关闭"必须单独一个测试二进制（与 `render_stats_x11.rs` 分开）。
//!
//! 钉住两件事：
//! 1. 关闭时 `take_render_stats()` 恒为 `None` —— 不是"有值但全 0"，
//!    也不是"上一帧的残留"；
//! 2. 关闭时渲染行为**完全不变**：同一棵树渲染两次结果都是 `Presented`
//!    （统计分支不得改变控制流）。

use std::time::Duration;

use y901_backend_x11::window::{CreateArgs, X11Window};
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree,
    Widget,
};

const W: u16 = 320;
const H: u16 = 180;

static SERVER_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn skip(reason: &str) {
    use std::io::Write as _;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/skip-count.txt");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "0\trender_stats_x11_disabled\t{reason}");
    }
    eprintln!("[stats-x11-disabled] 跳过：{reason}");
}

fn soft_for(w: u16, h: u16) -> y901_render::soft::SoftBackend {
    let size = 15.0f32;
    let (cw, ch) = y901_render::cell_metrics_for(size);
    let cell = Size::new(cw, ch);
    let engine = y901_render::MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("测试字体可读"),
        size,
        cell,
    )
    .expect("字形引擎");
    y901_render::soft::SoftBackend::new(w as u32, h as u32, Box::new(engine), cell, None)
        .expect("软光栅器")
}

fn create_window(d: &str) -> Option<X11Window> {
    for _ in 0..3 {
        match X11Window::create(CreateArgs {
            display: d,
            width: W,
            height: H,
            title: "y901-stats-x11-off",
            soft: soft_for(W, H),
            fullscreen: false,
        }) {
            Ok(x) => return Some(x),
            Err(e) => {
                skip(&format!("建窗失败：{e}"));
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    None
}

struct Square {
    id: &'static str,
    rect: Rect,
}

impl Widget for Square {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(W as f32, H as f32))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(
            Rect::new(0.0, 0.0, W as f32, H as f32),
            Color::from_rgb8(30, 30, 40),
        );
        ctx.draw_text(Pos::new(8.0, 24.0), "stats off", &TextStyle::default());
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn disabled_stats_stay_none_and_do_not_change_rendering() {
    // 显式设为 0（`enabled()` 的判据是"存在且不为 0"）。
    std::env::set_var("Y901_RENDER_STATS", "0");
    let _guard = SERVER_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let d = std::env::var("DISPLAY").unwrap_or_else(|_| ":99".to_string());
    let Some(mut win) = create_window(&d) else {
        return;
    };

    let mut tree = UiTree::new(Box::new(Square {
        id: "sq",
        rect: Rect::ZERO,
    }));
    tree.layout(Size::new(W as f32, H as f32), &soft_for(W, H));
    tree.invalidate_all();
    win.relayout(&mut tree);

    // 种类仍然自报（能力表与开关无关），但**不产生统计对象**。
    assert_eq!(
        win.render_stats_backend(),
        Some(y901_runtime::render_stats::BackendKind::X11),
        "种类自报与统计开关无关"
    );

    let outcome = win.render(&mut tree).expect("render 1");
    assert!(matches!(outcome, RenderOutcome::Presented));
    assert!(
        win.take_render_stats().is_none(),
        "统计关闭时不得交付统计（不是'有值但全 0'）"
    );

    // 第二帧（无脏区 → 跳过）同样不产生统计，且行为不变。
    let outcome = win.render(&mut tree).expect("render 2");
    assert!(matches!(outcome, RenderOutcome::Skipped));
    assert!(
        win.take_render_stats().is_none(),
        "跳过帧在关闭统计时也不得交付统计"
    );

    // 再来一帧完整渲染：证明"关闭"不会把状态机卡在跳过分支。
    tree.invalidate_all();
    let outcome = win.render(&mut tree).expect("render 3");
    assert!(matches!(outcome, RenderOutcome::Presented));
    assert!(win.take_render_stats().is_none());
}
