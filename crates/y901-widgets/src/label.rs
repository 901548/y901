//! 官方组件：文本标签。
//!
//! 只做一件事：把一段文字画出来。它是"如何写一个 Widget"的最小样本。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, FlexItem, MountCtx, Pos, Rect, RenderCtx, Role,
    Semantics, Size, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 静态文本；挂上 `Signal` 后由数据源驱动更新。
pub struct TextLabel {
    id: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
    text: String,
    /// 可选的响应式来源：挂载后由 Signal 驱动文本。
    signal: Option<y901_ui::Signal<String>>,
    color: Color,
    /// 是否加粗（UI 字重层级）。
    ///
    /// **为何只加粗不加字号**：`TextStyle` 目前没有 `size` 字段
    /// （字号来自全局 `FontConfig.size`），加字号需动框架度量链路与像素基准。
    /// 加粗只影响刷字字形选择，**不改度量口径**；但字形变粗会使
    /// `measure_text` 的宽度**变大**（见 `measure()`）。
    bold: bool,
    /// **字号**（逻辑像素）；`None` = 引擎默认字号。
    ///
    /// 与 `bold` 一样，它同时影响**度量**与**绘制** —— 两处必须同源
    /// （见 `to_text_style()`），否则宽度按默认算、绘制按实际字号画，布局错位。
    size: Option<f32>,
    item: FlexItem,
    rect: Rect,
}

impl TextLabel {
    /// 新建静态文本标签。
    ///
    /// 输入：`id`（树内唯一）、`text`（初始文本）。
    /// 返回：未挂载的组件；不带 Signal（文本不会自动更新）。
    /// 副作用：无。
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            id,
            key,
            text: text.into(),
            signal: None,
            color: Color::WHITE,
            bold: false,
            size: None,
            item: FlexItem::default(),
            rect: Rect::ZERO,
        }
    }

    /// 绑定一个 Signal<String>：值变化 → 框架自动标脏重绘。
    pub fn bind(mut self, signal: y901_ui::Signal<String>) -> Self {
        self.text = signal.get();
        self.signal = Some(signal);
        self
    }

    /// 当前应显示的文本（优先取 Signal）。
    fn current_text(&self) -> String {
        match &self.signal {
            Some(sig) => sig.get(),
            None => self.text.clone(),
        }
    }

    /// 覆盖文字颜色。
    ///
    /// 返回：自身。副作用：无（下次绘制生效；颜色不进脏区计算，
    /// 因此单独改颜色需要调用方自行 `invalidate`）。
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// 加粗文本（字重层级）。
    ///
    /// 返回：自身。副作用：无（下次绘制/布局生效）。
    /// **注意**：加粗后字形变宽，`measure()` 返回的宽度会变大，
    /// 若容器宽度刚好卡着，可能引入截断（由使用方评估）。
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// 设置字号（逻辑像素）。
    ///
    /// 返回：自身。副作用：无（下次布局/绘制生效）。
    /// 字号变大会使 `measure()` 返回的宽高都变大。
    pub fn size(mut self, px: f32) -> Self {
        self.size = Some(px);
        self
    }

    /// 设置 flex 伸缩参数。
    ///
    /// 返回：自身。副作用：无（父容器测量时读取）。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// **本控件的文本样式**。
    ///
    /// 为何提取成方法：`measure()` 与 `render()` **必须用同一份样式**。
    /// 两处各写一份的后果是——漏传 `bold` 就宽度少算，
    /// 漏传 `size` 就行高不足，都是"布局与绘制分家"。
    fn to_text_style(&self) -> TextStyle {
        let mut ts = TextStyle::fg(self.color);
        if self.bold {
            ts = ts.bold();
        }
        if let Some(px) = self.size {
            ts = ts.size(px);
        }
        ts
    }

    /// 当前应显示的文本（**分配一个新 `String`**）。
    ///
    /// 返回：绑定了 Signal 时取 Signal 的当前值，否则取内部文本。
    /// 副作用：无（但每次调用都会克隆，不要在渲染循环里反复调）。
    pub fn text(&self) -> String {
        self.current_text()
    }

    /// 直接替换内部文本（**不影响已绑定的 Signal**）。
    ///
    /// 输入：任意文本。返回：无。
    /// 副作用：改内部字段，**不**标记布局失效、**不**触发重绘——
    /// 挂载后调用需要调用方自己 `tree.mark_layout_dirty(id)` + `invalidate(id)`。
    /// 若绑定了 Signal，`text()` 仍会返回 Signal 的值（内部字段被忽略）。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Widget for TextLabel {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::TEXT
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // 与 `render()` **同源**：同一个 `to_text_style()`。
        let style = self.to_text_style();
        c.constrain(Size::new(
            metrics.measure_text(&self.current_text(), &style).w,
            metrics.line_height(&style),
        ))
    }

    /// 向下转型：Host 运行时调 `set_text` 的唯一路径（UiTree 不暴露具体类型）。
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
    fn arrange(
        &mut self,
        rect: Rect,
        _metrics: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        // 对外暴露动作，谁调都行，组件不关心调用者
        if let UiEvent::Action { name, args } = event {
            if name == "set_text" {
                if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                    self.text = text.to_string();
                    return EventResult::Relayout;
                }
            }
            if name == "set_color" {
                let Some(raw) = args.get("color").and_then(|v| v.as_str()) else {
                    // 参数缺失：不处理（网关会因 !handled 返回 400）
                    return EventResult::Ignored;
                };
                let Ok(color) = parse_color(raw) else {
                    // 格式非法：同样交给网关报 400（不静默改成默认色）
                    return EventResult::Ignored;
                };
                self.color = color;
                return EventResult::Redraw;
            }
        }
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.w <= 0.0 || self.rect.h <= 0.0 {
            return;
        }
        // **垂直居中**：以前直接用 `rect.y`（行框顶），于是字形贴在上边沿。
        // 按"行框高 vs 容器高"的差值下移一半；容器比行框矮时保持贴顶（不上溢）。
        let style = self.to_text_style();
        let line_h = ctx.line_height(&style);
        let dy = ((self.rect.h - line_h) * 0.5).max(0.0);
        ctx.draw_text(Pos::new(self.rect.x, self.rect.y + dy), &self.current_text(), &style);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
            .exposed(true)
            .label(self.current_text())
            .value(self.current_text())
            // 历史 bug：这里原本是 `.action("set_text")`（InputKind::None），
            // 但 event 里读的是 `args.text` —— 声明与实现不符，
            // 网关会以 "expects None argument" 拒绝任何带参数的 set_text。
            .action_spec("set_text", y901_ui::InputKind::Text)
            // 颜色是"视觉属性"，但外部工具改它属于**有副作用的状态变更**，
            // 因此声明为动作（走 invoke + if_rev），格式见 `parse_color`。
            .action_spec_named("set_color", y901_ui::InputKind::Text, "color")
    }

    fn on_mount(&mut self, ctx: &mut MountCtx) {
        // bind() 之后无需手动订阅：挂载时自动接上失效队列
        if let Some(sig) = &self.signal {
            sig.subscribe(ctx.id.clone(), ctx.invalidation.clone());
        }
    }

    fn on_unmount(&mut self) {
        if let Some(sig) = &self.signal {
            sig.unsubscribe(&self.id);
        }
    }
}

impl TextLabel {
    /// 用 Signal 的当前值刷新快照文本。
    pub fn refresh(&mut self) {
        if let Some(sig) = &self.signal {
            self.text = sig.get();
        }
    }
}

/// 解析颜色字符串：**只支持 `#rrggbb` 与 `#rrggbbaa`**。
///
/// 输入：`"#ff0000"`（RGB）或 `"#ff000080"`（RGBA，最后两位是 alpha）。
/// 返回：`Color`（分量 0..=1）。
/// 失败：长度不是 7/9、缺 `#`、含非 hex 字符 → `Err(说明支持格式的字符串)`。
///
/// 设计取舍：不支持命名颜色（`"red"`）与 `rgba(...)` 函数式写法——
/// 多一种格式就多一条解析分支与一类边界，而外部工具生成 hex 很容易。
#[doc(hidden)]
pub fn parse_color(raw: &str) -> Result<Color, String> {
    let err = || "颜色格式必须是 #rrggbb 或 #rrggbbaa（例如 #ff0000 / #ff000080）".to_string();
    let hex = raw.strip_prefix('#').ok_or_else(err)?;
    let byte = |i: usize| -> Result<u8, String> {
        u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| err())
    };
    match hex.len() {
        6 => Ok(Color::rgba(
            byte(0)? as f32 / 255.0,
            byte(2)? as f32 / 255.0,
            byte(4)? as f32 / 255.0,
            1.0,
        )),
        8 => Ok(Color::rgba(
            byte(0)? as f32 / 255.0,
            byte(2)? as f32 / 255.0,
            byte(4)? as f32 / 255.0,
            byte(6)? as f32 / 255.0,
        )),
        _ => Err(err()),
    }
}
