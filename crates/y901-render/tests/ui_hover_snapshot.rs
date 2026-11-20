//! hover 快照是否真的注入到渲染上下文（端到端，真 `SoftBackend`）。
//!
//! **为什么在 render crate 而不是 ui crate**：这条用例需要真实光栅器，而光栅器
//! （`y901-render`）正常依赖 `y901-ui`。若 `y901-ui` 再对本 crate 有（哪怕是 dev）
//! 依赖，就形成 ui ↔ render 环 —— crates.io 首次发布会因此死锁（发布 ui 要求 render
//! 已存在，发布 render 又要求 ui 已存在；cargo 打包时会解析 dev-dependencies）。
//! 所以"需要像素的 UI 用例"统一放在这里：`crates/y901-render/tests/ui_*.rs`。

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, EventCtx, EventResult, MouseButton, Pos, Rect, RenderCtx, Semantics, Size,
    TextMetrics, UiEvent, UiTree, Widget,
};

/// 只用于 layout 的度量实现（不画字）。
struct Dummy;

impl TextMetrics for Dummy {
    fn measure_text(&self, text: &str, _s: &y901_ui::TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &y901_ui::TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

/// 一个占满客户区的叶子节点（命中测试就会落在它上面）。
struct Leaf {
    id: &'static str,
    rect: Rect,
}

impl Widget for Leaf {
    fn id(&self) -> &str {
        self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(64.0, 64.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Handled
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(64, 64, Box::new(engine), cell).expect("soft")
}

/// 渲染一帧后，后端必须记得本帧的 hovered 节点；
/// 没有被悬停的 id 必须为 false（防止"记住上一次"或"恒为 true"）。
#[test]
fn hover_snapshot_reaches_render_ctx() {
    let mut t = UiTree::new(Box::new(Leaf {
        id: "a",
        rect: Rect::ZERO,
    }));
    t.layout(Size::new(64.0, 64.0), &Dummy);
    t.dispatch(&UiEvent::MouseMove {
        pos: Pos::new(10.0, 10.0),
    });
    assert_eq!(t.hovered(), Some("a"), "鼠标进入后树应记录 hovered");

    let mut s = soft();
    t.invalidate_all();
    t.render(&mut s);
    // 渲染后，后端应记得 hovered 是 "a"
    use y901_ui::RenderCtx as _;
    assert!(s.is_hovered("a"), "渲染注入后 a 应处于 hovered");
    assert!(!s.is_hovered("b"), "未悬停的 id 不能为 true");
    let _ = MouseButton::Left; // 保持导入被使用（与搬迁前一致）
}
