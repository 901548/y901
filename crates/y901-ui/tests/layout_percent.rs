//! **百分比尺寸在 measure 与 arrange 之间必须一致**。
//!
//! 曾经的实现两处各算一次：
//! - `measure` 按"父级给的可用空间"（`constraints.max`）解析百分比；
//! - `arrange` 按容器**实际 rect**（`inner`）重算。
//!
//! 只要容器自己的尺寸是内容驱动的（例如 `Align::Center` 下不拉伸），两者就会不同：
//! 量出来 100 高的容器，摆进去的子项只有 50 高 —— 列里留下 50px 空洞。
//!
//! 现在 assemble 直接用 measure 解出的尺寸，两个阶段不可能再分歧。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Align, Constraints, Dimension, EventCtx, EventResult, Flex, FlexItem, Rect, RenderCtx,
    Semantics, Size, TextMetrics, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 内容固定为 `size` 的叶子；`item` 决定它的 `FlexItem`。
struct Leaf {
    id: &'static str,
    size: Size,
    item: FlexItem,
    rect: Rect,
    measured: Rc<Cell<usize>>,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn flex(&self) -> FlexItem {
        self.item
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        self.measured.set(self.measured.get() + 1);
        c.constrain(self.size)
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn leaf(id: &'static str, w: f32, h: f32, item: FlexItem) -> Box<dyn Widget> {
    Box::new(Leaf {
        id,
        size: Size::new(w, h),
        item,
        rect: Rect::ZERO,
        measured: Rc::new(Cell::new(0)),
    })
}

/// 复现审查里的场景：横向 flex + `Align::Center` 的自动高度列 + 50% 高度的子项。
///
/// 断言："子项的高度不得超过它所在的列"——这是"百分比解析一致"的直接推论：
/// 百分比是相对**列自己**的高度，子项不可能比列还高。
#[test]
fn percent_child_never_exceeds_its_auto_sized_container() {
    let inner = Flex::column("c").push(leaf(
        "g",
        10.0,
        4.0,
        FlexItem {
            height: Dimension::Percent(50.0),
            ..FlexItem::NO_LIMITS
        },
    ));
    let root = Flex::row("root").align(Align::Center).push(Box::new(inner));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(1000.0, 200.0), &Mono);

    let c = tree.rect_of("c").expect("列应有 rect");
    let g = tree.rect_of("g").expect("子项应有 rect");
    assert!(
        g.h <= c.h + 0.01,
        "子项高度 {} 超过了容器高度 {}：measure 与 arrange 对百分比解析不一致",
        g.h,
        c.h
    );
    assert!(
        (g.h - c.h).abs() < 0.01,
        "唯一的子项应当填满容器（容器高度由它自己撑起来）：g.h={} c.h={}",
        g.h,
        c.h
    );
}

/// 有界且被拉伸时，百分比仍然按父级空间解析（保持既有语义）。
#[test]
fn percent_child_fills_a_stretched_container() {
    let root = Flex::column("root").push(leaf(
        "g",
        10.0,
        4.0,
        FlexItem {
            height: Dimension::Percent(50.0),
            ..FlexItem::NO_LIMITS
        },
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(200.0, 200.0), &Mono);

    let g = tree.rect_of("g").expect("应有 rect");
    assert!(
        (g.h - 100.0).abs() < 0.01,
        "竖向 flex 在 200 高的有界容器里，50% 应为 100，实际 {}",
        g.h
    );
}
