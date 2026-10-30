//! 滚动 = 局部重绘：与全量重绘**逐像素一致**。
//!
//! 背景：滚动会让子节点 rect 变化（`ListView::arrange` 里减掉 `offset`），
//! 因此必然触发一次重排（Relayout）。之前的实现一 Relayout 就把整窗标脏
//! （`dirty = full()`），`ls -la` 这类高频输出时等于每帧全屏重绘。
//!
//! 本测试验证：把 Relayout 的脏区收窄到"请求者矩形 + 布局 diff 并集"之后，
//! 局部重绘的结果仍与整窗重绘逐像素相同（也就是收窄是安全的）。

use y901_ui::{
    Color, Modifiers, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle, UiEvent, UiTree,
};
use y901_widgets::ListView;

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

/// 最小软件画布：只实现矩形填充与"方块文字"，够用来比对覆盖关系。
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

    fn restored(&self) -> Self {
        Self {
            w: self.w,
            h: self.h,
            px: self.px.clone(),
            clips: vec![Rect::new(0.0, 0.0, self.w as f32, self.h as f32)],
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
                let d = self.px[i];
                let a = c.a.clamp(0.0, 1.0);
                let mix = |s: f32, dv: u8| -> u8 {
                    ((s * a + (dv as f32 / 255.0) * (1.0 - a)) * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
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
                self.fill_rect(Rect::new(x, origin.y, 6.0, 12.0), style.fg);
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

fn build() -> UiTree {
    // ListView 自己维护行控件池（虚拟化），所以用 items 接口
    let items: Vec<String> = (0..100).map(|i| format!("row-{i}-xyz")).collect();
    let lv = ListView::new("list", 20.0).items(items);
    let mut tree = UiTree::new(Box::new(lv));
    tree.layout(Size::new(320.0, 200.0), &Mono);
    tree
}

#[test]
fn scroll_partial_redraw_matches_full_redraw() {
    let wheel = UiEvent::Wheel {
        delta: -120.0,
        pos: Pos::new(60.0, 60.0),
        mods: Modifiers::default(),
    };

    // A：整窗渲染（模拟"屏幕上已有内容"）
    let mut tree_a = build();
    let mut canvas = Canvas::new(320, 200);
    tree_a.invalidate_all();
    tree_a.render(&mut canvas);

    // B：滚动后用**局部**脏区在 A 的画布上继续画
    let r = tree_a.dispatch(&wheel);
    assert!(
        r.needs_redraw() || r.needs_relayout(),
        "滚动必须触发重绘/重排"
    );
    tree_a.layout(Size::new(320.0, 200.0), &Mono);
    eprintln!(
        "[diag] after scroll: full={} dirty={:?}",
        tree_a.dirty().is_full(),
        tree_a.dirty().bounds()
    );
    let mut partial = canvas.restored();
    tree_a.render(&mut partial);

    // C：另一棵同样的树，滚动后整窗重绘
    let mut tree_c = build();
    let _ = tree_c.dispatch(&wheel);
    tree_c.layout(Size::new(320.0, 200.0), &Mono);
    let mut full = Canvas::new(320, 200);
    tree_c.invalidate_all();
    tree_c.render(&mut full);

    // 逐像素比对
    let mut diff = 0;
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
        "滚动的局部重绘与全量重绘不一致：{diff} 像素不同，首个差异 {first:?}"
    );
}
