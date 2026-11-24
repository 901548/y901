//! **widget 层的可达性与输入正确性**（回归用例组）。
//!
//! 这一组对应 2026-11-24 全量复审的发现：能力"只写在 event 里、没写进 trait"，
//! 以及输入路径上的可见缺陷。每条都能在修复前红。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, Key, Modifiers, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiEvent, UiTree, Widget,
};
use y901_widgets::{NumberInput, Table, TableColumn};

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

fn key(k: Key) -> UiEvent {
    UiEvent::Key {
        key: k,
        modifiers: Modifiers::default(),
    }
}

#[test]
fn table_is_focusable_so_its_keyboard_navigation_can_run() {
    let mut t = Table::new("t")
        .column(TableColumn::flex("name"))
        .row(vec!["a".into()])
        .row(vec!["b".into()])
        .row(vec!["c".into()]);
    let c = Constraints::loose(Size::new(200.0, 200.0));
    let mut cache = LayoutCache::new();
    let _ = t.measure(&Mono, c, &mut cache);

    assert!(
        t.focusable(),
        "Table 实现了 Key 处理与 scroll_selected_into_view，必须声明 focusable，\
         否则 UiTree 永远不会把按键发给它（键盘导航不可达）"
    );

    // 反向证据：真把按键送进去，选中行必须能动。
    let mut ctx = EventCtx::new(Some("t".into()), Size::new(200.0, 200.0));
    let _ = t.event(&mut ctx, &key(Key::Down));
    let after = t.selected_index();
    assert_eq!(after, Some(0), "第一次 ↓ 应选中第 0 行（当前无选中）");
    let mut ctx = EventCtx::new(Some("t".into()), Size::new(200.0, 200.0));
    let _ = t.event(&mut ctx, &key(Key::Down));
    assert_eq!(t.selected_index(), Some(1), "第二次 ↓ 应下移一行");
}

#[test]
fn number_input_can_enter_a_negative_number() {
    let mut n = NumberInput::new("n", 0.0).range(-10.0, 10.0);
    let c = Constraints::loose(Size::new(120.0, 24.0));
    let mut cache = LayoutCache::new();
    let _ = n.measure(&Mono, c, &mut cache);
    // 必须真的排一次位置：命中测试用 `self.rect`，未 arrange 时是 ZERO。
    n.arrange(Rect::new(0.0, 0.0, 120.0, 24.0), &Mono, &mut cache);

    // 聚焦（缓冲会被预填成当前值 "0"）
    let mut ctx = EventCtx::new(Some("n".into()), Size::new(120.0, 24.0));
    let _ = n.event(
        &mut ctx,
        &UiEvent::MouseDown {
            pos: y901_ui::Pos::new(2.0, 2.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    // 输入 "-5"
    for ch in ['-', '5'] {
        let mut ctx = EventCtx::new(Some("n".into()), Size::new(120.0, 24.0));
        let _ = n.event(&mut ctx, &UiEvent::Text(ch.to_string()));
    }
    let mut ctx = EventCtx::new(Some("n".into()), Size::new(120.0, 24.0));
    let _ = n.event(&mut ctx, &key(Key::Enter));
    assert_eq!(
        n.value(),
        -5.0,
        "必须能输入负数（旧实现把 '-' 丢掉，得到 +5）"
    );
}

#[test]
fn number_input_step_notifies_the_app() {
    let mut n = NumberInput::new("n", 0.0)
        .step(1.0)
        .command(y901_ui::Command::app("set-n", y901_ui::json::Json::Null));
    let c = Constraints::loose(Size::new(120.0, 24.0));
    let mut cache = LayoutCache::new();
    let _ = n.measure(&Mono, c, &mut cache);
    n.arrange(Rect::new(0.0, 0.0, 120.0, 24.0), &Mono, &mut cache);

    let mut ctx = EventCtx::new(Some("n".into()), Size::new(120.0, 24.0));
    let _ = n.event(
        &mut ctx,
        &UiEvent::MouseDown {
            pos: y901_ui::Pos::new(2.0, 2.0),
            button: y901_ui::MouseButton::Left,
        },
    );
    let mut ctx = EventCtx::new(Some("n".into()), Size::new(120.0, 24.0));
    let _ = n.event(&mut ctx, &key(Key::Up));
    assert_eq!(n.value(), 1.0, "↑ 应步进到 1");
    assert!(
        !ctx.commands.is_empty(),
        "步进改变了值，必须发命令通知应用（否则单向数据流会把它弹回旧值）"
    );
}

/// 一个只用来喂像素的容器（避免 `Table`/`NumberInput` 直接当根时缺少布局约束）。
struct Host {
    rect: Rect,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for Host {
    fn id(&self) -> &str {
        "host"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(300.0, 300.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        let mut y = rect.y;
        for ch in &mut self.children {
            let s = ch.measure(m, Constraints::loose(rect.size()), cache);
            ch.arrange(Rect::new(rect.x, y, s.w, s.h), m, cache);
            y += s.h;
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
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// Tab 必须能把焦点送进 Table（此前它不可聚焦，Tab 永远跳过）。
#[test]
fn tab_reaches_a_table() {
    let table = Table::new("t")
        .column(TableColumn::flex("name"))
        .row(vec!["a".into()])
        .row(vec!["b".into()]);
    let mut tree = UiTree::new(Box::new(Host {
        rect: Rect::ZERO,
        children: vec![Box::new(table), Box::new(NumberInput::new("n", 0.0))],
    }));
    tree.layout(Size::new(300.0, 300.0), &Mono);
    tree.set_focus(Some("host".into()));
    // 从无焦点开始 Tab 一圈：Table 必须在焦点序列里。
    let mut seen_table = false;
    for _ in 0..4 {
        tree.focus_next(1);
        if tree.focus() == Some("t") {
            seen_table = true;
            break;
        }
    }
    assert!(seen_table, "Tab 应能到达 Table，实际焦点序列里没有它");
    let _ = EventResult::Handled;
}
