//! MenuBar：窗口顶部的**菜单栏**（文件 / 编辑 / 视图 …）。
//!
//! ## 结构
//!
//! 每个菜单是一个标题按钮；点一下在该标题下方弹出 `Menu` 浮层（走框架的
//! `ctx.request_popup`，因此不受父容器裁剪、自带弹出动画）。菜单项的选择交给
//! `Menu` 自己的 `action` / `on_select`，`MenuBar` 不重复实现一遍。
//!
//! ## 键盘
//!
//! `MenuBar` 本身只处理**焦点与左右移动**：`←`/`→` 在菜单标题之间移动，
//! `Enter`/`↓` 打开当前菜单。打开后的上下选择、Esc 关闭由 `Menu` 负责
//! （它是浮层，拿到键盘焦点）。
//!
//! ## 不做
//!
//! Alt 助记键（`Alt+F`）需要后端把 `Alt` 组合键作为独立事件送来，当前
//! `UiEvent::Key` 只带修饰键、不区分"被 Alt 吞掉的系统按键"，因此**明确不做**。

use y901_ui::widget::FlexItem;
use y901_ui::{
    Constraints, EventCtx, EventPhase, EventResult, Key, Pos, Rect, RenderCtx, Role, Semantics,
    Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

use crate::overlay::Menu;

/// 一个菜单的定义：标题 + 菜单项 + 选中时发给应用层的命令名。
#[derive(Clone, Debug)]
pub struct MenuEntry {
    /// 标题（显示在菜单栏上）。
    pub title: String,
    /// 菜单项（顺序即显示顺序）。
    pub items: Vec<String>,
    /// 选中某项时发给应用层的命令名。
    pub action: String,
}

impl MenuEntry {
    /// 新建一个菜单定义。
    pub fn new(title: impl Into<String>, items: Vec<String>, action: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            items,
            action: action.into(),
        }
    }
}

/// 菜单栏高度（逻辑像素）。
const BAR_H: f32 = 28.0;
/// 标题左右内边距。
const TITLE_PAD: f32 = 12.0;

/// 水平菜单栏。
pub struct MenuBar {
    id: String,
    /// 无障碍名称（读屏器念它）。空 = 不提供名称（**不再拿 id 顶替**）。
    label: String,
    entries: Vec<MenuEntry>,
    rect: Rect,
    item: FlexItem,
    /// 悬停的标题下标。
    hovered: Option<usize>,
    /// 键盘选中的标题下标（`-1` 表示无）。
    active: Option<usize>,
    /// 当前是否有本组菜单的浮层打开（用于绘制"按下"态；由 `Menu` 的关闭命令回写）。
    opened: Option<usize>,
    scale: f32,
}

impl MenuBar {
    /// 新建空菜单栏。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            entries: Vec::new(),
            rect: Rect::ZERO,
            item: FlexItem::default(),
            hovered: None,
            active: None,
            opened: None,
            scale: 1.0,
        }
    }

    /// 无障碍名称（读屏器念的名字；空 = 不提供）。
    ///
    /// 为什么不给默认值：语义里曾经写 `format!("… {}", self.id)`，于是读屏器把
    /// **内部 id** 念给用户听（真机 UIA 验收抓到的缺陷之一）。组件编不出人话，
    /// 就该留空让使用方补 —— 空名字比"念实现细节"好。
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }

    /// 追加一个菜单。
    pub fn menu(mut self, entry: MenuEntry) -> Self {
        self.entries.push(entry);
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 菜单数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 清掉"某个菜单处于打开态"的视觉标记。
    ///
    /// **为什么需要应用调用**：菜单项被点中后 `Menu` 自己关闭浮层并发动作命令，
    /// 而 `MenuBar` 收不到那个事件（浮层不是它的子节点），于是标题高亮会一直留着、
    /// `active_index()` 也一直返回 `Some(i)`。框架目前没有"我的浮层还开着吗"的查询
    /// API，所以在应用处理完菜单动作后显式调一次是唯一可靠的做法。
    ///
    /// 返回：`true` = 确实清掉了（需要重绘）。
    pub fn clear_open_state(&mut self) -> bool {
        if self.opened.is_none() && self.active.is_none() {
            return false;
        }
        self.opened = None;
        self.active = None;
        true
    }

    /// 当前打开（或键盘选中）的菜单下标。
    pub fn active_index(&self) -> Option<usize> {
        self.opened.or(self.active)
    }

    /// 各标题的矩形（布局后有效）。
    pub fn title_rects(&self) -> Vec<Rect> {
        let mut out = Vec::with_capacity(self.entries.len());
        let mut x = self.rect.x;
        for e in &self.entries {
            let w = self.title_width(&e.title);
            out.push(Rect::new(x, self.rect.y, w, self.rect.h));
            x += w;
        }
        out
    }

    /// 标题宽度：按字符数估算（框架的 TextMetrics 只在 measure 阶段可用，
    /// 而命中/绘制需要稳定的几何，因此用**固定字符宽**换算，与 `Menu` 的做法一致）。
    fn title_width(&self, title: &str) -> f32 {
        let chars = title.chars().count() as f32;
        // 中日韩字符按双宽估算。
        let wide = title
            .chars()
            .filter(|c| {
                matches!(*c as u32,
                    0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3
                    | 0xF900..=0xFAFF | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6)
            })
            .count() as f32;
        (chars + wide) * 7.0 * self.scale + TITLE_PAD * 2.0 * self.scale
    }

    fn title_at(&self, pos: Pos) -> Option<usize> {
        if !self.rect.contains(pos) {
            return None;
        }
        self.title_rects()
            .iter()
            .position(|r| pos.x >= r.x && pos.x < r.right())
    }

    fn open_menu(&mut self, ctx: &mut EventCtx, index: usize) {
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        let menu_id = format!("{}.menu{index}", self.id);
        let items = entry.items.clone();
        let action = entry.action.clone();
        let anchor = self.title_rects().get(index).copied().unwrap_or(self.rect);
        ctx.request_popup(
            Box::new(move || {
                // 作为浮层内容时必须先 open()，否则 render/event 都以它为门直接忽略。
                let mut menu = Menu::new(menu_id).items(items).action(action);
                menu.open();
                Box::new(menu)
            }),
            anchor,
        );
        self.opened = Some(index);
        self.active = Some(index);
    }
}

impl Widget for MenuBar {
    /// **键盘可达**：菜单栏在 `event` 里实现 ←/→/Enter 与 `Focus(true)` 初始化，
    /// 但 `set_focus` 只接受 `focusable` 节点 —— 不声明就整段逻辑不可达。
    fn focusable(&self) -> bool {
        true
    }
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        c.constrain(Size::new(c.max.w, BAR_H * self.scale))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if ctx.phase == EventPhase::Capture {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::MouseMove { pos } => {
                let was = self.hovered;
                self.hovered = self.title_at(*pos);
                if was != self.hovered {
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::MouseUp { pos, .. } => {
                if let Some(index) = self.title_at(*pos) {
                    ctx.request_focus(self.id.clone());
                    self.open_menu(ctx, index);
                    return EventResult::Redraw;
                }
                // 点在标题之外：浮层多半已经被框架关掉了，标题高亮要跟着清
                if self.clear_open_state() {
                    return EventResult::Redraw;
                }
                EventResult::Ignored
            }
            UiEvent::Focus(true) => {
                if self.active.is_none() {
                    self.active = Some(0);
                    return EventResult::Redraw;
                }
                EventResult::Ignored
            }
            UiEvent::Focus(false) => {
                self.active = None;
                self.opened = None;
                EventResult::Redraw
            }
            UiEvent::Key { key, .. } => {
                let n = self.entries.len();
                if n == 0 {
                    return EventResult::Ignored;
                }
                let cur = self.active.unwrap_or(0);
                match key {
                    Key::Left => {
                        self.active = Some(cur.saturating_sub(1));
                        EventResult::Redraw
                    }
                    Key::Right => {
                        self.active = Some((cur + 1).min(n - 1));
                        EventResult::Redraw
                    }
                    Key::Enter | Key::Down => {
                        self.open_menu(ctx, cur);
                        EventResult::Redraw
                    }
                    Key::Escape => {
                        self.active = None;
                        self.opened = None;
                        EventResult::Redraw
                    }
                    _ => EventResult::Ignored,
                }
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (bg, fg, muted, hi_bg) = {
            let theme = ctx.theme();
            (
                theme.style(Role::PANEL, State::Normal).bg,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.muted_text_color(),
                theme.style(Role::LIST_ITEM, State::Hovered).bg,
            )
        };
        ctx.fill_rect(self.rect, bg);
        let text_style = TextStyle::fg(fg);
        let line_h = ctx.line_height(&text_style);
        for (i, r) in self.title_rects().into_iter().enumerate() {
            let Some(entry) = self.entries.get(i) else {
                continue;
            };
            let is_open = self.opened == Some(i);
            let is_hover = self.hovered == Some(i);
            let is_active = self.active == Some(i);
            if is_open || is_hover {
                ctx.fill_rect(r, hi_bg);
            }
            let style = if is_active {
                TextStyle::fg(fg)
            } else {
                TextStyle::fg(muted)
            };
            let ty = r.y + (r.h - line_h) * 0.5;
            ctx.draw_text(
                Pos::new(r.x + TITLE_PAD * self.scale, ty),
                &entry.title,
                &style,
            );
        }
    }

    fn semantics(&self) -> Semantics {
        let mut s = Semantics::default()
            .exposed(true)
            .label(self.label.clone())
            .prop("menus", self.entries.len().to_string())
            .state("active", self.active.map(|i| i as u32).unwrap_or(u32::MAX));
        for (i, e) in self.entries.iter().enumerate() {
            s = s.prop(format!("menu{i}"), &e.title);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use y901_ui::UiTree;

    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
            Size::new(t.chars().count() as f32 * 7.0, 14.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            14.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    fn bar() -> MenuBar {
        MenuBar::new("bar")
            .menu(MenuEntry::new(
                "File",
                vec!["New".into(), "Open".into()],
                "file",
            ))
            .menu(MenuEntry::new("Edit", vec!["Undo".into()], "edit"))
            .menu(MenuEntry::new("View", vec!["Zoom".into()], "view"))
    }

    fn arrange(b: &mut MenuBar) {
        let mut cache = LayoutCache::new();
        b.measure(
            &Metrics,
            Constraints::loose(Size::new(400.0, 40.0)),
            &mut cache,
        );
        b.arrange(Rect::new(0.0, 0.0, 400.0, BAR_H), &Metrics, &mut cache);
    }

    #[test]
    fn lays_out_titles_left_to_right_without_overlap() {
        let mut b = bar();
        arrange(&mut b);
        let rects = b.title_rects();
        assert_eq!(rects.len(), 3);
        for w in rects.windows(2) {
            assert!(w[0].right() <= w[1].x, "标题不应重叠");
        }
        assert_eq!(rects[0].h, BAR_H);
        assert_eq!(b.len(), 3);
        assert!(!b.is_empty());
    }

    #[test]
    fn hit_test_maps_points_to_the_right_title() {
        let mut b = bar();
        arrange(&mut b);
        let rects = b.title_rects();
        for (i, r) in rects.iter().enumerate() {
            assert_eq!(b.title_at(Pos::new(r.x + 1.0, r.y + 1.0)), Some(i));
        }
        assert_eq!(
            b.title_at(Pos::new(rects[2].right() + 50.0, 5.0)),
            None,
            "超出最后一个标题不应命中"
        );
        assert_eq!(
            b.title_at(Pos::new(5.0, BAR_H + 10.0)),
            None,
            "在栏外不命中"
        );
    }

    #[test]
    fn click_opens_a_popup_menu() {
        // 走真实 UiTree：request_popup 是"分发结束后挂载"，只有整棵树才能验证。
        let mut tree = UiTree::new(Box::new(bar()));
        tree.layout(Size::new(400.0, 300.0), &Metrics);
        let r = tree.rect_of("bar").expect("bar 已布局");
        let _ = tree.dispatch(&UiEvent::MouseUp {
            pos: Pos::new(r.x + 10.0, r.y + 10.0),
            button: y901_ui::MouseButton::Left,
        });
        assert!(tree.has_popup(), "点击菜单标题应挂载浮层");
        assert!(
            tree.popup_rect("bar.menu0").is_some(),
            "浮层 id 应是 bar.menu0"
        );
    }

    #[test]
    fn keyboard_moves_between_titles_and_opens() {
        let mut b = bar();
        arrange(&mut b);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 200.0));
        b.event(&mut ctx, &UiEvent::Focus(true));
        assert_eq!(b.active_index(), Some(0));
        b.event(
            &mut ctx,
            &UiEvent::Key {
                key: Key::Right,
                modifiers: y901_ui::Modifiers::default(),
            },
        );
        assert_eq!(b.active_index(), Some(1));
        b.event(
            &mut ctx,
            &UiEvent::Key {
                key: Key::Left,
                modifiers: y901_ui::Modifiers::default(),
            },
        );
        assert_eq!(b.active_index(), Some(0));
        b.event(
            &mut ctx,
            &UiEvent::Key {
                key: Key::Escape,
                modifiers: y901_ui::Modifiers::default(),
            },
        );
        assert_eq!(b.active_index(), None);
    }

    #[test]
    fn semantics_lists_menus_and_active_index() {
        let mut b = bar();
        arrange(&mut b);
        let s = b.semantics();
        let props: Vec<_> = s
            .props
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert!(props.contains(&("menus", "3")));
        assert!(props.contains(&("menu0", "File")));
        assert!(props.contains(&("menu2", "View")));
        assert_eq!(
            s.state.get("active").and_then(|v| v.as_f64()),
            Some(u32::MAX as f64)
        );
    }
}
