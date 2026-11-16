//! 打字只该脏"输入框自己"：不能把整窗标脏。
//!
//! 症状来源：web 演示页里每敲一个字符都会出现一次整屏重绘（甚至连续 3~4 帧）。
//! 框架层的契约是：`TextInput` 的编辑返回 `Relayout`（内容尺寸可能变），
//! `finish_dispatch` 只把**目标矩形**标脏，随后布局用"旧 ∪ 新"补齐移动过的节点。
//! 本测试用"接近真实布局"的表单固化这条契约：
//! 1. 单次输入 → 需要重排，但脏区**不是整窗**，且落在输入框矩形内；
//! 2. 连续输入多个字符 → 每帧都保持局部脏区（不会累积到 16 块上限而降级整窗）；
//! 3. 固定宽度（Px）输入框与 Flex 输入框表现一致。

use y901_ui::{Flex, FlexItem, Rect, RenderCtx, Size, TextMetrics, TextStyle, UiEvent, UiTree};
use y901_widgets::{TextInput, TextLabel};

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        // 与真实字体同量级：ASCII 8px，CJK 16px
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

/// 不画任何东西的后端：只为让 `tree.render` 能清空脏区。
struct Null;

impl TextMetrics for Null {
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

impl RenderCtx for Null {
    fn size(&self) -> Size {
        Size::new(800.0, 600.0)
    }
    fn fill_rect(&mut self, _r: Rect, _c: y901_ui::Color) {}
    fn stroke_rect(&mut self, _r: Rect, _c: y901_ui::Color, _w: f32) {}
    fn draw_text(&mut self, _o: y901_ui::Pos, _t: &str, _s: &TextStyle) -> f32 {
        0.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

/// 还原演示页的表单骨架：标题 +（标签 + Flex 输入）+（标签 + 定宽输入）+ 状态行。
fn demo_like_tree() -> UiTree {
    let row = |name: &'static str, item: FlexItem| -> Flex {
        Flex::row(format!("{name}-row"))
            .gap(16.0)
            .push(Box::new(
                TextLabel::new(format!("{name}-lbl"), name).size(14.0),
            ))
            .push(Box::new(
                TextInput::new(format!("{name}-in"))
                    .with_text("admin")
                    .flex_item(item),
            ))
    };

    let root = Flex::column("root")
        .push(Box::new(TextLabel::new("title", "设置").size(18.0)))
        .push(Box::new(row("username", FlexItem::width(240.0))))
        .push(Box::new(row("port", FlexItem::width(100.0))))
        .push(Box::new(
            TextLabel::new("status", "上次保存：2026-11-14 10:30").size(12.0),
        ));

    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(800.0, 600.0), &Mono);
    tree
}

/// 渲染一帧以清空脏区（模拟主循环）。
fn settle(tree: &mut UiTree) {
    tree.render(&mut Null);
    tree.layout(Size::new(800.0, 600.0), &Mono);
    tree.render(&mut Null);
    assert!(tree.dirty().is_empty(), "前置：渲染后脏区应为空");
}

/// 单次输入：需要重排，但脏区必须局部（不整窗）。
#[test]
fn typing_marks_only_the_input_locally() {
    let mut tree = demo_like_tree();
    tree.set_focus(Some("username-in".to_string()));
    settle(&mut tree);

    let input = tree.rect_of("username-in").expect("input rect");
    assert!(input.w > 0.0 && input.h > 0.0, "前置：输入框已布局");

    let result = tree.dispatch(&UiEvent::Text("a".to_string()));
    assert!(
        result.needs_relayout(),
        "文本编辑必须要求重排（内容宽度可能变）"
    );
    assert!(
        !tree.dirty().is_full(),
        "打字不应把整窗标脏（症状：整屏重绘）"
    );

    for r in tree.dirty().rects() {
        // 允许外扩 1px（抗锯齿/边框），但不允许扩到整个窗口
        assert!(
            r.x >= input.x - 1.0
                && r.y >= input.y - 1.0
                && r.right() <= input.right() + 1.0
                && r.bottom() <= input.bottom() + 1.0,
            "脏区 {r:?} 必须落在输入框 {input:?} 内"
        );
    }
}

/// 连续输入：每一帧都保持局部脏区，不能因累积碎片降级为整窗。
#[test]
fn consecutive_keystrokes_stay_local() {
    let mut tree = demo_like_tree();
    tree.set_focus(Some("username-in".to_string()));
    settle(&mut tree);

    let input = tree.rect_of("username-in").expect("input rect");

    for (i, ch) in ["a", "b", "c", "d", "e"].iter().enumerate() {
        let result = tree.dispatch(&UiEvent::Text((*ch).to_string()));
        assert!(result.needs_relayout(), "第 {i} 次输入应要求重排");
        assert!(
            !tree.dirty().is_full(),
            "第 {i} 次输入把整窗标脏了（碎片累积降级？）"
        );
        for r in tree.dirty().rects() {
            assert!(
                r.right() <= input.right() + 1.0 && r.bottom() <= input.bottom() + 1.0,
                "第 {i} 次输入的脏区 {r:?} 超出输入框 {input:?}"
            );
        }
        // 走一帧：布局 + 渲染 + 清脏
        tree.layout(Size::new(800.0, 600.0), &Mono);
        tree.render(&mut Null);
        assert!(tree.dirty().is_empty(), "第 {i} 次渲染后应清空脏区");

        // 布局后输入框宽度必须保持稳定（Flex 宽度），否则说明尺寸跟着内容变了
        let after = tree.rect_of("username-in").expect("input rect");
        assert!(
            (after.w - input.w).abs() < 0.01,
            "第 {i} 次输入后宽度变了：{} -> {}",
            input.w,
            after.w
        );
    }
}

/// 定宽（Px）输入框：同样只脏自己。
#[test]
fn fixed_width_input_stays_local_too() {
    let mut tree = demo_like_tree();
    tree.set_focus(Some("port-in".to_string()));
    settle(&mut tree);

    let input = tree.rect_of("port-in").expect("port rect");
    tree.dispatch(&UiEvent::Text("8".to_string()));

    assert!(!tree.dirty().is_full(), "定宽输入框打字同样不该整窗重绘");
    for r in tree.dirty().rects() {
        assert!(
            r.right() <= input.right() + 1.0 && r.bottom() <= input.bottom() + 1.0,
            "脏区 {r:?} 超出定宽输入框 {input:?}"
        );
    }
}
