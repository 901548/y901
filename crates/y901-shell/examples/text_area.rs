//! 多行文本编辑示例（`TextArea`）。
//!
//! 运行：
//! ```text
//! cargo run -p y901-shell --example text_area
//! ```
//!
//! 验证点（打开窗口后可以手动确认）：
//! - 输入超过一行宽度会**软换行**，窗口变窄/变宽会重新折行；
//! - ↑/↓ 在**可视行**之间移动，并保持目标列（连续按不会左右漂移）；
//! - 回车插入真正的换行；在行首退格会把两行合并；
//! - 鼠标拖拽可以跨行选中；Ctrl+A / Ctrl+C / Ctrl+V 走应用注入的剪贴板；
//! - 文本超过可视高度时自动向下滚动，保证光标可见。

use std::time::Instant;

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Color, Command, Edges, Flex, FlexItem, UiTree};
use y901_widgets::{TextArea, TextLabel};

struct Shell;

impl Host for Shell {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in tree.poll_subscriptions(Instant::now()) {}
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false
    }
}

fn build_shell() -> UiTree {
    let editor = TextArea::new("editor")
        .with_text(
            "Y901 TextArea\n\
             在多行编辑器里，软换行由控件自己算：它把每个逻辑行按可视宽度切成若干可视行，\n\
             然后把「字节偏移 ↔ 屏幕坐标」的映射缓存下来，供点击命中与上下移动使用。\n\
             \n\
             试着按住 ↓ 一直走到底：光标会保持在同一列，视图会跟着滚动。\n\
             再按几次 End / Home，看看它们是怎么回到可视行首尾的。",
        )
        .placeholder("在这里输入…")
        .padding(Edges::symmetric(10.0, 8.0))
        .flex_item(FlexItem::fill());

    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(20.0))
        .push(Box::new(
            TextLabel::new("title", "TextArea 演示")
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(28.0)),
        ))
        .push(Box::new(editor));

    let mut tree = UiTree::new(Box::new(root));
    // 焦点给编辑器：打开就能直接打字。
    tree.set_focus(Some("editor".to_string()));
    tree
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(Shell),
        Box::new(build_shell),
        Options {
            title: "Y901 · TextArea".to_string(),
            width: 760,
            height: 520,
            ..Default::default()
        },
    )
}
