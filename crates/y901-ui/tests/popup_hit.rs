//! **浮层的命中 / 光标 / 快照可见性**（PLAN 批次 4）。
//!
//! 背景：`UiTree::popups` 里的浮层（菜单、下拉、Tooltip、Dialog）**不在普通节点索引里**
//!（`paint_order` 只含树节点），而 `hit_test` / `cursor_at` / `snapshot_json` 都只查树。
//! 后果是三条真实的用户可见缺陷：
//!
//! 1. 浮层盖住背景按钮时，命中返回**被遮挡的背景节点** → RPC/自动化点"菜单项"
//!    实际点到了下面的东西；
//! 2. 菜单项声明的 `Cursor::Pointer` 不生效（光标指到的是背景节点，通常是 `Default`）；
//! 3. `snapshot_json()`（RPC `get_tree`）里**看不到菜单项** → 外部工具无法驱动菜单。
//!
//! 本文件钉住修好后的语义：浮层在视觉最上层，命中/光标/快照必须与之一致。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Cursor, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle,
    UiTree, Widget,
};

/// 什么都不画的后端：**只需要让 `UiTree::render` 跑一遍**。
///
/// 为什么必须渲染：`paint_order`（命中测试用的绘制顺序索引）是在 render 期间建的，
/// 只 `layout` 不 `render` 的话 `hit_index` 拿不到任何候选 —— 这会让断言变成
/// "因为没画所以没命中"的假失败。
struct NullCtx;
impl TextMetrics for NullCtx {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}
impl RenderCtx for NullCtx {
    fn size(&self) -> Size {
        Size::new(VW, VH)
    }
    // `theme()` 有默认实现（DARK_THEME），不必覆盖。
    fn fill_rect(&mut self, _rect: Rect, _color: Color) {}
    fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {}
    fn draw_text(&mut self, _origin: Pos, _text: &str, _style: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _rect: Rect) {}
    fn pop_clip(&mut self) {}
}

const VW: f32 = 400.0;
const VH: f32 = 300.0;

/// 全树共用的最小度量（本文件不测文字）。
struct Metrics;
impl TextMetrics for Metrics {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 一个带 id / 光标 / 语义的叶子节点。
struct Leaf {
    id: &'static str,
    rect: Rect,
    cursor: Cursor,
    /// 无子节点时的理想尺寸（有子节点时按孩子累加）。
    size: Size,
    /// 子节点（只用于构造"浮层里的容器 + 菜单项"两层结构）。
    children: Vec<Box<dyn Widget>>,
}

impl Leaf {
    fn new(id: &'static str, cursor: Cursor, size: Size) -> Self {
        Self {
            id,
            rect: Rect::ZERO,
            cursor,
            size,
            children: Vec::new(),
        }
    }
    fn with_children(mut self, kids: Vec<Box<dyn Widget>>) -> Self {
        self.children = kids;
        self
    }
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        // 有子节点：按孩子大小（纵向堆叠）算；无子节点：吃满可用空间。
        if self.children.is_empty() {
            return c.constrain(self.size);
        }
        let mut h = 0.0;
        let mut w = 0.0f32;
        for child in &mut self.children {
            let s = child.measure(m, c, cache);
            h += s.h;
            w = w.max(s.w);
        }
        c.constrain(Size::new(w, h))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        // 子节点纵向排列（与菜单项布局一致）。
        let mut y = rect.y;
        for child in &mut self.children {
            let s = child.measure(m, Constraints::loose(Size::new(rect.w, rect.h)), cache);
            child.arrange(Rect::new(rect.x, y, rect.w, s.h), m, cache);
            y += s.h;
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    /// 必须有：`add_node` 只对返回 `Some` 的容器生效，否则"挂子节点"直接失败
    ///（本文件的 `snapshot_appends_popups_after_the_base_tree` 依赖它）。
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn children_mut_for_traversal(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn cursor_at(&self, _pos: Pos) -> Cursor {
        self.cursor
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::from_rgb8(240, 240, 240));
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn tree() -> UiTree {
    UiTree::new(Box::new(Leaf::new(
        "bg",
        Cursor::Default,
        Size::new(VW, VH),
    )))
}

fn layout_once(tree: &mut UiTree) {
    tree.layout(Size::new(VW, VH), &Metrics);
    // 命中测试依赖 `paint_order`，而它在 render 期间构建。
    tree.invalidate_all();
    tree.render(&mut NullCtx);
}

/// 浮层盖住背景时，`hit_test` 必须返回**浮层里的**节点。
#[test]
fn hit_test_sees_through_to_the_popup_not_the_covered_background() {
    let mut t = tree();
    layout_once(&mut t);

    // 两个菜单项，各 20px 高；浮层摆在左上角。
    let menu = Leaf::new("menu", Cursor::Default, Size::new(120.0, 40.0)).with_children(vec![
        Box::new(Leaf::new("item-a", Cursor::Pointer, Size::new(120.0, 20.0))),
        Box::new(Leaf::new("item-b", Cursor::Pointer, Size::new(120.0, 20.0))),
    ]);
    assert!(
        t.open_popup(Box::new(menu), Rect::new(10.0, 10.0, 40.0, 24.0)),
        "浮层应成功打开"
    );
    layout_once(&mut t);

    // 浮层内第二项的位置：由 arrange 决定，这里直接用节点 rect 反查（不硬编码）。
    let item = t.popup_rect("menu").expect("浮层应有 rect");
    assert!(!item.is_empty(), "浮层 rect 应非空，实际 {item:?}");

    // 点浮层内部（第一项中心）：必须命中 item-a，而不是 bg。
    let hit = t.hit_test(Pos::new(item.x + 4.0, item.y + 4.0));
    assert_eq!(
        hit.as_deref(),
        Some("item-a"),
        "浮层盖住背景时，命中必须是浮层里的菜单项（旧实现返回被遮挡的 bg）"
    );

    // 点浮层外：仍是背景。
    let outside = t.hit_test(Pos::new(VW - 5.0, VH - 5.0));
    assert_eq!(outside.as_deref(), Some("bg"), "浮层外仍应命中背景节点");
}

/// 光标要与命中一致：浮层里的 `Hand` 必须生效。
#[test]
fn cursor_at_follows_the_popup() {
    let mut t = tree();
    layout_once(&mut t);
    let menu =
        Leaf::new("menu", Cursor::Default, Size::new(120.0, 20.0)).with_children(vec![Box::new(
            Leaf::new("item", Cursor::Pointer, Size::new(120.0, 20.0)),
        )]);
    t.open_popup(Box::new(menu), Rect::new(10.0, 10.0, 40.0, 24.0));
    layout_once(&mut t);

    let r = t.popup_rect("menu").expect("浮层 rect");
    assert_eq!(
        t.cursor_at(Pos::new(r.x + 4.0, r.y + 4.0)),
        Cursor::Pointer,
        "浮层内的 Hand 光标必须生效（旧实现看到的是背景的 Default）"
    );
    // 浮层外仍是背景的 Default。
    assert_eq!(t.cursor_at(Pos::new(VW - 5.0, VH - 5.0)), Cursor::Default);
}

/// 模态浮层：浮层外的点**不能**穿透到底层。
#[test]
fn modal_popup_swallows_clicks_outside_it() {
    let mut t = tree();
    layout_once(&mut t);
    let menu =
        Leaf::new("dlg", Cursor::Default, Size::new(120.0, 24.0)).with_children(vec![Box::new(
            Leaf::new("ok", Cursor::Pointer, Size::new(120.0, 24.0)),
        )]);
    t.open_modal_popup(Box::new(menu), Rect::new(100.0, 100.0, 60.0, 24.0));
    layout_once(&mut t);

    let r = t.popup_rect("dlg").expect("模态浮层 rect");
    assert_eq!(
        t.hit_test(Pos::new(r.x + 4.0, r.y + 4.0)).as_deref(),
        Some("ok"),
        "模态浮层内部照常命中"
    );
    assert_eq!(
        t.hit_test(Pos::new(VW - 5.0, VH - 5.0)),
        None,
        "模态浮层外部必须吞掉点击（不能命中被遮住的背景）"
    );
}

/// `snapshot_json()` 必须包含浮层子树（RPC `get_tree` 才看得见菜单项）。
#[test]
fn snapshot_json_includes_popup_subtrees() {
    let mut t = tree();
    layout_once(&mut t);
    let menu =
        Leaf::new("menu", Cursor::Default, Size::new(120.0, 20.0)).with_children(vec![Box::new(
            Leaf::new("item", Cursor::Pointer, Size::new(120.0, 20.0)),
        )]);
    t.open_popup(Box::new(menu), Rect::new(10.0, 10.0, 40.0, 24.0));
    layout_once(&mut t);

    let json = format!("{:?}", t.snapshot_json());
    assert!(json.contains("\"bg\""), "树节点仍应在快照里：{json}");
    assert!(
        json.contains("menu") && json.contains("item"),
        "浮层与菜单项必须出现在快照里（旧实现只有树节点）：{json}"
    );
}
/// `snapshot()`（无障碍/外部工具用的 `NodeInfo` 树）里的浮层矩形，
/// 必须与**命中测试**用的是同一个矩形。
///
/// 为什么单独钉：AccessKit 的 `bounds` 由这个 `rect` 生成，读屏器的
/// "触摸探索 / 鼠标定位朗读"直接打在 bounds 上。若语义树报 `node.rect()`
/// 而命中用 `place_popup` 的结果，两者一旦不一致，读屏器指到的地方就点不中。
#[test]
fn snapshot_popup_rect_matches_the_hit_test_geometry() {
    let mut t = tree();
    layout_once(&mut t);
    let menu = Leaf::new("menu", Cursor::Default, Size::new(120.0, 40.0)).with_children(vec![
        Box::new(Leaf::new("item-a", Cursor::Pointer, Size::new(120.0, 20.0))),
        Box::new(Leaf::new("item-b", Cursor::Pointer, Size::new(120.0, 20.0))),
    ]);
    // 锚点贴着视口右下角：`place_popup` 会把它往左上翻，因此"放置后矩形"
    // 一定不同于锚点矩形 —— 正是旧实现容易报错的那种情形。
    assert!(t.open_popup(Box::new(menu), Rect::new(VW - 20.0, VH - 20.0, 40.0, 24.0)));
    layout_once(&mut t);

    let placed = t.popup_rect("menu").expect("浮层应有 rect");
    let snap = t.snapshot();
    let popup_info = snap
        .children
        .iter()
        .find(|c| c.id == "menu")
        .expect("浮层必须出现在语义树里");
    assert_eq!(
        popup_info.rect, placed,
        "语义树必须报 place_popup 之后的矩形（旧实现报的是控件自己的 rect）"
    );

    // 语义树里的矩形中心，命中必须落进浮层内部。
    let center = Pos::new(placed.x + placed.w * 0.5, placed.y + placed.h * 0.5);
    let hit = t.hit_test(center).expect("浮层中心应有命中");
    assert!(
        hit == "item-a" || hit == "item-b" || hit == "menu",
        "语义树矩形与命中几何不一致：center={center:?} 命中 {hit}"
    );
}

/// 浮层必须排在根节点的 `children` **末尾**（与绘制顺序一致：浮层最后画）。
///
/// AccessKit 的树内顺序等于读屏器的朗读顺序；把浮层混进基础内容中间，
/// 会造成"菜单项夹在正文里"的朗读顺序错误。
#[test]
fn snapshot_appends_popups_after_the_base_tree() {
    let mut t = tree();
    // 根节点本身没有孩子时，"浮层排最后"是无意义的断言 —— 先加一个真正的
    // 基础内容节点，才能区分"浮层混进基础树"与"浮层追加在后面"。
    assert!(
        t.add_node(
            "bg",
            Box::new(Leaf::new("btn", Cursor::Pointer, Size::new(80.0, 24.0)))
        ),
        "基础按钮应挂得上"
    );
    layout_once(&mut t);
    let menu =
        Leaf::new("menu", Cursor::Default, Size::new(120.0, 20.0)).with_children(vec![Box::new(
            Leaf::new("item", Cursor::Pointer, Size::new(120.0, 20.0)),
        )]);
    t.open_popup(Box::new(menu), Rect::new(10.0, 10.0, 40.0, 24.0));
    layout_once(&mut t);

    let snap = t.snapshot();
    let ids: Vec<&str> = snap.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids.last().copied(),
        Some("menu"),
        "浮层应是根节点的最后一个孩子（绘制顺序），实得 {ids:?}"
    );
    assert_eq!(
        ids,
        vec!["btn", "menu"],
        "基础内容在前、浮层在后（绘制/朗读顺序），实得 {ids:?}"
    );
    let menu_info = snap.children.last().expect("浮层");
    assert!(
        menu_info.children.iter().any(|c| c.id == "item"),
        "浮层子树也要进语义树，实得 {:?}",
        menu_info.children.iter().map(|c| &c.id).collect::<Vec<_>>()
    );
}
