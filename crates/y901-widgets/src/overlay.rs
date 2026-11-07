//! 浮层组件：`Menu`（Overlay）与 `Dialog`（Modal）。
//!
//! 都设计成**全屏宿主**：应用把它们放进 `Stack`（所有子节点同尺寸），
//! 于是它们的 rect 就是整个视口 —— 这样"点外部关闭/吞掉"才有几何依据。

use y901_ui::{
    Command, Constraints, Edges, EventCtx, EventResult, FlexItem, Json, Pos, Rect, RenderCtx, Role,
    Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 菜单项高度。
const ITEM_H: f32 = 26.0;
/// 菜单面板内边距。
const PAD: f32 = 6.0;

/// 选中回调类型。
type SelectCallback = Box<dyn Fn(usize, &str)>;

/// 下拉/右键菜单。作为浮层内容时需先调 `open()`——`render`/`event` 都以它为门。
pub struct Menu {
    id: String,
    items: Vec<String>,
    /// 触发时发给应用层的命令名。
    action: String,
    open: bool,
    /// 全屏 rect（宿主给的）。
    rect: Rect,
    /// 面板 rect（内容区）。
    panel: Rect,
    hover: Option<usize>,
    highlighted: usize,
    item: FlexItem,
    /// 选中回调（与 `action` 并存：前者给应用层，后者给构造者）。
    on_select: Option<SelectCallback>,
}

impl Menu {
    /// 新建下拉菜单（初始关闭，命令名默认 `menu.select`）。
    ///
    /// 输入：`id`（树内唯一）。返回：空菜单（无选项）。
    /// 副作用：无。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            action: "menu.select".to_string(),
            open: false,
            rect: Rect::ZERO,
            panel: Rect::ZERO,
            hover: None,
            highlighted: 0,
            item: FlexItem::default(),
            on_select: None,
        }
    }

    /// 设置菜单项（顺序即显示顺序）。
    ///
    /// 返回：自身。副作用：无（不重置 `highlighted` 与 `open`）。
    pub fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }

    /// 选中菜单项时发给应用层的命令名（默认 `"menu.select"`）。
    ///
    /// 输入：命令名。返回：自身。
    /// 副作用：无。命令参数固定为 `{index, label}`；框架不认识这个名字，
    /// 由 `Host::command` 处理。
    pub fn action(mut self, name: impl Into<String>) -> Self {
        self.action = name.into();
        self
    }

    /// 选中某项时回调（参数：索引、标题）。
    ///
    /// 为什么需要它：`action` 把命令发给**应用层**，
    /// 而“右键菜单”往往只想在构造处接一个回调，不得不去应用层转一圈。
    pub fn on_select(mut self, f: impl Fn(usize, &str) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }

    /// 第 `i` 项的屏幕矩形（布局后有效）。
    ///
    /// 对外可见的理由：鼠标点选是菜单的主要交互，
    /// 使用方与测试需要知道"点哪里能命中第 i 项"，不应去猜。
    pub fn item_rect_at(&self, i: usize) -> Option<Rect> {
        if i >= self.items.len() {
            return None;
        }
        Some(self.item_rect(i))
    }

    /// 确认选中第 `i` 项：先回调、再发命令、最后关闭自己与它所在的浮层。
    ///
    /// 为什么要 `request_close_popup`：菜单作为**浮层内容**时，`self.close()`
    /// 只改自己的 `open` 标志，浮层本身仍在树上——看起来就是"菜单没关掉"。
    fn commit_selection(&mut self, ctx: &mut EventCtx, i: usize) {
        let label = self.items.get(i).cloned().unwrap_or_default();
        if let Some(cb) = self.on_select.as_ref() {
            cb(i, &label);
        }
        ctx.emit_app(
            self.action.clone(),
            Json::obj().set("index", i as u32).set("label", label),
        );
        ctx.request_close_popup(self.id.clone());
        self.close();
    }

    /// 打开菜单，并把高亮重置到第一项。
    ///
    /// 返回：无。副作用：无（不会自动聚焦；需要键盘导航时由应用层 `set_focus`）。
    pub fn open(&mut self) {
        self.open = true;
        self.highlighted = 0;
    }

    /// 当前高亮的项下标（键盘 Up/Down 或悬停会改它）。
    ///
    /// 返回：`0..items.len()`；菜单为空时恒为 0。副作用：无。
    /// 注意：它是"键盘光标"不是"选中项"——选中要通过命令发给应用层。
    pub fn highlighted(&self) -> usize {
        self.highlighted
    }

    /// 关闭菜单，并清掉悬停态。
    ///
    /// 返回：无。副作用：无（不重置 `highlighted`，下次 `open` 会重置）。
    pub fn close(&mut self) {
        self.open = false;
        self.hover = None;
    }

    /// 菜单是否已打开（关闭时不绘制、不命中、不进索引）。
    ///
    /// 返回：`true` = 可见可交互。副作用：无。
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// 菜单面板的屏幕矩形（点击命中就看它）。
    pub fn panel_rect(&self) -> Rect {
        self.panel
    }

    fn item_rect(&self, index: usize) -> Rect {
        Rect::new(
            self.panel.x,
            self.panel.y + PAD + index as f32 * ITEM_H,
            self.panel.w,
            ITEM_H,
        )
    }
}

impl Widget for Menu {
    /// 向下转型：Host 运行时调 `open` / `close` 的唯一路径。
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("menu")
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(c.max)
    }

    fn arrange(
        &mut self,
        rect: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        let w = 220.0_f32.min((rect.w - 16.0).max(0.0));
        let h = PAD * 2.0 + self.items.len() as f32 * ITEM_H;
        // 用统一的弹层摆放：下方放不下向上翻转，右侧放不下向左平移，
        // 且不会超出父区域（命中矩形与绘制矩形都派生自 `panel`，天然同步）。
        let anchor = Rect::new(rect.x + 16.0, rect.y + 16.0, 0.0, 0.0);
        self.panel = y901_ui::place_popup(anchor, y901_ui::Size::new(w, h), rect);
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn clip(&self) -> Option<Rect> {
        Some(self.rect)
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if !self.open {
            return EventResult::Ignored;
        }
        match event {
            // 语义动作：外部工具不需要坐标，直接选第 index 项
            UiEvent::Action { name, args } if name == "select" => {
                let index = args.get("index").and_then(|v| v.as_f64()).unwrap_or(-1.0);
                if index < 0.0 || index as usize >= self.items.len() {
                    return EventResult::Ignored;
                }
                let i = index as usize;
                self.commit_selection(ctx, i);
                // 关闭会改变 is_open：必须重建索引，否则该项仍留在 paint_order 里
                EventResult::Relayout
            }
            UiEvent::MouseMove { pos } => {
                let hover = (0..self.items.len()).find(|i| self.item_rect(*i).contains(*pos));
                if hover != self.hover {
                    self.hover = hover;
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }
            UiEvent::MouseDown { pos, .. } => {
                if self.panel.contains(*pos) {
                    if let Some(i) =
                        (0..self.items.len()).find(|i| self.item_rect(*i).contains(*pos))
                    {
                        self.commit_selection(ctx, i);
                    }
                }
                // 无论点哪里，菜单都关闭；并吞掉这次点击，避免穿透到底层
                self.close();
                ctx.stop_propagation();
                EventResult::Relayout
            }
            UiEvent::Key {
                key: y901_ui::Key::Escape,
                ..
            } => {
                self.close();
                EventResult::Relayout
            }
            UiEvent::Key { key, .. } if !self.items.is_empty() => match key {
                y901_ui::Key::Down => {
                    self.highlighted = (self.highlighted + 1) % self.items.len();
                    EventResult::Redraw
                }
                y901_ui::Key::Up => {
                    self.highlighted = (self.highlighted + self.items.len() - 1) % self.items.len();
                    EventResult::Redraw
                }
                y901_ui::Key::Enter => {
                    let i = self.highlighted.min(self.items.len() - 1);
                    ctx.emit_app(
                        self.action.clone(),
                        Json::obj()
                            .set("index", i as u32)
                            .set("label", self.items[i].clone()),
                    );
                    self.close();
                    EventResult::Relayout
                }
                y901_ui::Key::Escape => {
                    self.close();
                    EventResult::Relayout
                }
                _ => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if !self.open {
            return;
        }
        // 半透明遮罩：视觉上把注意力收到菜单上
        ctx.fill_rect(self.rect, y901_ui::Color::rgba(0.0, 0.0, 0.0, 0.25));

        let panel_style = ctx.theme().style(Role("panel"), State::Normal);
        ctx.fill_rect(self.panel, panel_style.bg);
        ctx.stroke_rect(self.panel, y901_ui::Color::rgba(0.35, 0.38, 0.45, 1.0), 1.0);

        for (i, label) in self.items.iter().enumerate() {
            let r = self.item_rect(i);
            if self.hover == Some(i) || self.highlighted == i {
                let hover = ctx.theme().style(Role::LIST_ITEM, State::Hovered);
                ctx.fill_rect(r, hover.bg);
            }
            let fg = ctx.theme().style(Role::LIST_ITEM, State::Normal).fg;
            ctx.draw_text(Pos::new(r.x + 10.0, r.y + 4.0), label, &TextStyle::fg(fg));
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label("menu")
            .action_spec("select", y901_ui::InputKind::Number)
            .state("open", self.open)
            .state("items", self.items.len() as u32)
    }
}

/// 模态对话框：吃掉所有外部点击，焦点由框架自动移入。
pub struct Dialog {
    id: String,
    title: String,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    panel: Rect,
    open: bool,
    /// 底部按钮条：(文案, 动作名)。
    buttons: Vec<(String, String)>,
    /// arrange 时算出的按钮矩形，与 buttons 一一对应。
    button_rects: Vec<Rect>,
    item: FlexItem,
    /// 面板尺寸（运行时可由 `set_size` 改；默认 420×220）。
    panel_size: Size,
    /// 外部注入的面板样式（`None` = 走主题）。
    panel_style: Option<crate::style::WidgetStyle>,
    /// 底部按钮样式（`None` = 走主题）。
    button_style: Option<crate::style::WidgetStyle>,
    /// 是否画全屏遮罩（`false` 时可作为普通弹窗挂在 popup 层）。
    scrim: bool,
}

impl Dialog {
    /// 新建模态对话框（初始关闭，无内容、无按钮）。
    ///
    /// 输入：`id`（树内唯一）、`title`（标题栏文字）。
    /// 返回：未挂载的对话框。副作用：无。
    /// 注意：打开后焦点被限制在子树内（Modal 作用域）。
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            children: Vec::new(),
            rect: Rect::ZERO,
            panel: Rect::ZERO,
            open: false,
            buttons: Vec::new(),
            button_rects: Vec::new(),
            item: FlexItem::default(),
            panel_size: Size::new(420.0, 220.0),
            panel_style: None,
            button_style: None,
            scrim: true,
        }
    }

    /// 追加一个内容子节点（按调用顺序纵向排列）。
    ///
    /// 返回：自身。副作用：无（每个子节点都会在 Dialog 的 `children_mut` 里被排布）。
    pub fn push(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }

    /// 打开对话框（Modal 层：吞外部点击、焦点限制在子树内）。
    ///
    /// 返回：无。副作用：无（框架在下一次布局时执行 `enforce_modal_focus`，
    /// 自动把焦点移到第一个可聚焦子节点）。
    pub fn open(&mut self) {
        self.open = true;
    }

    /// 运行时修改标题。
    ///
    /// 输入：新标题。返回：无。
    /// 副作用：只改字段——**不自动标脏**（组件碰不到 `UiTree`，这是分层原则）。
    /// 调用方随后必须 `tree.invalidate(id)`（标题不影响布局，不需要 mark_layout_dirty）。
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = title.into();
    }

    /// 运行时替换按钮条。
    ///
    /// 输入：`(文案, 动作名)` 列表；动作名由应用层解释。
    /// 副作用：**清空 `button_rects`**（旧矩形与按钮数不匹配会导致命中错位），
    /// 等下一次 `arrange` 重建。**不自动标脏**：调用方随后必须
    /// `tree.mark_layout_dirty(id)` + `tree.invalidate(id)`（按钮数变化影响布局）。
    pub fn set_buttons(&mut self, buttons: Vec<(String, String)>) {
        self.buttons = buttons;
        self.button_rects.clear();
    }

    /// 运行时清空全部子节点（并 `on_unmount` 它们）。
    ///
    /// 副作用：子节点被 drop；**不自动标脏**——调用方随后必须
    /// `tree.mark_layout_dirty(id)` + `tree.invalidate(id)`。
    pub fn clear_children(&mut self) {
        for mut child in self.children.drain(..) {
            child.on_unmount();
        }
    }

    /// 运行时追加子节点（纵向排列，见 `arrange`）。
    ///
    /// 副作用：新子节点**不会被自动 `on_mount`**（它还没进树）。
    /// 如果这个 Dialog 已在树里，请改用 `tree.add_node(dialog_id, child)`，
    /// 那条路径会挂载、判重、标脏并请求重排。本方法只用于"组装还没挂载的 Dialog"。
    pub fn push_child(&mut self, child: Box<dyn Widget>) {
        self.children.push(child);
    }

    /// 当前面板矩形（`arrange` 之后有效；未布局时是 `Rect::ZERO`）。
    ///
    /// 用途：测试与诊断（例如断言 `set_size` 真的生效）——不是渲染或命中路径。
    /// 注意它已经过窗口约束（`min(panel_size, 父矩形 - 40)`），因此可能小于
    /// `set_size` 传入的值。
    pub fn panel_rect(&self) -> Rect {
        self.panel
    }

    /// 运行时设置面板尺寸（clamp 到 200..=800 × 150..=600）。
    ///
    /// 输入：期望宽高（逻辑像素）。负值/NaN 会被 clamp 到下限。
    /// 副作用：只改字段——**不自动标脏**。调用方随后必须
    /// `tree.mark_layout_dirty(id)` + `tree.invalidate(id)`，否则新尺寸不会生效。
    /// 实际绘制尺寸还会被窗口约束：`min(panel_size, 父矩形 - 40)`。
    pub fn set_size(&mut self, width: f32, height: f32) {
        let clamp = |v: f32, lo: f32, hi: f32| {
            if v.is_finite() {
                v.clamp(lo, hi)
            } else {
                lo
            }
        };
        self.panel_size = Size::new(clamp(width, 200.0, 800.0), clamp(height, 150.0, 600.0));
    }

    /// 运行时设置打开状态（`open()` / `close()` 的二合一）。
    ///
    /// 副作用：只改字段。
    ///
    /// **调用方必须 `tree.invalidate_all()`**（不是 `invalidate(id)`）——
    /// `is_open` 变化影响全局 `paint_order` / `modal_root` / `focus_order`，
    /// 不只是该节点矩形。只标该节点会导致索引/焦点仍按旧状态（bug：焦点会跑进
    /// 已关闭的对话框）。同时需要 `tree.mark_layout_dirty(id)` 让索引重建。
    /// 注入面板样式（背景渐变/圆角/描边）。
    ///
    /// 输入：`Some(style)` = 覆盖；`None` = 回到主题。
    /// 注意：不会自动标脏（调用方需 `tree.invalidate(id)`）。
    pub fn set_panel_style(&mut self, style: Option<crate::style::WidgetStyle>) -> bool {
        if self.panel_style == style {
            return false;
        }
        self.panel_style = style;
        true
    }

    /// 注入 Dialog 内部确认/取消按钮的样式。
    ///
    /// 与 [`Dialog::set_panel_style`] 一样不自动标脏；调用方随后必须
    /// `tree.invalidate(id)`。返回 `false` 表示样式未变化。
    pub fn set_button_style(&mut self, style: Option<crate::style::WidgetStyle>) -> bool {
        if self.button_style == style {
            return false;
        }
        self.button_style = style;
        true
    }

    /// 构建期直接设置按钮样式，等价于 `set_button_style(Some(style))`。
    pub fn button_style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.button_style = Some(style);
        self
    }

    /// 关闭全屏遮罩（当它作为 `open_modal_popup` 的内容时，
    /// 遮罩已由浯层层统一绘制，再画一层会叠加变黑）。
    pub fn set_scrim(&mut self, on: bool) -> bool {
        if self.scrim == on {
            return false;
        }
        self.scrim = on;
        true
    }

    /// 直接设置开启状态（不重置高亮）；用于由宿主控制弹层生命周期。
    pub fn set_open(&mut self, open: bool) {
        self.open = open;
    }

    /// 子节点（inherent 版本，调用方不必 `use y901_ui::Widget`）。
    pub fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    /// 配置底部按钮条：`[("Cancel", "cancel"), ("OK", "ok")]`。
    pub fn buttons(mut self, buttons: Vec<(impl Into<String>, impl Into<String>)>) -> Self {
        self.buttons = buttons
            .into_iter()
            .map(|(label, action)| (label.into(), action.into()))
            .collect();
        self
    }

    /// 关闭对话框（不触发 `on_close` 回调——`Dialog` 没有回调，
    /// 关闭动作通过命令发给应用层）。
    ///
    /// 返回：无。副作用：无。框架会把焦点还给之前的节点（若有）。
    pub fn close(&mut self) {
        self.open = false;
    }

    /// 是否已打开（关闭时不绘制、不命中、不进索引）。
    ///
    /// 返回：`true` = 模态生效中。副作用：无。
    pub fn is_open(&self) -> bool {
        self.open
    }
}

impl Widget for Dialog {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role("dialog")
    }

    /// 模态语义由 `is_modal` 表达（开着就阻塞底层）。
    ///
    /// `layer()` 保留 `Modal` 仅为可见性（画在最上层），不再驱动焦点限制。
    fn is_modal(&self) -> bool {
        self.open
    }

    fn is_open(&self) -> bool {
        self.open
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

    /// 向下转型：`Host` 要在运行时调 `set_title` / `set_buttons` / `set_open`，
    /// 唯一的公开路径是 `tree.find_mut(id)?.as_any_mut()`（`UiTree` 不暴露具体类型）。
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(c.max)
    }

    fn arrange(
        &mut self,
        rect: Rect,
        m: &dyn TextMetrics,
        cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        let w = self.panel_size.w.min(rect.w - 40.0);
        let h = self.panel_size.h.min(rect.h - 40.0);
        self.panel = Rect::new(
            rect.x + (rect.w - w) * 0.5,
            rect.y + (rect.h - h) * 0.5,
            w,
            h,
        );

        let inner = self.panel.inset(Edges::all(16.0));
        // 底部预留按钮条（36px），body 相应缩短
        let bar_h = if self.buttons.is_empty() { 0.0 } else { 36.0 };
        let body = Rect::new(
            inner.x,
            inner.y + 28.0,
            inner.w,
            (inner.h - 28.0 - bar_h).max(0.0),
        );
        // 子节点纵向排列（**全部**，不是只有第一个）。
        //
        // 历史 bug：这里原本只 `children.first_mut()`，导致 `push_child` 追加的
        // 第二个子节点永远拿不到矩形（既不排布也不会被画在正确位置）。
        // 现在按顺序排：每个子节点先测高，再给剩余高度，累加间距；
        // 剩余高度不足时给 0（不 panic、不越过 body 底部）。
        {
            const GAP: f32 = 8.0;
            let mut y = body.y;
            for child in self.children.iter_mut() {
                let remaining = (body.bottom() - y).max(0.0);
                let size =
                    child.measure(m, Constraints::loose(Size::new(body.w, remaining)), cache);
                let h = size.h.min(remaining);
                child.arrange(Rect::new(body.x, y, body.w, h), m, cache);
                y += h + GAP;
            }
        }

        // 按钮条：右对齐排布，等宽 88px，间距 8px
        self.button_rects.clear();
        if !self.buttons.is_empty() {
            const BTN_W: f32 = 88.0;
            const GAP: f32 = 8.0;
            let n = self.buttons.len() as f32;
            let total = n * BTN_W + (n - 1.0) * GAP;
            let mut x = inner.x + (inner.w - total).max(0.0);
            let y = self.panel.bottom() - 16.0 - 28.0;
            for _ in 0..self.buttons.len() {
                self.button_rects.push(Rect::new(x, y, BTN_W, 28.0));
                x += BTN_W + GAP;
            }
        }
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if !self.open {
            return EventResult::Ignored;
        }
        match event {
            // 语义动作：外部工具直接关闭对话框
            UiEvent::Action { name, .. } if name == "close" => {
                self.close();
                // is_open 变化必须触发重建：索引/脏区都基于"打开时"的快照，
                // 只 Redraw 会让关闭的节点继续留在 paint_order 里。
                EventResult::Relayout
            }
            // 按钮命中优先于"吞掉外部点击"
            UiEvent::MouseDown { pos, .. } => {
                // Capture 阶段不拦截，让子节点先看到事件
                if ctx.phase == y901_ui::EventPhase::Capture {
                    return EventResult::Ignored;
                }
                let hit = self
                    .button_rects
                    .iter()
                    .position(|r| r.contains(*pos))
                    .map(|i| (i, self.buttons[i].1.clone()));
                if let Some((_, action)) = hit {
                    ctx.emit_app(action, Json::obj());
                    self.close();
                    return EventResult::Relayout;
                }
                // 面板内的点击放行给子节点；只吞面板外的
                if self.panel.contains(*pos) {
                    return EventResult::Ignored;
                }
                ctx.stop_propagation();
                EventResult::Handled
            }
            UiEvent::MouseMove { .. } | UiEvent::Wheel { .. } => {
                // 面板内的事件交给子节点；只有面板外才吞掉
                if let UiEvent::MouseMove { pos } = event {
                    if self.panel.contains(*pos) {
                        return EventResult::Ignored;
                    }
                }
                ctx.stop_propagation();
                EventResult::Handled
            }
            UiEvent::Key {
                key: y901_ui::Key::Escape,
                ..
            } => {
                self.close();
                EventResult::Relayout
            }
            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if !self.open {
            return;
        }
        if self.scrim {
            ctx.fill_rect(self.rect, y901_ui::Color::rgba(0.0, 0.0, 0.0, 0.55));
        }
        let panel_style = ctx.theme().style(Role("panel"), State::Normal);
        match self.panel_style {
            Some(style) if style.has_bg() => {
                style.paint_bg(ctx, self.panel);
                style.paint_border(ctx, self.panel);
            }
            _ => {
                ctx.fill_rect(self.panel, panel_style.bg);
                ctx.stroke_rect(self.panel, y901_ui::Color::rgba(0.40, 0.44, 0.52, 1.0), 1.0);
            }
        }
        ctx.draw_text(
            Pos::new(self.panel.x + 16.0, self.panel.y + 10.0),
            &self.title,
            &TextStyle::fg(panel_style.fg),
        );

        // 底部按钮条：颜色全部来自主题
        for (i, (label, _)) in self.buttons.iter().enumerate() {
            let Some(r) = self.button_rects.get(i) else {
                break;
            };
            let style = ctx.theme().style(Role::BUTTON, State::Normal);
            if let Some(button_style) = self.button_style {
                if button_style.has_bg() {
                    button_style.paint_bg(ctx, *r);
                    button_style.paint_border(ctx, *r);
                } else {
                    ctx.fill_rect(*r, style.bg);
                }
            } else {
                ctx.fill_rect(*r, style.bg);
            }
            let w = ctx.measure_text(label, &TextStyle::fg(style.fg)).w;
            ctx.draw_text(
                Pos::new(r.x + (r.w - w) * 0.5, r.y + 4.0),
                label,
                &TextStyle::fg(style.fg),
            );
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.title.clone())
            .action("close")
            .state("open", self.open)
    }
}

/// 便捷构造：菜单项命令。
pub fn menu_command(menu: &str, open: bool) -> Command {
    Command::node(menu, if open { "open" } else { "close" }, Json::obj())
}
