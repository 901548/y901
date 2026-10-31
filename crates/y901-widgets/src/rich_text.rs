//! 轻量富文本：行内样式（粗体/斜体/颜色）+ 块级代码块。
//!
//! 设计取舍：
//! - **不做通用排版引擎**：按字符等宽换行（框架的文本度量本身就是格契约），
//!   够渲染 Markdown 段落与代码块，且可预测、可测。
//! - **只提供机制**：核心 API 是 `update(Vec<Block>)`。“流式追加”由调用方自己
//!   修改最后一个 `Block` 后再 `update`；配合 [`RichText::last_block_rect`] 可将脏区
//!   限制在最后一块。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextStyle,
    TextMetrics, UiEvent, Widget,
};

/// 行内样式（缺省 = 继承主题正文色、常规字重）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub color: Option<Color>,
}

impl InlineStyle {
    /// 粗体。
    pub fn bold() -> Self {
        Self {
            bold: true,
            ..Self::default()
        }
    }

    /// 斜体。
    pub fn italic() -> Self {
        Self {
            italic: true,
            ..Self::default()
        }
    }

    /// 指定颜色。
    pub fn color(color: Color) -> Self {
        Self {
            color: Some(color),
            ..Self::default()
        }
    }

    fn to_text_style(self, fallback: Color) -> TextStyle {
        let mut s = TextStyle::fg(self.color.unwrap_or(fallback));
        s.bold = self.bold;
        s.italic = self.italic;
        s
    }
}

/// 一段带样式的文本。
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    pub text: String,
    pub style: InlineStyle,
}

impl Span {
    /// 普通文本。
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: InlineStyle::default(),
        }
    }

    /// 带样式的文本。
    pub fn styled(text: impl Into<String>, style: InlineStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// 块级元素。
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// 段落：若干行内片段，按宽度自动折行。
    Paragraph(Vec<Span>),
    /// 代码块：等宽、带背景、保留原始换行。
    Code {
        lang: String,
        lines: Vec<String>,
    },
}

/// 富文本控件。
pub struct RichText {
    id: String,
    blocks: Vec<Block>,
    rect: Rect,
    item: y901_ui::FlexItem,
    /// 每块的 `(y 偏移, 高度)`，measure 时算好，render 直接复用。
    layout: Vec<(f32, f32)>,
    content_h: f32,
    version: u64,
    pad: f32,
    code_bg: Option<Color>,
    code_pad: f32,
}

impl RichText {
    /// 新建空富文本。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            blocks: Vec::new(),
            rect: Rect::ZERO,
            item: y901_ui::FlexItem::default(),
            layout: Vec::new(),
            content_h: 0.0,
            version: 0,
            pad: 6.0,
            code_bg: None,
            code_pad: 6.0,
        }
    }

    /// 设置代码块背景色（`None` = 用主题 panel 色）。
    pub fn code_background(mut self, color: Color) -> Self {
        self.code_bg = Some(color);
        self
    }

    /// 设置 flex 参数。
    pub fn flex_item(mut self, item: y901_ui::FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 追加一个段落。
    pub fn push_paragraph(&mut self, spans: Vec<Span>) {
        if !spans.is_empty() {
            self.blocks.push(Block::Paragraph(spans));
            self.version += 1;
        }
    }

    /// 追加一段普通文本（自动包成段落）。
    pub fn push_text(&mut self, text: impl Into<String>) {
        self.push_paragraph(vec![Span::new(text)]);
    }

    /// 追加一个代码块（按 `\n` 切行）。
    pub fn push_code(&mut self, lang: impl Into<String>, code: impl Into<String>) {
        let code = code.into();
        let lines: Vec<String> = code.lines().map(|l| l.to_string()).collect();
        self.blocks.push(Block::Code {
            lang: lang.into(),
            lines,
        });
        self.version += 1;
    }

    /// 更新整个块列表（核心 API）。
    ///
    /// 输入：新的 `Vec<Block>`。副作用：作废布局缓存、bump 版本（下一次
    /// `measure` 会重新排版）。
    ///
    /// 为什么不提供“追加 token”这种业务 API：框架只负责“把数据排好版”。
    /// 流式追加在调用方那边就是“改最后一个 Block 的文本”，然后 `update` 一次即可；
    /// 配合 [`RichText::last_block_rect`] 还能把脏区限制在最后一块。
    pub fn update(&mut self, blocks: Vec<Block>) {
        self.blocks = blocks;
        self.layout.clear();
        self.version += 1;
    }

    /// 当前的块列表（只读）。
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// 清空全部内容。
    pub fn clear(&mut self) {
        self.blocks.clear();
        self.layout.clear();
        self.content_h = 0.0;
        self.version += 1;
    }

    /// 块数量。
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// 内容总高度（最近一次 measure 的结果）。
    pub fn content_height(&self) -> f32 {
        self.content_h
    }

    /// 第 `index` 块在**屏幕坐标系**里的矩形（供 `invalidate_rect` 做局部重绘）。
    ///
    /// 返回：`None` = 索引越界或还没布局过。
    pub fn block_rect(&self, index: usize) -> Option<Rect> {
        let (y, h) = *self.layout.get(index)?;
        Some(Rect::new(self.rect.x, self.rect.y + y, self.rect.w, h))
    }

    /// 最后一块的屏幕矩形（流式追加时用它标脏）。
    pub fn last_block_rect(&self) -> Option<Rect> {
        self.block_rect(self.blocks.len().checked_sub(1)?)
    }

    /// 流式追加到最后一个 Code Block；没有 Code 时新建一个。
    pub fn append_code_text(&mut self, text: &str) -> usize {
        self.version += 1;
        if let Some(Block::Code { lines, .. }) = self.blocks.last_mut() {
            let mut parts = text.split('\n');
            if let Some(first) = parts.next() {
                if let Some(last) = lines.last_mut() {
                    last.push_str(first);
                } else {
                    lines.push(first.to_string());
                }
            }
            for line in parts {
                lines.push(line.to_string());
            }
            self.blocks.len() - 1
        } else {
            self.push_code("text", text);
            self.blocks.len() - 1
        }
    }

    /// 只重测最后一块；layout 未建立时返回 false，调用方退回全量 measure。
    pub fn measure_last(&mut self, metrics: &dyn TextMetrics, c: Constraints) -> bool {
        if self.layout.len() != self.blocks.len() || self.blocks.is_empty() {
            return false;
        }
        let width = c.max.w;
        let line_h = metrics.line_height(&TextStyle::default());
        let per_line = self.chars_per_line(metrics, width);
        let index = self.blocks.len() - 1;
        let (y, _) = self.layout[index];
        let h = match &self.blocks[index] {
            Block::Paragraph(spans) => {
                self.wrap(spans, per_line).len().max(1) as f32 * line_h
            }
            Block::Code { lines, .. } => {
                lines.len().max(1) as f32 * line_h + self.code_pad * 2.0
            }
        };
        self.layout[index] = (y, h);
        self.content_h = y + h + self.pad;
        true
    }

    /// 每行最多放多少个字符（等宽近似）。
    fn chars_per_line(&self, metrics: &dyn TextMetrics, w: f32) -> usize {
        let cw = metrics.measure_text("M", &TextStyle::default()).w.max(1.0);
        ((w - self.pad * 2.0).max(cw) / cw).floor().max(1.0) as usize
    }

    /// 段落折行：返回每行的 `(起始 span 索引, 文本)` 列表。
    fn wrap(&self, spans: &[Span], per_line: usize) -> Vec<Vec<(usize, String)>> {
        let mut lines: Vec<Vec<(usize, String)>> = Vec::new();
        let mut cur: Vec<(usize, String)> = Vec::new();
        let mut cur_len = 0usize;
        for (i, span) in spans.iter().enumerate() {
            let mut buf = String::new();
            for ch in span.text.chars() {
                if ch == '\n' || cur_len == per_line {
                    if !buf.is_empty() {
                        cur.push((i, std::mem::take(&mut buf)));
                    }
                    lines.push(std::mem::take(&mut cur));
                    cur_len = 0;
                    if ch == '\n' {
                        continue;
                    }
                }
                buf.push(ch);
                cur_len += 1;
            }
            if !buf.is_empty() {
                cur.push((i, buf));
            }
        }
        if !cur.is_empty() || lines.is_empty() {
            lines.push(cur);
        }
        lines
    }
}

impl Widget for RichText {
    fn id(&self) -> &str {
        &self.id
    }
    fn flex(&self) -> y901_ui::FlexItem {
        self.item
    }
    fn layout_version(&self) -> u64 {
        self.version
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        let width = c.max.w;
        let line_h = metrics.line_height(&TextStyle::default());
        let per_line = self.chars_per_line(metrics, width);
        self.layout.clear();
        let mut y = self.pad;
        for block in &self.blocks {
            let h = match block {
                Block::Paragraph(spans) => {
                    let lines = self.wrap(spans, per_line);
                    lines.len().max(1) as f32 * line_h
                }
                Block::Code { lines, .. } => lines.len().max(1) as f32 * line_h + self.code_pad * 2.0,
            };
            self.layout.push((y, h));
            y += h;
        }
        self.content_h = y + self.pad;
        c.constrain(Size::new(width, self.content_h))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut y901_ui::layout::LayoutCache) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let theme = ctx.theme();
        let fg = theme.style(y901_ui::Role::PANEL, y901_ui::State::Normal).fg;
        let line_h = ctx.line_height(&TextStyle::default());
        let per_line = self.chars_per_line(ctx, self.rect.w);
        let code_bg = self
            .code_bg
            .unwrap_or_else(|| theme.style(y901_ui::Role::PANEL, y901_ui::State::Hovered).bg);

        for (bi, block) in self.blocks.iter().enumerate() {
            let Some((by, bh)) = self.layout.get(bi).copied() else {
                continue;
            };
            let top = self.rect.y + by;
            match block {
                Block::Paragraph(spans) => {
                    let lines = self.wrap(spans, per_line);
                    for (li, run) in lines.iter().enumerate() {
                        let mut x = self.rect.x + self.pad;
                        let y = top + li as f32 * line_h;
                        for (si, text) in run {
                            let style = spans[*si].style.to_text_style(fg);
                            x += ctx.draw_text(Pos::new(x, y), text, &style);
                        }
                    }
                }
                Block::Code { lines, .. } => {
                    ctx.fill_round_rect(
                        Rect::new(self.rect.x + self.pad, top, self.rect.w - self.pad * 2.0, bh),
                        code_bg,
                        6.0,
                    );
                    let style = TextStyle::fg(fg);
                    for (li, text) in lines.iter().enumerate() {
                        let y = top + self.code_pad + li as f32 * line_h;
                        ctx.draw_text(Pos::new(self.rect.x + self.pad + self.code_pad, y), text, &style);
                    }
                }
            }
        }
    }

    fn semantics(&self) -> Semantics {
        let plain: String = self
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(spans) => spans.iter().map(|s| s.text.as_str()).collect::<String>(),
                Block::Code { lines, .. } => lines.join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        Semantics::default().exposed(true).label(plain)
    }
}
