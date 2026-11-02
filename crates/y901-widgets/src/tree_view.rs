//! TreeView：层级列表 + 虚拟化。
//!
//! ## 语义约定（先写清楚，避免"文档与实现不一致"）
//!
//! - **只渲染可见行**：`flatten()` 产出"展开且可见"的行 `(路径, 深度)`，渲染只走
//!   `visible_range()` 覆盖的那一段，1000 行也只画视口内的几行。
//! - **选中用路径**（`Vec<usize>`，从根开始的子索引），不是扁平索引——折叠父节点后
//!   扁平索引会整体错位，路径不会。
//! - **折叠父节点时**：若选中项在折叠子树里，选中**保留**（路径仍有效），但该行不可见；
//!   本版不自动跳到父节点，避免"我明明选中了却被改掉"的意外。
//! - **键盘**：Up/Down 在可见行间移动；Right 展开当前项或进入首个子项；Left 折叠当前项
//!   或回到父项。到头/到尾不循环（树里循环会让人失去方向感）。

use y901_ui::{
    Constraints, EventCtx, EventResult, Key, Modifiers, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextStyle, TextMetrics, UiEvent, Widget,
};

/// 每层缩进。
pub const INDENT: f32 = 16.0;
/// 行高。
pub const ROW_H: f32 = 24.0;
/// 展开图标所占宽度。
const CHEVRON: f32 = 16.0;

/// 树节点。
#[derive(Clone, Debug)]
pub struct TreeItem {
    /// 显示文本
    pub label: String,
    /// 子节点
    pub children: Vec<TreeItem>,
    /// 是否展开
    pub expanded: bool,
}

impl TreeItem {
    /// 叶子节点（无子项）。
    pub fn leaf(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
            expanded: false,
        }
    }

    /// 可展开节点（默认展开）。
    pub fn branch(label: impl Into<String>, children: Vec<TreeItem>) -> Self {
        Self {
            label: label.into(),
            children,
            expanded: true,
        }
    }
}

/// 层级列表控件。
pub struct TreeView {
    id: String,
    roots: Vec<TreeItem>,
    /// 选中路径
    selected: Option<Vec<usize>>,
    /// 垂直滚动偏移（像素）
    scroll_offset: f32,
    /// 行高
    row_h: f32,
    rect: Rect,
    /// 扁平化结果：`(路径, 深度)`
    visible: Vec<(Vec<usize>, usize)>,
}

impl TreeView {
    /// 创建树视图。
    pub fn new(id: impl Into<String>, roots: Vec<TreeItem>) -> Self {
        let mut t = Self {
            id: id.into(),
            roots,
            selected: None,
            scroll_offset: 0.0,
            row_h: ROW_H,
            rect: Rect::ZERO,
            visible: Vec::new(),
        };
        t.rebuild();
        t
    }

    /// 可见行数。
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// 可见行的路径副本（测试与诊断用）。
    pub fn visible_rows(&self) -> Vec<(Vec<usize>, usize)> {
        self.visible.clone()
    }

    /// 当前选中路径。
    pub fn selected(&self) -> Option<&Vec<usize>> {
        self.selected.as_ref()
    }

    /// 重新扁平化：只收集"所有祖先都展开"的节点。
    pub fn rebuild(&mut self) {
        self.visible.clear();
        fn walk(
            items: &[TreeItem],
            depth: usize,
            path: &mut Vec<usize>,
            out: &mut Vec<(Vec<usize>, usize)>,
        ) {
            for (i, item) in items.iter().enumerate() {
                path.push(i);
                out.push((path.clone(), depth));
                if item.expanded && !item.children.is_empty() {
                    walk(&item.children, depth + 1, path, out);
                }
                path.pop();
            }
        }
        let mut path = Vec::new();
        walk(&self.roots, 0, &mut path, &mut self.visible);
    }

    /// 视口内应绘制的行范围（虚拟化的核心）。
    pub fn visible_range(&self) -> std::ops::Range<usize> {
        if self.row_h <= 0.0 || self.visible.is_empty() {
            return 0..0;
        }
        let h = self.rect.h.max(0.0);
        let first = (self.scroll_offset / self.row_h).floor().max(0.0) as usize;
        let count = (h / self.row_h).ceil() as usize;
        let last = (first + count + 1).min(self.visible.len());
        first.min(self.visible.len())..last
    }

    /// 深度 `depth` 的行的文字起始 x（相对自身矩形）。
    ///
    /// 抽成公开函数的理由：缩进是外观契约，测试与使用方都需要能验证它，
    /// 而不应去重算 `depth * INDENT + CHEVRON` 这个公式。
    pub fn text_x(&self, depth: usize) -> f32 {
        self.rect.x + depth as f32 * INDENT + CHEVRON + 2.0
    }

    /// 第 `i` 行（扁平索引）的矩形。
    pub fn row_rect(&self, i: usize) -> Option<Rect> {
        if i >= self.visible.len() {
            return None;
        }
        Some(Rect::new(
            self.rect.x,
            self.rect.y + i as f32 * self.row_h - self.scroll_offset,
            self.rect.w.max(0.0),
            self.row_h,
        ))
    }

    /// 屏幕 y 坐标 → 扁平行索引（越界返回 None）。
    pub fn row_at(&self, pos: Pos) -> Option<usize> {
        if !self.rect.contains(pos) {
            return None;
        }
        let i = ((pos.y - self.rect.y + self.scroll_offset) / self.row_h).floor();
        if i < 0.0 {
            return None;
        }
        let i = i as usize;
        if i < self.visible.len() {
            Some(i)
        } else {
            None
        }
    }

    /// 按路径取节点。
    fn item_at(&self, path: &[usize]) -> Option<&TreeItem> {
        let mut items: &[TreeItem] = &self.roots;
        let mut node: &TreeItem = items.first()?;
        for (k, &i) in path.iter().enumerate() {
            node = items.get(i)?;
            if k + 1 < path.len() {
                items = &node.children;
            }
        }
        Some(node)
    }

    /// 按路径取节点的可变引用。
    fn item_at_mut(&mut self, path: &[usize]) -> Option<&mut TreeItem> {
        let mut items = &mut self.roots;
        let mut idx = 0usize;
        loop {
            let i = *path.get(idx)?;
            if idx + 1 == path.len() {
                return items.get_mut(i);
            }
            let item = items.get_mut(i)?;
            items = &mut item.children;
            idx += 1;
        }
    }

    /// 切换某路径的展开状态。
    pub fn toggle_expanded(&mut self, path: &[usize]) -> bool {
        let Some(item) = self.item_at_mut(path) else {
            return false;
        };
        if item.children.is_empty() {
            return false;
        }
        item.expanded = !item.expanded;
        self.rebuild();
        true
    }

    /// 选中某个路径（必须当前可见）。
    pub fn select_path(&mut self, path: &[usize]) -> bool {
        if !self.visible.iter().any(|(p, _)| p == path) {
            return false;
        }
        self.selected = Some(path.to_vec());
        true
    }

    /// 选中项在扁平列表里的下标。
    fn selected_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.visible.iter().position(|(p, _)| p == sel)
    }
}

impl Widget for TreeView {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn focusable(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let h = (self.visible.len() as f32 * self.row_h).min(c.max.h);
        c.constrain(Size::new(c.max.w, h))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        match event {
            UiEvent::MouseDown { pos, .. } => {
                let Some(i) = self.row_at(*pos) else {
                    return EventResult::Ignored;
                };
                let (path, depth) = self.visible[i].clone();
                ctx.request_focus(self.id.clone());
                // 点展开图标区域（缩进之后、文字之前的 16px）→ 切换展开
                let x0 = self.rect.x + depth as f32 * INDENT;
                if pos.x >= x0 && pos.x < x0 + CHEVRON && self.toggle_expanded(&path) {
                    return EventResult::Relayout;
                }
                self.selected = Some(path);
                EventResult::Redraw
            }
            UiEvent::Focus(focused) => {
                let _ = focused;
                EventResult::Redraw
            }
            UiEvent::Key { key, modifiers } => {
                if *modifiers != Modifiers::default() {
                    return EventResult::Ignored;
                }
                match key {
                    Key::Down => {
                        let next = match self.selected_index() {
                            Some(i) if i + 1 < self.visible.len() => i + 1,
                            Some(_) => return EventResult::Ignored,
                            None if !self.visible.is_empty() => 0,
                            None => return EventResult::Ignored,
                        };
                        self.selected = Some(self.visible[next].0.clone());
                        EventResult::Redraw
                    }
                    Key::Up => {
                        let next = match self.selected_index() {
                            Some(i) if i > 0 => i - 1,
                            Some(_) => return EventResult::Ignored,
                            None if !self.visible.is_empty() => 0,
                            None => return EventResult::Ignored,
                        };
                        self.selected = Some(self.visible[next].0.clone());
                        EventResult::Redraw
                    }
                    Key::Right => {
                        let Some(i) = self.selected_index() else {
                            return EventResult::Ignored;
                        };
                        let (path, _) = self.visible[i].clone();
                        if let Some(item) = self.item_at(&path) {
                            if !item.children.is_empty() && !item.expanded {
                                self.toggle_expanded(&path);
                                return EventResult::Relayout;
                            }
                            if item.expanded && i + 1 < self.visible.len() {
                                // 进入首个子项
                                self.selected = Some(self.visible[i + 1].0.clone());
                                return EventResult::Redraw;
                            }
                        }
                        EventResult::Ignored
                    }
                    Key::Left => {
                        let Some(i) = self.selected_index() else {
                            return EventResult::Ignored;
                        };
                        let (path, _) = self.visible[i].clone();
                        if let Some(item) = self.item_at(&path) {
                            if item.expanded && !item.children.is_empty() {
                                self.toggle_expanded(&path);
                                return EventResult::Relayout;
                            }
                        }
                        if path.len() > 1 {
                            self.selected = Some(path[..path.len() - 1].to_vec());
                            return EventResult::Redraw;
                        }
                        EventResult::Ignored
                    }
                    _ => EventResult::Ignored,
                }
            }
            UiEvent::Wheel { delta, .. } => {
                let total = self.visible.len() as f32 * self.row_h;
                let max = (total - self.rect.h).max(0.0);
                let next = (self.scroll_offset - delta * self.row_h).clamp(0.0, max);
                if (next - self.scroll_offset).abs() > 0.01 {
                    self.scroll_offset = next;
                    return EventResult::Redraw;
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let normal = ctx.theme().style(Role::LIST, State::Normal);
        let selected = ctx.theme().style(Role::LIST, State::Selected);
        let style = TextStyle::fg(normal.fg);

        // 只画可见范围内的行（虚拟化）。
        ctx.push_clip(self.rect);
        for i in self.visible_range() {
            let Some(r) = self.row_rect(i) else {
                continue;
            };
            if r.bottom() < self.rect.y || r.y > self.rect.bottom() {
                continue;
            }
            let (path, depth) = self.visible[i].clone();
            let is_sel = self.selected.as_ref() == Some(&path);
            if is_sel {
                ctx.fill_rect(r, selected.bg);
            }

            let x0 = r.x + depth as f32 * INDENT;
            // 展开图标：用逐行短条拼一个三角形（没有旋转图元）。
            let expanded = self
                .item_at(&path)
                .map(|it| !it.children.is_empty() && it.expanded)
                .unwrap_or(false);
            let has_children = self
                .item_at(&path)
                .map(|it| !it.children.is_empty())
                .unwrap_or(false);
            if has_children {
                let cy = r.y + r.h * 0.5;
                let cx = x0 + CHEVRON * 0.5;
                let color = normal.fg;
                for k in 0..5 {
                    let w = 6.0 - k as f32 * 1.0;
                    let y = if expanded {
                        cy - 3.0 + k as f32
                    } else {
                        // 收起：向右的三角
                        continue;
                    };
                    ctx.fill_rect(Rect::new(cx - w * 0.5, y, w, 1.0), color);
                }
                if !expanded {
                    for k in 0..5 {
                        let h = 6.0 - k as f32 * 1.0;
                        let x = cx - 3.0 + k as f32;
                        ctx.fill_rect(Rect::new(x, cy - h * 0.5, 1.0, h), color);
                    }
                }
            }

            let label = self
                .item_at(&path)
                .map(|it| it.label.clone())
                .unwrap_or_default();
            let tx = self.text_x(depth);
            let ty = r.y + (r.h - ctx.line_height(&style)) * 0.5;
            ctx.draw_text(Pos::new(tx, ty), &label, &style);
        }
        ctx.pop_clip();
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("tree {}", self.id))
            .value(
                self.selected
                    .as_ref()
                    .map(|p| p.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("."))
                    .unwrap_or_default(),
            )
            .prop("visible_rows", self.visible.len().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;
    use y901_ui::UiTree;

    const W: u32 = 240;
    const H: u32 = 200;

    fn soft() -> SoftBackend {
        let (cw, ch) = y901_render::cell_metrics_for(15.0);
        let cell = Size::new(cw, ch);
        let engine = MyFontEngine::new(
            y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
            15.0,
            cell,
        )
        .expect("engine");
        SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
    }

    /// 一棵 2 层树：root 展开，两个子项。
    fn two_level() -> Vec<TreeItem> {
        vec![TreeItem::branch(
            "父节点",
            vec![TreeItem::leaf("子项A"), TreeItem::leaf("子项B")],
        )]
    }

    #[test]
    fn flatten_only_includes_expanded_branches() {
        let mut t = TreeView::new("t", two_level());
        assert_eq!(t.visible_len(), 3, "展开时应是 父 + 2 子");

        let collapsed = t.toggle_expanded(&[0]);
        assert!(collapsed);
        assert_eq!(t.visible_len(), 1, "折叠后只应剩父节点");
        let rows = t.visible_rows();
        assert_eq!(rows[0].1, 0, "父节点深度 0");
    }

    #[test]
    fn thousand_children_are_virtualized() {
        // 1000 个子项：展开后扁平 1001 行，但可见范围只覆盖视口
        let children: Vec<TreeItem> = (0..1000)
            .map(|i| TreeItem::leaf(format!("item {i}")))
            .collect();
        let mut t = TreeView::new("t", vec![TreeItem::branch("root", children)]);
        t.rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(t.visible_len(), 1001);

        let range = t.visible_range();
        let count = range.end - range.start;
        eprintln!("[tree] 1001 行中可见范围 = {range:?}（{count} 行）");
        assert!(
            count <= 6,
            "视口 100px / 行高 24px 应只覆盖约 5 行，实际 {count}"
        );
    }

    #[test]
    fn visible_range_follows_scroll() {
        let children: Vec<TreeItem> = (0..100)
            .map(|i| TreeItem::leaf(format!("i{i}")))
            .collect();
        let mut t = TreeView::new("t", vec![TreeItem::branch("root", children)]);
        t.rect = Rect::new(0.0, 0.0, 200.0, 96.0);
        let r0 = t.visible_range();
        t.scroll_offset = ROW_H * 10.0;
        let r1 = t.visible_range();
        assert!(r1.start >= 10, "滚动 10 行后起始行应 >= 10，实际 {}", r1.start);
        assert!(r0.start < r1.start);
    }

    #[test]
    fn row_at_maps_y_to_index() {
        let mut t = TreeView::new("t", two_level());
        t.rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(t.row_at(Pos::new(10.0, 5.0)), Some(0));
        assert_eq!(t.row_at(Pos::new(10.0, ROW_H + 5.0)), Some(1));
        assert_eq!(t.row_at(Pos::new(500.0, 5.0)), None, "越界应 None");
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let mut t = TreeView::new("t", two_level());
        t.rect = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert!(t.select_path(&[0, 1]), "子项应可选中");
        assert_eq!(t.selected(), Some(&vec![0, 1]));
        assert!(!t.select_path(&[9]), "不可见的路径不应被选中");
    }

    #[test]
    fn child_text_is_indented_one_level() {
        // 纯几何断言：子项的文字起始 x 必须比父项多 INDENT。
        //
        // 为什么不用像素：实测发现 **SoftBackend 在这个无头夹具下不产生字形像素**
        // （只有实心矩形的展开三角形有墨，文字区域全空；已在报告里记为发现，不在本轮调）。
        let mut t = TreeView::new("t", two_level());
        t.rect = Rect::new(0.0, 0.0, 200.0, 3.0 * ROW_H);
        let parent_x = t.text_x(0);
        let child_x = t.text_x(1);
        assert!(
            (child_x - parent_x - INDENT).abs() < 0.01,
            "子项文字应比父项多 {INDENT}px，实际 {child_x} - {parent_x} = {}",
            child_x - parent_x
        );
    }

    #[test]
    fn collapsing_hides_child_rows_in_pixels() {
        let mut s = soft();
        let mut tree = UiTree::new(Box::new(TreeView::new("t", two_level())));
        tree.layout(Size::new(W as f32, H as f32), &s);
        tree.invalidate_all();
        s.clear();
        tree.render(&mut s);
        let rows_expanded = ink_rows(&s);

        // 折叠根节点
        tree.node_as_mut::<TreeView>("t")
            .expect("能拿到 TreeView")
            .toggle_expanded(&[0]);
        tree.invalidate_all();
        s.clear();
        tree.render(&mut s);
        let rows_collapsed = ink_rows(&s);

        eprintln!("[tree] 展开时有墨迹的行数={rows_expanded}，折叠后={rows_collapsed}");
        assert!(
            rows_collapsed < rows_expanded,
            "折叠后行数应减少：{rows_expanded} -> {rows_collapsed}"
        );
    }

    /// 有墨迹（非背景）的行数。
    fn ink_rows(s: &SoftBackend) -> usize {
        let bg = s.pixel(W - 1, H - 1).expect("bg");
        let mut n = 0;
        for y in 0..H {
            let mut ink = false;
            for x in 0..W {
                if s.pixel(x, y).expect("px") != bg {
                    ink = true;
                    break;
                }
            }
            if ink {
                n += 1;
            }
        }
        n
    }
}
