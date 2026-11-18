//! DPI 策略回归：`Flex` 的 `padding` / `gap` 必须**随 DPI 缩放**，
//! 而 `padding_fixed` / `gap_fixed` 必须**不缩放**。
//!
//! 背景（`docs/design/dpi-policy.md`）：框架曾经一半缩放（控件度量 × `self.scale`）、
//! 一半不缩放（容器留白固定），三者只在 100% 下自洽 —— 125% 下行距变大而留白不变，
//! 内容被挤在一起，还导致过"字形溢出固定高度的行、脏区外溢"的真 bug。
//!
//! 这里用**纯布局**验证，不需要 GPU/窗口。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, Edges, EventCtx, EventResult, Flex, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, Widget,
};

/// 可调 `scale` 的假度量：`scale` 就是 DPI 缩放比。
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

/// 固定尺寸的叶子：布局结果只由 Flex 的 padding/gap 决定。
struct Fixed {
    id: &'static str,
    size: Size,
    rect: Rect,
}

impl Fixed {
    fn new(id: &'static str, w: f32, h: f32) -> Self {
        Self {
            id,
            size: Size::new(w, h),
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Fixed {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, m: &dyn TextMetrics, _c: Constraints, _cache: &mut LayoutCache) -> Size {
        // 真实控件自己也会按 DPI 放大（`* self.scale`），这里照着做，
        // 否则"容器整体 ×1.25"这条断言会因为子控件没缩放而失败。
        Size::new(self.size.w * m.scale(), self.size.h * m.scale())
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 摆一遍布局，返回所有子控件的 rect 与容器 size。
fn arrange_all(mut flex: Flex, scale: f32) -> (Vec<Rect>, Size) {
    let metrics = Metrics { scale };
    let mut cache = LayoutCache::new();
    let constraints = Constraints::loose(Size::new(400.0, 300.0));
    let size = flex.measure(&metrics, constraints, &mut cache);
    flex.arrange(Rect::new(0.0, 0.0, size.w, size.h), &metrics, &mut cache);
    let kids = flex.children_mut().expect("Flex 必须有 children_mut");
    (kids.iter().map(|k| k.rect()).collect(), size)
}

#[test]
fn padding_and_gap_scale_with_dpi() {
    let build = || {
        Flex::row("row")
            .padding(Edges::all(10.0))
            .gap(8.0)
            .push(Box::new(Fixed::new("a", 40.0, 20.0)))
            .push(Box::new(Fixed::new("b", 40.0, 20.0)))
    };

    let (r1, size1) = arrange_all(build(), 1.0);
    let (r125, size125) = arrange_all(build(), 1.25);
    let (a1, b1) = (r1[0], r1[1]);
    let (a125, b125) = (r125[0], r125[1]);

    assert_eq!((a1.x, a1.y), (10.0, 10.0), "100% 下 padding 应为 10");
    assert_eq!(b1.x, 10.0 + 40.0 + 8.0, "100% 下 gap 应为 8");

    assert_eq!((a125.x, a125.y), (12.5, 12.5), "125% 下 padding 应为 12.5");
    // 子控件自己也随 DPI 放大（40 → 50），所以 b 的起点 = padding + a.w + gap
    assert_eq!(
        b125.x,
        12.5 + a125.w + 10.0,
        "125% 下 gap 应为 10（8 × 1.25）"
    );

    assert_eq!(size1.w, 10.0 + 40.0 + 8.0 + 40.0 + 10.0);
    assert_eq!(size125.w, 1.25 * size1.w, "容器宽度应整体 ×1.25");
}

#[test]
fn fixed_padding_does_not_scale() {
    let build = || {
        Flex::row("row")
            .padding_fixed(Edges::all(10.0))
            .gap_fixed(8.0)
            .push(Box::new(Fixed::new("a", 40.0, 20.0)))
            .push(Box::new(Fixed::new("b", 40.0, 20.0)))
    };

    let (r1, _) = arrange_all(build(), 1.0);
    let (r125, _) = arrange_all(build(), 1.25);
    let (a1, b1) = (r1[0], r1[1]);
    let (a125, b125) = (r125[0], r125[1]);

    assert_eq!((a1.x, a1.y), (10.0, 10.0));
    assert_eq!(
        (a125.x, a125.y),
        (10.0, 10.0),
        "padding_fixed 在 125% 下也不许变"
    );
    // 子控件 40 → 50（自己缩放），间距保持 8 —— 所以差值 = a.w + 8
    assert_eq!(b125.x - a125.x, a125.w + 8.0, "gap_fixed 不缩放（仍是 8）");
    assert_eq!(b125.x - a125.x - a125.w, b1.x - a1.x - a1.w, "差值同为 8");
}

#[test]
fn last_call_wins_between_padding_and_fixed() {
    let flex = Flex::row("row")
        .padding_fixed(Edges::all(10.0))
        .padding(Edges::all(10.0))
        .push(Box::new(Fixed::new("a", 40.0, 20.0)));
    let (r, _) = arrange_all(flex, 1.25);
    let a = r[0];
    assert_eq!(a.x, 12.5, "后设置的 padding() 应生效（缩放）");

    let flex = Flex::row("row")
        .padding(Edges::all(10.0))
        .padding_fixed(Edges::all(10.0))
        .push(Box::new(Fixed::new("a", 40.0, 20.0)));
    let (r, _) = arrange_all(flex, 1.25);
    let a = r[0];
    assert_eq!(a.x, 10.0, "后设置的 padding_fixed() 应生效（不缩放）");
}
