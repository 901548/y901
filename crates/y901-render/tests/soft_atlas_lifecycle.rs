//! **SoftBackend 的字体图集帧边界**回归（RENDER-BUG-02 / issue #8）。
//!
//! 背景：`MyFontEngine` 的 `begin_frame` / `take_dirty` 以前**只有 D3D11 后端**调用。
//! 软渲染（X11、Win32 `--force-soft`、Web）三条路径都不调，于是同一份引擎在两种后端
//! 行为分叉：
//!
//! 1. `frame` 恒为 0 → `evict_one` 的热集判定 `last_used < cutoff`（`cutoff = frame-512`）
//!    永远为假，只能走"全表最冷"回退，图集满时可能淘汰**正在用**的字形；
//! 2. `dirty` 只增不减（每个光栅化过的字形一条矩形，永不释放）。
//!
//! 修法：把帧边界收到 `RenderCtx::begin_frame`（`UiTree::render` 每帧调一次），
//! `SoftBackend` 在里面推进帧号并消费脏区。本文件钉住这两条，以及"热字形不被饿死"。
//!
//! 为什么这些用例能抓住回归：
//! - 去掉 `SoftBackend::begin_frame` 里的 `begin_frame()` → 帧号用例红；
//! - 去掉 `SoftBackend::begin_frame` 里的 `take_dirty()` → 脏区用例红（累积不再归零）。
//!
//! **为什么不断言"淘汰优先级"**：图集满后 `insert_into_atlas` 在 64 次驱逐失败后会
//! **整表重打包**（`SkylinePacker` 的空洞不回收），任何小图集用例都会被重打包主导，
//! 结果与 LRU 无关；而"不推进帧号"的对照实现里所有 `last_used` 都是 0（并列），
//! 淘汰靠 `HashMap` 迭代顺序，断言必然 flaky。帧号是否推进已由本文件直接钉住，
//! "驱逐选最冷"由 `font_engine::engine` 自己的 `glyphs_never_vanish_when_the_atlas_fills` 覆盖。

use std::cell::Cell;
use std::rc::Rc;

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::layout::LayoutCache;
use y901_ui::{
    Constraints, Pos, Rect, RenderCtx, Semantics, Size, TextMetrics, TextStyle, UiTree, Widget,
};

const W: u32 = 200;
const H: u32 = 90;
const PX: f32 = 15.0;

fn font_bytes() -> Vec<u8> {
    y901_render::load_font(&y901_render::default_test_font_path()).expect("测试字体可读")
}

fn cell() -> Size {
    let (cw, ch) = y901_render::cell_metrics_for(PX);
    Size::new(cw, ch)
}

/// 一个只画**一个字符**的叶子；字符由外部 `Rc<Cell<char>>` 控制，
/// 这样同一棵树可以在不同帧画不同字形（用来制造"每帧都有新字形"）。
struct GlyphLeaf {
    id: &'static str,
    rect: Rect,
    ch: Rc<Cell<char>>,
}

impl Widget for GlyphLeaf {
    fn id(&self) -> &str {
        self.id
    }

    fn measure(&mut self, _m: &dyn TextMetrics, c: Constraints, _cache: &mut LayoutCache) -> Size {
        c.constrain(Size::new(W as f32, 40.0))
    }

    fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = rect;
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        let text = self.ch.get().to_string();
        ctx.draw_text(
            Pos::new(self.rect.x + 4.0, self.rect.y + 20.0),
            &text,
            &TextStyle::default(),
        );
    }

    fn semantics(&self) -> Semantics {
        Semantics::default()
    }
}

/// 建一个软后端 + 单叶子树（字符可控）。
fn setup(ch: char) -> (SoftBackend, UiTree, Rc<Cell<char>>) {
    let cell = cell();
    let engine = MyFontEngine::new(font_bytes(), PX, cell).expect("字形引擎");
    let soft = SoftBackend::new_headless(W, H, Box::new(engine), cell).expect("软光栅器");
    let handle = Rc::new(Cell::new(ch));
    let mut tree = UiTree::new(Box::new(GlyphLeaf {
        id: "leaf",
        rect: Rect::ZERO,
        ch: handle.clone(),
    }));
    tree.layout(Size::new(W as f32, H as f32), &soft);
    (soft, tree, handle)
}

/// **帧号必须每帧 +1**：这是 LRU 的时钟。
///
/// 旧实现软路径从不调 `begin_frame` → 恒为 0。
#[test]
fn render_advances_atlas_frame_once_per_render() {
    let (mut soft, mut tree, _ch) = setup('A');
    assert_eq!(soft.font_frame(), 0, "新建后帧号应为 0");

    for i in 1..=3u64 {
        tree.invalidate_all();
        tree.render(&mut soft);
        assert_eq!(
            soft.font_frame(),
            i,
            "第 {i} 次 render 后帧号应为 {i}（每次 render 恰好推进一帧）"
        );
    }
}

/// **脏区必须每帧被消费**：软路径不上传图集，但增长必须停止。
///
/// 旧实现 `dirty` 只增不减：第 2 帧重复渲染同一字形，脏区数不会回到 0。
#[test]
fn atlas_dirty_is_consumed_every_frame() {
    let (mut soft, mut tree, ch) = setup('A');

    // 第 1 帧：光栅化 'A' → 产生 1 条脏区（本帧产生，下一帧开头被消费）
    tree.invalidate_all();
    tree.render(&mut soft);
    let first = soft.font_pending_dirty();
    assert_eq!(
        first, 1,
        "首帧光栅化 1 个字形 → 应恰好 1 条脏区，实际 {first}"
    );

    // 第 2 帧：同一字形命中缓存 → 不再光栅化；上一帧的脏区已在帧边界被取走
    tree.invalidate_all();
    tree.render(&mut soft);
    assert_eq!(
        soft.font_pending_dirty(),
        0,
        "上一帧脏区必须被消费（旧实现会一直累积在这里）"
    );

    // 之后每帧换一个新字形：脏区数只反映"本帧新光栅化的字形"，不随帧数增长
    let mut sizes = Vec::new();
    for (i, c) in ['B', 'C', 'D', 'E', 'F'].into_iter().enumerate() {
        ch.set(c);
        tree.invalidate_all();
        tree.render(&mut soft);
        sizes.push(soft.font_pending_dirty());
        assert!(
            sizes[i] <= 1,
            "第 {} 帧只光栅化 1 个新字形，脏区应 ≤ 1，实际 {:?}",
            i + 2,
            sizes
        );
    }
    assert_eq!(
        sizes,
        vec![1, 1, 1, 1, 1],
        "每帧只有 1 个新字形 → 每帧 1 条脏区；若累积会看到 2,3,4,…"
    );
    assert_eq!(soft.font_frame(), 7, "共 render 了 7 次");
}

/// 长时间重复渲染：脏区与内存占用都**不得随帧数增长**。
#[test]
fn dirty_does_not_grow_over_many_frames() {
    let (mut soft, mut tree, _ch) = setup('A');
    for _ in 0..30 {
        tree.invalidate_all();
        tree.render(&mut soft);
    }
    assert_eq!(soft.font_frame(), 30);
    assert_eq!(
        soft.font_pending_dirty(),
        0,
        "30 帧后不应留下任何未消费的脏区（旧实现会是 1，且每加一个新字形就 +1）"
    );
}

/// **光栅化量超过图集容量后，脏区仍然有界**（issue #8 验收第 3 条的"有界"部分）。
///
/// 62 个不同字形会把 64² 以上的图集反复挤满（含整表重打包）；无论怎么挤，
/// **每帧未消费的脏区都必须只反映本帧**，不随帧数单调增长。
#[test]
fn dirty_stays_bounded_past_atlas_capacity() {
    let (mut soft, mut tree, ch) = setup('0');

    // 62 个字形：足以超过任何默认图集一帧的容量，反复触发驱逐/重打包
    let pool: Vec<char> = ('0'..='9').chain('A'..='Z').chain('a'..='z').collect();

    let mut worst = 0usize;
    for c in &pool {
        ch.set(*c);
        tree.invalidate_all();
        tree.render(&mut soft);
        let pending = soft.font_pending_dirty();
        worst = worst.max(pending);
        assert!(
            pending <= 2,
            "每帧最多光栅化 1 个新字形（+至多 1 条驱逐脏区），实际 {pending}；\
             若脏区从不消费，这里会随帧数增长"
        );
    }
    assert_eq!(soft.font_frame(), pool.len() as u64);
    assert_eq!(
        worst, 1,
        "每帧只新增 1 个字形 → 未消费脏区应恒为 1，实测峰值 {worst}"
    );
}
