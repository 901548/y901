//! **焦点路径缓存必须跟着结构变化重解析**（回归用例）。
//!
//! `focus_path` / `captured_path` 存的是"子索引序列"，而键盘事件正是按这条路径投递的。
//! 只要同层增删一个节点，旧路径就会指向邻居：`tree.focus()` 仍报原来的 id，
//! 但按键全被发给另一个控件（或干脆丢掉）——直到用户重新点一次。
//!
//! 这条用例先聚焦第 2 个兄弟，再删掉第 1 个，然后验证按键仍到达聚焦的那个控件。

use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics,
    TextStyle, UiEvent, UiTree, Widget,
};

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

/// 一个可聚焦、会记录按键次数的叶子。
struct KeySink {
    id: String,
    rect: Rect,
    keys: std::rc::Rc<std::cell::Cell<u32>>,
}

impl Widget for KeySink {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(40.0, 20.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn focusable(&self) -> bool {
        true
    }
    fn event(&mut self, ctx: &mut EventCtx, e: &UiEvent) -> EventResult {
        if let UiEvent::Key { .. } = e {
            self.keys.set(self.keys.get() + 1);
            ctx.request_redraw();
            return EventResult::Handled;
        }
        if let UiEvent::MouseDown { .. } = e {
            ctx.request_focus(self.id.clone());
            return EventResult::Handled;
        }
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
}

struct Host {
    rect: Rect,
    children: Vec<Box<dyn Widget>>,
}

impl Widget for Host {
    fn id(&self) -> &str {
        "host"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(200.0, 100.0))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        let mut y = rect.y;
        for c in &mut self.children {
            let s = c.measure(m, Constraints::loose(rect.size()), cache);
            c.arrange(Rect::new(rect.x, y, s.w, s.h), m, cache);
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

#[test]
fn focus_survives_removing_an_earlier_sibling() {
    let a = std::rc::Rc::new(std::cell::Cell::new(0));
    let b = std::rc::Rc::new(std::cell::Cell::new(0));
    let c = std::rc::Rc::new(std::cell::Cell::new(0));
    let host = Host {
        rect: Rect::ZERO,
        children: vec![
            Box::new(KeySink {
                id: "a".into(),
                rect: Rect::ZERO,
                keys: a.clone(),
            }),
            Box::new(KeySink {
                id: "b".into(),
                rect: Rect::ZERO,
                keys: b.clone(),
            }),
            Box::new(KeySink {
                id: "c".into(),
                rect: Rect::ZERO,
                keys: c.clone(),
            }),
        ],
    };
    let mut tree = UiTree::new(Box::new(host));
    tree.layout(Size::new(200.0, 100.0), &Mono);

    // 点 b（第 2 个兄弟）聚焦
    tree.dispatch(&UiEvent::MouseDown {
        pos: Pos::new(5.0, 25.0),
        button: y901_ui::MouseButton::Left,
    });
    assert_eq!(tree.focus(), Some("b"), "点击应让 b 聚焦");

    // 删掉前面的兄弟 a：b 的索引路径从 [0,1] 变成 [0,0]
    assert!(tree.remove_node("a"), "应能移除 a");
    tree.layout(Size::new(200.0, 100.0), &Mono);

    // 按键必须仍然投递到 b
    tree.dispatch(&UiEvent::Key {
        key: y901_ui::Key::Enter,
        modifiers: y901_ui::Modifiers::default(),
    });
    assert_eq!(b.get(), 1, "按键应到达仍在聚焦的 b");
    assert_eq!(c.get(), 0, "按键不应误投到邻居 c");

    // 反向证据：若焦点路径没有重解析，b 的计数会是 0 而 c 可能被投递。
    assert_eq!(tree.focus(), Some("b"));
    let _ = (a.get(), Color::rgba(0.0, 0.0, 0.0, 1.0));
}
