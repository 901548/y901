//! **P1b 硬验收**：Tabs 切换过渡遮罩在真实 D3D11 路径上用**面板色**而不是默认黑。
//!
//! 为什么需要它：`tabs.rs::switch_transition_is_visible_in_pixels` 用的是 `SoftBackend`；
//! `render_ctx_parity.rs` 只做结构断言（两后端 `background_color` 同源）。
//! 两者都证明不了"D3D 屏幕上的过渡遮罩真的是面板色"。
//!
//! 做法：起真实 D3D 窗口 → 切 Tab（过渡 progress≈0，遮罩 alpha≈1）→ 截图 →
//! 取内容区中心像素，断言它 ≈ `Theme::DARK` 的 `Role::PANEL / Normal` 背景色、
//! 且**不是纯黑**；再把过渡跑完，断言内容真的显现（变白）。
//!
//! ```text
//! cargo test -p y901-backend-win32 --release --test tabs_transition_d3d -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::cell::Cell;
use std::rc::Rc;

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::theme::{State, Theme};
use y901_ui::{Color, MouseButton, Pos, Role, UiEvent, UiTree};
use y901_widgets::tabs::Tabs;

const W: u32 = 240;
const H: u32 = 160;
/// 与 `tabs.rs::BAR_H` 一致：内容区从 y=32 开始。
const BAR_H: u32 = 32;

fn options() -> Options {
    Options {
        title: "y901 tabs transition d3d".to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,
        fullscreen: false,
    }
}

/// Tabs + 句柄（选中索引）。
fn build() -> (Tabs, Rc<Cell<usize>>) {
    let tabs = Tabs::new(
        "tabs",
        vec!["one".into(), "two".into(), "three".into()],
        Box::new(|_index, ctx, rect, progress| {
            // 内容涂白，alpha 随 progress 上升（模拟淡入）
            let a = progress.clamp(0.0, 1.0);
            ctx.fill_rect(rect, Color::rgba(1.0, 1.0, 1.0, a));
        }),
    )
    // 内容区显式给 200px：不给的话 measure 只报最小高度 BAR_H+1，
    // 采样点就只能落在那 1px 上。
    .content_height(200.0);
    let handle = tabs.selection();
    (tabs, handle)
}

fn shot(window: &mut Window, tree: &mut UiTree) -> (Vec<u8>, u32, u32) {
    tree.invalidate_all();
    window.request_snapshot();
    window.render(tree);
    window
        .take_snapshot()
        .expect("快照失败（staging/Map）：需要真实 D3D11")
}

/// 取内容区**第一行**中点的像素（BGRA 前三个通道）。
///
/// 采样内容区中点。测试里 Tabs 用 `.content_height(200.0)` 撑开，
/// 所以内容区实际高度是 200px，中点是 `y = BAR_H + 100`。
fn content_center(px: &[u8], w: u32) -> [u8; 3] {
    let x = w / 2;
    // 内容区现在有真实高度（200px），采内容区中点。
    let y = BAR_H + 100;
    let i = ((y * w + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn tabs_transition_uses_panel_color_on_d3d() {
    let (tabs, handle) = build();
    let mut tree = UiTree::new(Box::new(tabs));
    let mut window = Window::create(&options()).expect("需要可用的 D3D11 设备");
    window.resize(&mut tree);

    // 切到第二个 Tab：点击标签栏（等分布局下第二格中心在 W/3*1.5）
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(W as f32 / 3.0 * 1.5, 16.0),
        button: MouseButton::Left,
    });
    assert_eq!(handle.get(), 1, "应切到第 1 页");

    // 过渡起点（progress≈0，遮罩 alpha≈1）：内容区应被**面板色**盖住
    let (px_dim, w, h) = shot(&mut window, &mut tree);
    let dim = content_center(&px_dim, w);

    // 参照色：默认暗色主题的 PANEL 背景（D3D 在 theme=None 时用的就是 THEME::DARK）
    let panel = Theme::DARK.style(Role::PANEL, State::Normal).bg;
    let expected = [
        (panel.b * 255.0).round() as i32,
        (panel.g * 255.0).round() as i32,
        (panel.r * 255.0).round() as i32,
    ];
    let got = [dim[0] as i32, dim[1] as i32, dim[2] as i32];
    let max_d = (0..3)
        .map(|i| (got[i] - expected[i]).abs())
        .max()
        .unwrap_or(0);
    eprintln!(
        "[tabs-d3d] 过渡起点内容区像素 BGRA={got:?}，期望≈PANEL bg={expected:?}（max diff {max_d}）"
    );

    // P1b 核心断言 1：不是纯黑（修复前 background_color 回落成黑，三通道≈0）
    assert!(
        got.iter().sum::<i32>() > 20,
        "过渡遮罩不应是纯黑（P1b 未生效）：{got:?}"
    );
    // P1b 核心断言 2：≈ 面板色
    assert!(
        max_d <= 16,
        "过渡遮罩应为面板色 {expected:?}，实际 {got:?}（max diff {max_d}）"
    );

    // 跑完过渡：Tabs 自己在 `UiEvent::Tick` 里按**墙钟**推进动画（与 tabs.rs 软测一致），
    // 所以这里必须 sleep，不能用 `tree.tick_animations(dt)`（那只管树级动画）。
    for _ in 0..10 {
        tree.dispatch(&UiEvent::Tick);
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    let (px_full, w2, h2) = shot(&mut window, &mut tree);
    assert_eq!((w, h), (w2, h2), "两次快照尺寸应一致");
    let full = content_center(&px_full, w2);
    eprintln!("[tabs-d3d] 过渡终态内容区像素 BGRA={full:?}");
    assert!(
        full.iter().all(|c| *c >= 200),
        "过渡结束后内容应显现为白：{full:?}"
    );
}
