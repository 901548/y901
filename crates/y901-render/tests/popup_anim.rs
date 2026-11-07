//! 弹层开启动画的像素验收：中间帧必须比终态"小一圈"。

#![cfg(test)]

use std::time::Duration;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Rect, RenderCtx, Semantics, Size, TextMetrics,
    UiEvent, UiTree, Widget,
};

const W: u32 = 240;
const H: u32 = 180;

/// 根节点：只占位。
struct Root {
    rect: Rect,
}

impl Widget for Root {
    fn id(&self) -> &str {
        "root"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(W as f32, H as f32))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 弹层内容：把自己的矩形涂白，便于用墨迹包围盒度量尺寸。
struct WhitePanel {
    rect: Rect,
}

impl Widget for WhitePanel {
    fn id(&self) -> &str {
        "panel"
    }
    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(120.0, 80.0))
    }
    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        ctx.fill_rect(self.rect, Color::WHITE);
    }
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
    SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("soft")
}

/// 亮像素（面板白底）的包围盒（`None` = 没有面板）。
///
/// 为什么必须用"亮"而不是"与背景不同"：弹层的淡入遮罩是一层 alpha 很低的背景色矩形，
/// 它盖住了**整个终态矩形**——用"非背景"判定会把遮罩算进包围盒，于是首帧和终态一样大，
/// 看起来像"变换没生效"。实测正是踩了这个坑（探针显示 fill_rect 的 transforms.len()=1，
/// 但被遮罩的矩形撑满了 bbox）。
fn panel_bbox(s: &SoftBackend) -> Option<(u32, u32, u32, u32)> {
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..H {
        for x in 0..W {
            let p = s.pixel(x, y).expect("像素");
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            // 半程时遮罩把白压到 ~224，仍高于 200
            if r < 200 || g < 200 || b < 200 {
                continue;
            }
            bbox = Some(match bbox {
                None => (x, y, x, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
            });
        }
    }
    bbox
}

#[test]
fn popup_open_animation_shrinks_the_first_frame() {
    let mut soft = soft();
    let mut tree = UiTree::new(Box::new(Root { rect: Rect::ZERO }));
    tree.layout(Size::new(W as f32, H as f32), &soft);

    // 打开弹层：push_popup 内部会启动 150ms 动画，此刻进度 = 0
    assert!(
        tree.open_popup(
            Box::new(WhitePanel { rect: Rect::ZERO }),
            Rect::new(100.0, 80.0, 10.0, 10.0)
        ),
        "弹层应打开"
    );
    tree.layout(Size::new(W as f32, H as f32), &soft);
    tree.invalidate_all();

    // 第一帧（progress = 0）整块被淡入遮罩盖住 → 没有画面对比意义；
    // 取"半程"（progress ≈ 0.5）与终态比：此时既缩放过、也还没完全显现。
    assert!(
        tree.tick_animations(Duration::from_millis(75)),
        "75ms 时动画应仍在推进"
    );
    tree.invalidate_all();
    soft.clear();
    tree.render(&mut soft);
    let first = panel_bbox(&soft).expect("半程应有弹层像素");

    // 推进到动画结束
    assert!(
        !tree.tick_animations(Duration::from_millis(200)),
        "275ms 后动画应已结束（总时长 150ms）"
    );
    tree.invalidate_all();
    soft.clear();
    tree.render(&mut soft);
    let last = panel_bbox(&soft).expect("终态应有弹层像素");

    let size = |b: (u32, u32, u32, u32)| (b.2 - b.0 + 1, b.3 - b.1 + 1);
    let (fw, fh) = size(first);
    let (lw, lh) = size(last);
    eprintln!("[popup] 第一帧 {fw}x{fh} @{first:?}，终态 {lw}x{lh} @{last:?}");

    assert!(
        fw < lw && fh < lh,
        "动画中间帧应比终态小：第一帧 {fw}x{fh} vs 终态 {lw}x{lh}"
    );
    // 缩放下限 0.6，首帧宽度应约为终态的 0.8（scale = 0.8 + 0.2*0）
    let ratio = fw as f32 / lw as f32;
    assert!(
        (0.93..0.99).contains(&ratio),
        "首帧宽度应约为终态的 0.975 倍（progress 0.875），实际 {ratio:.2}"
    );
}
