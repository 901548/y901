//! Grid 溢出的**实际行为**记录：目前不裁剪。
//!
//! ## 这不是"验收通过"，而是把一个已知缺口写成可执行的记录
//!
//! 设计意图是"子控件超出单元格时裁剪"。实现时发现框架的渲染模型不允许容器裁子控件：
//!
//! - 树是**逐个节点**渲染的（`render_pass` 递归），容器不负责画子控件；
//! - 裁剪只能由节点自己通过 `Widget::clip()` 声明；
//! - 容器**无法**为子控件设置 clip（没有这个 API）。
//!
//! 所以"容器裁剪溢出内容"需要框架级 API（例如 `children_clip()` 或让布局把
//! 单元格 clip 交给子控件）。在那之前，本测试记录真实行为，避免文档与实现不符。

#![cfg(test)]

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::grid::{Grid, GridTrack};
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, Edges, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    UiEvent, Widget,
};

const W: u32 = 200;
const H: u32 = 200;

fn soft() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let cell = Size::new(cw, ch);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        cell,
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

/// 无视给定尺寸、硬画 100×100 的子控件。
struct Overflow {
    id: String,
    rect: Rect,
}

impl Widget for Overflow {
    fn id(&self) -> &str {
        &self.id
    }
    fn measure(&mut self, _m: &dyn TextMetrics, _c: Constraints, _c2: &mut LayoutCache) -> Size {
        Size::new(100.0, 100.0)
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _c: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(Rect::new(self.rect.x, self.rect.y, 100.0, 100.0), Color::WHITE);
    }
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

#[test]
fn overflow_is_visible_today_because_containers_cannot_clip_children() {
    let mut soft = soft();
    soft.clear();

    let grid = Grid::new(
        "g",
        vec![GridTrack::Fixed(50.0), GridTrack::Fixed(50.0)],
        vec![GridTrack::Fixed(50.0)],
    )
    .padding(Edges::ZERO)
    .with_children(vec![Box::new(Overflow {
        id: "big".into(),
        rect: Rect::ZERO,
    })]);

    let mut tree = y901_ui::UiTree::new(Box::new(grid));
    tree.layout(Size::new(W as f32, H as f32), &soft);
    tree.invalidate_all();
    tree.render(&mut soft);

    let white = |x: u32, y: u32| -> bool {
        let p = soft.pixel(x, y).expect("像素");
        let (r, g, b) = (
            ((p >> 16) & 0xFF) as u8,
            ((p >> 8) & 0xFF) as u8,
            (p & 0xFF) as u8,
        );
        r > 200 && g > 200 && b > 200
    };

    assert!(white(25, 25), "单元格内应被内容覆盖");
    // 记录真实行为：单元格高 50，但子控件画到 100，y=75 处仍然是白的。
    assert!(
        white(25, 75),
        "若这里变成 false，说明框架已支持容器裁剪子控件，请把本测试改成断言下方应被裁剪"
    );

    eprintln!("[grid-clip] 记录：溢出内容当前不被裁剪（容器无法为子控件设置 clip）");
}
