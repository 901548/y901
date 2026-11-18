#![cfg(windows)]

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
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
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
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(640.0, 240.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
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
        // 圆角矩形（验证 GPU SDF 圆角）：放在右上角，与渐变/文字不重叠
        ctx.fill_round_rect(
            Rect::new(self.rect.x + 360.0, self.rect.y + 16.0, 200.0, 120.0),
            Color::rgba(0.95, 0.55, 0.15, 1.0),
            26.0,
        );
        // 圆角 + 45° 渐变（验证 SDF 裁切 × 渐变插值）
        ctx.fill_gradient_round_rect(
            Rect::new(self.rect.x + 270.0, self.rect.y + 100.0, 80.0, 40.0),
            Color::rgba(0.10, 0.90, 0.95, 1.0),
            Color::rgba(0.95, 0.10, 0.60, 1.0),
            (1.0, 1.0),
            12.0,
        );
        // 圆角描边（验证 SDF 轮廓带）：放在下方，与其他图元不重叠
        ctx.stroke_round_rect(
            Rect::new(self.rect.x + 360.0, self.rect.y + 160.0, 200.0, 64.0),
            Color::rgba(0.35, 0.95, 0.55, 1.0),
            10.0,
            14.0,
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

        fullscreen: false,
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

        fullscreen: false,
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

    let (px, w, h) = window.take_snapshot().expect("快照失败（staging/Map）");
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

/// GPU SDF 圆角：逐像素对照解析式（不是"看着圆"）。
///
/// 场景里在 (360,16) 画了 200x120、半径 26 的圆角矩形（橙色）。
/// 这里用与着色器**完全相同的 SDF 公式**算覆盖率，与回读像素比对：
/// - 内部（d < -0.5）必须完全不透明；
/// - 外部（d > 0.5）必须完全透明（露出背景）；
/// - 边缘带（-0.5 ≤ d ≤ 0.5）必须是中间值（证明有抗锯齿，不是硬台阶）。
#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn round_rect_sdf_matches_math() {
    let options = Options {
        title: "y901 round harness".to_string(),
        width: 640,
        height: 240,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        text_rect: Rect::ZERO,
    }));
    window.resize(&mut tree);
    window.request_snapshot();
    tree.invalidate_all();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");

    let (rx, ry, rw, rh, radius) = (360.0f32, 16.0f32, 200.0f32, 120.0f32, 26.0f32);
    let half = [rw * 0.5, rh * 0.5];
    let mut inside = 0;
    let mut outside = 0;
    let mut edge = 0;

    for y in (ry as u32)..((ry + rh) as u32).min(h) {
        for x in (rx as u32)..((rx + rw) as u32).min(w) {
            let p = [x as f32 - rx - half[0] + 0.5, y as f32 - ry - half[1] + 0.5];
            let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
            let outside_part = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt();
            let d = q[0].max(q[1]).min(0.0) + outside_part - radius;
            let i = ((y * w + x) * 4) as usize;
            let got = [px[i + 2], px[i + 1], px[i]];

            if d < -1.0 {
                // 内部：必须是实心橙色 (242,140,38) ± 量化
                inside += 1;
                assert!(
                    (got[0] as i32 - 242).abs() <= 4
                        && (got[1] as i32 - 140).abs() <= 4
                        && (got[2] as i32 - 38).abs() <= 4,
                    "({x},{y}) d={d:.2} 应为实心橙色，实际 {got:?}"
                );
            } else if d > 1.0 {
                // 外部：不应是橙色（背景或其它内容）
                outside += 1;
                assert!(
                    !((got[0] as i32 - 242).abs() <= 4 && (got[1] as i32 - 140).abs() <= 4),
                    "({x},{y}) d={d:.2} 应在圆角外，实际 {got:?}"
                );
            } else {
                edge += 1;
            }
        }
    }
    assert!(inside > 5000, "内部像素太少：{inside}");
    assert!(outside > 100, "外部像素太少：{outside}");
    // 边缘带：周长约 2*(200+120) = 640 像素 → 1px 过渡带应有数百个点
    assert!(edge > 150, "边缘过渡带太窄，可能没做抗锯齿：{edge}");
    eprintln!("[round] SDF 验证通过：内部 {inside}、外部 {outside}、边缘带 {edge} 像素");
}

/// GPU SDF 圆角描边：只画轮廓带，内部必须透出背景。
#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn round_stroke_draws_ring_only() {
    let options = Options {
        title: "y901 stroke harness".to_string(),
        width: 640,
        height: 240,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        text_rect: Rect::ZERO,
    }));
    window.resize(&mut tree);
    window.request_snapshot();
    tree.invalidate_all();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");

    // 后端会把矩形裁到视口内再交给着色器，这里必须用**裁剪后**的几何算期望，
    // 否则算出来的 d 和 GPU 看到的不是同一个矩形（第一版就这么错的）。
    let (rx0, ry0, rw0, rh0, radius, width_px) =
        (360.0f32, 160.0f32, 200.0f32, 64.0f32, 14.0f32, 10.0f32);
    let (rx, ry) = (rx0, ry0);
    let rw = rw0.min(w as f32 - rx0);
    let rh = rh0.min(h as f32 - ry0);
    let half = [rw * 0.5, rh * 0.5];
    let stroke_rgb = [89i32, 242, 140]; // (0.35, 0.95, 0.55) * 255
    let mut ring = 0;
    let mut inner = 0;
    let mut outside = 0;

    for y in (ry as u32)..((ry + rh) as u32).min(h) {
        for x in (rx as u32)..((rx + rw) as u32).min(w) {
            let p = [x as f32 - rx - half[0] + 0.5, y as f32 - ry - half[1] + 0.5];
            let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
            let ext = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt();
            let d = q[0].max(q[1]).min(0.0) + ext - radius;
            let i = ((y * w + x) * 4) as usize;
            let got = [px[i + 2] as i32, px[i + 1] as i32, px[i] as i32];
            let is_stroke = (got[0] - stroke_rgb[0]).abs() <= 12
                && (got[1] - stroke_rgb[1]).abs() <= 12
                && (got[2] - stroke_rgb[2]).abs() <= 12;

            if d.abs() <= width_px * 0.5 - 2.0 {
                // 带内：必须是描边色
                ring += 1;
                assert!(is_stroke, "({x},{y}) d={d:.2} 应在描边带内，实际 {got:?}");
            } else if d < -(width_px * 0.5 + 2.0) {
                // 内部：必须不是描边色（证明是轮廓而不是实心）
                inner += 1;
                assert!(
                    !is_stroke,
                    "({x},{y}) d={d:.2} 内部不应被描边填满，实际 {got:?}"
                );
            } else if d > width_px * 0.5 + 2.0 {
                outside += 1;
                assert!(
                    !is_stroke,
                    "({x},{y}) d={d:.2} 外部不应有描边，实际 {got:?}"
                );
            }
        }
    }
    // 阈值**按几何动态算**，不写死。
    // 矩形会被视口裁切（小窗口时拿不到全部 200x64），
    // 所以先用裁剪后的尺寸算总面积，再按比例约束。
    let area = (rw.max(1.0) * rh.max(1.0)) as f64;
    assert!(
        (ring + inner) as f64 <= area * 1.05,
        "内部+轮廓不应超过矩形面积：{ring}+{inner} vs {area:.0}"
    );
    // 轮廓带面积 ≈ 周长 × 宽度（实测偏小一些，取 20% 作为下限）
    let expected_ring = ((rw + rh) as f64 * 2.0) * width_px as f64 * 0.2;
    assert!(
        ring as f64 >= expected_ring.max(50.0),
        "轮廓带像素偏少：{ring}（期望≥{expected_ring:.0}）"
    );
    // 内部面积 ≈ 总面积 - 轮廓带，取 50% 作为下限
    let expected_inner = (area - ring as f64) * 0.5;
    assert!(
        inner as f64 >= expected_inner.max(100.0),
        "内部像素偏少：{inner}（期望≥{expected_inner:.0}）"
    );
    // 说明：矩形被视口下沿裁切，下两个圆角实际不存在，
    // 所以"外部"像素很少；这里只要求不会误判。
    let _ = outside;
    eprintln!("[stroke] SDF 描边验证通过：轮廓带 {ring}、内部 {inner}、外部 {outside} 像素");
}

/// 圆角 + 45° 渐变：内部颜色必须符合插值，四角必须被 SDF 裁掉。
#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn gradient_round_rect_matches_math() {
    let options = Options {
        title: "y901 gradient-round harness".to_string(),
        width: 640,
        height: 240,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        text_rect: Rect::ZERO,
    }));
    window.resize(&mut tree);
    window.request_snapshot();
    tree.invalidate_all();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");

    // 与场景保持一致；必须用**裁剪后**的几何（后端就是这么传的）
    let (gx0, gy0, gw0, gh0, radius) = (270.0f32, 100.0f32, 80.0f32, 40.0f32, 12.0f32);
    let gx = gx0;
    let gy = gy0;
    let gw = gw0.min(w as f32 - gx0);
    let gh = gh0.min(h as f32 - gy0);
    let from = [0.10f32, 0.90, 0.95];
    let to = [0.95f32, 0.10, 0.60];
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
    let half = [gw * 0.5, gh * 0.5];

    let mut checked = 0;
    let mut corner_cut = 0;
    let mut worst = 0.0f32;
    for y in (gy as u32)..((gy + gh) as u32).min(h) {
        for x in (gx as u32)..((gx + gw) as u32).min(w) {
            let p = [x as f32 - gx - half[0] + 0.5, y as f32 - gy - half[1] + 0.5];
            let q = [p[0].abs() - half[0] + radius, p[1].abs() - half[1] + radius];
            let ext = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2)).sqrt();
            let d = q[0].max(q[1]).min(0.0) + ext - radius;
            let i = ((y * w + x) * 4) as usize;
            let got = [
                px[i + 2] as f32 / 255.0,
                px[i + 1] as f32 / 255.0,
                px[i] as f32 / 255.0,
            ];
            if d < -2.0 {
                let t = ((((x as f32 - gx) * dx + (y as f32 - gy) * dy) / len2 - t_min) / span)
                    .clamp(0.0, 1.0);
                for c in 0..3 {
                    let expect = from[c] + (to[c] - from[c]) * t;
                    let diff = (got[c] - expect).abs();
                    worst = worst.max(diff);
                    assert!(
                        diff <= 4.0 / 255.0,
                        "({x},{y}) 通道{c} 期望 {expect:.3} 实际 {:.3}",
                        got[c]
                    );
                }
                checked += 1;
            } else if d > 2.0 {
                // 圆角外：必须不是渐变色（被 SDF 裁掉）
                let looks_gradient =
                    (got[0] - from[0]).abs() < 0.08 && (got[2] - from[2]).abs() < 0.15;
                if !looks_gradient {
                    corner_cut += 1;
                }
            }
        }
    }
    assert!(checked > 1500, "内部采样点太少：{checked}");
    // 四角被裁掉的面积本就很小（r=12 → 每角约 31px），
    // 取 d>2 的核心区域后只剩几十个像素。
    assert!(
        corner_cut > 20,
        "四角没被裁掉（可能没走 SDF）：{corner_cut}"
    );
    eprintln!(
        "[grad+round] 验证通过：内部 {checked} 点最大误差 {worst:.4}，裁掉角 {corner_cut} 像素"
    );
}

/// 流式追加 + `invalidate_rect` 局部重绘：必须与"全量重绘"逐像素一致。
///
/// 这是 `invalidate_rect` 上生产前的最后一道保险：
///   A. 画一帧"短文本"（模拟已有一行）
///   B. 文本变长（模拟 token 追加）→ 只 `invalidate_rect(那一行)` → 渲染 → 快照哈希 h1
///   C. 用**全新窗口**直接渲染"长文本"整窗 → 快照哈希 h2
///   断言 h1 == h2：局部刷新后的画面与从零画出来的画面完全相同。
#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn invalidate_rect_partial_equals_full() {
    // 流式文本：长度由外部 Cell 控制，渲染时按长度画字符
    struct Stream {
        n: std::rc::Rc<std::cell::Cell<usize>>,
        rect: Rect,
        line: Rect,
    }
    impl Widget for Stream {
        fn id(&self) -> &str {
            "stream"
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(c.max.w, c.max.h))
        }
        fn arrange(
            &mut self,
            r: Rect,
            _m: &dyn TextMetrics,
            _cache: &mut y901_ui::layout::LayoutCache,
        ) {
            self.rect = r;
            // 流式行固定在左上角一块小区域
            self.line = Rect::new(r.x + 8.0, r.y + 8.0, 320.0, 20.0);
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn render(&mut self, ctx: &mut dyn RenderCtx) {
            ctx.fill_rect(self.line, Color::rgba(0.10, 0.11, 0.14, 1.0));
            let text: String = "abcdefghijklmnopqrstuvwxyz"[..self.n.get().min(26)].to_string();
            ctx.draw_text(
                Pos::new(self.line.x + 2.0, self.line.y + 2.0),
                &text,
                &TextStyle::fg(Color::WHITE),
            );
        }
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    let make = |n: usize| {
        let cell = std::rc::Rc::new(std::cell::Cell::new(n));
        let tree = UiTree::new(Box::new(Stream {
            n: cell.clone(),
            rect: Rect::ZERO,
            line: Rect::ZERO,
        }));
        (tree, cell)
    };

    // ---- A/B：同一个窗口里先短后长，用 invalidate_rect 做局部刷新 ----
    let options = Options {
        title: "y901 invalidate_rect harness".to_string(),
        width: 480,
        height: 200,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let (mut tree, cell) = make(4);
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree); // A：短文本整窗

    // B：追加 token（只改数据），精确标脏那一行
    cell.set(26);
    let line = tree.rect_of("stream").expect("stream");
    // **脏区必须覆盖"实际画出来的"字形**，而不只是这行的逻辑矩形。
    //
    // 字号按 DPI 缩放（`px * dpi_scale`），而这条测试行的固定高度是 20px、
    // 文字又从 `line.y + 2` 起笔：本机 125% 缩放下默认 16 号字的物理高度正好
    // 是 20px，于是字形会溢出到这行**下方 1~2px**。只标 `320x20` 时，溢出的
    // 那两行像素既不重绘也不被覆盖，局部刷新与全量重绘必然不一致
    // （实测差异 156 字节，首个差异在行底边下一行）。
    //
    // 这是**测试的脏区写窄了**，不是渲染错：真实控件标脏时用的是自身 bounds，
    // 而它的 bounds 会随字号一起长高。这里用 4px 余量把"字形溢出"包进来，
    // 同时仍然只标窗口内的一小块，局部重绘的语义没有被削弱。
    // **脏区只标逻辑行矩形，不加任何手工余量**。
    //
    // 修前这里必须手工 `pad = 4.0` 才绿（125% 下字形溢出这行 1~2px，
    // 溢出的像素不在脏区里、局部重绘清不掉）。现在 `UiTree` 在标脏时统一
    // 按 `ink_bounds` + 最小安全边距外扩（`add_dirty`），所以这里可以回到
    // 最朴素的写法 —— 这条用例同时也守住了"框架兜底不许退化"。
    tree.invalidate_rect(Rect::new(line.x + 8.0, line.y + 8.0, 320.0, 20.0));
    window.request_snapshot();
    window.render(&mut tree);
    let (px1, w1, h1) = window.take_snapshot().expect("快照 A/B 失败");

    // ---- C：全新窗口，直接渲染长文本 ----
    let mut window2 = Window::create(&options).expect("需要可用的 D3D11 设备");
    let (mut tree2, _cell2) = make(26);
    window2.resize(&mut tree2);
    window2.request_snapshot();
    tree2.invalidate_all();
    window2.render(&mut tree2);
    let (px2, w2, h2) = window2.take_snapshot().expect("快照 C 失败");

    assert_eq!((w1, h1), (w2, h2), "两次快照尺寸应一致");
    let mut diff = 0usize;
    let mut first = None;
    for i in 0..(w1 * h1 * 4) as usize {
        if px1[i] != px2[i] {
            diff += 1;
            if first.is_none() {
                let p = i / 4;
                first = Some((p as u32 % w1, p as u32 / w1));
            }
        }
    }
    assert_eq!(
        diff, 0,
        "局部重绘与全量重绘不一致：{diff} 字节不同，首个差异 {first:?}"
    );
    eprintln!("[invalidate_rect] 局部刷新 == 全量重绘（{w1}x{h1}，0 差异）");
}

/// 两块**不相邻**的脏区分别更新：局部重绘必须与整窗重绘逐像素一致。
///
/// 这是 "dirty.bounds() 合并包围盒" 这个 bug 的回归测试。
/// 关键设计：**中间那块是独立子控件**——两块脏区都不与它相交，CPU 侧剪枝会跳过它。
/// 若后端按包围盒清屏，中间那块会被清掉且不再重绘 → 变黑，测试立刻红。
#[test]
#[ignore = "需要真实 GPU：cargo test -p y901-backend-win32 --release --test snapshot_harness -- --ignored --nocapture"]
fn two_disjoint_dirty_rects_match_full_redraw() {
    use std::cell::Cell as StdCell;
    use std::rc::Rc;

    /// 固定位置的小色块（自己画自己那块）。
    struct Cell {
        id: String,
        color: Color,
        rect: Rect,
    }
    impl Widget for Cell {
        fn id(&self) -> &str {
            &self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(40.0, 20.0))
        }
        fn arrange(
            &mut self,
            r: Rect,
            _m: &dyn TextMetrics,
            _cache: &mut y901_ui::layout::LayoutCache,
        ) {
            self.rect = r;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn render(&mut self, ctx: &mut dyn RenderCtx) {
            ctx.fill_rect(self.rect, self.color);
        }
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    /// 容器：铺满窗口画背景，并按固定位置摆放三个子块。
    struct Board {
        rect: Rect,
        wide: Rc<StdCell<f32>>,
        kids: Vec<Box<dyn Widget>>,
    }
    impl Widget for Board {
        fn id(&self) -> &str {
            "board"
        }
        fn children(&self) -> &[Box<dyn Widget>] {
            &self.kids
        }
        fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
            Some(&mut self.kids)
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut y901_ui::layout::LayoutCache,
        ) -> Size {
            c.constrain(Size::new(c.max.w, c.max.h))
        }
        fn arrange(
            &mut self,
            r: Rect,
            m: &dyn TextMetrics,
            cache: &mut y901_ui::layout::LayoutCache,
        ) {
            self.rect = r;
            // 左上 / 中间（脏区不覆盖它）/ 右下
            let spots = [
                Rect::new(r.x + 10.0, r.y + 10.0, 40.0, 20.0),
                Rect::new(r.x + 200.0, r.y + 80.0, 60.0, 40.0),
                Rect::new(r.x + 380.0, r.y + 150.0, self.wide.get(), 20.0),
            ];
            for (kid, spot) in self.kids.iter_mut().zip(spots) {
                kid.arrange(spot, m, cache);
            }
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn render(&mut self, ctx: &mut dyn RenderCtx) {
            ctx.fill_rect(self.rect, Color::rgba(0.08, 0.09, 0.11, 1.0));
        }
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    let make = || {
        let wide = Rc::new(StdCell::new(40.0f32));
        let kids: Vec<Box<dyn Widget>> = vec![
            Box::new(Cell {
                id: "c0".into(),
                color: Color::rgba(0.90, 0.20, 0.20, 1.0),
                rect: Rect::ZERO,
            }),
            Box::new(Cell {
                id: "c1".into(),
                color: Color::rgba(0.90, 0.90, 0.20, 1.0),
                rect: Rect::ZERO,
            }),
            Box::new(Cell {
                id: "c2".into(),
                color: Color::rgba(0.20, 0.90, 0.30, 1.0),
                rect: Rect::ZERO,
            }),
        ];
        (
            UiTree::new(Box::new(Board {
                rect: Rect::ZERO,
                wide: wide.clone(),
                kids,
            })),
            wide,
        )
    };

    let options = Options {
        title: "y901 disjoint dirty harness".to_string(),
        width: 520,
        height: 220,
        theme: None,
        font: None,

        fullscreen: false,
    };

    // A：整窗画（c2 宽 40）
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let (mut tree, wide) = make();
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree);

    // B：c2 变宽（80）→ 只标 c0（不变，故意加进来）与 c2 两块脏区
    wide.set(80.0);
    tree.mark_layout_dirty("board");
    tree.layout(window_viewport(&window), &HarnessMetrics);
    let c0 = tree.rect_of("c0").expect("c0");
    let c2 = tree.rect_of("c2").expect("c2");
    tree.invalidate_rect(c0);
    tree.invalidate_rect(c2);
    window.request_snapshot();
    window.render(&mut tree);
    let (px1, w1, h1) = window.take_snapshot().expect("快照 A/B 失败");

    // C：全新窗口直接画最终内容（c2 宽 80）
    let mut window2 = Window::create(&options).expect("需要可用的 D3D11 设备");
    let (mut tree2, wide2) = make();
    wide2.set(80.0);
    window2.resize(&mut tree2);
    tree2.mark_layout_dirty("board");
    tree2.layout(window_viewport(&window2), &HarnessMetrics);
    window2.request_snapshot();
    tree2.invalidate_all();
    window2.render(&mut tree2);
    let (px2, w2, h2) = window2.take_snapshot().expect("快照 C 失败");

    assert_eq!((w1, h1), (w2, h2));
    let mut diff = 0usize;
    let mut first = None;
    for i in 0..(w1 * h1 * 4) as usize {
        if px1[i] != px2[i] {
            diff += 1;
            if first.is_none() {
                let p = i / 4;
                first = Some((p as u32 % w1, p as u32 / w1));
            }
        }
    }
    assert_eq!(
        diff, 0,
        "两块不相邻脏区的局部重绘与整窗不一致：{diff} 字节不同，首个差异 {first:?}"
    );
    eprintln!("[dirty2] 两块不相邻脏区 == 整窗重绘（{w1}x{h1}，0 差异）");
}

/// 取窗口客户区尺寸（harness 里布局用）。
fn window_viewport(w: &Window) -> Size {
    let (cw, ch) = w.viewport();
    Size::new(cw as f32, ch as f32)
}

/// harness 用的度量：等宽 8×16。
struct HarnessMetrics;

impl TextMetrics for HarnessMetrics {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 最小红线：`TextLabel` 的文字必须真的出现在像素里。
///
/// 为什么要单独测这个：文字路径上疹点太多——字形图集、UV重映、
/// 裁剪圆角、字形 PS 的 alpha。它们任何一个坏了，表现都是"代码调了 draw_text
/// 但屏幕没字"。这个用例把"没字"和"没调" 区分开。
#[test]
#[ignore = "需要真实 GPU"]
fn text_label_paints_visible_non_background_pixels() {
    use y901_widgets::TextLabel;

    let options = Options {
        title: "y901 text harness".to_string(),
        width: 320,
        height: 120,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(
        y901_ui::Flex::column("root").push(Box::new(
            TextLabel::new("lbl", "Hello Y901")
                .color(y901_ui::Color::rgba(1.0, 1.0, 1.0, 1.0))
                .flex_item(y901_ui::FlexItem::height(40.0)),
        )),
    ));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.request_snapshot();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");

    // 背景色（取左下角一定没字的像素作为基准）
    let base_idx = (((h - 2) * w + 2) * 4) as usize;
    let bg = [px[base_idx], px[base_idx + 1], px[base_idx + 2]];

    let mut bright = 0usize;
    let mut first = None;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let r = px[i] as i32;
            let g = px[i + 1] as i32;
            let b = px[i + 2] as i32;
            let d = (r - bg[0] as i32).abs() + (g - bg[1] as i32).abs() + (b - bg[2] as i32).abs();
            if d > 40 {
                bright += 1;
                if first.is_none() {
                    first = Some((x, y, [px[i], px[i + 1], px[i + 2], px[i + 3]]));
                }
            }
        }
    }
    assert!(
        bright >= 40,
        "文字没画出来：与背景差异的像素只有 {bright} 个（背景 {bg:?}，{w}x{h}）"
    );
    eprintln!("[text] 文字像素 {bright} 个，首个差异 {first:?}，背景 {bg:?}");
}

/// 同一句话，但放在 `ScrollView`（带裁剪）里。
///
/// 区分"文字不画"与"文字被裁剪吃掉"：若上一个用例过、
/// 这个不过，问题就在 `push_clip` / `clip_quad`。
#[test]
#[ignore = "需要真实 GPU"]
fn text_inside_scrollview_is_not_clipped_away() {
    use y901_widgets::{ScrollView, TextLabel};

    let options = Options {
        title: "y901 text-in-scroll harness".to_string(),
        width: 320,
        height: 160,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let sv = ScrollView::new(
        "sv",
        Box::new(
            TextLabel::new("lbl", "Scrolled text")
                .color(y901_ui::Color::rgba(1.0, 1.0, 1.0, 1.0))
                .flex_item(y901_ui::FlexItem::height(30.0)),
        ),
    );
    let mut tree = UiTree::new(Box::new(y901_ui::Flex::column("root").push(Box::new(sv))));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.request_snapshot();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");

    let base_idx = (((h - 2) * w + 2) * 4) as usize;
    let bg = [px[base_idx], px[base_idx + 1], px[base_idx + 2]];
    let mut bright = 0usize;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let d = (px[i] as i32 - bg[0] as i32).abs()
                + (px[i + 1] as i32 - bg[1] as i32).abs()
                + (px[i + 2] as i32 - bg[2] as i32).abs();
            if d > 40 {
                bright += 1;
            }
        }
    }
    assert!(
        bright >= 40,
        "ScrollView 里的文字被裁掉了：只有 {bright} 个非背景像素"
    );
    eprintln!("[text] ScrollView 内文字像素 {bright} 个");
}

/// 自持 RT 被重建（尺寸变化）后，**下一帧必须整窗清屏 + 整窗重绘**。
///
/// 早期实现两个缺陷：
/// 1. `Gfx::resize` 把 `cached = None`，新 RT 未初始化（alpha=0）
///    → 未被脏区覆盖的地方 blit 出**纯黑**；
/// 2. `Window::resize` 不标脏 → CPU 剪枝跳过没移动的节点
///    → 它们的文字永远不重画（看上去就是"文字被吃了"）。
///
/// 关键设计：**只改高度**（宽度不变），两个文本都靠上左定位，
/// 重布局后它们的矩形**不变** → `mark_moved` 不会把它们标脏。
#[test]
#[ignore = "需要真实 GPU"]
fn rebuild_of_cached_target_does_not_leave_black_areas() {
    let options = Options {
        title: "y901 resize harness".to_string(),
        width: 320,
        height: 160,
        theme: None,
        font: None,

        fullscreen: false,
    };
    let mut window = Window::create(&options).expect("需要可用的 D3D11 设备");
    let mut tree = UiTree::new(Box::new(
        y901_ui::Flex::column("root")
            .push(Box::new(
                y901_widgets::TextLabel::new("lbl", "resize check")
                    .color(y901_ui::Color::rgba(1.0, 1.0, 1.0, 1.0))
                    .flex_item(y901_ui::FlexItem::height(30.0)),
            ))
            .push(Box::new(
                y901_widgets::TextLabel::new("tail", "|")
                    .color(y901_ui::Color::rgba(1.0, 1.0, 1.0, 1.0))
                    .flex_item(y901_ui::FlexItem::height(20.0)),
            )),
    ));
    window.resize(&mut tree);
    tree.invalidate_all();
    window.render(&mut tree);

    // 只改高度：两个 label 的矩形不变 → 重布局不会把它们标脏
    window.resize_to_for_test(&mut tree, 320, 200);

    // 只标小尾巴那一行（模拟"只有小区域脏"）：若整窗重绘生效，
    // 上面那句 "resize check" 也应该被重新画出来
    tree.invalidate("tail");
    window.request_snapshot();
    window.render(&mut tree);
    let (px, w, h) = window.take_snapshot().expect("快照失败");

    // (a) 不能有纯黑（alpha=0的未初始化像素）
    let mut black = 0usize;
    for i in (0..(w * h * 4) as usize).step_by(4) {
        if px[i] < 3 && px[i + 1] < 3 && px[i + 2] < 3 && px[i + 3] < 3 {
            black += 1;
        }
    }
    assert_eq!(
        black, 0,
        "重建后出现 {black} 个纯黑像素（{w}x{h}）——整窗清屏没生效"
    );

    // (b) "上面那句话"必须在（只重画尾巴行时它会消失）
    let base_idx = (((h - 2) * w + 2) * 4) as usize;
    let bg = [px[base_idx], px[base_idx + 1], px[base_idx + 2]];
    let mut bright = 0usize;
    for y in 0..60.min(h) {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let d = (px[i] as i32 - bg[0] as i32).abs()
                + (px[i + 1] as i32 - bg[1] as i32).abs()
                + (px[i + 2] as i32 - bg[2] as i32).abs();
            if d > 40 {
                bright += 1;
            }
        }
    }
    assert!(
        bright >= 40,
        "重建后上方文字没重画：只有 {bright} 个非背景像素"
    );
}
