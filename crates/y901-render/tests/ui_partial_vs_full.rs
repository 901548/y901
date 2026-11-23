//! **局部重绘 == 全量重绘**（CPU 软光栅版，跨平台）。
//!
//! 为什么需要它：Windows 上有一条 `snapshot_harness::invalidate_rect_partial_equals_full`
//! 做同样的事，但它 `#[ignore]`、需要真机 D3D11、且只在 Windows 跑。
//! 而"布局变化后脏区是否覆盖了所有变化的像素"是**框架核心不变量** ——
//! 一旦 `UiTree` 的脏区推导（旧 rect ∪ 新 rect、墨迹外扩、跳过布局时不动索引表）
//! 出问题，表现就是残影、错位、旧内容擦不掉，而这类 bug 靠肉眼很难在 CI 里抓到。
//!
//! 这里用 `SoftBackend`（纯 CPU）把同一份状态画两遍：
//!
//! - **局部**：接着上一帧的缓冲，只按脏区重绘；
//! - **全量**：全新缓冲，`invalidate_all()` 后整树重绘。
//!
//! 两者必须**逐像素相等**。
//!
//! 放在 `y901-render/tests/` 而不是 `y901-ui`：需要真实光栅器，而 ui 不能依赖 render
//! （会形成发布死锁，见 `ui_hover_snapshot.rs` 顶部说明）。

use std::cell::Cell;
use std::rc::Rc;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Color, Constraints, EventCtx, EventResult, Flex, FlexItem, Rect, RenderCtx, Semantics, Size,
    TextMetrics, TextStyle, UiTree, Widget,
};

const W: u32 = 160;
const H: u32 = 120;

// ---------------------------------------------------------------- 度量

struct Mono;

impl TextMetrics for Mono {
    fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
        Size::new(text.chars().count() as f32 * 8.0, 16.0)
    }
    fn line_height(&self, _s: &TextStyle) -> f32 {
        16.0
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

// ---------------------------------------------------------------- 可变块

/// 一个纯色块：高度与颜色都来自外部 `Cell`，改了就 bump 版本号。
struct Block {
    id: &'static str,
    height: Rc<Cell<f32>>,
    color: Rc<Cell<[u8; 3]>>,
    version: Rc<Cell<u64>>,
    rect: Rect,
}

impl Widget for Block {
    fn id(&self) -> &str {
        self.id
    }

    fn layout_version(&self) -> u64 {
        self.version.get()
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(c.max.w, self.height.get()))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn flex(&self) -> FlexItem {
        FlexItem::height(self.height.get())
    }

    fn event(&mut self, _c: &mut EventCtx, _e: &y901_ui::UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let [r, g, b] = self.color.get();
        let c = Color::from_rgb8(r, g, b);
        ctx.fill_rect(self.rect, c);
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 一个可变块的三个共享句柄：高度、颜色、版本号。
///
/// 单独起别名是为了让 `Scene`/`Block` 的签名可读 —— 直接写四元组会被 clippy 判
/// `type_complexity`，而这就是个测试夹具，不值得为它写泛型。
type BlockHandles = (
    &'static str,
    Rc<Cell<f32>>,
    Rc<Cell<[u8; 3]>>,
    Rc<Cell<u64>>,
);

struct Scene {
    blocks: Vec<BlockHandles>,
}

impl Scene {
    fn new(heights: &[f32]) -> Self {
        let blocks = heights
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let id: &'static str = Box::leak(format!("b{i}").into_boxed_str());
                // 每块颜色不同，改动才可能在像素上体现出来
                let color = [(i as u8 + 1) * 40, 60, 200 - (i as u8) * 30];
                (
                    id,
                    Rc::new(Cell::new(*h)),
                    Rc::new(Cell::new(color)),
                    Rc::new(Cell::new(0u64)),
                )
            })
            .collect();
        Self { blocks }
    }

    fn tree(&self) -> UiTree {
        let mut col = Flex::column("root");
        for (id, h, color, version) in &self.blocks {
            col = col.push(BlockRef::make(
                id,
                h.clone(),
                color.clone(),
                version.clone(),
            ));
        }
        UiTree::new(Box::new(col))
    }

    /// 改高度：bump 版本 + 让框架知道该节点要重排。
    fn set_height(&self, tree: &mut UiTree, index: usize, height: f32) {
        let (id, h, _, version) = &self.blocks[index];
        h.set(height);
        version.set(version.get() + 1);
        tree.mark_layout_dirty(id);
        tree.invalidate(id);
    }

    /// 改颜色：尺寸不变，只需重绘。
    fn set_color(&self, tree: &mut UiTree, index: usize, rgb: [u8; 3]) {
        let (id, _, color, version) = &self.blocks[index];
        color.set(rgb);
        version.set(version.get() + 1);
        tree.invalidate(id);
    }
}

/// 构造期辅助：把 `Block` 塞进 `Box<dyn Widget>`。
struct BlockRef;

impl BlockRef {
    fn make(
        id: &'static str,
        height: Rc<Cell<f32>>,
        color: Rc<Cell<[u8; 3]>>,
        version: Rc<Cell<u64>>,
    ) -> Box<dyn Widget> {
        Box::new(Block {
            id,
            height,
            color,
            version,
            rect: Rect::ZERO,
        })
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

fn snapshot(s: &SoftBackend) -> Vec<u8> {
    s.pixels().to_vec()
}

// ---------------------------------------------------------------- 用例

/// 改一个块的高度：后面的块整体下移/上移。
/// 局部重绘必须把"旧位置"和"新位置"都覆盖，否则旧像素留在屏幕上（残影）。
#[test]
fn height_change_partial_equals_full() {
    let scene = Scene::new(&[20.0, 20.0, 20.0, 20.0]);
    let mut tree = scene.tree();
    tree.layout(Size::new(W as f32, H as f32), &Mono);

    let mut backend = soft();
    tree.invalidate_all();
    tree.render(&mut backend);

    // 第 2 块变高 → 第 3、4 块下移
    scene.set_height(&mut tree, 1, 36.0);
    tree.layout(Size::new(W as f32, H as f32), &Mono);
    assert!(!tree.dirty().is_empty(), "布局变化后必须有脏区");
    tree.render(&mut backend);
    let partial = snapshot(&backend);

    // 对照：同一份状态，全新缓冲 + 整树重绘
    let mut fresh_tree = scene.tree();
    fresh_tree.layout(Size::new(W as f32, H as f32), &Mono);
    let mut fresh = soft();
    fresh_tree.invalidate_all();
    fresh_tree.render(&mut fresh);
    let full = snapshot(&fresh);

    assert_eq!(partial.len(), full.len());
    let diff = partial
        .iter()
        .zip(full.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(diff, 0, "局部重绘与全量重绘有 {diff} 字节不同（残影/漏画）");
}

/// 只改颜色（尺寸不变）：脏区应等于该块自身矩形，局部重绘后逐像素一致。
#[test]
fn colour_change_partial_equals_full() {
    let scene = Scene::new(&[20.0, 20.0, 20.0]);
    let mut tree = scene.tree();
    tree.layout(Size::new(W as f32, H as f32), &Mono);

    let mut backend = soft();
    tree.invalidate_all();
    tree.render(&mut backend);

    scene.set_color(&mut tree, 1, [250, 20, 20]);
    tree.layout(Size::new(W as f32, H as f32), &Mono);
    tree.render(&mut backend);
    let partial = snapshot(&backend);

    let mut fresh_tree = scene.tree();
    fresh_tree.layout(Size::new(W as f32, H as f32), &Mono);
    let mut fresh = soft();
    fresh_tree.invalidate_all();
    fresh_tree.render(&mut fresh);

    let diff = partial
        .iter()
        .zip(snapshot(&fresh).iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(diff, 0, "改色后局部重绘不一致：{diff} 字节");
}

/// 连续两帧都不变：第二帧应当**什么都不画**（脏区空 → `render` 早退），
/// 缓冲内容保持不变。这条盯的是"静止帧不该有 O(n) 工作"的契约。
#[test]
fn idle_frame_renders_nothing_and_keeps_pixels() {
    let scene = Scene::new(&[20.0, 20.0]);
    let mut tree = scene.tree();
    tree.layout(Size::new(W as f32, H as f32), &Mono);

    let mut backend = soft();
    tree.invalidate_all();
    tree.render(&mut backend);
    let first = snapshot(&backend);
    assert!(tree.dirty().is_empty(), "渲染后脏区必须清空");

    // 不动任何东西，再 layout + render 一次
    tree.layout(Size::new(W as f32, H as f32), &Mono);
    assert!(
        tree.layout_stats().skipped,
        "无变化时布局必须被跳过（否则静止帧仍在 O(n) 重算）"
    );
    tree.render(&mut backend);
    let second = snapshot(&backend);
    assert_eq!(first, second, "静止帧不应改变缓冲");
}

/// 块被移出可见区（高度改成 0）后再改回来：中间那次局部重绘必须擦掉旧像素。
#[test]
fn shrink_to_zero_then_grow_back() {
    let scene = Scene::new(&[30.0, 30.0, 30.0]);
    let mut tree = scene.tree();
    tree.layout(Size::new(W as f32, H as f32), &Mono);

    let mut backend = soft();
    tree.invalidate_all();
    tree.render(&mut backend);

    // 中间块塌成 0
    scene.set_height(&mut tree, 1, 0.0);
    tree.layout(Size::new(W as f32, H as f32), &Mono);
    tree.render(&mut backend);

    // 再长回来
    scene.set_height(&mut tree, 1, 30.0);
    tree.layout(Size::new(W as f32, H as f32), &Mono);
    tree.render(&mut backend);
    let partial = snapshot(&backend);

    let mut fresh_tree = scene.tree();
    fresh_tree.layout(Size::new(W as f32, H as f32), &Mono);
    let mut fresh = soft();
    fresh_tree.invalidate_all();
    fresh_tree.render(&mut fresh);

    let diff = partial
        .iter()
        .zip(snapshot(&fresh).iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(diff, 0, "塌缩再恢复后局部重绘不一致：{diff} 字节");
}
