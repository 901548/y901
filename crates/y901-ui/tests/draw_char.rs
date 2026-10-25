//! `RenderCtx::draw_char` 的契约：与 `draw_text` 等价、返回值是字符宽度。
//!
//! 为什么要它：终端每格一个字符，`draw_text(&ch.to_string())` 会为每格分配一个
//! `String`（80×24 = 每帧 1920 次）。`draw_char` 用栈缓冲，零堆分配。

use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextStyle, TextMetrics};

/// 记录 `draw_text` 调用的后端。
#[derive(Default)]
struct Recorder {
    calls: Vec<(Pos, String)>,
}

impl TextMetrics for Recorder {
    fn measure_text(&self, t: &str, _s: &TextStyle) -> Size {
        // 简化度量：ASCII 8px/字符，其余 16px/字符
        let w: f32 = t
            .chars()
            .map(|c| if (c as u32) < 0x2E80 { 8.0 } else { 16.0 })
            .sum();
        Size::new(w, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Recorder {
    fn size(&self) -> Size {
        Size::new(200.0, 100.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, origin: Pos, text: &str, _s: &TextStyle) -> f32 {
        self.calls.push((origin, text.to_string()));
        self.measure_text(text, _s).w
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

#[test]
fn draw_char_matches_draw_text() {
    let style = TextStyle::default();
    let pos = Pos::new(12.0, 34.0);

    for ch in ['M', '中', '😀', ' '] {
        let mut via_char = Recorder::default();
        let w_char = via_char.draw_char(pos, ch, &style);

        let mut via_text = Recorder::default();
        let w_text = via_text.draw_text(pos, &ch.to_string(), &style);

        assert_eq!(
            via_char.calls, via_text.calls,
            "{ch:?}：draw_char 与 draw_text 的绘制指令必须完全一致"
        );
        assert_eq!(
            w_char, w_text,
            "{ch:?}：返回值（前进宽度）必须一致"
        );
    }
}

#[test]
fn draw_char_returns_character_width() {
    let mut ctx = Recorder::default();
    let style = TextStyle::default();
    assert_eq!(ctx.draw_char(Pos::new(0.0, 0.0), 'M', &style), 8.0);
    assert_eq!(ctx.draw_char(Pos::new(0.0, 0.0), '中', &style), 16.0);
    assert_eq!(ctx.draw_char(Pos::new(0.0, 0.0), '😀', &style), 16.0);
}
