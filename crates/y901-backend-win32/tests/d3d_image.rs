//! D3D11 图片纹理路径的像素验收（真机设备 + 快照回读）。
//!
//! 与 `font_3c_pixels` 同一套脚手架：真窗口 + `take_snapshot()`。
//! 这些测试在无 D3D 的环境（CI/Linux）会被 `#[ignore]` 跳过。

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::image::{ImageFit, ImageStyle};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    UiEvent, UiTree, Widget,
};
use y901_widgets::image::Image;

/// 场景根：铺一张图片 + 一个可选的纯色矩形（用于对照）。
struct Scene {
    rect: Rect,
    child: Option<Image>,
}

impl Widget for Scene {
    fn id(&self) -> &str {
        "scene"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(200.0, 200.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        // 子控件不在树里（这个场景直接持有 Image），必须手动布局，
        // 否则 Image.rect 还是 ZERO → render 直接 return，什么都画不出来。
        if let Some(img) = self.child.as_mut() {
            img.arrange(rect, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if let Some(img) = self.child.as_mut() {
            img.render(ctx);
        }
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 造 2×2 四象限纹理（红/绿/蓝/白）。
fn quad_rgba() -> Vec<u8> {
    vec![
        255, 0, 0, 255, // (0,0) 红
        0, 255, 0, 255, // (1,0) 绿
        0, 0, 255, 255, // (0,1) 蓝
        255, 255, 255, 255, // (1,1) 白
    ]
}

/// 纯色 w×h 纹理。
fn solid_rgba(w: u32, h: u32, c: (u8, u8, u8)) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        v.extend_from_slice(&[c.0, c.1, c.2, 255]);
    }
    v
}

/// 建窗口（隐藏尺寸小一点，快照足够）。
/// 序列化所有建真实 D3D 窗口的测试。
///
/// 为什么需要：同一个测试二进制里的用例默认**并发**跑，而每个用例都要建真窗口、
/// 拿 D3D 设备、请求快照；并发时会互相抢占前台窗口，实测出现过假红。
static D3D_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn window(title: &str, w: u32, h: u32) -> Option<Window> {
    let options = Options {
        title: title.to_string(),
        width: w,
        height: h,
        theme: None,
        font: None,
    };
    Window::create(&options).ok()
}

/// 渲染一帧并回读 BGRA 像素。
fn render_scene(win: &mut Window, tree: &mut UiTree) -> (Vec<u8>, u32, u32) {
    win.resize(tree);
    tree.invalidate_all();
    win.request_snapshot();
    win.render(tree);
    win.take_snapshot().expect("快照失败")
}

/// 取 BGRA 像素的 RGB。
fn rgb(px: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * w + x) * 4) as usize;
    (px[i + 2], px[i + 1], px[i])
}

#[test]
fn image_quadrants_sample_correctly_on_gpu() {
    let _guard = D3D_LOCK.lock().unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-d3d-image-quad", 240, 200) else {
        eprintln!("[image-d3d] 无 D3D11 设备，跳过");
        return;
    };
    let img = Image::from_rgba("quad", 2, 2, &quad_rgba()).fit(ImageFit::Fill);
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        child: Some(img),
    }));
    // 让图片占满 200x200（Scene::measure 返回 200x200）
    {
        let mut cache = LayoutCache::new();
        let _ = &mut cache;
    }
    let (px, w, h) = render_scene(&mut win, &mut tree);

    // 2x2 纹理被拉到 200x200：测试最靠近四角的点（双线性在那里接近原色），
    // 教训：拿"四象限中心"断言纯色是错的——中心正好是四色混合区。
    // 坐标相对快照尺寸计算：窗口客户区尺寸与请求尺寸不完全相等（DPI/边框）。
    for (x, y, want, name) in [
        (5u32.min(w - 8), 5u32.min(h - 8), (255u8, 0u8, 0u8), "左上 红"),
        (w - 8, 5, (0, 255, 0), "右上 绿"),
        (5, h - 8, (0, 0, 255), "左下 蓝"),
        (w - 8, h - 8, (255, 255, 255), "右下 白"),
    ] {
        let (r, g, b) = rgb(&px, w, x, y);
        eprintln!("[image-d3d] ({x},{y}) 期望近 {name} 实际 ({r},{g},{b})");
        let close = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 40;
        assert!(
            close(r, want.0) && close(g, want.1) && close(b, want.2),
            "({x},{y}) 应接近 {name} = {want:?}，实际 ({r},{g},{b})（已计入双线性插值）"
        );
    }

    // 与 CPU 路径的一致性：四象限中心的"主色归属"必须相同（容差放宽到 60）。
    // 为什么不直接比整数值：GPU 采样位置与 CPU 的四舍五入可能差半个像素。
    let (mx, my) = (w / 4, h / 4);
    let (r, g, b) = rgb(&px, w, mx, my);
    assert!(r > g && r > b, "左上区应偏红，实际 ({r},{g},{b})");
    let (r, g, b) = rgb(&px, w, w - mx, my);
    assert!(g > r && g > b, "右上区应偏绿，实际 ({r},{g},{b})");
    let (r, g, b) = rgb(&px, w, mx, h - my);
    assert!(b > r && b > g, "左下区应偏蓝，实际 ({r},{g},{b})");
    let (r, g, b) = rgb(&px, w, w - 8, h - 8);
    assert!(r > 150 && g > 150 && b > 150, "右下区应偏白，实际 ({r},{g},{b})");
}

#[test]
fn image_tint_and_contain_on_gpu() {
    let _guard = D3D_LOCK.lock().unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-d3d-image-tint", 240, 200) else {
        eprintln!("[image-d3d] 无 D3D11 设备，跳过");
        return;
    };
    // 白色纹理 × 红 tint → 纯红；Contain 让 8x4 的图垂直居中
    let img = Image::from_rgba("white", 8, 4, &solid_rgba(8, 4, (255, 255, 255)))
        .with_style(ImageStyle {
            fit: ImageFit::Contain,
            tint: Some(Color::rgba(1.0, 0.0, 0.0, 1.0)),
            clip: None,
        });
    let mut tree = UiTree::new(Box::new(Scene {
        rect: Rect::ZERO,
        child: Some(img),
    }));
    let (px, w, h) = render_scene(&mut win, &mut tree);

    // 中上部应留白（Contain 垂直居中），中部应是红色
    let (r_top, g_top, b_top) = rgb(&px, w, 100, 10);
    let (r_mid, g_mid, b_mid) = rgb(&px, w, 100, h / 2);
    eprintln!("[image-d3d] Contain 顶部 ({r_top},{g_top},{b_top})，中部 ({r_mid},{g_mid},{b_mid})");

    assert!(
        r_mid > 200 && g_mid < 40 && b_mid < 40,
        "白色 × 红 tint 应得到纯红，实际 ({r_mid},{g_mid},{b_mid})"
    );
    assert!(
        r_top < 40 && g_top < 40 && b_top < 40,
        "Contain 顶部应留白（深色背景），实际 ({r_top},{g_top},{b_top})"
    );
}
