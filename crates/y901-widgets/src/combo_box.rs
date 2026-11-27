//! ComboBox：下拉选择（走框架 popup 浮层）。
//!
//! 闭合态：输入框样式 + 当前值 + 右侧 ▾。
//! 展开态：**由 `ctx.request_popup` 弹出的独立列表 widget（`ComboList`）**，
//! 因此**不受父容器裁剪**，且自带框架的 150ms 弹出动画。
//!
//! 已知代价（框架 popup 的固有限制，已接受）：popup 内容与父组件**不共享状态**，
//! 所以选中结果通过 `Rc<Cell<Option<usize>>>` 回传，ComboBox 在**下一次事件**
//! （`Tick` / `MouseUp`）里读取并落库、再触发回调。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::widget::FlexItem;
use y901_ui::{
    Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Role, Semantics, Size, State,
    TextMetrics, TextStyle, UiEvent, Widget,
};

/// 闭合框高度。
const BOX_H: f32 = 32.0;
/// 列表项高度。
const ITEM_H: f32 = 28.0;
/// 列表内边距。
const LIST_PAD: f32 = 4.0;
/// 列表最小宽度。
const LIST_MIN_W: f32 = 180.0;

/// 下拉选择。
pub struct ComboBox {
    id: String,
    items: Vec<String>,
    selected: Option<usize>,
    /// 选中项变化时发出的命令（args = {"index": i}）。
    command: Option<y901_ui::Command>,
    /// 待发送的命令（`absorb_shared` 没有 `ctx`，由 `event` 代发）。
    pending_command: Option<y901_ui::Command>,
    /// popup 侧写入的选中项（由 `ComboList` 设置）。
    shared: Rc<Cell<Option<usize>>>,
    rect: Rect,
    item: FlexItem,
    /// 可选样式覆盖（仅容器型表面）。
    style: Option<crate::style::WidgetStyle>,
    /// **DPI 缩放**（`measure` 里唯一写入点；未 measure 时 1.0 = 兜底）。
    scale: f32,
}

impl ComboBox {
    /// 创建下拉框。
    pub fn new(id: impl Into<String>, items: Vec<impl Into<String>>) -> Self {
        Self {
            id: id.into(),
            items: items.into_iter().map(Into::into).collect(),
            selected: None,
            command: None,
            pending_command: None,
            shared: Rc::new(Cell::new(None)),
            rect: Rect::ZERO,
            item: FlexItem::default(),
            style: None,
            scale: 1.0,
        }
    }

    /// 预选中项。
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index.filter(|i| *i < self.items.len());
        self
    }

    /// 选中变化回调。
    pub fn command(mut self, cmd: y901_ui::Command) -> Self {
        self.command = Some(cmd);
        self
    }

    /// **覆盖容器型表面**（背景 / 描边 / 圆角）。
    ///
    /// 注意：语义色（如 switch 轨道、error 红、empty 灰）**不**由它覆盖 ——
    /// 那些走 `Theme`，否则会破坏 DARK/LIGHT 自适应。
    pub fn style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// 设置 flex 约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 当前选中项。
    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    /// 浮层 id（`push_popup` 用它去重）。
    fn popup_id(&self) -> String {
        format!("{}-list", self.id)
    }

    /// 闭合框矩形。
    fn box_rect(&self) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y,
            self.rect.w,
            (BOX_H * self.scale).min(self.rect.h),
        )
    }

    /// 当前显示的文字。
    fn display_text(&self) -> &str {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(String::as_str)
            .unwrap_or("请选择")
    }

    /// 把 popup 回传的选中项落库；返回 true = 有变化（需要重绘）。
    fn absorb_shared(&mut self) -> bool {
        let Some(i) = self.shared.take() else {
            return false;
        };
        if i >= self.items.len() {
            return false;
        }
        // **同值不重复发命令**：三态列表里"又选了当前项"很常见，
        // 若每次都发 `select`，应用会收到一串重复事件（幂等但噪音大，
        // 而且上位机可能把它当成"用户确实改了"）。
        if self.selected == Some(i) {
            return false;
        }
        self.selected = Some(i);
        if let Some(cmd) = self.command.as_ref() {
            let args = y901_ui::json::Json::obj().set("index", i as i32);
            self.pending_command = Some(crate::command_with_args(cmd, args));
        }
        true
    }
}

impl Widget for ComboBox {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::BUTTON
    }

    fn focusable(&self) -> bool {
        true
    }

    fn measure(
        &mut self,
        m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.scale = m.scale();
        c.constrain(Size::new(c.max.w, BOX_H * self.scale))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    /// 需要每帧 `Tick`：用来**吸收 popup 回传的选中项**
    /// （popup 与父组件不共享状态，只能靠共享单元 + Tick 同步）。
    fn wants_tick(&self) -> bool {
        true
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        // 先吸收 popup 的选中结果（上次点击写入的）
        if matches!(event, UiEvent::Tick | UiEvent::MouseUp { .. }) {
            let changed = self.absorb_shared();
            if let Some(cmd) = self.pending_command.take() {
                ctx.emit_command(cmd);
            }
            if changed {
                return EventResult::Redraw;
            }
        }
        match event {
            // **闭合态键盘**：↑↓ 直接改选中（不打开列表），Enter 打开列表。
            UiEvent::Key { key, .. } => {
                let n = self.items.len();
                let changed = match key {
                    y901_ui::Key::Down if n > 0 => {
                        let cur = self.selected.unwrap_or(0);
                        let next = (cur + 1).min(n - 1);
                        if Some(next) != self.selected {
                            self.selected = Some(next);
                            if let Some(cmd) = self.command.as_ref() {
                                let args = y901_ui::json::Json::obj().set("index", next as i32);
                                self.pending_command = Some(crate::command_with_args(cmd, args));
                            }
                            true
                        } else {
                            false
                        }
                    }
                    y901_ui::Key::Up if n > 0 => {
                        let cur = self.selected.unwrap_or(0);
                        let next = cur.saturating_sub(1);
                        if Some(next) != self.selected {
                            self.selected = Some(next);
                            if let Some(cmd) = self.command.as_ref() {
                                let args = y901_ui::json::Json::set(
                                    y901_ui::json::Json::obj(),
                                    "index",
                                    next as i32,
                                );
                                self.pending_command = Some(crate::command_with_args(cmd, args));
                            }
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                if changed {
                    if let Some(cmd) = self.pending_command.take() {
                        ctx.emit_command(cmd);
                    }
                    return EventResult::Redraw;
                }
                EventResult::Ignored
            }
            UiEvent::MouseUp { pos, .. } if self.box_rect().contains(*pos) => {
                ctx.request_focus(self.id.clone());
                let popup_id = self.popup_id();
                let items = self.items.clone();
                let selected = self.selected;
                let shared = self.shared.clone();
                let list = ComboList {
                    id: popup_id,
                    items,
                    selected,
                    shared,
                    // 初始高亮 = 当前选中项（没有则 0）
                    highlight: selected.unwrap_or(0),
                    width: self.box_rect().w.max(LIST_MIN_W),
                    rect: Rect::ZERO,
                };
                ctx.request_popup(Box::new(move || Box::new(list)), self.box_rect());
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (box_bg, box_border, text_fg, muted) = {
            let theme = ctx.theme();
            let input = theme.style(Role::TEXT_INPUT, State::Normal);
            (
                input.bg,
                input.border,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.muted_text_color(),
            )
        };
        let b = self.box_rect();
        // **样式覆盖**：底色 / 描边 / 圆角
        let box_bg = self.style.and_then(|s| s.bg).unwrap_or(box_bg);
        let box_border = self.style.and_then(|s| s.border).unwrap_or(box_border);
        let radius = match self.style.map(|s| s.radius) {
            Some(r) if r > 0.0 => r,
            _ => 4.0 * self.scale,
        };
        ctx.fill_round_rect(b, box_bg, radius);
        ctx.stroke_round_rect(b, box_border, 1.0, radius);

        // **占位符不能和选中值同色**：未选中时 `display_text()` 返回
        // 占位提示，那是"还没选"的语义，应用 muted；选中后才用 text fg。
        // 之前两者共用 `text_fg`，于是"请选择"和真正的值看上去一样。
        let is_placeholder = self.selected.is_none();
        let style = TextStyle::fg(if is_placeholder { muted } else { text_fg });
        let ty = b.y + (b.h - ctx.line_height(&style)) * 0.5;
        ctx.draw_text(
            Pos::new(b.x + 10.0 * self.scale, ty),
            self.display_text(),
            &style,
        );

        let arrow = "▾";
        let astyle = TextStyle::fg(muted);
        let aw = ctx.measure_text(arrow, &astyle).w;
        ctx.draw_text(Pos::new(b.right() - 10.0 - aw, ty), arrow, &astyle);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("combobox {}", self.id))
            .value(self.display_text().to_string())
            .prop("items", self.items.len().to_string())
            .action("select")
    }
}

/// **私有**：ComboBox 弹出的列表（由 popup 层承载，因此不受父容器裁剪）。
struct ComboList {
    id: String,
    items: Vec<String>,
    selected: Option<usize>,
    shared: Rc<Cell<Option<usize>>>,
    /// 高亮项（键盘 ↑↓ 移动，**到边界停住**）。
    highlight: usize,
    width: f32,
    rect: Rect,
}

impl ComboList {
    /// 第 i 项矩形（与闭合框的位置无关：popup 层会按 anchor 摆放整块）。
    fn item_rect(&self, i: usize) -> Rect {
        Rect::new(
            self.rect.x,
            self.rect.y + LIST_PAD + i as f32 * ITEM_H,
            self.rect.w,
            ITEM_H,
        )
    }
}

impl Widget for ComboList {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::LIST
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        _c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        Size::new(
            self.width,
            self.items.len() as f32 * ITEM_H + LIST_PAD * 2.0,
        )
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
        // **键盘：** Esc 关闭（不改选中）、↑↓ 移动高亮（到边界**停住**）、Enter 确认。
        if let UiEvent::Key { key, .. } = event {
            match key {
                y901_ui::Key::Escape => {
                    ctx.request_close_popup(self.id.clone());
                    return EventResult::Redraw;
                }
                y901_ui::Key::Up => {
                    let next = self.highlight.saturating_sub(1);
                    if next != self.highlight {
                        self.highlight = next;
                        return EventResult::Redraw;
                    }
                    return EventResult::Handled;
                }
                y901_ui::Key::Down => {
                    let last = self.items.len().saturating_sub(1);
                    let next = (self.highlight + 1).min(last);
                    if next != self.highlight {
                        self.highlight = next;
                        return EventResult::Redraw;
                    }
                    return EventResult::Handled;
                }
                y901_ui::Key::Enter => {
                    if self.highlight < self.items.len() {
                        self.shared.set(Some(self.highlight));
                    }
                    ctx.request_close_popup(self.id.clone());
                    return EventResult::Redraw;
                }
                _ => return EventResult::Ignored,
            }
        }
        let UiEvent::MouseUp { pos, .. } = event else {
            return EventResult::Ignored;
        };
        for i in 0..self.items.len() {
            if self.item_rect(i).contains(*pos) {
                self.shared.set(Some(i));
                ctx.request_close_popup(self.id.clone());
                return EventResult::Redraw;
            }
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        let (bg, border, text_fg, hover_bg, sel_bg) = {
            let theme = ctx.theme();
            let input = theme.style(Role::TEXT_INPUT, State::Normal);
            (
                input.bg,
                input.border,
                theme.style(Role::TEXT, State::Normal).fg,
                theme.style(Role::LIST_ITEM, State::Hovered).bg,
                theme.style(Role::LIST_ITEM, State::Pressed).bg,
            )
        };
        ctx.fill_round_rect(self.rect, bg, 6.0);
        ctx.stroke_round_rect(self.rect, border, 1.0, 6.0);
        for i in 0..self.items.len() {
            let r = self.item_rect(i);
            if self.selected == Some(i) {
                ctx.fill_rect(r, sel_bg);
            } else if i == self.highlight {
                // **键盘高亮必须画出来**：旧实现只改了 `highlight` 字段，屏幕上
                // 唯一的标记是"已选中项"（或奇数行的斑马纹），用户按 ↓ 两次也看不见
                // Enter 会选哪一项。这里把键盘游标画成 hover 底色。
                ctx.fill_rect(r, hover_bg);
            } else if i % 2 == 1 {
                ctx.fill_rect(r, hover_bg);
            }
            let s = TextStyle::fg(text_fg);
            let ly = r.y + (r.h - ctx.line_height(&s)) * 0.5;
            ctx.draw_text(Pos::new(r.x + 10.0, ly), &self.items[i], &s);
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(format!("combobox list {}", self.id))
            .prop("items", self.items.len().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;

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

    #[test]
    fn click_box_requests_popup() {
        let mut cb = ComboBox::new("c", vec!["a", "b", "c"]).selected(Some(1));
        let mut cache = LayoutCache::new();
        cb.arrange(Rect::new(0.0, 0.0, 200.0, BOX_H), &Metrics, &mut cache);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert!(ctx.popup_requests.is_empty());
        let r = cb.event(
            &mut ctx,
            &UiEvent::MouseUp {
                pos: Pos::new(20.0, 10.0),
                button: y901_ui::MouseButton::Left,
            },
        );
        assert_eq!(r, EventResult::Redraw);
        assert_eq!(ctx.popup_requests.len(), 1, "点击闭合框应请求弹出列表");
        assert!(ctx.popup_requests[0].anchor.w > 0.0);
    }

    #[test]
    fn popup_writes_selection_back_to_combo() {
        let mut cb = ComboBox::new("c", vec!["a", "b"]).command(y901_ui::Command::app(
            "combo_box:select",
            y901_ui::json::Json::Null,
        ));
        let mut cache = LayoutCache::new();
        cb.arrange(Rect::new(0.0, 0.0, 200.0, BOX_H), &Metrics, &mut cache);
        // 模拟 popup 侧写入 index 1
        cb.shared.set(Some(1));
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert_eq!(cb.event(&mut ctx, &UiEvent::Tick), EventResult::Redraw);
        assert_eq!(cb.selected_index(), Some(1));
        assert_eq!(ctx.commands.len(), 1, "选中后应发出命令");
        // 第二次 Tick：已吸收，无变化
        assert_eq!(cb.event(&mut ctx, &UiEvent::Tick), EventResult::Ignored);
        assert_eq!(ctx.commands.len(), 1, "选中后应发出命令");
    }

    /// 构造一个已布局的 ComboBox，并把它的列表弹出物取出来。
    fn popup_list(selected: Option<usize>, n: usize) -> ComboList {
        let items: Vec<String> = (0..n).map(|i| format!("item{i}")).collect();
        let mut list = ComboList {
            id: "c-list".to_string(),
            items,
            selected,
            shared: Rc::new(Cell::new(None)),
            highlight: selected.unwrap_or(0),
            width: 200.0,
            rect: Rect::ZERO,
        };
        let mut cache = y901_ui::layout::LayoutCache::new();
        list.arrange(
            Rect::new(0.0, 0.0, 200.0, 3.0 * ITEM_H),
            &Metrics,
            &mut cache,
        );
        list
    }

    fn key(k: y901_ui::Key) -> UiEvent {
        UiEvent::Key {
            key: k,
            modifiers: y901_ui::Modifiers::default(),
        }
    }

    #[test]
    fn escape_closes_without_changing_selection() {
        let mut list = popup_list(Some(1), 3);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Escape)),
            EventResult::Redraw
        );
        assert_eq!(ctx.close_popup_requests.len(), 1, "Esc 应请求关闭浮层");
        assert_eq!(list.shared.get(), None, "Esc 不应改变选中");
    }

    #[test]
    fn down_moves_highlight_and_stops_at_bottom() {
        let mut list = popup_list(Some(0), 3);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Down)),
            EventResult::Redraw
        );
        assert_eq!(list.highlight, 1);
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Down)),
            EventResult::Redraw
        );
        assert_eq!(list.highlight, 2);
        // 已到最后一项：再按不动（停住，不循环）
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Down)),
            EventResult::Handled
        );
        assert_eq!(list.highlight, 2);
    }

    #[test]
    fn up_moves_highlight_and_stops_at_top() {
        let mut list = popup_list(Some(1), 3);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Up)),
            EventResult::Redraw
        );
        assert_eq!(list.highlight, 0);
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Up)),
            EventResult::Handled
        );
        assert_eq!(list.highlight, 0);
    }

    /// 键盘高亮**必须真的画在屏幕上**（旧实现只改字段，屏幕上看不出 Enter 会选谁）。
    ///
    /// 断言用"偶数行"：斑马纹只给奇数行上底色，因此一个**偶数行**被填色只可能
    /// 来自键盘高亮 —— 这样即使斑马纹策略改了，这条用例仍然只测高亮。
    #[test]
    fn keyboard_highlight_is_actually_painted_on_an_even_row() {
        struct Rec {
            fills: std::cell::RefCell<Vec<(Rect, y901_ui::Color)>>,
        }
        impl TextMetrics for Rec {
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
        impl RenderCtx for Rec {
            fn size(&self) -> Size {
                Size::new(400.0, 400.0)
            }
            fn fill_rect(&mut self, r: Rect, c: y901_ui::Color) {
                self.fills.borrow_mut().push((r, c));
            }
            fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
            fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
                0.0
            }
            fn push_clip(&mut self, _r: Rect) {}
            fn pop_clip(&mut self) {}
            // 圆角填充会绕开 `fill_rect`：这里退化成矩形填充，好让断言只看"哪块被上了色"。
            fn fill_round_rect(&mut self, r: Rect, c: y901_ui::Color, _radius: f32) {
                self.fills.borrow_mut().push((r, c));
            }
            fn stroke_round_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32, _radius: f32) {}
        }

        // 不选中任何项，让"已选中"底色不参与；从 highlight=0（偶数行）开始。
        let mut list = popup_list(None, 4);
        let mut arranged = EventCtx::new(None, Size::new(400.0, 400.0));
        let _ = list.event(&mut arranged, &key(y901_ui::Key::Down)); // highlight=1（奇数行，有斑马纹）
        let mut ctx = Rec {
            fills: std::cell::RefCell::new(Vec::new()),
        };
        list.render(&mut ctx);
        let paints = ctx.fills.borrow();
        // 收集"被上过色的行矩形"（按 y 聚合）。
        let filled_rows: Vec<f32> = paints
            .iter()
            .filter(|(r, _)| r.h > 1.0 && r.w > 1.0)
            .map(|(r, _)| r.y)
            .collect();
        assert!(
            !filled_rows.is_empty(),
            "渲染必须至少画了列表背景与某一行的高亮"
        );
        let row_h = ITEM_H;
        let highlighted_y = list.item_rect(1).y;
        assert!(
            filled_rows
                .iter()
                .any(|y| (y - highlighted_y).abs() < row_h * 0.5),
            "键盘高亮的第 2 行（y≈{highlighted_y}）必须被涂色，实际涂色行：{filled_rows:?}"
        );
        // 对照：第 3 行（偶数行、未被高亮）不应被涂色 —— 斑马纹不会给它上色。
        let even_y = list.item_rect(2).y;
        assert!(
            !filled_rows.iter().any(|y| (y - even_y).abs() < 1.0),
            "未高亮的偶数行不该有底色（有的话说明断言抓错了对象）：{filled_rows:?}"
        );
    }

    #[test]
    fn enter_commits_highlighted_item_and_closes() {
        let mut list = popup_list(Some(0), 3);
        let mut ctx = EventCtx::new(None, Size::new(400.0, 400.0));
        let _ = list.event(&mut ctx, &key(y901_ui::Key::Down));
        let _ = list.event(&mut ctx, &key(y901_ui::Key::Down));
        assert_eq!(
            list.event(&mut ctx, &key(y901_ui::Key::Enter)),
            EventResult::Redraw
        );
        assert_eq!(list.shared.get(), Some(2), "应提交高亮项");
        assert_eq!(ctx.close_popup_requests.len(), 1);
    }

    #[test]
    fn exposes_select_action() {
        let cb = ComboBox::new("c", vec!["a", "b"]);
        let acts = cb.semantics().actions;
        assert!(
            acts.iter().any(|a| a.name == "select"),
            "combo_box 应暴露 select"
        );
    }
}
