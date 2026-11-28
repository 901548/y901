//! 官方容器：纵向列表。
//!
//! 这是"如何写一个容器"的样本：自己决定子节点怎么摆，
//! 框架只负责把矩形传进来、把事件转过去。

use y901_ui::layout::{flex_arrange, flex_measure};
use y901_ui::{
    Axis, Constraints, Edges, EventCtx, EventResult, FlexItem, Rect, RenderCtx, Role, Semantics,
    Size, TextMetrics, UiEvent, Widget,
};

fn scaled(edges: Edges, scale: f32) -> Edges {
    Edges {
        left: edges.left * scale,
        top: edges.top * scale,
        right: edges.right * scale,
        bottom: edges.bottom * scale,
    }
}

/// 垂直列表容器：等宽、依次排列、支持 gap 与 padding。
pub struct VerticalList {
    id: String,
    /// 无障碍名称（读屏器念它）。空 = 不提供名称（**不再拿 id 顶替**）。
    label: String,
    children: Vec<Box<dyn Widget>>,
    /// Padding base values in DIP, unless `fixed_padding` is set.
    padding: Edges,
    fixed_padding: Option<Edges>,
    /// Gap base value in DIP, unless `fixed_gap` is set.
    gap: f32,
    fixed_gap: Option<f32>,
    item: FlexItem,
    rect: Rect,
}

impl VerticalList {
    /// 新建纵向列表容器（**非虚拟化**：所有子节点都会被排布）。
    ///
    /// 输入：`id`（树内唯一）。返回：空列表（`gap = 4`、无内边距）。
    /// 副作用：无。行数很多时请改用 `ListView`（虚拟化）。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            children: Vec::new(),
            padding: Edges::ZERO,
            fixed_padding: None,
            gap: 4.0,
            fixed_gap: None,
            item: FlexItem::default(),
            rect: Rect::ZERO,
        }
    }

    /// 无障碍名称（读屏器念的名字；空 = 不提供）。
    ///
    /// 语义里曾经把内部 id（和行数）拼进名字，读屏器会把这些实现细节念出来。
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// 设置内边距（DIP，随 DPI 缩放；子节点整体内缩）。
    ///
    /// 与 `Flex::padding()` 口径一致。需要固定物理像素时使用
    /// [`VerticalList::padding_fixed`]。后设置者优先。
    /// 返回：自身。副作用：无（下次布局生效）。
    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self.fixed_padding = None;
        self
    }

    /// 设置不随 DPI 缩放的物理像素内边距。
    ///
    /// 与 `Flex::padding_fixed()` 口径一致。普通留白应使用 [`VerticalList::padding`]。
    /// 后设置者优先。
    pub fn padding_fixed(mut self, padding: Edges) -> Self {
        self.fixed_padding = Some(padding);
        self.padding = Edges::ZERO;
        self
    }

    /// 设置行间距（DIP，随 DPI 缩放）。
    ///
    /// 与 `Flex::gap()` 口径一致。需要固定物理像素时使用 [`VerticalList::gap_fixed`]。
    /// 后设置者优先。间距可为负，但通常无意义。
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self.fixed_gap = None;
        self
    }

    /// 设置不随 DPI 缩放的物理像素行间距。
    ///
    /// 与 `Flex::gap_fixed()` 口径一致。后设置者优先。
    pub fn gap_fixed(mut self, gap: f32) -> Self {
        self.fixed_gap = Some(gap);
        self.gap = 0.0;
        self
    }

    /// 设置 flex 伸缩参数。
    ///
    /// 返回：自身。副作用：无。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 追加一行（任意组件，不限于文本）。
    ///
    /// 返回：自身。副作用：无（挂载前调用最安全；挂载后追加需自行触发重排）。
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
        let scale = metrics.scale();
        let padding = self
            .fixed_padding
            .unwrap_or_else(|| scaled(self.padding, scale));
        let gap = self.fixed_gap.unwrap_or(self.gap * scale);
        flex_measure(
            Axis::Vertical,
            padding,
            gap,
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
        let scale = metrics.scale();
        let padding = self
            .fixed_padding
            .unwrap_or_else(|| scaled(self.padding, scale));
        let gap = self.fixed_gap.unwrap_or(self.gap * scale);
        flex_arrange(
            Axis::Vertical,
            padding,
            gap,
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
            // **必须显式暴露**：`Semantics::default()` 是 `exposed_to_ai: false`，
            // 于是整棵列表（连同它的项）此前**完全进不了无障碍树** ——
            // 读屏器看不到列表、也念不出项数。
            .exposed(true)
            // 名字里不念 id/数量；数量留在 prop("count")。
            .label(self.label.clone())
            .prop("count", self.children.len().to_string())
    }
}
