//! 轻量富文本：行内样式（粗体/斜体/颜色）+ 块级代码块。
//!
//! 设计取舍：
//! - **不做通用排版引擎**：按字符等宽换行（框架的文本度量本身就是格契约），
//!   够渲染 Markdown 段落与代码块，且可预测、可测。
//! - **只提供机制**：核心 API 是 `update(Vec<Block>)`。“流式追加”由调用方自己
//!   修改最后一个 `Block` 后再 `update`；配合 [`StyledText::last_block_rect`] 可将脏区
//!   限制在最后一块。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, Widget,
};

/// 行内样式（缺省 = 继承主题正文色、常规字重）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineStyle {
    /// 粗体标志（`SGR 1` / `22`）。
    pub bold: bool,
    /// 斜体标志。
    pub italic: bool,
    /// 覆盖颜色；`None` = 用主题色。
    pub color: Option<Color>,
    /// **字号**；`None` = 用引擎默认字号。
    ///
    /// 给 `Some(px)` 时：绘制按该字号，且 `StyledText` 的**行高按该行 max(size) 算**（见
    /// `compute_line_heights`）。默认 `None` 时行高与加这个字段之前**逐像素一致**。
    pub size: Option<f32>,
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

    /// 指定字号（逻辑像素）。
    pub fn size(px: f32) -> Self {
        Self {
            size: Some(px),
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
        // **字号要传下去**：否则行高算了大字号、绘制却用默认字号，字会溢出行高。
        s.size = self.size;
        s
    }
}

/// 一段带样式的文本。
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// 本段文本。
    pub text: String,
    /// 本段样式（与文本一起构成 `Span`）。
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
        /// 代码块的语言名（仅作标识，不做高亮）。
        lang: String,
        /// 预切好的行；避免每帧重新切分。
        lines: Vec<String>,
    },
}

/// 富文本控件（通用名：`StyledText`）。
/// 通用富文本控件：接收 `Vec<Block>`，负责排版与绘制。
///
/// 它**不知道**什么叫 Markdown、什么叫"流式输出"——那些是调用方的事。
pub struct StyledText {
    /// 本帧的墨迹边距（`measure` 里按最大 span 字号算好，`render` 用来放宽裁剪）。
    ///
    /// 为什么需要缓存：`render` 的 `RenderCtx` 只暴露 `line_height`，拿不到 `ink_bounds`；
    /// 而裁剪框要按墨迹放宽，否则大字号段落的**下伸笔画会被自己的裁剪切掉**。
    ink_margin: f32,
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

impl StyledText {
    /// 新建空富文本。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            ink_margin: 0.0,
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
    /// 配合 [`StyledText::last_block_rect`] 还能把脏区限制在最后一块。
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
                let lines = self.wrap(spans, per_line);
                let hs = Self::compute_line_heights(metrics, spans, &lines, line_h);
                // 空段落保底一行（与旧行为一致）。
                if hs.is_empty() {
                    line_h
                } else {
                    hs.iter().sum()
                }
            }
            Block::Code { lines, .. } => lines.len().max(1) as f32 * line_h + self.code_pad * 2.0,
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

    /// **逐行行高**（三处共用：`measure` / `measure_last` / `render`）。
    ///
    /// 规则：`row_h = max(base, 该行各 span 的 line_height(size))`。
    /// **全 `None` span 时每行都等于 `base`** —— 与加字号前的 `n * base` **逐像素等价**。
    /// 所有段落 span 里最大的墨迹边距（代码块按默认 16px 口径，含在初值内）。
    fn max_span_margin(metrics: &dyn TextMetrics, blocks: &[Block]) -> f32 {
        let mut worst = y901_ui::ink_margin_for(metrics, &TextStyle::default());
        for block in blocks {
            let spans: &[Span] = match block {
                Block::Paragraph(spans) => spans,
                Block::Code { .. } => continue,
            };
            for span in spans {
                let Some(px) = span.style.size else { continue };
                worst = worst.max(y901_ui::ink_margin_for(
                    metrics,
                    &TextStyle::default().size(px),
                ));
            }
        }
        worst
    }

    fn compute_line_heights(
        metrics: &dyn TextMetrics,
        spans: &[Span],
        lines: &[Vec<(usize, String)>],
        base: f32,
    ) -> Vec<f32> {
        lines
            .iter()
            .map(|run| {
                let mut h = base;
                for (si, _) in run {
                    if let Some(px) = spans.get(*si).and_then(|sp| sp.style.size) {
                        h = h.max(metrics.line_height(&TextStyle::default().size(px)));
                    }
                }
                h
            })
            .collect()
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

impl Widget for StyledText {
    fn id(&self) -> &str {
        &self.id
    }
    fn flex(&self) -> y901_ui::FlexItem {
        self.item
    }

    /// 混合字号文本的墨迹余量：取**所有 span 字号里最大的那个**。
    ///
    /// 为什么不是默认 16px：`RichText` 的行高按每行最大 `size` 算，
    /// 所以溢出也由最大字号决定；用树级默认会低估 `InlineStyle::size(32)` 这类行。
    fn ink_overflow(&self, metrics: &dyn TextMetrics) -> Option<f32> {
        Some(Self::max_span_margin(metrics, &self.blocks))
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
        self.ink_margin = Self::max_span_margin(metrics, &self.blocks);
        self.layout.clear();
        let mut y = self.pad;
        for block in &self.blocks {
            let h = match block {
                Block::Paragraph(spans) => {
                    let lines = self.wrap(spans, per_line);
                    let hs = Self::compute_line_heights(metrics, spans, &lines, line_h);
                    if hs.is_empty() {
                        line_h
                    } else {
                        hs.iter().sum()
                    }
                }
                Block::Code { lines, .. } => {
                    lines.len().max(1) as f32 * line_h + self.code_pad * 2.0
                }
            };
            self.layout.push((y, h));
            y += h;
        }
        self.content_h = y + self.pad;
        c.constrain(Size::new(width, self.content_h))
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

    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        if self.rect.is_empty() {
            return;
        }
        // 横向仍裁到自身矩形：长行不能画到邻居身上（邻居只标自己脏区时，
        // 那些越界像素会被清掉却无人重画 → 残影/黑块）。
        //
        // **纵向必须按墨迹放宽**：行高是"行距"，大字号的下伸笔画（g/y/j）会落到
        // 最后一行行框之下；若只裁到 rect，它们会被自己的裁剪切掉 —— 这是渲染错误，
        // 不是"保护邻居"。放宽量与脏区外扩共用同一个 `ink_margin`。
        let m = self.ink_margin.max(0.0);
        ctx.push_clip(Rect::new(
            self.rect.x,
            self.rect.y - m,
            self.rect.w,
            self.rect.h + 2.0 * m,
        ));
        let theme = ctx.theme();
        // 文本颜色必须取 `TEXT` 槽位。
        // 早期写成 `PANEL`：一旦主题里 panel 的 fg 是深色（浅色面板），
        // 文字就与背景同色 → "代码画了但屏幕没字"。
        let fg = theme.style(y901_ui::Role::TEXT, y901_ui::State::Normal).fg;
        let line_h = ctx.line_height(&TextStyle::default());
        let per_line = self.chars_per_line(ctx, self.rect.w);
        let code_bg = self.code_bg.unwrap_or_else(|| {
            theme
                .style(y901_ui::Role::PANEL, y901_ui::State::Hovered)
                .bg
        });

        for (bi, block) in self.blocks.iter().enumerate() {
            let Some((by, bh)) = self.layout.get(bi).copied() else {
                continue;
            };
            let top = self.rect.y + by;
            match block {
                Block::Paragraph(spans) => {
                    let lines = self.wrap(spans, per_line);
                    // **与 measure 同源**：逐行行高，累加偏移（不是 `li * line_h`）。
                    let hs = Self::compute_line_heights(ctx, spans, &lines, line_h);
                    let mut line_y = top;
                    for (li, run) in lines.iter().enumerate() {
                        let mut x = self.rect.x + self.pad;
                        for (si, text) in run {
                            let style = spans[*si].style.to_text_style(fg);
                            x += ctx.draw_text(Pos::new(x, line_y), text, &style);
                        }
                        // 行高不足（数组越界）时回退到基准行高。
                        line_y += hs.get(li).copied().unwrap_or(line_h);
                    }
                }
                Block::Code { lines, .. } => {
                    ctx.fill_round_rect(
                        Rect::new(
                            self.rect.x + self.pad,
                            top,
                            self.rect.w - self.pad * 2.0,
                            bh,
                        ),
                        code_bg,
                        6.0,
                    );
                    let style = TextStyle::fg(fg);
                    for (li, text) in lines.iter().enumerate() {
                        let y = top + self.code_pad + li as f32 * line_h;
                        ctx.draw_text(
                            Pos::new(self.rect.x + self.pad + self.code_pad, y),
                            text,
                            &style,
                        );
                    }
                }
            }
        }
        ctx.pop_clip();
    }

    fn semantics(&self) -> Semantics {
        let plain: String = self
            .blocks
            .iter()
            .map(|b| match b {
                Block::Paragraph(spans) => {
                    spans.iter().map(|s| s.text.as_str()).collect::<String>()
                }
                Block::Code { lines, .. } => lines.join("\n"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        Semantics::default().exposed(true).label(plain)
    }
}

/// 兼容别名：旧代码用 `RichText`，新代码请用 [`StyledText`]。
///
/// 为什么留着：它只是一个 `pub type`，不会带来任何行为差异；
/// 删掉它会让使用方的代码在升级时突然断裂，而改个名字并不值得那个代价。
pub type RichText = StyledText;
