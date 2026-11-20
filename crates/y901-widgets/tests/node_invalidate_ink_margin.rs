//! **节点级标脏必须按"该节点自己的字号"外扩墨迹**，而不是按树级默认 16px。
//!
//! 背景（第二轮复审 A-1）：`UiTree::layout()` 只用 `TextStyle::default()` 算一次
//! `ink_margin`，然后把这个常数套到所有节点上。可 `TextLabel::size(24/32/48)` 的真实
//! 墨迹溢出随字号线性增长（6.95 / 9.27 / 13.90px），而树级常数只有 4.63px ——
//! 大字号文本做局部重绘时会留下残影（实测 24px 差 72 像素、48px 差 312 像素）。
//!
//! 本用例对 4 个字号各做一遍"同宽同高文本变更 + 只 `invalidate(id)` + 局部清屏重绘"，
//! 与全新画布整窗重绘逐像素比对；同时断言脏区外扩量确实是 **per-node** 的
//! （否则这条回归会像上一版一样对 24/48px 完全失明）。

use std::collections::HashMap;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
};
use y901_widgets::{Block, InlineStyle, Span, StyledText, TextLabel};

const W: u32 = 260;
const H: u32 = 140;
const BG: [u8; 4] = [0, 0, 0, 255];

/// 按字号缩放的假度量：行高 = 字号，墨迹深度 = 1.3 × 字号（溢出 0.3×字号）。
struct ScaledMono;

impl ScaledMono {
    fn px(style: &TextStyle) -> f32 {
        style.size.unwrap_or(16.0).max(1.0)
    }
}

impl TextMetrics for ScaledMono {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * Self::px(s) * 0.5, Self::px(s))
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        Self::px(s)
    }
    fn ink_bounds(&self, s: &TextStyle) -> (f32, f32) {
        (0.0, Self::px(s) * 1.2)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 纯 CPU 画布：矩形填充 + "方块字形"（下伸字多画一条越出 rect 的笔画）。
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<[u8; 4]>,
    clips: Vec<Rect>,
}

impl Canvas {
    fn new(w: u32, h: u32, bg: [u8; 4]) -> Self {
        Self {
            w,
            h,
            px: vec![bg; (w * h) as usize],
            clips: vec![Rect::new(0.0, 0.0, w as f32, h as f32)],
        }
    }

    fn restored(&self) -> Self {
        Self {
            w: self.w,
            h: self.h,
            px: self.px.clone(),
            clips: vec![Rect::new(0.0, 0.0, self.w as f32, self.h as f32)],
        }
    }

    /// 模拟后端局部刷新：只把脏区清成背景色。
    fn clear(&mut self, r: Rect) {
        let r = r.intersect(*self.clips.last().unwrap());
        if r.is_empty() {
            return;
        }
        let x0 = r.x.floor().max(0.0) as u32;
        let y0 = r.y.floor().max(0.0) as u32;
        let x1 = (r.right().ceil() as u32).min(self.w);
        let y1 = (r.bottom().ceil() as u32).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                self.px[(y * self.w + x) as usize] = BG;
            }
        }
    }
}

impl TextMetrics for Canvas {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Size::new(
            t.chars().count() as f32 * ScaledMono::px(s) * 0.5,
            ScaledMono::px(s),
        )
    }
    fn line_height(&self, s: &TextStyle) -> f32 {
        ScaledMono::px(s)
    }
    fn ink_bounds(&self, s: &TextStyle) -> (f32, f32) {
        (0.0, ScaledMono::px(s) * 1.2)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Canvas {
    fn size(&self) -> Size {
        Size::new(self.w as f32, self.h as f32)
    }
    fn fill_rect(&mut self, r: Rect, c: Color) {
        let r = r.intersect(*self.clips.last().unwrap());
        if r.is_empty() || c.a <= 0.0 {
            return;
        }
        let x0 = r.x.floor().max(0.0) as i32;
        let y0 = r.y.floor().max(0.0) as i32;
        let x1 = (r.right().ceil() as i32).min(self.w as i32);
        let y1 = (r.bottom().ceil() as i32).min(self.h as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y as u32 * self.w + x as u32) as usize;
                let a = c.a.clamp(0.0, 1.0);
                let mix = |s: f32, dv: u8| -> u8 {
                    ((s * a + (dv as f32 / 255.0) * (1.0 - a)) * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
                let d = self.px[i];
                self.px[i] = [mix(c.r, d[0]), mix(c.g, d[1]), mix(c.b, d[2]), 255];
            }
        }
    }
    fn stroke_rect(&mut self, r: Rect, c: Color, w: f32) {
        self.fill_rect(Rect::new(r.x, r.y, r.w, w), c);
        self.fill_rect(Rect::new(r.x, r.bottom() - w, r.w, w), c);
        self.fill_rect(Rect::new(r.x, r.y, w, r.h), c);
        self.fill_rect(Rect::new(r.right() - w, r.y, w, r.h), c);
    }
    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
        let size = ScaledMono::px(style);
        let mut x = origin.x;
        for ch in text.chars() {
            if ch != ' ' {
                // 字身占行高的 80%
                self.fill_rect(Rect::new(x, origin.y, size * 0.4, size * 0.8), style.fg);
                // 下伸笔画：越过行框 0.4×字号（j/g/y 才有）
                // 墨迹总高 = 0.8 + 0.4 = 1.2 × 字号，与 `ink_bounds` 一致。
                if matches!(ch, 'j' | 'g' | 'y') {
                    self.fill_rect(
                        Rect::new(x, origin.y + size * 0.8, size * 0.4, size * 0.4),
                        style.fg,
                    );
                }
            }
            x += size * 0.5;
        }
        x - origin.x
    }
    fn push_clip(&mut self, r: Rect) {
        let c = r.intersect(*self.clips.last().unwrap());
        self.clips.push(c);
    }
    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }
}

/// 根容器：文本节点摆在视口内部（根节点自己总被撑满视口）。
struct Host {
    rect: Rect,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for Host {
    fn id(&self) -> &str {
        "host"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(W as f32, H as f32))
    }
    fn arrange(&mut self, r: Rect, m: &dyn TextMetrics, cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
        for child in &mut self.children {
            let size = child.measure(m, Constraints::loose(Size::new(200.0, H as f32)), cache);
            child.arrange(Rect::new(10.0, 10.0, size.w, size.h), m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _c: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn build(text: &str, size: f32) -> UiTree {
    UiTree::new(Box::new(Host {
        rect: Rect::ZERO,
        children: vec![Box::new(
            TextLabel::new("t", text).size(size).color(Color::WHITE),
        )],
    }))
}

/// 断言：字号 `size` 下"局部重绘 == 整窗重绘"，且脏区外扩量是 per-node 的。
fn assert_size(size: f32) {
    let metrics = ScaledMono;
    let mut tree = build("jjjj", size);
    tree.layout(Size::new(W as f32, H as f32), &metrics);

    let mut canvas = Canvas::new(W, H, BG);
    tree.invalidate_all();
    tree.render(&mut canvas);

    let node = tree.rect_of("t").expect("text node");
    let expected_margin = (size * 1.2 - size).max(2.0).min(size);

    // 前置：下伸笔画真的画到了节点矩形之外（否则用例证明不了任何事）
    let below = (node.bottom() as u32 + 1).min(H - 1);
    let ink_below = (0..W)
        .filter(|x| canvas.px[(below * W + x) as usize] != BG)
        .count();
    assert!(
        ink_below > 0,
        "{size}px 前置失败：节点矩形下方没有墨迹，无法复现溢出"
    );

    // 等长文本变更（j→o），只做节点级标脏，不重排
    tree.with_node_mut("t", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(label) = any.downcast_mut::<TextLabel>() {
                label.set_text("oooo");
            }
        }
    });
    tree.invalidate("t");
    assert!(!tree.dirty().is_full(), "{size}px 节点级标脏不能退化成整窗");

    let dirty_bounds = tree.dirty().bounds().expect("应有脏区");
    // **per-node 断言**：外扩量必须随字号增长。树级常数（16px 的 4.63）会让
    // 24/32/48 的这条 assert 失败 —— 这正是上一版对 A-1 失明的原因。
    //
    // 期望值要按**视口裁切**算：节点在 (10,10)，48px 时上/左各扩 14.4 会被裁到 0。
    let want_top = (node.y - expected_margin).max(0.0);
    let want_bottom = (node.bottom() + expected_margin).min(H as f32);
    assert!(
        (dirty_bounds.y - want_top).abs() < 0.01
            && (dirty_bounds.bottom() - want_bottom).abs() < 0.01,
        "{size}px 脏区应外扩 {expected_margin:.2}px（裁到视口后 y={want_top:.2}..{want_bottom:.2}，         实际 {:.2}..{:.2}，脏区 {dirty_bounds:?}，节点 {node:?}）",
        dirty_bounds.y,
        dirty_bounds.bottom()
    );

    let dirty_rects = tree.dirty().rects().to_vec();
    let mut partial = canvas.restored();
    for r in &dirty_rects {
        partial.clear(*r);
    }
    tree.render(&mut partial);

    // 全新画布整窗重绘 "oooo"
    let mut tree_full = build("oooo", size);
    tree_full.layout(Size::new(W as f32, H as f32), &metrics);
    let mut full = Canvas::new(W, H, BG);
    tree_full.invalidate_all();
    tree_full.render(&mut full);

    let mut diff = 0usize;
    let mut first = None;
    for i in 0..partial.px.len() {
        if partial.px[i] != full.px[i] {
            diff += 1;
            if first.is_none() {
                first = Some((i as u32 % partial.w, i as u32 / partial.w));
            }
        }
    }
    assert_eq!(
        diff, 0,
        "{size}px：节点级标脏的局部重绘与整窗重绘不一致：{diff} 像素不同，\
         首个差异 {first:?}（节点 {node:?}，脏区 {dirty_rects:?}）"
    );
}

/// 富文本（`StyledText`）的混合字号：裁剪框按墨迹放宽后，下伸笔画不再被自裁剪切掉，
/// 因此可以做与 `TextLabel` 相同的**像素级**局部重绘回归。
fn build_rt(text: &str, size: f32) -> UiTree {
    let mut rt = StyledText::new("rt");
    rt.push_paragraph(vec![Span::styled(text, InlineStyle::size(size))]);
    UiTree::new(Box::new(Host {
        rect: Rect::ZERO,
        children: vec![Box::new(rt)],
    }))
}

#[test]
fn mixed_size_rich_text_invalidate_matches_full_redraw() {
    let metrics = ScaledMono;
    // 48px：富文本自带 pad，字号太小则下伸笔画仍落在节点矩形内，测不出残影。
    let size = 48.0f32;
    let mut tree = build_rt("jjjj", size);
    tree.layout(Size::new(W as f32, H as f32), &metrics);

    let mut canvas = Canvas::new(W, H, BG);
    tree.invalidate_all();
    tree.render(&mut canvas);

    let node = tree.rect_of("rt").expect("rich text node");
    let expected_margin = (size * 1.2 - size).max(2.0).min(size);
    let below = (node.bottom() as u32 + 1).min(H - 1);
    let ink_below = (0..W)
        .filter(|x| canvas.px[(below * W + x) as usize] != BG)
        .count();
    assert!(
        ink_below > 0,
        "富文本前置失败：节点矩形下方没有墨迹（size={size}）—— 自裁剪是否又收窄了？"
    );

    // 同宽同高内容变更 + 只做节点级标脏
    tree.with_node_mut("rt", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(rt) = any.downcast_mut::<StyledText>() {
                rt.update(vec![Block::Paragraph(vec![Span::styled(
                    "oooo",
                    InlineStyle::size(size),
                )])]);
            }
        }
    });
    // `update()` 会让内部排版失效（`layout` 被 clear）。真实主循环里调用方会
    // `request_layout()`，根节点缓存因此不再 fresh、整棵树重新 measure；
    // 测试里直接 `clear_layout_cache()` 等价（而且更明确：这里要的就是强制重排）。
    tree.clear_layout_cache();
    tree.request_layout();
    tree.layout(Size::new(W as f32, H as f32), &metrics);
    tree.invalidate("rt");

    let dirty_bounds = tree.dirty().bounds().expect("应有脏区");
    let want_top = (node.y - expected_margin).max(0.0);
    let want_bottom = (node.bottom() + expected_margin).min(H as f32);
    assert!(
        (dirty_bounds.y - want_top).abs() < 0.01
            && (dirty_bounds.bottom() - want_bottom).abs() < 0.01,
        "富文本脏区应按最大 span 外扩 {expected_margin:.2}px（期望 {want_top:.2}..{want_bottom:.2}，\
         实际 {:.2}..{:.2}，节点 {node:?}）",
        dirty_bounds.y,
        dirty_bounds.bottom()
    );

    let dirty_rects = tree.dirty().rects().to_vec();
    let mut partial = canvas.restored();
    for r in &dirty_rects {
        partial.clear(*r);
    }
    tree.render(&mut partial);

    let mut tree_full = build_rt("oooo", size);
    tree_full.layout(Size::new(W as f32, H as f32), &metrics);
    let mut full = Canvas::new(W, H, BG);
    tree_full.invalidate_all();
    tree_full.render(&mut full);

    let mut diff = 0usize;
    let mut first = None;
    for i in 0..partial.px.len() {
        if partial.px[i] != full.px[i] {
            diff += 1;
            if first.is_none() {
                first = Some((i as u32 % partial.w, i as u32 / partial.w));
            }
        }
    }
    assert_eq!(
        diff, 0,
        "富文本节点级标脏的局部重绘与整窗重绘不一致：{diff} 像素不同，首个差异 {first:?} \
         （节点 {node:?}，脏区 {dirty_rects:?}）"
    );
}

/// `StyledText` 声明的是"最大 span 字号"的边距。
///
/// **为什么这条是单元级而不是像素级**：`StyledText::render` 会 `push_clip(self.rect)`，
/// 它的墨迹**不可能**越出自己的矩形（这是有意的：长行不能画到邻居身上）。
/// 所以像素回归在这里测不到残影；能测的是"它确实按最大字号报告了 per-node 边距"，
/// 而不是退回树级 16px 默认 —— 后者会让将来的取消裁剪/阴影/描边直接踩坑。
#[test]
fn rich_text_reports_the_largest_span_margin() {
    let metrics = ScaledMono;
    let default_margin = y901_ui::ink_margin_for(&metrics, &TextStyle::default());

    let mut mixed = StyledText::new("rt-mixed");
    mixed.push_paragraph(vec![
        Span::styled("small", InlineStyle::size(16.0)),
        Span::styled("BIG", InlineStyle::size(48.0)),
    ]);
    let margin = mixed
        .ink_overflow(&metrics)
        .expect("富文本应声明自己的墨迹边距");
    // 48px、墨迹 1.2×：期望 0.2×48 = 9.6
    assert!(
        (margin - 9.6).abs() < 0.01,
        "混合字号应取最大 span 的边距（期望 9.6，实际 {margin:.2}）"
    );
    assert!(
        margin > default_margin,
        "最大 span 边距应大于树级默认（{margin:.2} vs {default_margin:.2}）"
    );

    let mut small = StyledText::new("rt-small");
    small.push_paragraph(vec![Span::styled("x", InlineStyle::size(16.0))]);
    let small_margin = small.ink_overflow(&metrics).unwrap();
    assert!(
        (small_margin - default_margin).abs() < 0.01,
        "16px 单字号应与树级默认一致（{small_margin:.2} vs {default_margin:.2}）"
    );
}

#[test]
fn per_node_ink_margin_matches_each_font_size() {
    for size in [16.0, 24.0, 32.0, 48.0] {
        assert_size(size);
    }
}

/// 反向对照：树级默认边距（16px 口径）不足以覆盖 24px 的溢出。
///
/// 这条不依赖框架内部实现，直接验证"两种边距确实不同"——
/// 若有人把 per-node 覆盖删掉、退回单一常数，上一组用例会红，这一条给出原因。
#[test]
fn tree_level_margin_is_too_small_for_large_fonts() {
    use y901_ui::ink_margin_for;

    let tree_level = ink_margin_for(&ScaledMono, &TextStyle::default());
    let at_48 = ink_margin_for(&ScaledMono, &TextStyle::default().size(48.0));
    assert!(
        at_48 > tree_level + 5.0,
        "48px 的边距应显著大于 16px 默认（实际 {at_48:.2} vs {tree_level:.2}）"
    );
    let mut seen = HashMap::new();
    for size in [16.0f32, 24.0, 32.0, 48.0] {
        seen.insert(
            size as u32,
            ink_margin_for(&ScaledMono, &TextStyle::default().size(size)),
        );
    }
    assert_eq!(seen[&16], tree_level, "16px 应与树级默认一致");
}
