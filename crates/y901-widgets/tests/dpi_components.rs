//! 组件**内部** padding 必须与 `Flex` 同口径（随 DPI 缩放）。
//!
//! 背景（审查 P1-6）：`Flex::padding` 自 v0.2.0 起是 DIP（× `metrics.scale()`），
//! 而 `TextInput` 内部 padding 当时还按固定像素用 —— "一半缩放一半不缩放"的问题
//! 从容器搬进了控件：125% 下输入框内边距会比周围留白更紧。
//!
//! 这条用例只量 `measure` 出的尺寸并跑一次 `render` 冒烟，不需要 GPU。

use y901_ui::layout::LayoutCache;
use y901_ui::{Color, Constraints, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle, Widget};
use y901_widgets::TextInput;

/// 可配 `scale` 的假度量：行高与字宽都随 scale 放大（与真实后端同口径）。
struct Metrics {
    scale: f32,
}

impl TextMetrics for Metrics {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        Size::new(
            text.chars().count() as f32 * 8.0 * self.scale,
            16.0 * self.scale,
        )
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        16.0 * self.scale
    }
    fn scale(&self) -> f32 {
        self.scale
    }
}

/// 最小渲染上下文：只用于"不 panic"冒烟。
struct Ctx;

impl TextMetrics for Ctx {
    fn measure_text(&self, text: &str, style: &TextStyle) -> Size {
        Metrics { scale: 1.25 }.measure_text(text, style)
    }
    fn line_height(&self, style: &TextStyle) -> f32 {
        Metrics { scale: 1.25 }.line_height(style)
    }
    fn scale(&self) -> f32 {
        1.25
    }
}

impl RenderCtx for Ctx {
    fn size(&self) -> Size {
        Size::new(400.0, 200.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

fn input() -> TextInput {
    // 默认内部 padding = symmetric(8, 4)：竖向共 8，横向共 16（DIP）。
    TextInput::new("t").with_text("abc")
}

fn measured(scale: f32) -> (Size, TextInput) {
    let m = Metrics { scale };
    let mut w = input();
    let mut cache = LayoutCache::default();
    let size = w.measure(&m, Constraints::loose(Size::new(400.0, 200.0)), &mut cache);
    (size, w)
}

#[test]
fn text_input_padding_follows_dpi() {
    // 100%：行高 16 + padding_v 8 = 24
    assert_eq!(measured(1.0).0.h, 24.0);
    // 125%：行高 20 + padding_v 10 = 30（padding 若没缩放会是 28）
    assert_eq!(
        measured(1.25).0.h,
        30.0,
        "TextInput 内部 padding 未随 DPI 缩放（P1-6 回归）：期望 30 = 20 + 8×1.25"
    );
}

#[test]
fn text_input_render_does_not_panic_at_125() {
    let m = Metrics { scale: 1.25 };
    let (size, mut w) = measured(1.25);
    let mut cache = LayoutCache::default();
    w.arrange(
        Rect::new(0.0, 0.0, size.w.max(120.0), size.h),
        &m,
        &mut cache,
    );
    let mut ctx = Ctx;
    w.render(&mut ctx);
}
