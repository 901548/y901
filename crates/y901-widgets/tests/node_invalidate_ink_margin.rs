//! **节点级标脏（`invalidate(id)`）也必须按墨迹外扩**：文字内容变了、尺寸没变时，
//! 下伸字的墨迹落在节点矩形之外，只标节点矩形会留下残影。
//!
//! 背景（第二轮审查 #2）：早期只有 `invalidate_rect` 走墨迹外扩，`invalidate(id)`、
//! 布局新旧矩形并集等路径仍按精确矩形标脏。于是把 "j" 改成 "o" 这种**同宽同高**
//! 的文本变更，在 125% DPI 下会留下 164 个差异像素（行 56~71）。
//!
//! 本用例用纯 CPU 画布复现该场景：
//!   A. 画 "jjjj"（下伸笔画出节点矩形 3px）
//!   B. 改成 "oooo"（等长等尺寸，不触发重排）→ 只 `invalidate("text")`
//!   C. 局部重绘（只清脏区 + 重画相交节点）必须与"全新画布整窗重绘"逐像素一致。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiTree, Widget,
};

const W: u32 = 240;
const H: u32 = 100;
const BG: [u8; 4] = [0, 0, 0, 255];

/// 行高 16、墨迹深度 20 → 框架的墨迹边距 = max(4, 2) = 4px。
struct InkMono;

impl TextMetrics for InkMono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn ink_bounds(&self, _s: &TextStyle) -> (f32, f32) {
        (0.0, 20.0)
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 纯 CPU 画布：支持裁剪栈、矩形填充与"方块字形"。
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

    /// 模拟后端的局部刷新：只把脏区清成背景色。
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
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn ink_bounds(&self, _s: &TextStyle) -> (f32, f32) {
        (0.0, 20.0)
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
        let mut x = origin.x;
        for ch in text.chars() {
            if ch != ' ' {
                // 字身：12px 高，落在节点矩形内
                self.fill_rect(Rect::new(x, origin.y, 6.0, 12.0), style.fg);
                // 下伸笔画：越过节点矩形底边 4px（j 才有，o 没有）。
                // 4px 恰好等于本夹具的墨迹边距（ink_depth 20 - line_height 16）。
                if ch == 'j' || ch == 'g' || ch == 'y' {
                    self.fill_rect(Rect::new(x, origin.y + 12.0, 6.0, 6.0), style.fg);
                }
            }
            x += 8.0;
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

struct Text {
    id: String,
    text: Rc<RefCell<String>>,
    rect: Rect,
}

impl Widget for Text {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        // **尺寸固定**：文本内容变化不会引起重排，所以只能靠标脏重绘。
        c.constrain(Size::new(200.0, 16.0))
    }
    fn arrange(
        &mut self,
        r: Rect,
        _m: &dyn TextMetrics,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::rgb(0.10, 0.11, 0.14));
        let text = self.text.borrow().clone();
        ctx.draw_text(
            Pos::new(self.rect.x + 2.0, self.rect.y + 2.0),
            &text,
            &TextStyle::fg(Color::WHITE),
        );
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 根容器：把文本节点摆在视口内部（根节点自己总被撑满视口）。
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
            child.arrange(Rect::new(10.0, 10.0, 200.0, 16.0), m, cache);
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

fn build(text: &str) -> (UiTree, Rc<RefCell<String>>) {
    let cell = Rc::new(RefCell::new(text.to_string()));
    let tree = UiTree::new(Box::new(Host {
        rect: Rect::ZERO,
        children: vec![Box::new(Text {
            id: "text".to_string(),
            text: cell.clone(),
            rect: Rect::ZERO,
        })],
    }));
    (tree, cell)
}

#[test]
fn same_size_text_change_invalidate_by_id_matches_full_redraw() {
    // A：屏幕上先有 "jjjj"
    let (mut tree, cell) = build("jjjj");
    tree.layout(Size::new(W as f32, H as f32), &InkMono);
    let mut canvas = Canvas::new(W, H, BG);
    tree.invalidate_all();
    tree.render(&mut canvas);

    let node = tree.rect_of("text").expect("text node");
    // 前置：下伸笔画真的画到了节点矩形之外（否则这条用例证明不了任何事）
    let below = (node.bottom() as u32 + 1).min(H - 1);
    let ink_below = (0..W)
        .filter(|x| canvas.px[(below * W + x) as usize] != BG)
        .count();
    assert!(
        ink_below > 0,
        "前置失败：节点矩形下方没有墨迹，用例无法复现溢出场景"
    );

    // B：等长文本变更（j→o），只做节点级标脏，不重排
    *cell.borrow_mut() = "oooo".to_string();
    tree.invalidate("text");
    assert!(!tree.dirty().is_full(), "节点级标脏不能退化成整窗");

    // C：局部刷新模拟（脏区清背景 + 重画相交节点）
    let dirty_bounds = tree.dirty().bounds().expect("应有脏区");
    let dirty_rects = tree.dirty().rects().to_vec();
    let mut partial = canvas.restored();
    for r in &dirty_rects {
        partial.clear(*r);
    }
    tree.render(&mut partial);

    // D：全新画布整窗重绘 "oooo"
    let (mut tree_full, _) = build("oooo");
    tree_full.layout(Size::new(W as f32, H as f32), &InkMono);
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
        "节点级标脏的局部重绘与整窗重绘不一致：{diff} 像素不同，首个差异 {first:?} \
         （节点矩形 {node:?}，脏区 {dirty_bounds:?} / {dirty_rects:?}）"
    );
}
