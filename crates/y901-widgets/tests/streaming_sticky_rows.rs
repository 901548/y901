//! 流式输出 / 粘底 / 多列列表 —— 三项基建的回归测试。
//!
//! 1. `UiTree::invalidate_rect`：流式 token 追加只标脏小矩形，绝不退化成整窗。
//! 2. `ScrollView` 粘底：内容变高自动跟到底部；用户上滚后失效，滚回底部再激活。
//! 3. `ListView::RowData`：多列 + 图标；`invalidate_row` 交出单行矩形。

use std::cell::Cell;
use std::rc::Rc;

use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, FlexItem, Modifiers, Pos, Rect, RenderCtx,
    Semantics, Size, TextStyle, TextMetrics, UiEvent, UiTree,
};
use y901_widgets::{ListView, ScrollView};

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

/// 记录文字的探针：验证多列渲染确实画了每一列。
#[derive(Default)]
struct TextRec {
    texts: Rc<std::cell::RefCell<Vec<String>>>,
    fills: Rc<std::cell::RefCell<Vec<Rect>>>,
}

impl TextMetrics for TextRec {
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

impl RenderCtx for TextRec {
    fn size(&self) -> Size {
        Size::new(400.0, 300.0)
    }
    fn fill_rect(&mut self, r: Rect, _c: Color) {
        self.fills.borrow_mut().push(r);
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, t: &str, _s: &TextStyle) -> f32 {
        self.texts.borrow_mut().push(t.to_string());
        t.chars().count() as f32 * 8.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

// ---------------------------------------------------------------- 任务 1

#[test]
fn invalidate_rect_keeps_dirty_local() {
    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.render(&mut Sink);
    assert!(tree.dirty().is_empty());

    // 模拟流式输出：右下角一小块反复标脏 100 次
    for i in 0..100 {
        let y = 260.0 + (i % 5) as f32;
        tree.invalidate_rect(Rect::new(300.0, y, 60.0, 12.0));
        assert!(!tree.dirty().is_full(), "invalidate_rect 绝不能退化成整窗");
    }

    let bounds = tree.dirty().bounds().expect("应有脏区");
    assert!(
        bounds.w <= 100.0 && bounds.h <= 32.0,
        "脏区必须局限在小矩形内，实际 {bounds:?}"
    );
    assert!(bounds.x >= 290.0, "脏区应贴在右下角，实际 {bounds:?}");
}

#[test]
fn invalidate_rect_clamps_and_ignores_empty() {
    let mut tree = UiTree::new(Box::new(Flex::column("root")));
    tree.layout(Size::new(400.0, 300.0), &Mono);
    tree.render(&mut Sink);

    tree.invalidate_rect(Rect::ZERO);
    assert!(tree.dirty().is_empty(), "空矩形不应标脏");

    // 完全在视口外
    tree.invalidate_rect(Rect::new(1000.0, 1000.0, 50.0, 50.0));
    assert!(tree.dirty().is_empty(), "视口外不应标脏");

    // 部分越界：应被裁到视口内
    tree.invalidate_rect(Rect::new(380.0, 280.0, 100.0, 100.0));
    let b = tree.dirty().bounds().expect("应有脏区");
    assert!(b.right() <= 400.0 && b.bottom() <= 300.0, "必须裁到视口：{b:?}");
}

// ---------------------------------------------------------------- 任务 2

fn scroll_with_content(rows: usize) -> UiTree {
    let mut col = Flex::column("content");
    for i in 0..rows {
        col = col.push(Box::new(y901_widgets::TextLabel::new(
            format!("r{i}"),
            format!("row {i}"),
        )));
    }
    let sv = ScrollView::new("sv", Box::new(col)).stick_to_bottom(true);
    let mut tree = UiTree::new(Box::new(sv));
    tree.layout(Size::new(300.0, 200.0), &Mono);
    tree
}

#[test]
fn stick_to_bottom_follows_content_growth() {
    // 先放少量内容（不需要滚动）
    let mut tree = scroll_with_content(3);
    tree.render(&mut Sink);
    let before = tree.rect_of("sv").expect("sv");
    assert!(before.h > 0.0);

    // 内容变多：重新布局后应自动停在底部
    let mut col = Flex::column("content");
    for i in 0..80 {
        col = col.push(Box::new(y901_widgets::TextLabel::new(
            format!("r{i}"),
            format!("row {i}"),
        )));
    }
    let mut sv = ScrollView::new("sv", Box::new(col)).stick_to_bottom(true);
    // 手动推到"底部"状态
    sv.set_stick_to_bottom(true);
    let mut tree2 = UiTree::new(Box::new(sv));
    tree2.layout(Size::new(300.0, 200.0), &Mono);

    let at_bottom = {
        let mut ok = false;
        tree2.with_node_mut("sv", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(s) = any.downcast_mut::<ScrollView>() {
                    ok = s.is_at_bottom();
                }
            }
        });
        ok
    };
    assert!(at_bottom, "开启粘底后内容超出时必须停在底部");
}

#[test]
fn user_scroll_up_disarms_stick_until_back_at_bottom() {
    let mut tree = scroll_with_content(80);
    // 先到底
    tree.with_node_mut("sv", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(s) = any.downcast_mut::<ScrollView>() {
                s.scroll_to_end();
            }
        }
    });

    let viewport = tree.rect_of("sv").expect("sv");
    // 用户上滚
    let _ = tree.dispatch(&UiEvent::Wheel {
        delta: 120.0,
        pos: Pos::new(viewport.x + 10.0, viewport.y + 10.0),
        mods: Modifiers::default(),
    });
    tree.layout(Size::new(300.0, 200.0), &Mono);

    let stuck_after_up = {
        let mut at_bottom = false;
        tree.with_node_mut("sv", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(s) = any.downcast_mut::<ScrollView>() {
                    at_bottom = s.is_at_bottom();
                }
            }
        });
        at_bottom
    };
    assert!(!stuck_after_up, "用户上滚后不应仍在底部");

    // 再滚回底部：粘底重新激活
    tree.with_node_mut("sv", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(s) = any.downcast_mut::<ScrollView>() {
                s.scroll_to_end();
            }
        }
    });
    let back = {
        let mut at_bottom = false;
        tree.with_node_mut("sv", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(s) = any.downcast_mut::<ScrollView>() {
                    at_bottom = s.is_at_bottom();
                }
            }
        });
        at_bottom
    };
    assert!(back, "滚回底部后应重新处于底部");
}

// ---------------------------------------------------------------- 任务 3

#[test]
fn listview_renders_multiple_columns_and_icon() {
    // 通用列表不存数据：行内容由回调自己画。
    // 这里模拟 Demo 的三列文件行（图标 + 名称 + 大小 + 时间）。
    let rows: Vec<(&str, &str, &str)> = vec![("src", "4 KiB", "10:20"), ("main.rs", "2 KiB", "10:21")];
    let lv = ListView::new("list", 20.0).len(rows.len()).painter(move |i, rect, ctx| {
        let Some((name, size, at)) = rows.get(i) else { return };
        let fg = ctx.theme().style(y901_ui::Role::TEXT, y901_ui::State::Normal).fg;
        let st = TextStyle::fg(fg);
        ctx.fill_round_rect(
            Rect::new(rect.x + 4.0, rect.y + 4.0, 12.0, 12.0),
            Color::rgba(0.9, 0.6, 0.2, 1.0),
            3.0,
        );
        ctx.draw_text(Pos::new(rect.x + 24.0, rect.y + 2.0), name, &st);
        ctx.draw_text(Pos::new(rect.x + 200.0, rect.y + 2.0), size, &st);
        ctx.draw_text(Pos::new(rect.x + 300.0, rect.y + 2.0), at, &st);
    });
    let mut tree = UiTree::new(Box::new(lv.flex_item(FlexItem::height(120.0))));
    tree.layout(Size::new(400.0, 200.0), &Mono);

    let rec = TextRec::default();
    tree.invalidate_all();
    let mut ctx = TextRec {
        texts: rec.texts.clone(),
        fills: rec.fills.clone(),
    };
    tree.render(&mut ctx);

    let texts = rec.texts.borrow();
    for expected in ["src", "4 KiB", "10:20", "main.rs", "2 KiB", "10:21"] {
        assert!(
            texts.iter().any(|t| t == expected),
            "第 {expected} 列没被画出来：{texts:?}"
        );
    }
}

#[test]
fn invalidate_row_returns_row_rect_and_stays_local() {
    let lv = ListView::from_strings("list", (0..50).map(|i| format!("row{i}")).collect());
    let mut tree = UiTree::new(Box::new(lv.flex_item(FlexItem::height(200.0))));
    tree.layout(Size::new(400.0, 200.0), &Mono);
    tree.render(&mut Sink);

    // 第 3 行的屏幕矩形
    let rect = {
        let mut out = None;
        tree.with_node_mut("list", &mut |w| {
            if let Some(any) = w.as_any_mut() {
                if let Some(l) = any.downcast_mut::<ListView>() {
                    out = l.invalidate_row(3);
                }
            }
        });
        out
    };
    let rect = rect.expect("第 3 行应可见");
    assert!(rect.h <= 20.5, "单行高度应≈row_height，实际 {rect:?}");

    tree.invalidate_rect(rect);
    assert!(!tree.dirty().is_full(), "单行标脏不能退化成整窗");
    let b = tree.dirty().bounds().expect("应有脏区");
    assert!(b.h <= 20.5, "脏区高度应只有一行：{b:?}");
}

#[test]
fn invalidate_row_out_of_range_returns_none() {
    let lv = ListView::from_strings("list", vec!["only".to_string()]);
    let mut tree = UiTree::new(Box::new(lv.flex_item(FlexItem::height(100.0))));
    tree.layout(Size::new(400.0, 200.0), &Mono);

    let mut got = Some(Rect::ZERO);
    tree.with_node_mut("list", &mut |w| {
        if let Some(any) = w.as_any_mut() {
            if let Some(l) = any.downcast_mut::<ListView>() {
                got = l.invalidate_row(99);
            }
        }
    });
    assert!(got.is_none(), "越界行应返回 None");
}

/// 让未使用的导入不触发告警（约束/事件类型在部分测试里用不到）。
// 测试 API：测试内的类型见证/未用常量，仅测试代码引用
#[allow(dead_code)]
fn _unused(_c: Constraints, _e: &mut EventCtx, _r: EventResult, _s: Semantics, _cell: &Cell<u32>) {}
