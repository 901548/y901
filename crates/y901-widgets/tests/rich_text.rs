//! 富文本：排版、代码块、流式追加的脏区局部性。

use std::cell::RefCell;
use std::rc::Rc;

use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle, UiTree};
use y901_widgets::{InlineStyle, RichText, Span};

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

/// 记录绘制调用。
#[derive(Default)]
struct Rec {
    texts: Rc<RefCell<Vec<(Pos, String, TextStyle)>>>,
    round: Rc<RefCell<Vec<(Rect, Color, f32)>>>,
}

impl TextMetrics for Rec {
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

impl RenderCtx for Rec {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
        self.texts
            .borrow_mut()
            .push((origin, text.to_string(), *style));
        text.chars().count() as f32 * 8.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
    fn fill_round_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        self.round.borrow_mut().push((rect, color, radius));
    }
}

struct Sink;

impl TextMetrics for Sink {
    fn measure_text(&self, _t: &str, _s: &TextStyle) -> Size {
        Size::ZERO
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for Sink {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

fn build(widget: RichText) -> UiTree {
    let mut tree = UiTree::new(Box::new(
        y901_ui::Flex::column("root").push(Box::new(widget)),
    ));
    tree.layout(Size::new(320.0, 400.0), &Mono);
    tree
}

#[test]
fn paragraphs_and_code_block_render() {
    let mut rt = RichText::new("rt");
    rt.push_paragraph(vec![
        Span::new("hello "),
        Span::styled("bold", InlineStyle::bold()),
        Span::styled(" red", InlineStyle::color(Color::rgba(1.0, 0.0, 0.0, 1.0))),
    ]);
    rt.push_code("rust", "fn main() {\n    println!(\"hi\");\n}");

    let mut tree = build(rt);
    tree.invalidate_all();
    let rec = Rec::default();
    let mut ctx = Rec {
        texts: rec.texts.clone(),
        round: rec.round.clone(),
    };
    tree.render(&mut ctx);

    let texts = rec.texts.borrow();
    let joined: String = texts.iter().map(|(_, t, _)| t.as_str()).collect();
    assert!(joined.contains("hello"), "段落文本应渲染：{joined:?}");
    assert!(joined.contains("bold"), "粗体片段应渲染：{joined:?}");
    assert!(joined.contains("fn main()"), "代码块内容应渲染：{joined:?}");

    // 粗体与颜色确实进了 TextStyle
    let bold = texts
        .iter()
        .find(|(_, t, _)| t == "bold")
        .expect("bold 片段");
    assert!(bold.2.bold, "粗体样式应传给文本引擎");
    let red = texts
        .iter()
        .find(|(_, t, _)| t == " red")
        .expect("red 片段");
    assert_eq!(red.2.fg, Color::rgba(1.0, 0.0, 0.0, 1.0));

    // 代码块必须画一个圆角背景
    let round = rec.round.borrow();
    assert_eq!(round.len(), 1, "代码块应画一个圆角背景");
    assert!(round[0].2 > 0.0, "代码块背景要带圆角");
}

#[test]
fn update_relayouts_and_returns_last_block_rect() {
    let mut rt = RichText::new("rt");
    rt.update(vec![y901_widgets::Block::Paragraph(vec![Span::new(
        "first line",
    )])]);
    let mut tree = build(rt);
    tree.invalidate_all();
    tree.render(&mut Sink);

    let before = {
        let mut h = 0.0;
        tree.with_node_mut("rt", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(r) = any.downcast_mut::<RichText>() {
                    h = r.content_height();
                }
            }
        });
        h
    };

    // 调用方自己把最后一段改长，然后 update 一次
    tree.with_node_mut("rt", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(r) = any.downcast_mut::<RichText>() {
                let mut blocks = r.blocks().to_vec();
                if let Some(y901_widgets::Block::Paragraph(spans)) = blocks.last_mut() {
                    spans.push(Span::new(" word".repeat(20)));
                }
                r.update(blocks);
            }
        }
    });
    tree.mark_layout_dirty("rt");
    tree.layout(Size::new(320.0, 400.0), &Mono);

    let (after, rect) = {
        let mut h = 0.0;
        let mut r = None;
        tree.with_node_mut("rt", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(rt2) = any.downcast_mut::<RichText>() {
                    h = rt2.content_height();
                    r = rt2.last_block_rect();
                }
            }
        });
        (h, r)
    };
    assert!(after > before, "更新后内容应变高：{before} → {after}");
    let rect = rect.expect("应有最后一块矩形");
    assert!(rect.h > 0.0 && rect.w > 0.0, "块矩形必须有效：{rect:?}");
}

#[test]
fn update_then_partial_invalidate_stays_local() {
    let mut rt = RichText::new("rt");
    rt.update(vec![
        y901_widgets::Block::Paragraph(vec![Span::new("line one")]),
        y901_widgets::Block::Paragraph(vec![Span::new("line two")]),
    ]);
    let mut tree = build(rt);
    tree.invalidate_all();
    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty());

    // 1) 改数据
    tree.with_node_mut("rt", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(rt2) = any.downcast_mut::<RichText>() {
                let mut blocks = rt2.blocks().to_vec();
                if let Some(y901_widgets::Block::Paragraph(spans)) = blocks.last_mut() {
                    spans.push(Span::new("..."));
                }
                rt2.update(blocks);
            }
        }
    });
    // 2) 重排（update 只作废缓存，不会自己跑布局）
    tree.mark_layout_dirty("rt");
    tree.layout(Size::new(320.0, 400.0), &Mono);
    // 3) 拿最后一块的屏幕矩形
    let rect = {
        let mut r = None;
        tree.with_node_mut("rt", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(rt2) = any.downcast_mut::<RichText>() {
                    r = rt2.last_block_rect();
                }
            }
        });
        r.expect("应有最后一块矩形")
    };
    tree.invalidate_rect(rect);

    assert!(!tree.dirty().is_full(), "局部更新不能退化成整窗重绘");
    let b = tree.dirty().bounds().expect("应有脏区");
    // `invalidate_rect` 会按墨迹溢出量外扩（上下各 >=2px），所以容差 +5。
    assert!(
        b.h <= rect.h + 5.0,
        "脏区应只覆盖最后一块（高度 {} vs 块高 {}）",
        b.h,
        rect.h
    );
}

#[test]
fn code_block_keeps_blank_lines() {
    let mut rt = RichText::new("rt");
    rt.push_code("txt", "a\n\nb");
    let mut tree = build(rt);
    tree.invalidate_all();
    let rec = Rec::default();
    let mut ctx = Rec {
        texts: rec.texts.clone(),
        round: rec.round.clone(),
    };
    tree.render(&mut ctx);
    let texts = rec.texts.borrow();
    assert_eq!(texts.len(), 3, "三行（含空行）都应各画一次：{texts:?}");
    assert_eq!(texts[1].1, "", "中间应是空行");
}
