//! **P1a 硬验收**：Popup 开启动画在真实 D3D11 路径上真的在动。
//!
//! 为什么需要它：`popup_anim.rs`（y901-render）只验证了 `SoftBackend` 的像素；
//! `render_ctx_parity.rs` 只能做结构断言（方法存在 + 语义同源）。
//! 两者都证明不了"D3D 屏幕上弹层从缩小状态长到终态"。
//!
//! 做法：起一个真实 D3D 窗口 → `open_popup`（自动启动 150ms 动画）→
//! 推进到半程拍一张、推进到结束再拍一张 → 比较白色弹层面板的墨迹包围盒。
//!
//! 需要真实 GPU（会建窗口），默认 `#[ignore]`：
//! ```text
//! cargo test -p y901-backend-win32 --release --test popup_anim_d3d -- --ignored --nocapture
//! ```

#![cfg(windows)]

use std::time::Duration;

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    UiEvent, UiTree, Widget,
};

const W: u32 = 240;
const H: u32 = 180;

/// 根节点：只占位（弹层画在它上面）。
struct Root {
    rect: Rect,
}

impl Widget for Root {
    fn id(&self) -> &str {
        "root"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(W as f32, H as f32))
    }
    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 弹层内容：整块涂白，便于用"亮像素包围盒"量尺寸。
struct WhitePanel {
    rect: Rect,
}

impl Widget for WhitePanel {
    fn id(&self) -> &str {
        "white-panel"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(120.0, 80.0))
    }
    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::WHITE);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn options() -> Options {
    Options {
        title: "y901 popup anim d3d".to_string(),
        width: W,
        height: H,
        theme: None,
        font: None,
        fullscreen: false,
    }
}

/// 亮像素（弹层面板）的包围盒；`None` = 画面上没有面板。
///
/// 阈值 200 的理由与 `popup_anim.rs` 相同：半程时淡入遮罩把白压到 ~224，
/// 但仍高于 200；而用"非背景"判定会把遮罩本身（盖住终态矩形）算进去。
fn bright_bbox(px: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            // 快照是 BGRA；三通道都亮即可，顺序不影响判定
            if px[i] < 200 || px[i + 1] < 200 || px[i + 2] < 200 {
                continue;
            }
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    bbox
}

fn shot(window: &mut Window, tree: &mut UiTree) -> (Vec<u8>, u32, u32) {
    tree.invalidate_all();
    window.request_snapshot();
    window.render(tree);
    window
        .take_snapshot()
        .expect("快照失败（staging/Map）：需要真实 D3D11")
}

#[test]
#[ignore = "需要真实 GPU：D3D11 设备 + 窗口"]
fn popup_animation_progresses_on_d3d() {
    let mut window = Window::create(&options()).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(Root { rect: Rect::ZERO }));
    window.resize(&mut tree);

    // 打开弹层：push_popup 会启动 150ms 动画（此刻 progress = 0）
    assert!(
        tree.open_popup(
            Box::new(WhitePanel { rect: Rect::ZERO }),
            Rect::new(100.0, 80.0, 10.0, 10.0)
        ),
        "弹层应打开"
    );
    window.resize(&mut tree);

    // ---- 半程帧（progress ≈ 0.5）----
    assert!(
        tree.tick_animations(Duration::from_millis(75)),
        "75ms 时动画应仍在推进"
    );
    let (px_mid, w1, h1) = shot(&mut window, &mut tree);
    let mid = bright_bbox(&px_mid, w1, h1).expect("半程应有弹层面板像素");

    // ---- 终态帧（动画结束）----
    assert!(
        !tree.tick_animations(Duration::from_millis(200)),
        "275ms 后动画应已结束（总时长 150ms）"
    );
    let (px_end, w2, h2) = shot(&mut window, &mut tree);
    let end = bright_bbox(&px_end, w2, h2).expect("终态应有弹层面板像素");

    assert_eq!((w1, h1), (w2, h2), "两次快照尺寸应一致");

    let size = |b: (u32, u32, u32, u32)| (b.2 - b.0 + 1, b.3 - b.1 + 1);
    let (mw, mh) = size(mid);
    let (ew, eh) = size(end);

    // 逐像素差异（证明画面真的变了，而不只是包围盒数字变了）
    let diff = px_mid
        .chunks_exact(4)
        .zip(px_end.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();

    eprintln!(
        "[popup-d3d] 半程 {mw}x{mh} @{mid:?}，终态 {ew}x{eh} @{end:?}，差异像素 {diff}"
    );

    assert!(
        mw < ew && mh < eh,
        "D3D 上动画中间帧应比终态小：半程 {mw}x{mh} vs 终态 {ew}x{eh}"
    );
    assert!(diff > 0, "两帧应有像素差异（动画没动？）");

    // 缩放下限 0.6、终态 1.0；75ms/150ms 的 EaseOut 下应明显小于终态
    let ratio = mw as f32 / ew as f32;
    assert!(
        (0.70..0.99).contains(&ratio),
        "半程宽度应明显小于终态（0.70~0.99），实际 {ratio:.3}"
    );
}
