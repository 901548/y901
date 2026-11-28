//! **`TextInput` 多状态叠加的不变量：密码 × 占位 × IME 组合态**。
//!
//! 这三个状态单独看都简单，叠在一起才是事故现场：
//!
//! - **密码**：原文一个字都不能出现在**语义**（外部工具只读语义）或**绘制文本**里 ——
//!   只挡一处等于没挡；
//! - **占位提示**：不得进语义 label/value/description（敏感应用的提示会被读走），
//!   这条是 `password_placeholder.rs` 明确要求的；
//! - **IME 预编辑**：组字串只画不提交，组字中的 Backspace/Delete **不能碰正文**
//!   （删字归输入法管）。
//!
//! 本文件把三者交叉验证：掩码长度按**字符数**而非字节数（"中" 显示 1 个 `*`，
//! 不是 3 个 —— 按字节数打码会直接泄漏"这个字段里有几个字节的中文"）。

use y901_ui::{
    Color, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};
use y901_widgets::TextInput;

struct Rec {
    texts: Vec<String>,
    fills: Vec<(Rect, Color)>,
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
        Size::new(320.0, 100.0)
    }
    fn fill_rect(&mut self, r: Rect, c: Color) {
        self.fills.push((r, c));
    }
    fn stroke_rect(&mut self, _r: Rect, _c: Color, _w: f32) {}
    fn draw_text(&mut self, _o: Pos, t: &str, _s: &TextStyle) -> f32 {
        self.texts.push(t.to_string());
        t.chars().count() as f32 * 8.0
    }
    fn push_clip(&mut self, _r: Rect) {}
    fn pop_clip(&mut self) {}
}

fn tree_with(input: TextInput) -> UiTree {
    let mut tree = UiTree::new(Box::new(
        y901_ui::layout::Flex::column("root").push(Box::new(input)),
    ));
    tree.layout(Size::new(320.0, 100.0), &Metrics);
    tree.set_focus(Some("ti".to_string()));
    tree.dispatch(&UiEvent::Focus(true));
    tree.layout(Size::new(320.0, 100.0), &Metrics);
    tree
}

struct Metrics;
impl TextMetrics for Metrics {
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

fn draw(tree: &mut UiTree) -> Rec {
    let mut rec = Rec {
        texts: Vec::new(),
        fills: Vec::new(),
    };
    tree.invalidate_all();
    tree.render(&mut rec);
    rec
}

fn widget_text(tree: &mut UiTree) -> String {
    tree.node_as_mut::<TextInput>("ti")
        .map(|w| w.text().to_string())
        .unwrap_or_default()
}

fn widget_sem(tree: &mut UiTree) -> Semantics {
    tree.node_as_mut::<TextInput>("ti")
        .map(|w| Widget::semantics(w))
        .unwrap_or_default()
}

/// **密码模式：原文一个字都不能出现在语义或绘制文本里**。
#[test]
fn password_mode_never_leaks_the_plaintext() {
    let secrets = ["hunter2", "中文密码", "p@ss w0rd!", "s3cr3t!"];
    for secret in secrets {
        let tree = tree_with(TextInput::new("ti").with_text(secret).password(true));
        let sem = {
            let mut t = tree;
            widget_sem(&mut t)
        };
        assert_eq!(
            sem.value.chars().count(),
            secret.chars().count(),
            "掩码长度必须等于字符数（中文 1 个 *，不是 3 个）：secret={secret:?} value={:?}",
            sem.value
        );
        assert!(
            sem.value.chars().all(|c| c == '*'),
            "语义 value 里出现了非掩码字符：secret={secret:?} value={:?}",
            sem.value
        );
        assert!(
            !sem.label.contains(secret) && !sem.description.contains(secret),
            "语义 label/description 泄漏了原文：secret={secret:?} label={:?}",
            sem.label
        );
    }
}

/// **密码模式的绘制文本也必须是纯掩码**（不能只挡语义挡不住像素）。
#[test]
fn password_mode_draws_only_masks() {
    for secret in ["hello", "中文", "x y"] {
        let mut tree = tree_with(TextInput::new("ti").with_text(secret).password(true));
        let rec = draw(&mut tree);
        let joined: String = rec.texts.join("");
        assert!(
            !joined.is_empty(),
            "密码框应画出掩码占位（secret={secret:?}）"
        );
        for ch in joined.chars() {
            assert_eq!(
                ch, '*',
                "绘制文本里出现非掩码字符 {ch:?}：secret={secret:?} 画出={joined:?}"
            );
        }
        assert_eq!(
            joined.chars().count(),
            secret.chars().count(),
            "掩码个数应等于字符数：secret={secret:?} 画出={joined:?}"
        );
    }
}

/// **占位提示不得进入语义**（敏感应用的提示会被外部工具读走）。
#[test]
fn placeholder_never_becomes_the_semantic_label() {
    let mut tree = tree_with(
        TextInput::new("ti")
            .placeholder("请输入口令")
            .password(true),
    );
    let sem = widget_sem(&mut tree);
    assert!(
        !sem.label.contains("请输入口令"),
        "占位提示泄漏进了 label：{:?}",
        sem.label
    );
    assert!(
        !sem.value.contains("请输入口令"),
        "占位提示泄漏进了 value：{:?}",
        sem.value
    );
    assert!(
        !sem.description.contains("请输入口令"),
        "占位提示泄漏进了 description：{:?}",
        sem.description
    );
}

/// **IME 预编辑串不得进入正文**，且组合态下删键不碰缓冲区。
#[test]
fn preedit_never_enters_the_buffer_and_blocks_deletion() {
    let mut tree = tree_with(TextInput::new("ti").with_text("abc"));
    // 组字中：预编辑串只画不提交
    tree.dispatch(&UiEvent::Compose("zhong".to_string()));
    assert_eq!(widget_text(&mut tree), "abc", "组字中不应改正文");
    // 组字中按 Backspace：必须被吃掉，不动正文
    tree.dispatch(&UiEvent::Key {
        key: y901_ui::Key::Backspace,
        modifiers: y901_ui::Modifiers::default(),
    });
    assert_eq!(
        widget_text(&mut tree),
        "abc",
        "组字中按 Backspace 不能删正文（删字归输入法管）"
    );
    // 提交：预编辑串进入正文
    tree.dispatch(&UiEvent::Text("中".to_string()));
    assert!(
        widget_text(&mut tree).starts_with("abc"),
        "提交后正文应保留原内容：{:?}",
        widget_text(&mut tree)
    );
    assert!(
        widget_text(&mut tree).contains('中'),
        "提交的字符应进入正文：{:?}",
        widget_text(&mut tree)
    );
}
