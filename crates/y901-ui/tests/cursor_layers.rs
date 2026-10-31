//! `cursor_at` 与 `hit_index` 的三个语义差异（修复前后对比）。
//!
//! `hit_index` 的语义（`rebuild_indices` 构建的索引）：
//! 1. 关闭节点整棵跳过；
//! 2. 祖先 `clip()` 逐层累加，命中用 `rect ∩ 累计clip`；
//! 3. 按 layer 排序后取最上层。
//!
//! `cursor_at` 必须与之一致，否则"光标形状"会指向一个点不到的元素。

use y901_ui::{
    Constraints, Cursor, EventCtx, EventResult, Flex, Pos, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiTree, Widget,
};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::new(8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 固定矩形 + 固定光标 + 可配置 open/clip 的探针。
struct Probe {
    id: &'static str,
    rect: Rect,
    cursor: Cursor,
    open: bool,
    clip: Option<Rect>,
}

impl Probe {
    fn new(id: &'static str, rect: Rect, cursor: Cursor) -> Self {
        Self {
            id,
            rect,
            cursor,
            open: true,
            clip: None,
        }
    }
    fn closed(mut self) -> Self {
        self.open = false;
        self
    }
}

impl Widget for Probe {
    fn id(&self) -> &str {
        self.id
    }
    fn cursor(&self) -> Cursor {
        self.cursor
    }
    fn is_open(&self) -> bool {
        self.open
    }
    fn clip(&self) -> Option<Rect> {
        self.clip
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.max
    }
    fn arrange(&mut self, _r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {}
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

/// 手动摆位的容器（不做 flex 计算，直接按给定顺序 arrange 到全屏）。
struct Col {
    id: &'static str,
    kids: Vec<Box<dyn Widget>>,
    rect: Rect,
    clip: Option<Rect>,
}

impl Widget for Col {
    fn id(&self) -> &str {
        self.id
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn clip(&self) -> Option<Rect> {
        self.clip
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.max
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, c: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        let kids = std::mem::take(&mut self.kids);
        for mut k in kids {
            k.arrange(r, m, c);
            self.kids.push(k);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
}

fn layout(root: Box<dyn Widget>) -> UiTree {
    let mut tree = UiTree::new(root);
    tree.layout(Size::new(400.0, 400.0), &Mono);
    tree
}

// ---------- Bug 1：跨层重叠时取了下层的 cursor ----------

#[test]
fn cursor_at_matches_hit_index_with_overlapping_layers() {
    // Overlay 在前、Base 在后（树序），两者都覆盖 (10,10)
    let root = Col {
        id: "root",
        // 没有 Layer 了："在上层"用**树序**表达——后访问的在上，
        // 所以 base 放前面、overlay 放后面。
        kids: vec![
            Box::new(Probe::new(
                "base",
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Cursor::Text,
            )),
            Box::new(Probe::new(
                "overlay",
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Cursor::Pointer,
            )),
        ],
        rect: Rect::ZERO,
        clip: None,
    };
    let tree = layout(Box::new(root));

    assert_eq!(
        tree.hit_test(Pos::new(10.0, 10.0)).as_deref(),
        Some("overlay")
    );
    assert_eq!(
        tree.cursor_at(Pos::new(10.0, 10.0)),
        Cursor::Pointer,
        "光标必须来自最上层（overlay），实际不是"
    );
}

// ---------- Bug 2：关闭的浮层仍贡献 cursor ----------

#[test]
fn cursor_at_skips_closed_overlay() {
    let root = Col {
        id: "root",
        kids: vec![Box::new(
            Probe::new("menu", Rect::new(0.0, 0.0, 100.0, 100.0), Cursor::Pointer).closed(),
        )],
        rect: Rect::ZERO,
        clip: None,
    };
    let tree = layout(Box::new(root));

    assert_ne!(
        tree.hit_test(Pos::new(10.0, 10.0)).as_deref(),
        Some("menu"),
        "前置：关闭节点不参与命中（命中的只能是 root）"
    );
    assert_eq!(
        tree.cursor_at(Pos::new(10.0, 10.0)),
        Cursor::Default,
        "关闭的浮层不能贡献光标"
    );
}

// ---------- Bug 3：不累加祖先 clip() ----------

#[test]
fn cursor_at_respects_ancestor_clip() {
    // 容器只允许 (0,0)-(100,100) 可见；子节点在 (200,200) 处（理论上永远不可见）
    let outer = Col {
        id: "viewport",
        kids: vec![Box::new(Probe::new(
            "hidden_child",
            Rect::new(200.0, 200.0, 50.0, 50.0),
            Cursor::Pointer,
        ))],
        rect: Rect::ZERO,
        clip: Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
    };
    let tree = layout(Box::new(outer));

    // 命中测试在视口外：不命中
    assert_eq!(
        tree.hit_test(Pos::new(210.0, 210.0)),
        None,
        "前置：视口外的子节点不参与命中"
    );
    // 光标也必须不来自该子节点
    assert_eq!(
        tree.cursor_at(Pos::new(210.0, 210.0)),
        Cursor::Default,
        "祖先 clip 之外的子节点不能贡献光标"
    );
}

// ---------- 回归：基本情形仍然工作 ----------

#[test]
fn cursor_at_still_works_for_simple_hover() {
    let root = Col {
        id: "root",
        kids: vec![Box::new(Probe::new(
            "btn",
            Rect::new(0.0, 0.0, 200.0, 100.0),
            Cursor::Pointer,
        ))],
        rect: Rect::ZERO,
        clip: None,
    };
    let tree = layout(Box::new(root));

    assert_eq!(tree.cursor_at(Pos::new(50.0, 50.0)), Cursor::Pointer);
    assert_eq!(
        tree.cursor_at(Pos::new(300.0, 300.0)),
        Cursor::Default,
        "不在任何节点上时必须是 Default"
    );
}

// ---------- depth 分支：父子同层时更深者胜 ----------

/// 带光标与子节点的容器（父节点自己也可能贡献 cursor）。
struct CursorBox {
    id: &'static str,
    rect: Rect,
    cursor: Cursor,
    kids: Vec<Box<dyn Widget>>,
}

impl Widget for CursorBox {
    fn id(&self) -> &str {
        self.id
    }
    fn cursor(&self) -> Cursor {
        self.cursor
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.kids
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.kids)
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _c: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.max
    }
    fn arrange(&mut self, _r: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {}
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

/// 父节点（Base, Pointer）与子节点（Base, Text）同点重叠：子节点更深 → 胜出。
///
/// 这条测试盯的是 `depth` 分支：若把 `depth >= bd` 改成 `depth > bd`，
/// 同深度时先访问的父节点会胜出，测试立刻红。
#[test]
fn cursor_at_child_overrides_parent_same_layer() {
    let parent = CursorBox {
        id: "parent",
        rect: Rect::new(0.0, 0.0, 100.0, 100.0),
        cursor: Cursor::Pointer,
        kids: vec![Box::new(Probe::new(
            "child",
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Cursor::Text,
        ))],
    };
    let tree = layout(Box::new(parent));

    assert_eq!(
        tree.cursor_at(Pos::new(10.0, 10.0)),
        Cursor::Text,
        "同层重叠时，更深的子节点应覆盖父节点的光标"
    );
}

/// 同层同深度的**兄弟**重叠：树序靠后者胜（与 `hit_index` 的 `rposition` 一致）。
///
/// 这条测试盯的是 `>=` 与 `>` 的差别：
/// - `>=`（当前实现）：后访问的兄弟胜出 → 与 paint_order 的 rposition 一致；
/// - `>`（若被误改）：先访问的兄弟胜出 → 与 hit_index 相反，本测试立刻红。
#[test]
fn cursor_at_later_sibling_wins_same_layer() {
    let root = Col {
        id: "root",
        kids: vec![
            Box::new(Probe::new(
                "first",
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Cursor::Pointer,
            )),
            Box::new(Probe::new(
                "second",
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Cursor::Text,
            )),
        ],
        rect: Rect::ZERO,
        clip: None,
    };
    let tree = layout(Box::new(root));

    // 前置：命中测试取 paint_order 最后一个匹配 → second
    assert_eq!(
        tree.hit_test(Pos::new(10.0, 10.0)).as_deref(),
        Some("second")
    );
    assert_eq!(
        tree.cursor_at(Pos::new(10.0, 10.0)),
        Cursor::Text,
        "同层同深度时，树序靠后的兄弟必须胜出（与 hit_index 一致）"
    );
}

// 占位：确认 Flex 仍可导入（后续测试会用）
#[allow(dead_code)]
fn _unused() -> Flex {
    Flex::column("x")
}
