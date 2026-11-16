//! **裁剪区泄漏回归测试**：渲染完之后，画布的裁剪区必须回到"无裁剪"。
//!
//! 症状来源：真机上点击输入框后整屏变黑，**只剩下那个输入框**。
//! 这种"只有某个控件可见"的画面几乎只有一个解释：某个组件的 `render`
//! 调用了 `push_clip` 却没有配对 `pop_clip`，于是**后续所有绘制都被裁在它的矩形里**。
//!
//! 检测手法：正常渲染一帧之后，**再画一个覆盖全屏的矩形**。
//! 如果裁剪区已经回到全屏，这个矩形应当覆盖 (0,0) 与右下角；
//! 若仍被限制在某个小组件上，这两个角的像素就不会变。
//!
//! 这个测试不依赖 GPU，也不依赖浏览器，任何一处 clip 泄漏都会让它变红。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Constraints, Edges, Flex, FlexItem, Rect, RenderCtx, Size, UiEvent, UiTree};
use y901_widgets::TextInput;

const W: u32 = 400;
const H: u32 = 200;

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), Size::new(cw, ch)).expect("soft")
}

fn tree_with(input: TextInput) -> UiTree {
    let root = Flex::column("root")
        .padding(Edges::all(8.0))
        .push(Box::new(input.flex_item(FlexItem::fixed(200.0, 32.0))));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(W as f32, H as f32), &backend());
    tree
}

/// 渲染一帧，然后画全屏矩形；断言四角都被涂成目标色（= 裁剪区已回到全屏）。
fn assert_clip_is_clean(tree: &mut UiTree, tag: &str) {
    let mut soft = backend();
    soft.clear();
    tree.render(&mut soft);

    // **关键探针**：现在裁剪区必须已是全屏，否则这个矩形只能覆盖一部分。
    let probe = Color::rgba(1.0, 0.0, 1.0, 1.0);
    soft.fill_rect(Rect::new(0.0, 0.0, W as f32, H as f32), probe);

    // 探针颜色的 BGRA 字节（soft 内部是 0xAARRGGBB，但 `pixel` 返回的就是这个值）
    let probe_rgb = 0x00FF00FFu32; // A=00 R=FF G=00 B=FF → 不透明版在下面算
    let _ = probe_rgb;
    let px = |x: u32, y: u32| soft.pixel(x, y).expect("pixel");
    let corners = [
        (0u32, 0u32),
        (W - 1, 0),
        (0, H - 1),
        (W - 1, H - 1),
    ];
    for (x, y) in corners {
        // 探针是不透明品红：RGB 必须一致（alpha 已在 §探针里写死为 0xFF）
        let got = px(x, y);
        assert_eq!(
            got & 0x00FF_FFFF,
            0x00FF_00FF,
            "[{tag}] 角落 ({x},{y}) 没被全屏矩形覆盖：\
             说明渲染结束后裁剪区仍被某个组件限制着（clip 泄漏）"
        );
    }
}

#[test]
fn unfocused_text_input_does_not_leak_clip() {
    let mut tree = tree_with(TextInput::new("t-in"));
    assert_clip_is_clean(&mut tree, "未聚焦");
}

#[test]
fn focused_text_input_does_not_leak_clip() {
    let mut tree = tree_with(TextInput::new("t-in").with_text("hello"));
    tree.set_focus(Some("t-in".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    assert_clip_is_clean(&mut tree, "聚焦");
}

#[test]
fn text_input_with_preedit_does_not_leak_clip() {
    // 组字中（有预编辑串）会多画一层高亮，它最容易引入额外的 push_clip
    let mut tree = tree_with(TextInput::new("t-in"));
    tree.set_focus(Some("t-in".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    tree.dispatch(&UiEvent::Compose("nihao".to_string()));
    assert_clip_is_clean(&mut tree, "组字中");
}

/// 输入框太小、装不下内容时（inner 极窄）也不能泄漏。
#[test]
fn tiny_text_input_does_not_leak_clip() {
    let root = Flex::column("root").push(Box::new(
        TextInput::new("t-in")
            .with_text("abcdefghij")
            .flex_item(FlexItem::fixed(6.0, 10.0)),
    ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(W as f32, H as f32), &backend());
    tree.set_focus(Some("t-in".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    assert_clip_is_clean(&mut tree, "极窄");
}

/// **全量帧下的空矩形节点**：`render_pass` 会跳过它的 render，
/// 不能因此把外层裁剪状态搞乱。
#[test]
fn empty_rect_sibling_does_not_break_clip_stack() {
    let root = Flex::column("root")
        .push(Box::new(
            TextInput::new("a-in").flex_item(FlexItem::fixed(0.0, 0.0)),
        ))
        .push(Box::new(
            TextInput::new("b-in").flex_item(FlexItem::fixed(120.0, 24.0)),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.layout(Size::new(W as f32, H as f32), &backend());
    tree.set_focus(Some("b-in".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    assert_clip_is_clean(&mut tree, "零尺寸兄弟");
}

/// 一个约束不被使用的静态断言，避免 `Constraints` 未使用告警（该类型在别处用于布局）。
#[test]
fn constraints_are_available() {
    let _ = Constraints::loose(Size::new(10.0, 10.0));
}
