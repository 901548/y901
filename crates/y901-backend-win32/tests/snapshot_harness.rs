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
