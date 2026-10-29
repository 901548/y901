//! 确定性合成树 + 像素回读（渲染改动的客观验收探针）。
//!
//! 为什么需要：示例程序是活 shell，两次运行屏幕内容不同，无法做逐像素对比。
//! 这里用一棵**固定**的合成树（纯色矩形 + 中英文 + 多种前景色），
//! 渲染 → 回读 → 比较哈希/亮度，全程不依赖肉眼。
//!
//! 需要真实 GPU（会创建窗口），因此默认 `#[ignore]`：
//!
//! ```text
//! cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture
//! Y901_GAMMA_AB=1 cargo test ... （对比 gamma 校正前后的亮度）
//! ```

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiTree, Widget,
};

/// 固定场景：背景块 + 两行文字 + 一个纯色块。
struct Scene {
    rect: Rect,
    text_rect: Rect,
}

impl Widget for Scene {
    fn id(&self) -> &str {
        "scene"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(640.0, 240.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        self.text_rect = Rect::new(r.x + 16.0, r.y + 16.0, r.w - 32.0, r.h - 32.0);
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 背景
        ctx.fill_rect(self.rect, Color::rgba(0.10, 0.11, 0.13, 1.0));
        // 45° 线性渐变（验证 GPU 渐变图元：逐像素插值）
        ctx.fill_gradient_rect_dir(
            Rect::new(self.rect.x + 16.0, self.rect.y + 60.0, 240.0, 80.0),
            Color::rgba(1.0, 0.10, 0.05, 1.0),
            Color::rgba(0.05, 0.20, 1.0, 1.0),
            (1.0, 1.0),
        );
        // 一个纯色块（验证 fill_rect 路径）
        ctx.fill_rect(
            Rect::new(self.rect.x + 16.0, self.rect.y + 150.0, 120.0, 60.0),
            Color::rgba(0.85, 0.25, 0.20, 1.0),
        );
        // 中英文混排（验证字形 + ClearType 子像素）
        let styles = [
            (Color::WHITE, "ASCII ABC xyz 0123"),
            (Color::rgba(0.60, 0.85, 0.40, 1.0), "中文 汉字 渲染 测试"),
            (Color::rgba(0.40, 0.70, 1.00, 1.0), "The quick brown fox"),
        ];
        let mut y = self.text_rect.y;
        for (fg, text) in styles {
            let style = TextStyle::fg(fg);
            ctx.draw_text(Pos::new(self.text_rect.x, y), text, &style);
            y += 28.0;
        }
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn snapshot_harness_is_deterministic() {
    let options = Options {
        title: "y901 snapshot harness".to_string(),
        width: 640,
        height: 240,
        theme: None,
        font: None,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        text_rect: Rect::ZERO,
    }));
    window.resize(&mut tree);

    // 第一帧：整窗
    tree.invalidate_all();
    window.render(&mut tree);
    // 第二帧：再来一次（快照自检/哈希都在 render 内部打印）
    tree.invalidate_all();
    window.render(&mut tree);

    // 探针只在两帧之后生效；这里再渲染一帧给 `Y901_*` 自检用
    tree.invalidate_all();
    window.render(&mut tree);
    println!("[harness] 三次渲染完成（快照/对拍输出见 stderr）");
}

/// GPU 渐变必须**逐像素**符合数学插值（不是"看着像"）。
///
/// 场景里画了一块的 45° 渐变（红色 → 蓝色）。这里按与着色器完全相同的公式
/// 算出每个像素的期望颜色，与回读像素对比（允许 8-bit 量化的 ±3）。
#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn gradient_matches_math_at_45_degrees() {
    let options = Options {
        title: "y901 gradient harness".to_string(),
        width: 640,
        height: 240,
        theme: None,
        font: None,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        text_rect: Rect::ZERO,
    }));
    window.resize(&mut tree);
    // 快照必须在 present 之前取：先请求，再渲染一帧。
    window.request_snapshot();
    tree.invalidate_all();
    window.render(&mut tree);

    let (px, w, h) = window
        .take_snapshot()
        .expect("快照失败（staging/Map）");
    assert_eq!(px.len(), (w * h * 4) as usize, "回读尺寸不对");

    // 与 Scene::render 里的渐变矩形保持一致
    let gx = 16.0f32;
    let gy = 60.0f32;
    let gw = 240.0f32;
    let gh = 80.0f32;
    let from = [1.0f32, 0.10, 0.05];
    let to = [0.05f32, 0.20, 1.0];
    let (dx, dy) = (1.0f32, 1.0f32);
    let len2 = dx * dx + dy * dy;
    let mut t_min = f32::MAX;
    let mut t_max = f32::MIN;
    for (cx, cy) in [(0.0, 0.0), (gw, 0.0), (0.0, gh), (gw, gh)] {
        let t = (cx * dx + cy * dy) / len2;
        t_min = t_min.min(t);
        t_max = t_max.max(t);
    }
    let span = t_max - t_min;

    let mut checked = 0;
    let mut worst = 0.0f32;
    // 采样步长 3px，避开边界像素（边缘可能有 1px 覆盖差异）
    // 采样区域必须在实际帧缓冲内（客户区尺寸可能比请求的小）
    // 只采样"纯渐变"带：避开字形（字在 y=16/44/72 三行，每行 ~16px）
    // 与纯色块（y ≈ 150 起），否则采样到的是字形/纯色像素。
    let x_end = ((gx + gw) as u32).min(w).saturating_sub(2);
    let y_begin = ((gy as u32) + 36).max(96);
    let y_end = ((gy + gh) as u32).min(h).saturating_sub(4);
    assert!(y_end > y_begin, "采样带为空：{y_begin}..{y_end}");
    for y in (y_begin..y_end).step_by(3) {
        for x in ((gx as u32) + 2..x_end).step_by(3) {
            let lx = x as f32 - gx;
            let ly = y as f32 - gy;
            let t = (((lx * dx + ly * dy) / len2 - t_min) / span).clamp(0.0, 1.0);
            let expect = [
                from[0] + (to[0] - from[0]) * t,
                from[1] + (to[1] - from[1]) * t,
                from[2] + (to[2] - from[2]) * t,
            ];
            // BGRA
            let i = ((y * w + x) * 4) as usize;
            let got = [
                px[i + 2] as f32 / 255.0,
                px[i + 1] as f32 / 255.0,
                px[i] as f32 / 255.0,
            ];
            for c in 0..3 {
                let d = (got[c] - expect[c]).abs();
                worst = worst.max(d);
                assert!(
                    d <= 3.0 / 255.0,
                    "({x},{y}) 通道{c} 期望 {:.3} 实际 {:.3}（t={t:.3}）",
                    expect[c],
                    got[c]
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 1000, "采样点太少：{checked}");
    eprintln!("[gradient] 逐像素验证通过：{checked} 个采样点，最大误差 {worst:.4}");
}
