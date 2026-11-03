//! 无头像素验证：弹窗关闭后，局部重绘必须和全量重绘**逐像素一致**。
//!
//! 做法：用一个纯内存的软件画布（`Canvas`）实现 `RenderCtx`，
//! 它把 `fill_rect` / `draw_text` 光栅化进一块 RGBA 缓冲，并遵守 `push_clip`。
//!
//! 步骤：
//!   A. 弹窗打开 → 整窗重绘 → 记下画布（模拟"屏幕上现在有弹窗"）
//!   B. 弹窗关闭 → 用**局部脏区**在 A 的画布上接着画 → 得到 B
//!   C. 弹窗关闭 → 在**空白画布**上整窗重绘 → 得到 C
//! 断言 B == C：说明"弹窗消失留下的空洞"被脏区完整覆盖。
//!
//! 不需要 GPU、不需要窗口，可在无前台环境跑。

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, Stack,
    TextMetrics, TextStyle, UiTree, Widget,
};
use y901_widgets::Dialog;

/// 软件画布：RGBA8 缓冲 + 裁剪栈。
struct Canvas {
    w: u32,
    h: u32,
    px: Vec<[u8; 4]>,
    clips: Vec<Rect>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![[0, 0, 0, 255]; (w * h) as usize],
            clips: vec![Rect::new(0.0, 0.0, w as f32, h as f32)],
        }
    }

    fn clip(&self) -> Rect {
        *self.clips.last().unwrap()
    }

    fn blend(&mut self, x: i32, y: i32, c: Color) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let i = (y as u32 * self.w + x as u32) as usize;
        let a = c.a.clamp(0.0, 1.0);
        let dst = self.px[i];
        let mix = |s: f32, d: u8| -> u8 {
            ((s * a + (d as f32 / 255.0) * (1.0 - a)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        self.px[i] = [mix(c.r, dst[0]), mix(c.g, dst[1]), mix(c.b, dst[2]), 255];
    }

    fn fill(&mut self, r: Rect, c: Color) {
        let clip = self.clip();
        let r = r.intersect(clip);
        if r.is_empty() || c.a <= 0.0 {
            return;
        }
        let x0 = r.x.floor().max(0.0) as i32;
        let y0 = r.y.floor().max(0.0) as i32;
        let x1 = (r.right().ceil() as i32).min(self.w as i32);
        let y1 = (r.bottom().ceil() as i32).min(self.h as i32);
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, c);
            }
        }
    }
}

impl TextMetrics for Canvas {
    fn measure_text(&self, t: &str, s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0 * s.fg.a.max(1.0))
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
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
        self.fill(r, c);
    }
    fn stroke_rect(&mut self, r: Rect, c: Color, w: f32) {
        // 近似：四条细边
        self.fill(Rect::new(r.x, r.y, r.w, w), c);
        self.fill(Rect::new(r.x, r.bottom() - w, r.w, w), c);
        self.fill(Rect::new(r.x, r.y, w, r.h), c);
        self.fill(Rect::new(r.right() - w, r.y, w, r.h), c);
    }
    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
        // 用"每个字符一个实心块"近似字形：够用来验证覆盖关系，且完全确定。
        let mut x = origin.x;
        for ch in text.chars() {
            if ch != ' ' {
                self.fill(Rect::new(x, origin.y, 6.0, 12.0), style.fg);
            }
            x += 8.0;
        }
        x - origin.x
    }
    fn push_clip(&mut self, r: Rect) {
        let c = r.intersect(self.clip());
        self.clips.push(c);
    }
    fn pop_clip(&mut self) {
        if self.clips.len() > 1 {
            self.clips.pop();
        }
    }
}

/// 背景：一块纯色 + 几行"文字"。
struct Backdrop {
    rect: Rect,
    color: Color,
}

impl Widget for Backdrop {
    fn id(&self) -> &str {
        "backdrop"
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        c: Constraints,
        _cache: &mut y901_ui::layout::LayoutCache,
    ) -> Size {
        c.constrain(Size::new(c.max.w, c.max.h))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut y901_ui::layout::LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, self.color);
        let style = TextStyle::fg(Color::WHITE);
        ctx.draw_text(
            Pos::new(self.rect.x + 8.0, self.rect.y + 8.0),
            "hello world",
            &style,
        );
        ctx.draw_text(
            Pos::new(self.rect.x + 8.0, self.rect.y + 32.0),
            "second line",
            &style,
        );
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 搭一棵固定树：Stack（全屏叠放）→ 背景 + 弹窗。
fn build(open: bool) -> UiTree {
    let mut dialog = Dialog::new("dlg", "Confirm");
    dialog.set_buttons(vec![
        ("ok".to_string(), "OK".to_string()),
        ("cancel".to_string(), "Cancel".to_string()),
    ]);
    dialog.set_open(open);

    let root = Stack::new("root")
        .push(Box::new(Backdrop {
            rect: Rect::ZERO,
            color: Color::rgba(0.12, 0.13, 0.16, 1.0),
        }))
        .push(Box::new(dialog));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(320.0, 240.0), &Mono);
    tree
}

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        Size::new(t.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 对比两块画布，返回不同的像素数与首个差异坐标。
fn diff(a: &Canvas, b: &Canvas) -> (usize, Option<(u32, u32)>) {
    let mut n = 0;
    let mut first = None;
    for i in 0..a.px.len() {
        if a.px[i] != b.px[i] {
            n += 1;
            if first.is_none() {
                first = Some((i as u32 % a.w, i as u32 / a.w));
            }
        }
    }
    (n, first)
}

#[test]
fn closing_dialog_partial_redraw_matches_full_redraw() {
    // ---------- A：弹窗打开，整窗重绘 ----------
    let mut tree = build(true);
    let mut canvas = Canvas::new(320, 240);
    tree.invalidate_all();
    tree.render(&mut canvas);
    assert!(tree.dirty().is_empty(), "A 渲染后脏区应清空");

    // 弹窗像素确实存在（中心点应该是弹窗面板色）
    let center = canvas.px[(120 * 320 + 160) as usize];
    assert_ne!(center, [0, 0, 0, 255], "弹窗打开时中心必须有内容");

    // ---------- B：关闭弹窗，用局部脏区在 A 上接着画 ----------
    tree.with_dialog_closed("dlg");
    tree.mark_layout_dirty("dlg");
    tree.layout(Size::new(320.0, 240.0), &Mono);
    assert!(!tree.dirty().is_empty(), "关闭弹窗后必须产生脏区");

    let mut after_partial = Canvas {
        w: canvas.w,
        h: canvas.h,
        px: canvas.px.clone(),
        clips: vec![Rect::new(0.0, 0.0, canvas.w as f32, canvas.h as f32)],
    };
    tree.render(&mut after_partial);

    // ---------- C：关闭弹窗，空白画布整窗重绘 ----------
    let mut tree2 = build(false);
    let mut full = Canvas::new(320, 240);
    tree2.invalidate_all();
    tree2.render(&mut full);

    // ---------- 断言：B == C ----------
    let (n, first) = diff(&after_partial, &full);
    assert_eq!(
        n, 0,
        "局部重绘与全量重绘不一致：{n} 个像素不同，首个差异 {first:?}"
    );
}

/// 便捷：把 UiTree 里指定 id 的 Dialog 关掉（走框架的 `with_node_mut`）。
trait CloseDialog {
    fn with_dialog_closed(&mut self, id: &str);
}

impl CloseDialog for UiTree {
    fn with_dialog_closed(&mut self, id: &str) {
        let mut ok = false;
        self.with_node_mut(id, &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(d) = any.downcast_mut::<Dialog>() {
                    d.set_open(false);
                    ok = true;
                }
            }
        });
        assert!(ok, "没找到 Dialog {id}");
    }
}
