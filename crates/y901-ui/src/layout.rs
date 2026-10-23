//! 手写 Flex 布局 + 布局缓存。
//!
//! 三个核心保证：
//!
//! 1. **每个节点每次布局最多 measure 一次**。容器必须通过 [`LayoutCache::measure`]
//!    访问子节点，框架在那里去重。
//!
//! 2. **相同约束下重复布局为 O(1)**。`measure` 结果按
//!    `(id, constraints, layout_version)` 记忆；约束与版本都没变时整棵子树不再进入。
//!
//! 3. **无界约束不会产生 inf / NaN**。[`crate::Constraints::unbounded_main`]
//!    只放宽上限，返回的内容尺寸始终有限；滚动容器的子节点排在**内容坐标系**里，
//!    平移由渲染/命中阶段处理。
//!
//! 刻意不做：wrap / align-items / absolute / aspect-ratio。

use std::collections::{HashMap, HashSet};

use crate::color::Color;
use crate::event::{EventResult, UiEvent};
use crate::geometry::{Edges, Rect, Size};
use crate::widget::{
    Axis, Constraints, Dimension, EventCtx, FlexItem, RenderCtx, Role, Semantics, TextMetrics,
    Widget,
};

// ============================ 布局缓存 ============================

/// 一次布局的统计。测试与线上诊断都用它。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    /// 真正进入 `Widget::measure` 的次数（缓存命中不算）。
    pub measured: u32,
    /// 被 arrange 的节点数。
    pub arranged: u32,
    /// 整次布局是否被跳过（约束与版本都没变）。
    pub skipped: bool,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    constraints: Constraints,
    version: u64,
    size: Size,
}

/// 布局缓存：记忆化 + 脏传播。
pub struct LayoutCache {
    entries: HashMap<String, Entry>,
    dirty: HashSet<String>,
    parents: HashMap<String, String>,
    /// 当前正在测量的祖先链（用于填写 `parents`）。
    stack: Vec<String>,
    stats: LayoutStats,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            dirty: HashSet::new(),
            parents: HashMap::new(),
            stack: Vec::new(),
            stats: LayoutStats::default(),
        }
    }

    pub fn stats(&self) -> LayoutStats {
        self.stats
    }

    /// 标记某节点的尺寸可能变化：它自己和所有祖先都要重测。
    pub fn mark_dirty(&mut self, id: &str) {
        let mut cursor = Some(id.to_string());
        let mut guard = 0;
        while let Some(current) = cursor {
            if !self.dirty.insert(current.clone()) {
                break; // 已在脏集合里，祖先必然也已标记
            }
            cursor = self.parents.get(&current).cloned();
            guard += 1;
            if guard > 4096 {
                break; // 防御错误数据形成环
            }
        }
    }

    /// 整棵树失效（主题变更、字体变化等）。
    pub fn mark_all_dirty(&mut self) {
        self.entries.clear();
        self.dirty.clear();
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.dirty.clear();
        self.parents.clear();
    }

    /// 某节点上次测得的尺寸（不管约束）。arrange 阶段复用。
    pub fn last_size(&self, id: &str) -> Option<Size> {
        self.entries.get(id).map(|e| e.size)
    }

    pub fn is_dirty(&self, id: &str) -> bool {
        self.dirty.contains(id)
    }

    fn is_fresh(&self, id: &str, constraints: Constraints, version: u64) -> bool {
        if self.dirty.contains(id) {
            return false;
        }
        match self.entries.get(id) {
            Some(entry) => entry.version == version && entry.constraints == constraints,
            None => false,
        }
    }

    /// **测量入口**：所有容器都必须用它访问子节点。
    ///
    /// 命中缓存时直接返回，不进入 `Widget::measure`，整棵子树被切断。
    pub fn measure(
        &mut self,
        widget: &mut dyn Widget,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
    ) -> Size {
        let id = widget.id().to_string();
        let version = widget.layout_version();

        if self.is_fresh(&id, constraints, version) {
            return self.entries.get(&id).map(|e| e.size).unwrap_or(Size::ZERO);
        }

        if let Some(parent) = self.stack.last().cloned() {
            self.parents.insert(id.clone(), parent);
        }

        self.stats.measured += 1;
        self.stack.push(id.clone());
        let size = widget.measure(metrics, constraints, self);
        self.stack.pop();

        self.entries.insert(
            id.clone(),
            Entry {
                constraints,
                version,
                size,
            },
        );
        self.dirty.remove(&id);
        size
    }

    /// arrange 阶段取尺寸：优先复用上次测量结果，取不到才补测。
    pub fn size_for_arrange(
        &mut self,
        widget: &mut dyn Widget,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
    ) -> Size {
        if let Some(size) = self.last_size(widget.id()) {
            return size;
        }
        self.measure(widget, metrics, constraints)
    }

    fn begin_layout(&mut self) {
        self.stats = LayoutStats::default();
    }

    fn finish_layout(&mut self) {
        // 本次没被访问到的脏节点不会影响后续布局，清掉。
        self.dirty.clear();
    }
}

// ============================ Flex 算法 ============================

/// 容器测量：返回内容 + padding 后的期望尺寸。
pub fn flex_measure(
    axis: Axis,
    padding: Edges,
    gap: f32,
    children: &mut [Box<dyn Widget>],
    metrics: &dyn TextMetrics,
    constraints: Constraints,
    cache: &mut LayoutCache,
) -> Size {
    // 主轴无界时，子节点在主轴方向不设上限（ScrollView 场景）
    let inner_max = Size::new(
        if constraints.is_unbounded(Axis::Horizontal) {
            f32::INFINITY
        } else {
            (constraints.max.w - padding.horizontal()).max(0.0)
        },
        if constraints.is_unbounded(Axis::Vertical) {
            f32::INFINITY
        } else {
            (constraints.max.h - padding.vertical()).max(0.0)
        },
    );

    let mut main_total = 0.0f32;
    let mut cross_max = 0.0f32;
    let mut visible = 0usize;

    for child in children.iter_mut() {
        let item = child.flex();
        let size = cache.measure(child.as_mut(), metrics, Constraints::loose(inner_max));

        let (main_dim, cross_dim) = match axis {
            Axis::Horizontal => (item.width, item.height),
            Axis::Vertical => (item.height, item.width),
        };
        let (main_content, cross_content) = match axis {
            Axis::Horizontal => (size.w, size.h),
            Axis::Vertical => (size.h, size.w),
        };

        let main = match main_dim {
            Dimension::Px(v) => v,
            Dimension::Percent(p) => {
                let base = match axis {
                    Axis::Horizontal => inner_max.w,
                    Axis::Vertical => inner_max.h,
                };
                if base.is_finite() {
                    base * (p / 100.0)
                } else {
                    main_content // 无界主轴不能算百分比
                }
            }
            Dimension::Flex(_) | Dimension::Auto => main_content,
        };
        let cross = match cross_dim {
            Dimension::Px(v) => v,
            Dimension::Percent(p) => {
                let base = match axis {
                    Axis::Horizontal => inner_max.h,
                    Axis::Vertical => inner_max.w,
                };
                if base.is_finite() {
                    base * (p / 100.0)
                } else {
                    cross_content
                }
            }
            _ => cross_content,
        };

        main_total += main;
        cross_max = cross_max.max(cross);
        visible += 1;
    }

    if visible > 1 {
        main_total += gap * (visible - 1) as f32;
    }

    let (w, h) = match axis {
        Axis::Horizontal => (main_total + padding.horizontal(), cross_max + padding.vertical()),
        Axis::Vertical => (cross_max + padding.horizontal(), main_total + padding.vertical()),
    };

    // 关键：返回的永远是有限的内容尺寸
    constraints.constrain(Size::new(w, h))
}

/// 容器排布：把 `rect` 内的空间分给子组件并逐个 `arrange`。
///
/// 尺寸全部来自缓存，因此 arrange 阶段**不会**触发 `Widget::measure`。
pub fn flex_arrange(
    axis: Axis,
    padding: Edges,
    gap: f32,
    children: &mut [Box<dyn Widget>],
    rect: Rect,
    metrics: &dyn TextMetrics,
    cache: &mut LayoutCache,
) {
    if children.is_empty() {
        return;
    }
    let inner = rect.inset(padding);
    let (main_avail, cross_avail) = match axis {
        Axis::Horizontal => (inner.w, inner.h),
        Axis::Vertical => (inner.h, inner.w),
    };

    // 第一遍：固定尺寸 + flex 因子（全部走缓存）
    let mut fixed_total = 0.0f32;
    let mut flex_total = 0.0f32;
    let mut sizes: Vec<Size> = Vec::with_capacity(children.len());

    for child in children.iter_mut() {
        let item = child.flex();
        let main_dim = match axis {
            Axis::Horizontal => item.width,
            Axis::Vertical => item.height,
        };
        let size =
            cache.size_for_arrange(child.as_mut(), metrics, Constraints::loose(inner.size()));
        sizes.push(size);

        if let Some(factor) = main_dim.flex_factor() {
            flex_total += factor;
        } else {
            fixed_total += match (axis, main_dim) {
                (Axis::Horizontal, Dimension::Px(v)) => v,
                (Axis::Vertical, Dimension::Px(v)) => v,
                (Axis::Horizontal, Dimension::Percent(p)) if inner.w.is_finite() => {
                    inner.w * (p / 100.0)
                }
                (Axis::Vertical, Dimension::Percent(p)) if inner.h.is_finite() => {
                    inner.h * (p / 100.0)
                }
                (Axis::Horizontal, _) => size.w,
                (Axis::Vertical, _) => size.h,
            };
        }
    }

    let gaps = if children.len() > 1 {
        gap * (children.len() - 1) as f32
    } else {
        0.0
    };
    let leftover = (main_avail - fixed_total - gaps).max(0.0);

    // 第二遍：分配并排布
    let mut cursor = 0.0f32;
    for (idx, child) in children.iter_mut().enumerate() {
        let item = child.flex();
        let size = sizes[idx];
        let (main_dim, cross_dim) = match axis {
            Axis::Horizontal => (item.width, item.height),
            Axis::Vertical => (item.height, item.width),
        };

        let main = match main_dim.flex_factor() {
            Some(factor) if flex_total > 0.0 => leftover * factor / flex_total,
            _ => match main_dim {
                Dimension::Px(v) => v,
                Dimension::Percent(p) if inner.w.is_finite() && inner.h.is_finite() => match axis {
                    Axis::Horizontal => inner.w * (p / 100.0),
                    Axis::Vertical => inner.h * (p / 100.0),
                },
                _ => match axis {
                    Axis::Horizontal => size.w,
                    Axis::Vertical => size.h,
                },
            },
        };

        let cross = match cross_dim {
            Dimension::Px(v) => v,
            Dimension::Percent(p) if cross_avail.is_finite() => cross_avail * (p / 100.0),
            _ => cross_avail,
        };

        let cross = if cross.is_finite() { cross } else { 0.0 };
        let child_rect = match axis {
            Axis::Horizontal => Rect::new(inner.x + cursor, inner.y, main, cross.min(cross_avail)),
            Axis::Vertical => Rect::new(inner.x, inner.y + cursor, cross.min(cross_avail), main),
        };

        cache.stats.arranged += 1;
        child.arrange(child_rect, metrics, cache);
        cursor += main + gap;
    }
}

// ============================ 内容坐标系 ============================

/// 把内容矩形放到视口原点，再按 `offset` 平移。
///
/// 滚动容器用这个把"内容坐标"映射到"屏幕坐标"：子节点永远在
/// `[0, content_height]` 里排布，屏幕上再减去偏移，因此**不会出现
/// 1e6 像素的浮点坐标**（那是 NaN 的温床）。
pub fn content_rect(viewport: Rect, content: Size, offset: f32) -> Rect {
    let height = if content.h.is_finite() {
        content.h
    } else {
        viewport.h
    };
    let offset = if offset.is_finite() { offset } else { 0.0 };
    Rect::new(viewport.x, viewport.y - offset, viewport.w, height)
}

// ============================ Flex 容器 ============================

pub struct Flex {
    id: String,
    axis: Axis,
    padding: Edges,
    gap: f32,
    background: Option<Color>,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    item: FlexItem,
}

impl Flex {
    pub fn new(id: impl Into<String>, axis: Axis) -> Self {
        Self {
            id: id.into(),
            axis,
            padding: Edges::ZERO,
            gap: 0.0,
            background: None,
            children: Vec::new(),
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    pub fn row(id: impl Into<String>) -> Self {
        Self::new(id, Axis::Horizontal)
    }

    pub fn column(id: impl Into<String>) -> Self {
        Self::new(id, Axis::Vertical)
    }

    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
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

    pub fn push_boxed(&mut self, child: Box<dyn Widget>) {
        self.children.push(child);
    }
}

impl Widget for Flex {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
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
        constraints: Constraints,
        cache: &mut LayoutCache,
    ) -> Size {
        flex_measure(
            self.axis,
            self.padding,
            self.gap,
            &mut self.children,
            metrics,
            constraints,
            cache,
        )
    }

    fn arrange(&mut self, rect: Rect, metrics: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        flex_arrange(
            self.axis,
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
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if let Some(bg) = self.background {
            ctx.fill_rect(self.rect, bg);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

// ============================ 布局入口 ============================

/// 布局入口。
///
/// 根节点约束与版本都没变、且无脏标记时，整次布局跳过（O(1)）。
pub fn layout_tree(
    root: &mut dyn Widget,
    viewport: Size,
    metrics: &dyn TextMetrics,
    cache: &mut LayoutCache,
) -> LayoutStats {
    cache.begin_layout();

    let tight = Constraints::tight(viewport);
    if cache.is_fresh(root.id(), tight, root.layout_version()) {
        cache.stats.skipped = true;
        return cache.stats;
    }

    let _ = cache.measure(root, metrics, tight);
    cache.stats.arranged += 1;
    root.arrange(Rect::from_size(viewport), metrics, cache);

    cache.finish_layout();
    cache.stats
}
/// 叠放容器：所有子节点拿到**同一个矩形**。
///
/// 浮层（Menu/Dialog）需要一个"全屏宿主"，否则它们的 rect 会被父容器的
/// flex 槽位限制住，既盖不住全屏，也无法用几何判断"点在外面"。
pub struct Stack {
    id: String,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    item: FlexItem,
}

impl Stack {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            children: Vec::new(),
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    pub fn push(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }

    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }
}

impl Widget for Stack {
    fn id(&self) -> &str {
        &self.id
    }
    fn role(&self) -> Role {
        Role::PANEL
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
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        for child in self.children.iter_mut() {
            let _ = cache.measure(child.as_mut(), m, c);
        }
        c.constrain(c.max)
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        for child in self.children.iter_mut() {
            child.arrange(rect, m, cache);
        }
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