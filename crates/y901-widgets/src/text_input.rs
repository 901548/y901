//! 单行文本输入框。
//!
//! 完全不依赖平台：键盘、鼠标、IME 都只是 `UiEvent`，
//! 出图只经 `RenderCtx`，文本逻辑全在 [`TextBuffer`]。
//!
//! 关键设计：**事件里没有字体度量**，所以鼠标点击无法直接调用
//! `offset_for_x`。解决办法是在 measure/render 时缓存一份"字符 → X"的排版表，
//! 事件阶段查表即可，O(log n) 定位，不需要把 `TextMetrics` 塞进事件。

use std::time::Duration;
use web_time::Instant;

use y901_ui::text::TextBuffer;
use y901_ui::{
    Color, Constraints, Dimension, Edges, EventCtx, EventResult, FlexItem, Json, Key, Modifiers,
    Pos, Rect, RenderCtx, Role, Semantics, Size, State, TextMetrics, TextStyle, UiEvent, Widget,
};

/// 光标闪烁周期。
const BLINK: Duration = Duration::from_millis(530);
/// 光标线宽。
const CARET_W: f32 = 1.5;

/// 剪贴板：组件只认这个接口，具体实现由应用层注入。
pub trait Clipboard {
    /// 读取剪贴板文本；`None` 表示不可用或为空（控件只需安静地不粘）。
    fn get(&mut self) -> Option<String>;
    /// 写入剪贴板；失败被忽略——剪贴板不可用不应阻断 UI。
    fn set(&mut self, text: &str);
}

/// 默认空实现：没有系统剪贴板时复制/粘贴退化为无操作。
#[derive(Default)]
pub struct NullClipboard;

impl Clipboard for NullClipboard {
    fn get(&mut self) -> Option<String> {
        None
    }
    fn set(&mut self, _text: &str) {}
}

/// 内存实现：测试与无系统剪贴板的平台用。
#[derive(Default)]
pub struct MemoryClipboard {
    text: String,
}

impl Clipboard for MemoryClipboard {
    fn get(&mut self) -> Option<String> {
        if self.text.is_empty() {
            None
        } else {
            Some(self.text.clone())
        }
    }
    fn set(&mut self, text: &str) {
        self.text = text.to_string();
    }
}

/// 排版表里的一项：某个字符的起始字节、X 起点、宽度。
#[derive(Clone, Copy, Debug)]
struct GlyphX {
    byte: usize,
    x: f32,
    w: f32,
}

/// 单行文本输入框：光标、选区、滚动、剪贴板、IME。
pub struct TextInput {
    id: String,
    /// 预算好的 FNV-1a key（避免每次布局重算字符串哈希）。
    key: u64,
    buffer: TextBuffer,
    style: TextStyle,
    selection_color: Color,
    caret_color: Color,
    padding: Edges,
    item: FlexItem,
    rect: Rect,

    /// 字符排版表（measure/render 时按需重建）。
    layout: Vec<GlyphX>,
    layout_width: f32,
    layout_dirty: bool,

    /// 水平滚动：文本比可视区宽时，保证光标可见。
    scroll_x: f32,
    /// 最近一次 arrange 得到的内容宽度。
    inner_w: f32,

    focused: bool,
    /// 外部注入的控件样式（`None` = 走主题）。
    /// 注意：不能叫 `style`——本结构已有 `style: TextStyle`（文本样式）。
    widget_style: Option<crate::style::WidgetStyle>,
    dragging: bool,
    caret_visible: bool,
    last_blink: Instant,

    /// IME 预编辑串：画成覆盖层，不进 buffer。
    preedit: String,

    clipboard: Box<dyn Clipboard>,
    /// 内容版本：文本变化时 +1，供布局缓存判断。
    version: u64,
    /// 禁用：不参与 Tab，也不接收事件。
    disabled: bool,
    /// 密码模式：渲染与语义全部打码（原文仍可从 `text()` 读到）。
    password: bool,
    /// 占位提示（不进 buffer，空文本时临时绘制）。
    placeholder: Option<String>,
}

impl TextInput {
    /// 新建一个空输入框，默认样式（深色底 + 白字），未聚焦。
    ///
    /// 输入：`id`（树内唯一）。返回：未挂载的组件。
    /// 副作用：无（不读剪贴板、不订阅）。
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let key = y901_ui::layout::id_key(&id);
        Self {
            id,
            key,
            buffer: TextBuffer::new(),
            style: TextStyle::fg(Color::WHITE).with_bg(Color::rgba(0.10, 0.11, 0.14, 1.0)),
            selection_color: Color::rgba(0.24, 0.45, 0.75, 0.55),
            caret_color: Color::rgba(0.85, 0.90, 1.0, 1.0),
            padding: Edges::symmetric(8.0, 4.0),
            item: FlexItem::default(),
            rect: Rect::ZERO,
            layout: Vec::new(),
            layout_width: 0.0,
            layout_dirty: true,
            scroll_x: 0.0,
            inner_w: 0.0,
            focused: false,
            widget_style: None,
            dragging: false,
            caret_visible: true,
            last_blink: Instant::now(),
            preedit: String::new(),
            clipboard: Box::new(NullClipboard),
            version: 0,
            disabled: false,
            password: false,
            placeholder: None,
        }
    }

    /// 密码模式（构建器）：渲染与 `semantics()` 全部打码，`text()` 仍返回原文。
    ///
    /// 打码范围：正文每个字符画 `*`（空格也画，否则长度不可见）、IME 预编辑串
    /// 同样打码、`semantics().label/value/state("text")` 全部替换为 `*`，
    /// 并置 `sensitive = true`（网关二次脱敏）。
    pub fn password(mut self, on: bool) -> Self {
        self.password = on;
        self
    }

    /// 运行时开关密码模式（**不自动标脏**：调用方随后必须 `invalidate(id)`，
    /// 因为渲染结果变了但尺寸不变，不需要 `mark_layout_dirty`）。
    pub fn set_password(&mut self, on: bool) {
        self.password = on;
    }

    /// 占位提示（构建器）：**不进 `TextBuffer`**，只在
    /// `buffer.text().is_empty() && preedit.is_empty()` 时以暗灰绘制。
    ///
    /// 不参与布局、不参与语义；文本一变就自动消失。
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// 设置初始文本，并把光标放到末尾。
    ///
    /// 输入：任意文本（含中文/emoji）。返回：自身（构建器）。
    /// 副作用：版本号 +1、标记布局失效（挂载后调用会触发一次重排）。
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.buffer = TextBuffer::from_text(text);
        self.version += 1;
        self.layout_dirty = true;
        self
    }

    /// 覆盖文本样式（前景/背景/粗体等）。
    ///
    /// 返回：自身。副作用：标记布局失效（字体变化会影响测量宽度）。
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self.layout_dirty = true;
        self
    }

    /// 设置内边距（文字与边框的距离）。
    ///
    /// 返回：自身。副作用：影响 `measure` 结果，挂载后改需自行触发重排。
    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    /// 设置 flex 伸缩参数（宽/高策略）。
    ///
    /// 返回：自身。副作用：无（只在父容器 `measure` 时被读取）。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 禁用后不参与 Tab 遍历，也不接收任何事件。
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 替换剪贴板实现（默认 `NullClipboard`：Ctrl+C/V 无效果）。
    ///
    /// 输入：`Box<dyn Clipboard>`（应用层接系统剪贴板）。
    /// 返回：自身。副作用：无（只换实现，不读写）。
    pub fn with_clipboard(mut self, clipboard: Box<dyn Clipboard>) -> Self {
        self.clipboard = clipboard;
        self
    }

    // ---------- 只读访问（测试 / 语义树） ----------

    /// 运行时设置文本（预填用）：重置光标到末尾、清空选区与预编辑。
    ///
    /// 输入：任意文本（含中文/emoji；非法 UTF-8 由 `&str` 类型排除）。
    /// 副作用：`version += 1` → `layout_version()` 变 → 测量缓存失效；
    /// **不自动标脏**（组件碰不到 `UiTree`）：调用方随后必须
    /// `tree.mark_layout_dirty(id)` + `tree.invalidate(id)`。
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.buffer = TextBuffer::from_text(text);
        self.version += 1;
        self.layout_dirty = true;
        self.preedit.clear();
    }

    /// 清空文本、光标、选区与预编辑（等价于 `set_text("")`）。
    ///
    /// 副作用：同 [`TextInput::set_text`]（版本 +1、需调用方标脏）。
    pub fn clear(&mut self) {
        self.set_text("");
    }

    /// 当前文本内容（不含 IME 预编辑串）。
    ///
    /// 返回：`&str`（借用内部缓冲，零拷贝）。副作用：无。
    /// 当前预编辑串（IME 组字中的拼音）。
    ///
    /// **读取用**：预编辑串与 `buffer` 是两份独立状态，提交（`UiEvent::Text`）
    /// 时必须清空，否则会与提交后的正文重叠渲染。
    pub fn preedit(&self) -> &str {
        &self.preedit
    }

    /// 当前正文（不含预编辑串）。
    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// 底层文本缓冲（含光标/选择/撤销等全部状态）。
    ///
    /// 返回：只读引用。副作用：无。复杂编辑逻辑（按行取文本等）走它。
    pub fn buffer(&self) -> &TextBuffer {
        &self.buffer
    }

    /// 光标字节偏移（不是字符数；中文/emoji 各占多字节）。
    ///
    /// 返回：`0..=len_bytes()`。副作用：无。
    /// 注意：用它切字符串前必须确认落在 UTF-8 边界上（`TextBuffer` 保证是）。
    pub fn cursor(&self) -> usize {
        self.buffer.cursor()
    }

    /// 选择锚点（Shift+方向键选区的另一端）。
    ///
    /// 返回：字节偏移；没有选区时等于 `cursor()`。副作用：无。
    pub fn anchor(&self) -> usize {
        self.buffer.anchor()
    }

    /// 当前选区（已排序的 `[start, end)` 字节区间）。
    ///
    /// 返回：`None` = 无选区（光标与锚点重合）。副作用：无。
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.buffer.selection()
    }

    /// 组件自己记录的聚焦状态（框架 `Focus` 事件的镜像）。
    ///
    /// 返回：`true` 表示正在接收键盘输入。副作用：无。
    ///
    /// **契约**：权威来源是 `UiTree::focus()`；正常路径下两者一致
    /// （框架派发 `UiEvent::Focus(bool)` 时本字段同步更新）。
    /// 若出现不一致，说明有代码绕过了框架直接改 `focused` 字段——那是调用方的 bug，
    /// 不是 `is_focused` 的责任（它只如实反映镜像值）。
    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// 水平滚动偏移（文本比可视区宽时，为保证光标可见而滚动）。
    ///
    /// 返回：像素偏移（0 表示没滚动）。副作用：无。
    pub fn scroll_x(&self) -> f32 {
        self.scroll_x
    }

    // ---------- 内部 ----------

    fn touch(&mut self) {
        self.version += 1;
        self.layout_dirty = true;
        self.caret_visible = true;
        self.last_blink = Instant::now();
    }

    /// 用任意"字符 → 宽度"函数重建排版表。
    fn layout_with(&mut self, mut width_of: impl FnMut(char) -> f32) {
        self.layout.clear();
        let mut pen = 0.0;
        for (byte, ch) in self.buffer.text().char_indices() {
            let w = width_of(ch).max(0.0);
            self.layout.push(GlyphX { byte, x: pen, w });
            pen += w;
        }
        self.layout_width = pen;
        self.layout_dirty = false;
    }

    fn layout_from_metrics(&mut self, metrics: &dyn TextMetrics) {
        if !self.layout_dirty {
            return;
        }
        let style = self.style;
        self.layout_with(|ch| TextBuffer::char_width(ch, metrics, &style));
    }

    /// 字节偏移 → 行内 X。落不到字符上时按文本末尾处理。
    fn x_of_byte(&self, byte: usize) -> f32 {
        if self.layout.is_empty() {
            return 0.0;
        }
        match self.layout.binary_search_by_key(&byte, |g| g.byte) {
            Ok(idx) => self.layout[idx].x,
            Err(_) => self.layout_width,
        }
    }

    /// 行内 X → 最近的插入点（字符左半边取前、右半边取后）。
    fn byte_at_x(&self, x: f32) -> usize {
        if self.layout.is_empty() {
            return 0;
        }
        if x <= 0.0 {
            return 0;
        }
        for g in &self.layout {
            let mid = g.x + g.w * 0.5;
            if x < mid {
                return g.byte;
            }
        }
        self.buffer.len_bytes()
    }

    /// 让光标保持在可视区内。
    fn ensure_caret_visible(&mut self) {
        if self.inner_w <= 0.0 {
            return;
        }
        let caret = self.x_of_byte(self.buffer.cursor());
        let max_scroll = (self.layout_width - self.inner_w).max(0.0);

        if caret - self.scroll_x > self.inner_w {
            self.scroll_x = caret - self.inner_w;
        } else if caret < self.scroll_x {
            self.scroll_x = caret;
        }
        self.scroll_x = self.scroll_x.clamp(0.0, max_scroll);
    }

    /// 鼠标 X（屏幕坐标）→ 字节偏移。
    fn byte_at_screen_x(&self, x: f32) -> usize {
        let inner_x = self.rect.x + self.padding.left;
        let local = x - inner_x + self.scroll_x;
        self.byte_at_x(local)
    }

    fn handle_key(&mut self, key: Key, mods: Modifiers) -> EventResult {
        // Ctrl 组合优先
        if mods.ctrl {
            match key {
                Key::Char('a') | Key::Char('A') => {
                    self.buffer.select_all();
                    return EventResult::Redraw;
                }
                Key::Char('c') | Key::Char('C') => {
                    if let Some(text) = self.buffer.selected_text() {
                        let owned = text.to_string();
                        self.clipboard.set(&owned);
                    }
                    return EventResult::Handled;
                }
                Key::Char('v') | Key::Char('V') => {
                    if let Some(text) = self.clipboard.get() {
                        self.buffer.insert_str(&text);
                        self.touch();
                        return EventResult::Redraw;
                    }
                    return EventResult::Handled;
                }
                _ => return EventResult::Ignored,
            }
        }

        // **组字中不碰正文**：预编辑串非空时，删字归输入法管。
        //
        // 正常链路里 DOM 层已经把组字中的按键过滤掉（`event.isComposing`），
        // 但那是"不抛给我们"而不是"我们不能被误伤"。一旦下游直接投递
        // `UiEvent::Key`（测试、RPC、将来的其他后端），这里不设防就会删掉已提交的正文。
        if !self.preedit.is_empty() && matches!(key, Key::Backspace | Key::Delete) {
            return EventResult::Handled;
        }

        match key {
            Key::Backspace => {
                if self.buffer.backspace() {
                    self.touch();
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Delete => {
                if self.buffer.delete_forward() {
                    self.touch();
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Left => {
                if self.buffer.move_left(mods.shift) {
                    self.caret_visible = true;
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Right => {
                if self.buffer.move_right(mods.shift) {
                    self.caret_visible = true;
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                }
            }
            Key::Home => {
                self.buffer.move_line_start(mods.shift);
                EventResult::Redraw
            }
            Key::End => {
                self.buffer.move_line_end(mods.shift);
                EventResult::Redraw
            }
            // 单行输入：回车不插入换行
            Key::Enter => EventResult::Handled,
            Key::Char(c) if !c.is_control() => {
                self.buffer.insert_char(c);
                self.touch();
                EventResult::Redraw
            }
            _ => EventResult::Ignored,
        }
    }

    /// 注入控件样式（背景/圆角/描边/焦点圈）。返回 `self` 便于链式构造。
    ///
    /// 与 [`TextInput::style`] 的区别：那个是**文本**样式（字色/字体），
    /// 这个是**控件**样式（背景/圆角/描边）。
    pub fn widget_style(mut self, style: crate::style::WidgetStyle) -> Self {
        self.widget_style = Some(style);
        self
    }
}

impl Widget for TextInput {
    fn id(&self) -> &str {
        &self.id
    }

    fn role(&self) -> Role {
        Role::TEXT_INPUT
    }

    fn focusable(&self) -> bool {
        true
    }

    fn is_disabled(&self) -> bool {
        self.disabled
    }

    fn cursor(&self) -> y901_ui::Cursor {
        y901_ui::Cursor::Text
    }

    fn revision(&self) -> u64 {
        self.version
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    /// 向下转型：Host 需要拿到 `&mut TextInput` 本体（例如读光标、设选区）时用。
    ///
    /// 只读文本**优先**走 `semantics().value`（不需要 downcast）。
    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn layout_version(&self) -> u64 {
        self.version
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        self.layout_from_metrics(metrics);

        let line_h = metrics.line_height(&self.style);
        let padding_h = self.padding.horizontal();
        let padding_v = self.padding.vertical();

        // **宽度不能跟着内容走**。
        //
        // 旧实现：非定宽时返回 `layout_width + padding_h`，于是每敲一个字，
        // measure 就变宽 → 父 Flex 重算分配 → 整棵子树的 rect 变化 →
        // `layout()` 的"旧 ∪ 新"把卡片（昪 26%）甚至全屏标脏。
        // web 上表现就是"打字卡一下"。
        //
        // 现在：**单行输入框的宽度由父级分配决定**。
        // - 定宽（`Dimension::Px`）→ 用那个宽度；
        // - 有效上界（Flex 分配的剩余空间）→ **直接填满它**；
        // - 上界无限（例如被 loose 测量）→ 才回退到内容宽度。
        // 内容变长只影响内部滚动（`ensure_caret_visible`），不再影响布局。
        let content_w = match self.item.width {
            Dimension::Px(v) => v,
            _ if constraints.max.w.is_finite() => constraints.max.w,
            _ => self.layout_width + padding_h,
        };
        constraints.constrain(Size::new(
            content_w.max(padding_h + 1.0),
            line_h + padding_v,
        ))
    }

    fn arrange(
        &mut self,
        rect: Rect,
        metrics: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = rect;
        self.inner_w = (rect.w - self.padding.horizontal()).max(0.0);
        self.layout_from_metrics(metrics);
        self.ensure_caret_visible();
    }

    /// 只有获焦时才需要每帧 `Tick`（光标闪烁）；未获焦时不占用每帧广播。
    fn wants_tick(&self) -> bool {
        self.focused
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, ctx: &mut EventCtx, event: &UiEvent) -> EventResult {
        if self.disabled {
            return EventResult::Ignored;
        }
        match event {
            UiEvent::Focus(on) => {
                self.focused = *on;
                self.caret_visible = *on;
                self.last_blink = Instant::now();
                EventResult::Redraw
            }

            // ---- 鼠标 ----
            UiEvent::MouseDown { pos, button } => {
                if !matches!(button, y901_ui::MouseButton::Left) {
                    return EventResult::Ignored;
                }
                if !self.rect.contains(*pos) {
                    return EventResult::Ignored;
                }
                // 只**请求**焦点，不在这里假定成功：请求可能被 Modal 作用域拒绝。
                // 真正的 focused 只由框架的 Focus(true) 事件设置（单一真相）。
                ctx.request_focus(self.id.clone());
                // 捕获指针：拖出控件后仍能收到 MouseMove（拖拽选中靠它）
                ctx.set_pointer_capture(self.id.clone());
                self.dragging = true;

                let byte = self.byte_at_screen_x(pos.x);
                self.buffer.set_cursor(byte);
                EventResult::Redraw
            }

            UiEvent::MouseMove { pos } => {
                if !self.dragging {
                    return EventResult::Ignored;
                }
                // 拖拽扩选：锚点保持按下时的位置，光标跟随鼠标
                let byte = self.byte_at_screen_x(pos.x);
                let anchor = self.buffer.anchor();
                self.buffer.set_selection(anchor, byte);
                self.caret_visible = true;
                EventResult::Redraw
            }

            UiEvent::MouseUp { .. } => {
                if self.dragging {
                    self.dragging = false;
                    ctx.release_pointer_capture();
                    return EventResult::Handled;
                }
                EventResult::Ignored
            }

            // ---- 文本（键盘输入 / 粘贴 / IME 提交）----
            UiEvent::Text(text) => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                // **提交即消费预编辑串**：`Compose` 只写 `preedit`，`Text` 才写 `buffer`。
                // 若不清，上一次 `compositionupdate` 的字串会继续被画在游标处，
                // 看上去就像"多出一个字"。
                self.preedit.clear();
                self.buffer.insert_str(text);
                self.touch();
                // **布局不变**：宽度已由父级锁死（见 `measure`），
                // 文字变长只影响内部滚动，那在 `render` 里完成。
                EventResult::Redraw
            }

            UiEvent::Key { key, modifiers } => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                self.handle_key(*key, *modifiers)
            }

            // ---- IME 预编辑 ----
            UiEvent::Compose(text) => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                self.preedit = text.clone();
                // 让应用层把 IME 候选窗钉在光标处
                let caret_x =
                    self.rect.x + self.padding.left + self.x_of_byte(self.buffer.cursor())
                        - self.scroll_x;
                let caret_y = self.rect.y + self.padding.top;
                ctx.set_ime_candidate(Pos::new(caret_x, caret_y + 16.0));
                EventResult::Redraw
            }

            // ---- 光标闪烁 ----
            UiEvent::Tick => {
                if !self.focused {
                    return EventResult::Ignored;
                }
                if self.last_blink.elapsed() >= BLINK {
                    self.caret_visible = !self.caret_visible;
                    self.last_blink = Instant::now();
                    EventResult::Redraw
                } else {
                    EventResult::Ignored
                }
            }

            // ---- RPC 动作 ----
            UiEvent::Action { name, args } => match name.as_str() {
                "set_text" => {
                    let text = args
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.buffer = TextBuffer::from_text(text);
                    self.touch();
                    EventResult::Redraw
                }
                "focus" => {
                    // 同上：只请求，等框架的 Focus(true) 回来再置位
                    ctx.request_focus(self.id.clone());
                    EventResult::Redraw
                }
                "select_all" => {
                    self.buffer.select_all();
                    EventResult::Redraw
                }
                "insert_text" => {
                    let text = args
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    self.buffer.insert_str(&text);
                    self.touch();
                    EventResult::Redraw
                }
                _ => EventResult::Ignored,
            },

            _ => EventResult::Ignored,
        }
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let rect = self.rect;
        let inner = rect.inset(self.padding);
        if inner.is_empty() {
            return;
        }

        // **0) 光标跟随内部滚动**。
        //
        // 为什么放在渲染而不是 `arrange`：新状态机里编辑只返回
        // `Redraw`（宽度已锁死，布局不会变），因此 `arrange` 不会被再次调用。
        // 把 `ensure_caret_visible` 放在这里，"Redraw 也能让光标跑进可视区"。
        if self.focused && !self.inner_w.is_nan() {
            self.ensure_caret_visible();
        }

        // 1) 排版表（借用 ctx 的度量能力，用完立即释放）
        if self.layout_dirty {
            let style = self.style;
            {
                let m: &dyn RenderCtx = &*ctx;
                self.layout_with(|ch| {
                    let mut buf = [0u8; 4];
                    m.measure_text(ch.encode_utf8(&mut buf), &style).w
                });
            }
        }
        self.ensure_caret_visible();

        // 2) 背景（来自主题）
        let state = if self.disabled {
            State::Disabled
        } else if self.focused {
            State::Focused
        } else {
            State::Normal
        };
        let themed = ctx.theme().style(Role::TEXT_INPUT, state);
        match self.widget_style {
            Some(style) if style.has_bg() => {
                style.paint_bg(ctx, rect);
                style.paint_border(ctx, rect);
                style.paint_focus(ctx, rect, self.focused);
            }
            _ => {
                ctx.fill_rect(rect, themed.bg);
                if self.focused {
                    ctx.stroke_round_rect(rect, themed.fg, 1.0, 0.0);
                }
            }
        }

        // **裁剪区向上撑 4px**：字形的上挑量可能超出 `inner` 顶边，
        // 严格按 `inner` 裁会把它切掉（"8080" → "ouou"）。
        ctx.push_clip(Rect::new(
            inner.x,
            inner.y - 4.0,
            inner.w,
            inner.h + 4.0,
        ));

        let line_h = ctx.line_height(&self.style);
        // **行框在 `inner` 里绝对居中**：不再 `max(0.0)`（它会在容器偏矮时
        // 退化为贴顶），也不再按字形高取小（那会把字往下推到溢出）。
        // 实测：inner=[222,242] (h=20)、line_h=16 → text_y = 224，行框 [224,240]，上下各 2px。
        let text_y = inner.y + (inner.h - line_h) * 0.5;
        let origin_x = inner.x - self.scroll_x;

        // 占位提示：只在"真的一点内容都没有"时画（含 IME 预编辑也算内容）。
        // 不参与布局，画完就结束——后续选区/正文/光标逻辑完全不感知它。
        if self.buffer.text().is_empty() && self.preedit.is_empty() {
            if let Some(hint) = self.placeholder.as_deref() {
                if !hint.is_empty() {
                    ctx.draw_text(
                        Pos::new(inner.x, text_y),
                        hint,
                        &TextStyle::fg(Color::rgba(0.55, 0.58, 0.64, 1.0)),
                    );
                }
            }
        }

        // 3) 选区高亮（画在文字下面）
        if let Some((start, end)) = self.buffer.selection() {
            let sx = origin_x + self.x_of_byte(start);
            let ex = origin_x + self.x_of_byte(end);
            if ex > inner.x && sx < inner.right() {
                let x0 = sx.max(inner.x);
                let x1 = ex.min(inner.right());
                if x1 > x0 {
                    ctx.fill_rect(
                        Rect::new(x0, inner.y, x1 - x0, inner.h),
                        self.selection_color,
                    );
                }
            }
        }

        // 4) 逐字符绘制（视口外跳过）
        let mut text = TextStyle::fg(self.style.fg);
        text.bold = self.style.bold;
        for g in &self.layout {
            let x = origin_x + g.x;
            if x + g.w < inner.x {
                continue;
            }
            if x > inner.right() {
                break;
            }
            if let Some(ch) = self.buffer.text()[g.byte..].chars().next() {
                // 密码模式：每个字符都画 `*`（**空格也要画**，否则长度不可见）
                if self.password {
                    ctx.draw_text(Pos::new(x, text_y), "*", &text);
                    continue;
                }
                if ch == ' ' {
                    continue;
                }
                let mut buf = [0u8; 4];
                ctx.draw_text(Pos::new(x, text_y), ch.encode_utf8(&mut buf), &text);
            }
        }

        // 5) IME 预编辑（画在光标处，带下划线）
        if !self.preedit.is_empty() {
            let mut pen = origin_x + self.x_of_byte(self.buffer.cursor());
            let preedit_style = TextStyle::fg(self.style.fg);
            for ch in self.preedit.chars() {
                let mut buf = [0u8; 4];
                // 密码模式：预编辑串同样打码（否则中文输入法会把明文画出来）
                let s = if self.password {
                    "*"
                } else {
                    ch.encode_utf8(&mut buf)
                };
                let w = ctx.measure_text(s, &preedit_style).w;
                if pen + w >= inner.x && pen <= inner.right() {
                    ctx.draw_text(Pos::new(pen, text_y), s, &preedit_style);
                    ctx.fill_rect(
                        Rect::new(pen, text_y + line_h - 2.0, w, 1.5),
                        Color::rgba(0.55, 0.72, 1.0, 1.0),
                    );
                }
                pen += w;
            }
        }

        // 6) 光标：只有聚焦时才画
        if self.focused && self.caret_visible {
            let cx = origin_x + self.x_of_byte(self.buffer.cursor());
            if cx >= inner.x - 1.0 && cx <= inner.right() {
                ctx.fill_rect(Rect::new(cx, inner.y, CARET_W, inner.h), self.caret_color);
            }
        }

        ctx.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        // 密码模式：**原文一个字都不能出现在语义里**（外部工具只读语义）。
        // 掩码长度用字符数而非字节数——"中" 显示 1 个 `*`，不是 3 个。
        let masked: String = if self.password {
            "*".repeat(self.buffer.text().chars().count())
        } else {
            self.buffer.text().to_string()
        };
        let label = if self.password {
            "password".to_string()
        } else {
            self.buffer.text().to_string()
        };
        // **空输入框要有名字**：之前 label 直接取缓冲区文本，
        // 空的时候就是空串，外部工具无法按名辨认。用 placeholder 兜底，
        // 再没有就叫 "TextInput"。
        // **不能用 placeholder 兜底**：`password_placeholder.rs` 明确要求占位提示
        // 不得进入语义 label（否则敏感应用的提示会被外部工具读走）。
        // 这里只在**完全空**时给一个中性名字。
        let label = if label.is_empty() {
            "TextInput".to_string()
        } else {
            label
        };
        let sem = Semantics::default()
            .exposed(true)
            .label(label)
            .value(masked.clone())
            .action_spec("set_text", y901_ui::InputKind::Text)
            .action("focus")
            .prop("cursor", self.buffer.cursor().to_string())
            .prop("anchor", self.buffer.anchor().to_string())
            .prop("len", self.buffer.len_bytes().to_string())
            .prop("focused", self.focused.to_string())
            .prop("scroll_x", format!("{:.1}", self.scroll_x))
            .state("text", masked)
            .state("caret", self.buffer.cursor())
            .state("anchor", self.buffer.anchor())
            // focused 是框架拥有的语义字段，由 UiTree 注入顶层；不进 state
            // disabled 是顶层语义字段，不进 state（与 Button 保持一致）
            .disabled(self.disabled)
            .sensitive(self.password);
        sem
    }

    fn padding(&self) -> Edges {
        self.padding
    }
}

/// 便捷构造：给 RPC `set_text` 用的参数。
pub fn set_text_args(text: impl Into<String>) -> Json {
    Json::obj().set("text", text.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::layout::LayoutCache;
    use y901_ui::{Axis, MouseButton};

    /// 等宽度量：ASCII 8px，CJK 16px。
    struct Mono;

    impl TextMetrics for Mono {
        fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
            let mut w = 0.0;
            for ch in text.chars() {
                w += if (ch as u32) >= 0x2E80 { 16.0 } else { 8.0 };
            }
            Size::new(w, 16.0)
        }
        fn line_height(&self, _style: &TextStyle) -> f32 {
            16.0
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    /// 记录绘制指令的后端。
    #[derive(Default)]
    struct Recorder {
        fills: Vec<Rect>,
        texts: Vec<(f32, String)>,
    }

    impl TextMetrics for Recorder {
        fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
            Mono.measure_text(t, s)
        }
        fn line_height(&self, s: &TextStyle) -> f32 {
            Mono.line_height(s)
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    impl RenderCtx for Recorder {
        fn size(&self) -> Size {
            Size::new(400.0, 200.0)
        }
        fn fill_rect(&mut self, r: Rect, _c: Color) {
            self.fills.push(r);
        }
        fn stroke_rect(&mut self, r: Rect, _c: Color, _w: f32) {
            self.fills.push(r);
        }
        fn draw_text(&mut self, o: Pos, t: &str, _s: &TextStyle) -> f32 {
            self.texts.push((o.x, t.to_string()));
            t.chars().count() as f32 * 8.0
        }
        fn push_clip(&mut self, _r: Rect) {}
        fn pop_clip(&mut self) {}
    }

    fn mount(widget: &mut TextInput) {
        let mut cache = LayoutCache::new();
        widget.measure(
            &Mono,
            Constraints::loose(Size::new(240.0, 24.0)),
            &mut cache,
        );
        widget.arrange(Rect::new(0.0, 0.0, 240.0, 24.0), &Mono, &mut cache);
    }

    fn ctx() -> EventCtx {
        EventCtx::new(None, Size::new(240.0, 24.0))
    }

    /// 挂载并让"框架"授予焦点。
    ///
    /// 生产路径里 `focused` **只**由框架的 `Focus(true)` 事件设置；
    /// 单元测试里没有 UiTree，所以显式模拟一次。
    fn mount_focused(widget: &mut TextInput) {
        mount(widget);
        let mut c = ctx();
        let _ = widget.event(&mut c, &UiEvent::Focus(true));
    }

    // ---------- 综合流程 ----------

    #[test]
    fn full_flow_click_shift_select_replace_backspace() {
        let mut w = TextInput::new("input").with_text("abc中");
        mount_focused(&mut w);

        // 1) 点击：padding.left=8，文字起点 x=8；点 x=28 → 行内 20
        //    边界 0/8/16/24/40，20 落在 'c' 与 '中' 之间 → byte 3
        let mut c = ctx();
        let r = w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(28.0, 12.0),
                button: MouseButton::Left,
            },
        );
        assert!(r.needs_redraw());
        // 新契约：点击只"请求"焦点，focused 由框架的 Focus(true) 设置\n        assert!(!w.is_focused(), "点击本身不应直接置位 focused");
        assert_eq!(c.focus_request.as_deref(), Some("input"));
        assert_eq!(w.cursor(), 3);
        assert_eq!(w.anchor(), 3);
        assert_eq!(w.selection(), None);

        // 2) Shift+Right 两次：先选 '中'，再尝试右移（已到末尾）
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Right,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
        );
        assert_eq!(w.selection(), Some((3, 6)), "应选中整个 '中'");
        assert_eq!(w.buffer().selected_text(), Some("中"));

        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Right,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
        );
        assert_eq!(w.selection(), Some((3, 6)), "已在末尾，选区不变");

        // 3) 输入替换选区
        let r = w.event(&mut c, &UiEvent::Text("X".to_string()));
        // **新状态机：编辑只需 `Redraw`**。
        // 宽度已由父级锁死（`measure` 不再跟内容走），所以文字变化
        // 不会改变布局；旧期望 `Relayout` 是"宽度跟内容走"那个时代的产物，
        // 而那正是打字引发整窗重绘的根因，已被废弃。
        assert!(r.needs_redraw() && !r.needs_relayout());
        assert_eq!(w.text(), "abcX");
        assert_eq!(w.cursor(), 4);
        assert_eq!(w.anchor(), 4);
        assert_eq!(w.selection(), None);

        // 4) 退格删除 'X'
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Backspace,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(w.text(), "abc");
        assert_eq!(w.cursor(), 3);
        assert_eq!(w.anchor(), 3);
    }

    #[test]
    fn shift_selection_then_ctrl_a_then_replace() {
        let mut w = TextInput::new("i").with_text("hello");
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );
        assert_eq!(w.cursor(), 0);

        // Shift+Right 三次 → 选中 "hel"
        for _ in 0..3 {
            w.event(
                &mut c,
                &UiEvent::Key {
                    key: Key::Right,
                    modifiers: Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                },
            );
        }
        assert_eq!(w.selection(), Some((0, 3)));
        assert_eq!(w.buffer().selected_text(), Some("hel"));

        // Ctrl+A 全选
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Char('a'),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
        );
        assert_eq!(w.selection(), Some((0, 5)));

        // 输入替换全部
        w.event(&mut c, &UiEvent::Text("Y901".to_string()));
        assert_eq!(w.text(), "Y901");
        assert_eq!(w.cursor(), 4);
    }

    // ---------- 鼠标拖拽 ----------

    #[test]
    fn drag_creates_selection() {
        let mut w = TextInput::new("i").with_text("abcdef");
        mount_focused(&mut w);
        let mut c = ctx();

        // 按下在 x=8（byte 0）
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );
        assert_eq!(w.anchor(), 0);

        // 拖到 x=8+24 → byte 3
        w.event(
            &mut c,
            &UiEvent::MouseMove {
                pos: Pos::new(32.0, 12.0),
            },
        );
        assert_eq!(w.selection(), Some((0, 3)));
        assert_eq!(w.buffer().selected_text(), Some("abc"));

        // 拖回 x=8+8 → byte 1（反向选择，锚点不动）
        // 拖回 x=8+8 → byte 1（反向选择，锚点不动；选区被规范化为 (0,1)）
        w.event(
            &mut c,
            &UiEvent::MouseMove {
                pos: Pos::new(16.0, 12.0),
            },
        );
        assert_eq!(w.selection(), Some((0, 1)));

        // 松开后继续移动不再改选区
        w.event(
            &mut c,
            &UiEvent::MouseUp {
                pos: Pos::new(16.0, 12.0),
                button: MouseButton::Left,
            },
        );
        w.event(
            &mut c,
            &UiEvent::MouseMove {
                pos: Pos::new(100.0, 12.0),
            },
        );
        assert_eq!(w.selection(), Some((0, 1)), "松开后选区保持");
    }

    // ---------- 键盘细节 ----------

    #[test]
    fn home_end_and_delete() {
        let mut w = TextInput::new("i").with_text("abc中");
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );

        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::End,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(w.cursor(), 6);

        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Home,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(w.cursor(), 0);

        // Delete 删掉光标后的 'a'
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Delete,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(w.text(), "bc中");
        assert_eq!(w.cursor(), 0);
    }

    #[test]
    fn backspace_across_cjk() {
        let mut w = TextInput::new("i").with_text("a中");
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(200.0, 12.0),
                button: MouseButton::Left,
            },
        );
        assert_eq!(w.cursor(), 4, "点最右侧应落到末尾");

        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Backspace,
                modifiers: Modifiers::default(),
            },
        );
        assert_eq!(w.text(), "a", "一次退格删掉整个 '中'");
        assert_eq!(w.cursor(), 1);
    }

    #[test]
    fn unfocused_ignores_keyboard() {
        let mut w = TextInput::new("i").with_text("x");
        mount(&mut w);
        let mut c = ctx();

        let r = w.event(&mut c, &UiEvent::Text("z".to_string()));
        assert_eq!(r, EventResult::Ignored);
        assert_eq!(w.text(), "x");

        let r = w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Backspace,
                modifiers: Default::default(),
            },
        );
        assert_eq!(r, EventResult::Ignored);
        assert_eq!(w.text(), "x");
    }

    // ---------- 剪贴板 ----------

    #[test]
    fn copy_paste_through_clipboard() {
        let mut w = TextInput::new("i")
            .with_text("hello world")
            .with_clipboard(Box::new(MemoryClipboard::default()));
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );
        // 选中 "hello"
        for _ in 0..5 {
            w.event(
                &mut c,
                &UiEvent::Key {
                    key: Key::Right,
                    modifiers: Modifiers {
                        shift: true,
                        ..Default::default()
                    },
                },
            );
        }
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Char('c'),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
        );

        // 移到末尾粘贴
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::End,
                modifiers: Default::default(),
            },
        );
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Char('v'),
                modifiers: Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
            },
        );
        assert_eq!(w.text(), "hello worldhello");
    }

    // ---------- 渲染 ----------

    #[test]
    fn render_draws_background_selection_caret() {
        let mut w = TextInput::new("i").with_text("ab");
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );
        w.event(
            &mut c,
            &UiEvent::Key {
                key: Key::Right,
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
        );

        let mut rec = Recorder::default();
        w.render(&mut rec);

        assert!(
            rec.fills
                .iter()
                .any(|r| *r == Rect::new(0.0, 0.0, 240.0, 24.0)),
            "应画背景"
        );
        assert!(
            rec.fills.iter().any(|r| r.x == 8.0 && r.w == 8.0),
            "应有 8px 宽的选区高亮"
        );
        assert!(rec.texts.iter().any(|(_, t)| t == "a"));
        assert!(rec.texts.iter().any(|(_, t)| t == "b"));
        // 光标线（1.5px 宽）
        assert!(rec.fills.iter().any(|r| (r.w - 1.5).abs() < 0.01));
    }

    #[test]
    fn long_text_scrolls_to_follow_caret() {
        let mut w = TextInput::new("i");
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );
        // 输入 40 个字符 = 320px，远超 224px 可视宽度
        w.event(&mut c, &UiEvent::Text("x".repeat(40)));
        assert_eq!(w.text().len(), 40);

        let mut rec = Recorder::default();
        w.render(&mut rec);
        assert!(w.scroll_x() > 0.0, "光标应把视口推着走");
        // 光标必须落在可视区内
        let caret_world = 8.0 + 40.0 * 8.0 - w.scroll_x();
        assert!(caret_world <= 240.0 - 8.0 + 1.0);
    }

    #[test]
    fn drag_outside_widget_still_selects_via_pointer_capture() {
        let mut w = TextInput::new("i").with_text("abcdef");
        mount_focused(&mut w);
        let mut c = ctx();

        // 在 x=16（byte 1）按下 → 请求指针捕获
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(16.0, 12.0),
                button: MouseButton::Left,
            },
        );
        assert_eq!(
            c.pointer_capture,
            Some(y901_ui::PointerCapture::Capture("i".to_string())),
            "MouseDown 必须请求指针捕获"
        );
        assert_eq!(w.cursor(), 1);

        // 拖到控件之外（x、y 都越界）：仍然要更新选区
        w.event(
            &mut c,
            &UiEvent::MouseMove {
                pos: Pos::new(1000.0, 500.0),
            },
        );
        assert_eq!(w.selection(), Some((1, 6)), "越界拖拽应扩选到末尾");

        // 在控件外抬起 → 释放捕获
        w.event(
            &mut c,
            &UiEvent::MouseUp {
                pos: Pos::new(1000.0, 500.0),
                button: MouseButton::Left,
            },
        );
        assert_eq!(
            c.pointer_capture,
            Some(y901_ui::PointerCapture::Release),
            "MouseUp 必须释放指针捕获"
        );
        assert!(!w.dragging);
    }

    #[test]
    fn semantics_expose_editor_state() {
        let mut w = TextInput::new("i").with_text("hi");
        mount_focused(&mut w);
        let mut c = ctx();
        w.event(
            &mut c,
            &UiEvent::MouseDown {
                pos: Pos::new(8.0, 12.0),
                button: MouseButton::Left,
            },
        );
        let sem = w.semantics();
        assert_eq!(sem.value, "hi");
        assert!(sem.actions.iter().any(|a| a.name == "set_text"));
        assert!(sem.props.iter().any(|(k, v)| k == "cursor" && v == "0"));
        assert!(sem.props.iter().any(|(k, _)| k == "focused"));
        assert_eq!(w.role(), Role::TEXT_INPUT);
        assert_eq!(Axis::Horizontal, Axis::Horizontal);
        let _ = set_text_args("x");
    }

    /// **提交即消费预编辑串**：`Compose` 写 `preedit`，`Text` 写 `buffer` 并清 `preedit`。
    ///
    /// 为何要测：真机上曾出现"多出一个字"，就是因为提交后 `preedit`
    /// 没清，界面把它叠在正文上又画了一遍。
    #[test]
    fn text_commit_clears_preedit() {
        let mut input = TextInput::new("t");
        let mut ctx = EventCtx::new(None, Size::new(200.0, 60.0));

        input.event(&mut ctx, &UiEvent::Focus(true));
        input.event(&mut ctx, &UiEvent::Compose("shezhi".into()));
        assert_eq!(input.preedit(), "shezhi", "组字中应显示拼音");

        input.event(&mut ctx, &UiEvent::Text("设置".into()));
        assert_eq!(input.text(), "设置", "提交的正文应进 buffer");
        assert!(input.preedit().is_empty(), "提交后必须清空预编辑串");
    }
}
