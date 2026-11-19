//! VerticalList padding and gap use the same DPI units as Flex.

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, Edges, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, Widget,
};
use y901_widgets::VerticalList;

struct Metrics(f32);

impl TextMetrics for Metrics {
    fn measure_text(&self, _text: &str, _style: &TextStyle) -> Size {
        Size::ZERO
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        0.0
    }
    fn scale(&self) -> f32 {
        self.0
    }
}

struct Leaf {
    id: &'static str,
    rect: Rect,
}

impl Leaf {
    fn new(id: &'static str) -> Self {
        Self {
            id,
            rect: Rect::ZERO,
        }
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, _c: Constraints, _cache: &mut LayoutCache) -> Size {
        Size::new(20.0, 10.0)
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn layout(list: &mut VerticalList, scale: f32) -> (Size, Vec<Rect>) {
    let metrics = Metrics(scale);
    let mut cache = LayoutCache::default();
    let size = list.measure(
        &metrics,
        Constraints::loose(Size::new(300.0, 300.0)),
        &mut cache,
    );
    list.arrange(Rect::new(0.0, 0.0, size.w, size.h), &metrics, &mut cache);
    let rects = list.children().iter().map(|child| child.rect()).collect();
    (size, rects)
}

fn list() -> VerticalList {
    VerticalList::new("list")
        .padding(Edges::all(8.0))
        .gap(4.0)
        .push(Box::new(Leaf::new("a")))
        .push(Box::new(Leaf::new("b")))
}

#[test]
fn padding_and_gap_follow_dpi_like_flex() {
    let mut at_100 = list();
    let (size_100, rects_100) = layout(&mut at_100, 1.0);
    let mut at_125 = list();
    let (size_125, rects_125) = layout(&mut at_125, 1.25);

    assert_eq!(rects_100[0].y, 8.0);
    assert_eq!(rects_100[1].y - rects_100[0].bottom(), 4.0);
    assert_eq!(rects_125[0].y, 10.0, "padding DIP scales: 8 × 1.25");
    assert_eq!(
        rects_125[1].y - rects_125[0].bottom(),
        5.0,
        "gap DIP scales: 4 × 1.25"
    );
    assert_eq!(size_100.h, 8.0 + 10.0 + 4.0 + 10.0 + 8.0);
    assert_eq!(size_125.h, 10.0 + 10.0 + 5.0 + 10.0 + 10.0);
}

#[test]
fn fixed_padding_and_gap_do_not_scale() {
    let build = || {
        VerticalList::new("list")
            .padding_fixed(Edges::all(8.0))
            .gap_fixed(4.0)
            .push(Box::new(Leaf::new("a")))
            .push(Box::new(Leaf::new("b")))
    };
    let mut at_100 = build();
    let (_, rects_100) = layout(&mut at_100, 1.0);
    let mut at_125 = build();
    let (_, rects_125) = layout(&mut at_125, 1.25);

    assert_eq!(rects_100[0].y, 8.0);
    assert_eq!(rects_125[0].y, 8.0);
    assert_eq!(rects_100[1].y - rects_100[0].bottom(), 4.0);
    assert_eq!(rects_125[1].y - rects_125[0].bottom(), 4.0);
}

#[test]
fn padding_and_gap_fixed_setters_are_last_call_wins() {
    let mut padding_scaled_last = VerticalList::new("p")
        .padding_fixed(Edges::all(6.0))
        .padding(Edges::all(8.0))
        .push(Box::new(Leaf::new("a")));
    let (_, rects) = layout(&mut padding_scaled_last, 1.25);
    assert_eq!(rects[0].y, 10.0);

    let mut padding_fixed_last = VerticalList::new("p")
        .padding(Edges::all(8.0))
        .padding_fixed(Edges::all(6.0))
        .push(Box::new(Leaf::new("a")));
    let (_, rects) = layout(&mut padding_fixed_last, 1.25);
    assert_eq!(rects[0].y, 6.0);

    let mut gap_scaled_last = VerticalList::new("g")
        .gap_fixed(6.0)
        .gap(8.0)
        .push(Box::new(Leaf::new("a")))
        .push(Box::new(Leaf::new("b")));
    let (_, rects) = layout(&mut gap_scaled_last, 1.25);
    assert_eq!(rects[1].y - rects[0].bottom(), 10.0);

    let mut gap_fixed_last = VerticalList::new("g")
        .gap(8.0)
        .gap_fixed(6.0)
        .push(Box::new(Leaf::new("a")))
        .push(Box::new(Leaf::new("b")));
    let (_, rects) = layout(&mut gap_fixed_last, 1.25);
    assert_eq!(rects[1].y - rects[0].bottom(), 6.0);
}
