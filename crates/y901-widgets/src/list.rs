//! 官方容器：纵向列表。
//!
//! 这是"如何写一个容器"的样本：自己决定子节点怎么摆，
//! 框架只负责把矩形传进来、把事件转过去。

use y901_ui::{
    Axis, Constraints, Edges, EventCtx, EventResult, FlexItem, Rect, RenderCtx, Role, Semantics,
    Size, TextMetrics, UiEvent, Widget,
};
use y901_ui::layout::{flex_arrange, flex_measure};

pub struct VerticalList {
    id: String,
    children: Vec<Box<dyn Widget>>,
    padding: Edges,
    gap: f32,
    item: FlexItem,
    rect: Rect,
}

impl VerticalList {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            children: Vec::new(),
            padding: Edges::ZERO,
            gap: 4.0,
            item: FlexItem::default(),
            rect: Rect::ZERO,
        }
    }

    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    pub fn push(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }
}

impl Widget for VerticalList {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        c: Constraints,
        cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        flex_measure(
            Axis::Vertical,
            self.padding,
            self.gap,
            &mut self.children,
            metrics,
            c,
            cache,
        )
    }

    fn arrange(
        &mut self,
        rect: Rect,
        metrics: &dyn TextMetrics,
        cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        flex_arrange(
            Axis::Vertical,
            self.padding,
            self.gap,
            &mut self.children,
            rect,
            metrics,
            cache,
        );
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn padding(&self) -> Edges {
        self.padding
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        // 容器自己不处理事件；框架会把事件投递给命中的子节点。
        EventResult::Ignored
    }

    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .label(format!("list({})", self.children.len()))
            .prop("count", self.children.len().to_string())
    }
}
