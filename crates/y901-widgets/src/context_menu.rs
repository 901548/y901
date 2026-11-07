//! ContextMenu：右键点击处弹出菜单。
//!
//! 复用现有 `Menu` 作为浮层内容（不另造菜单控件）；本控件只负责：
//! 把右键位置变成 `anchor`、把菜单挂上浮层、并把选中结果转成回调。

use std::rc::Rc;

/// 选中回调类型（抽名避免 clippy 的复杂类型警告）。
type SelectCallback = Rc<dyn Fn(usize, &str)>;

use crate::overlay::Menu;
use y901_ui::{
    Constraints, EventCtx, EventResult, MouseButton, Rect, RenderCtx, Role, Semantics, Size,
    TextMetrics, UiEvent, Widget,
};

/// 右键菜单宿主。
pub struct ContextMenu {
    id: String,
    items: Vec<String>,
    /// 选中回调（索引、标题）
    on_select: Option<SelectCallback>,
    /// 触发区域（通常铺满父容器）
    rect: Rect,
}

impl ContextMenu {
    /// 创建右键菜单宿主。
    pub fn new(id: impl Into<String>, items: Vec<String>) -> Self {
        Self {
            id: id.into(),
            items,
            on_select: None,
            rect: Rect::ZERO,
        }
    }

    /// 选中回调。
    pub fn on_select(mut self, f: impl Fn(usize, &str) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// 浮层里菜单的 id（关闭时要用）。
    pub fn menu_id(&self) -> String {
        format!("{}__menu", self.id)
    }
}

impl Widget for ContextMenu {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, c.max.h))
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
        // 只处理右键：左键交给下层（选择、拖拽等）
        let UiEvent::MouseDown { pos, button } = event else {
            return EventResult::Ignored;
        };
        if *button != MouseButton::Right || !self.rect.contains(*pos) {
            return EventResult::Ignored;
        }
        if self.items.is_empty() {
            return EventResult::Ignored;
        }

        // anchor 用鼠标位置（零尺寸矩形）：`open_popup` 会以它为锚点做边界翻转，
        // 于是"贴着屏幕右下角右键"时菜单会自动往上/往左翻。
        let anchor = Rect::new(pos.x, pos.y, 0.0, 0.0);
        let id = self.menu_id();
        let items = self.items.clone();
        let cb = self.on_select.clone();
        ctx.request_popup(
            Box::new(move || {
                let mut menu = Menu::new(id).items(items);
                if let Some(cb) = cb {
                    menu = menu.on_select(move |i, label| cb(i, label));
                }
                // **必须显式 open**：Menu 的 render/event 都以 `open` 为门。
                // 实测教训：不调用它时浮层虽然挂上了，但 Menu 自己不画、不响应，
                // 表现为"右键什么都没发生"。
                menu.open();
                Box::new(menu)
            }),
            anchor,
        );
        EventResult::Handled
    }

    fn render(&mut self, _ctx: &mut dyn RenderCtx) {
        // 宿主本身不画任何东西（它只是"右键接收区"）。
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("context menu {}", self.id))
            .prop("items", self.items.len().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc as StdRc;
    use y901_render::soft::SoftBackend;
    use y901_render::MyFontEngine;
    use y901_ui::{Pos, UiTree};

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

    fn tree_with(hits: StdRc<Cell<usize>>) -> UiTree {
        let menu = ContextMenu::new("ctx", vec!["复制".into(), "粘贴".into(), "删除".into()])
            .on_select(move |_i, _l| hits.set(hits.get() + 1));
        let mut t = UiTree::new(Box::new(menu));
        t.layout(Size::new(W as f32, H as f32), &soft());
        t
    }

    fn right_click(pos: Pos) -> UiEvent {
        UiEvent::MouseDown {
            pos,
            button: MouseButton::Right,
        }
    }

    #[test]
    fn right_click_opens_a_menu_popup() {
        let hits = StdRc::new(Cell::new(0));
        let mut tree = tree_with(hits);
        tree.dispatch(&right_click(Pos::new(40.0, 40.0)));
        assert_eq!(tree.popups().len(), 1, "右键后应出现一个浮层");
        eprintln!("[ctx] 右键后浮层数 = {}", tree.popups().len());
    }

    #[test]
    fn left_click_does_not_open_the_menu() {
        let hits = StdRc::new(Cell::new(0));
        let mut tree = tree_with(hits);
        tree.dispatch(&UiEvent::MouseDown {
            pos: Pos::new(40.0, 40.0),
            button: MouseButton::Left,
        });
        assert!(tree.popups().is_empty(), "左键不应弹出右键菜单");
    }

    #[test]
    fn menu_is_painted_with_real_pixels() {
        let mut soft = soft();
        let hits = StdRc::new(Cell::new(0));
        let mut tree = tree_with(hits);

        // 基线：未弹出
        tree.invalidate_all();
        soft.clear();
        tree.render(&mut soft);
        let before = soft.pixels().to_vec();

        // 右键弹出后再渲染
        tree.dispatch(&right_click(Pos::new(60.0, 60.0)));
        tree.layout(Size::new(W as f32, H as f32), &soft);
        tree.invalidate_all();
        soft.clear();
        tree.render(&mut soft);
        let after = soft.pixels().to_vec();

        let changed = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        eprintln!("[ctx] 弹出前后差异像素 = {changed}");

        // 为什么用"差异像素"而不是"非背景像素"：Menu 会在整个可见区盖一层 25% 黑色遮罩，
        // 任何一个像素都"不等于遮罩前的背景"，因此在弹出帧里找不到可用的背景基准。
        assert!(changed > 100, "菜单应有可见内容，实测变化像素 {changed}");
    }

    #[test]
    fn clicking_a_menu_item_fires_the_callback_and_closes_the_popup() {
        let hits = StdRc::new(Cell::new(0));
        let mut tree = tree_with(hits.clone());
        let soft = soft();
        tree.dispatch(&right_click(Pos::new(40.0, 40.0)));
        tree.layout(Size::new(W as f32, H as f32), &soft);

        // 用新增的 `popup_as_mut` 拿到浮层里的具体类型，从而知道"点哪里命中第 0 项"
        let item = tree
            .popup_as_mut::<Menu>("ctx__menu")
            .and_then(|m| m.item_rect_at(0))
            .expect("应能拿到菜单的第 0 项矩形");

        tree.dispatch(&UiEvent::MouseDown {
            pos: Pos::new(item.x + item.w * 0.5, item.y + item.h * 0.5),
            button: MouseButton::Left,
        });

        assert_eq!(hits.get(), 1, "点菜单项应触发一次回调");
        assert_eq!(tree.popups().len(), 0, "点菜单项后浮层应关闭");
    }

    #[test]
    fn popup_as_mut_returns_none_for_the_wrong_type() {
        let hits = StdRc::new(Cell::new(0));
        let mut tree = tree_with(hits);
        tree.dispatch(&right_click(Pos::new(40.0, 40.0)));
        // 类型不对 → None（不 panic）
        assert!(tree
            .popup_as_mut::<crate::button::Button>("ctx__menu")
            .is_none());
        // id 不对 → None
        assert!(tree.popup_as_mut::<Menu>("nope").is_none());
        // 正确类型 → Some
        assert!(tree.popup_as_mut::<Menu>("ctx__menu").is_some());
    }

    #[test]
    fn clicking_outside_closes_the_popup() {
        let hits = StdRc::new(Cell::new(0));
        let mut tree = tree_with(hits.clone());
        tree.dispatch(&right_click(Pos::new(40.0, 40.0)));
        let soft = soft();
        tree.layout(Size::new(W as f32, H as f32), &soft);
        assert_eq!(tree.popups().len(), 1);

        // 点一个远离菜单的位置（右下角）
        tree.dispatch(&UiEvent::MouseDown {
            pos: Pos::new(W as f32 - 2.0, H as f32 - 2.0),
            button: MouseButton::Left,
        });
        assert_eq!(tree.popups().len(), 0, "点击外部应关闭菜单");
        assert_eq!(hits.get(), 0, "点击外部不应触发选中回调");
    }

    #[test]
    fn empty_menu_does_not_open() {
        let mut t = UiTree::new(Box::new(ContextMenu::new("ctx", Vec::new())));
        t.layout(Size::new(W as f32, H as f32), &soft());
        t.dispatch(&right_click(Pos::new(20.0, 20.0)));
        assert!(t.popups().is_empty(), "没有菜单项时不应弹出空菜单");
    }
}
