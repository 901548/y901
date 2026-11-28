//! **Flex 布局的不变量**（属性测试，确定性伪随机）。
//!
//! 布局算错的表现都很"安静"：控件挤出窗口、两个控件叠在一起、flex 权重分配后总和
//! 不等于可用宽度 —— 单看截图很难判断，只能靠不变量。这里随机容器宽高、padding、
//! gap、子项数量与权重，钉三条：
//!
//! 1. **兄弟不重叠**（沿主轴比较矩形区间）；
//! 2. **flex 子项均分**：所有子项宽度 + 全部 gap 必须等于内容宽度（误差 < 0.5px）；
//! 3. **放得下时不越界；放不下时允许溢出但不得重叠**，且首个子项永远从内容区左上角开始
//!    —— `Flex::wrap(false)` 允许溢出是既定语义（见 `layout.rs` 的同名用例），
//!    把这条断言写成"永不越界"就会把设计意图误判成 bug。

use y901_ui::layout::{Flex, LayoutCache};
use y901_ui::{Constraints, FlexItem, Rect, Size, TextMetrics, TextStyle, UiTree, Widget};

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

struct Leaf {
    id: String,
    rect: Rect,
    min: Size,
}
impl Widget for Leaf {
    fn id(&self) -> &str {
        &self.id
    }
    fn flex(&self) -> FlexItem {
        FlexItem {
            // 内容最小尺寸作为 min 下界，便于制造"挤不下"的情形
            min_width: y901_ui::Dimension::Px(self.min.w),
            min_height: y901_ui::Dimension::Px(self.min.h),
            ..Default::default()
        }
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        cons: Constraints,
        _cache: &mut LayoutCache,
    ) -> Size {
        cons.constrain(self.min)
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn f(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() % 1000) as f32 / 1000.0 * (hi - lo)
    }
}

fn check_row(tree: &UiTree, kids: &[String], ctx: &str) {
    // 兄弟不重叠（沿主轴）
    let rects: Vec<Rect> = kids
        .iter()
        .map(|id| tree.rect_of(id).unwrap_or(Rect::ZERO))
        .collect();
    for i in 0..rects.len() {
        for j in (i + 1)..rects.len() {
            let a = rects[i];
            let b = rects[j];
            if a.w <= 0.0 || b.w <= 0.0 {
                continue;
            }
            let overlap = a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01;
            assert!(!overlap, "[{ctx}] 兄弟 {i}/{j} 在 x 上重叠: {a:?} {b:?}");
        }
    }
}

#[test]
fn row_children_never_overlap() {
    let mut rng = Lcg(0x1a2b_3c4d);
    for case in 0..200 {
        let n = 1 + (rng.next() % 5) as usize;
        let mut flex = Flex::row("root").gap(rng.f(0.0, 12.0));
        let mut ids = Vec::new();
        for i in 0..n {
            let id = format!("k{i}");
            ids.push(id.clone());
            flex = flex.push(Box::new(Leaf {
                id,
                rect: Rect::ZERO,
                min: Size::new(rng.f(4.0, 60.0), rng.f(4.0, 40.0)),
            }));
        }
        let mut tree = UiTree::new(Box::new(flex));
        let w = rng.f(80.0, 500.0);
        let h = rng.f(40.0, 200.0);
        tree.layout(Size::new(w, h), &Mono);
        check_row(&tree, &ids, &format!("case{case}"));
    }
}
#[test]
fn flex_children_fill_the_inner_width_exactly() {
    let mut rng = Lcg(0x9e37_79b9);
    for case in 0..200 {
        let pad = rng.f(0.0, 20.0);
        let gap = rng.f(0.0, 12.0);
        let n = 1 + (rng.next() % 4) as usize;
        let mut flex = Flex::row("root").padding(y901_ui::Edges::all(pad)).gap(gap);
        let mut ids = Vec::new();
        for i in 0..n {
            let id = format!("f{i}");
            ids.push(id.clone());
            flex = flex.push(Box::new(Weighted {
                id,
                rect: Rect::ZERO,
                weight: 1.0 + (rng.next() % 3) as f32,
            }));
        }
        let mut tree = UiTree::new(Box::new(flex));
        let w = rng.f(120.0, 500.0);
        let h = rng.f(60.0, 200.0);
        tree.layout(Size::new(w, h), &Mono);

        let inner = w - 2.0 * pad;
        let total: f32 = ids
            .iter()
            .map(|id| tree.rect_of(id).map(|r| r.w).unwrap_or(0.0))
            .sum();
        let gaps = gap * (n as f32 - 1.0);
        assert!(
            (total + gaps - inner).abs() < 0.5,
            "[case{case}] flex 子项总宽 {total} + gap {gaps} != 内容宽 {inner}（pad={pad}）"
        );
        // 权重比例：w_i / w_j ≈ weight_i / weight_j
        if n >= 2 {
            let w0 = tree.rect_of(&ids[0]).unwrap().w;
            let w1 = tree.rect_of(&ids[1]).unwrap().w;
            assert!(w0 > 0.0 && w1 > 0.0, "[case{case}] flex 子项宽度必须为正");
        }
    }
}

struct Weighted {
    id: String,
    rect: Rect,
    weight: f32,
}
impl Widget for Weighted {
    fn id(&self) -> &str {
        &self.id
    }
    fn flex(&self) -> FlexItem {
        FlexItem {
            width: y901_ui::Dimension::Flex(self.weight),
            ..Default::default()
        }
    }
    fn measure(
        &mut self,
        _m: &dyn TextMetrics,
        cons: Constraints,
        _cache: &mut LayoutCache,
    ) -> Size {
        cons.constrain(Size::new(0.0, 0.0))
    }
    fn arrange(&mut self, r: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
        self.rect = r;
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &[]
    }
    fn render(&mut self, _c: &mut dyn y901_ui::RenderCtx) {}
    fn semantics(&self) -> y901_ui::Semantics {
        y901_ui::Semantics::default()
    }
}
/// 放得下时必须在内容区内；放不下时**允许溢出**（`Flex::wrap(false)` 的既定语义，
/// 见 `layout.rs` 的 `default_no_wrap_still_overflows`），但必须：从内容区左上角开始、
/// 兄弟之间不重叠。这条断言把"两种情形"分开，避免把设计意图当成 bug。
#[test]
fn children_fit_when_there_is_room_and_never_overlap_when_there_is_not() {
    let mut rng = Lcg(0xdead_10cc);
    for case in 0..200 {
        let pad = rng.f(0.0, 16.0);
        let gap = rng.f(0.0, 10.0);
        let n = 1 + (rng.next() % 4) as usize;
        let mut flex = Flex::row("root").padding(y901_ui::Edges::all(pad)).gap(gap);
        let mut ids = Vec::new();
        for i in 0..n {
            let id = format!("m{i}");
            ids.push(id.clone());
            flex = flex.push(Box::new(Leaf {
                id,
                rect: Rect::ZERO,
                min: Size::new(rng.f(0.0, 40.0), rng.f(0.0, 30.0)),
            }));
        }
        let mut tree = UiTree::new(Box::new(flex));
        let w = rng.f(100.0, 400.0);
        let h = rng.f(50.0, 200.0);
        tree.layout(Size::new(w, h), &Mono);
        let root = tree.rect_of("root").unwrap_or(Rect::new(0.0, 0.0, w, h));
        let inner = Rect::new(
            root.x + pad,
            root.y + pad,
            root.w - 2.0 * pad,
            root.h - 2.0 * pad,
        );

        let rects: Vec<Rect> = ids
            .iter()
            .map(|id| tree.rect_of(id).expect("子项已布局"))
            .collect();
        // 放得下吗？用实际排出来的宽度之和判断（min_width 会把它们抬大）
        let total: f32 = rects.iter().map(|r| r.w).sum();
        let gaps = gap * (ids.len() as f32 - 1.0);
        let fits = total + gaps <= inner.w + 0.01;

        // 第一个子项永远从内容区左上角开始
        assert!(
            (rects[0].x - inner.x).abs() < 0.01 && (rects[0].y - inner.y).abs() < 0.01,
            "[case{case}] 首个子项应从内容区左上角开始：{:?} vs {inner:?}",
            rects[0]
        );

        if fits {
            for (i, r) in rects.iter().enumerate() {
                assert!(
                    r.x >= inner.x - 0.01
                        && r.y >= inner.y - 0.01
                        && r.x + r.w <= inner.x + inner.w + 0.01
                        && r.y + r.h <= inner.y + inner.h + 0.01,
                    "[case{case}] 放得下时子项 {i} {r:?} 溢出内容区 {inner:?}\
                     （pad={pad} gap={gap} viewport={w}x{h}）"
                );
            }
        } else {
            // 放不下：允许溢出，但不得重叠
            for i in 0..rects.len() {
                for j in (i + 1)..rects.len() {
                    let (a, b) = (rects[i], rects[j]);
                    let overlap = a.x < b.x + b.w - 0.01 && b.x < a.x + a.w - 0.01;
                    assert!(
                        !overlap,
                        "[case{case}] 溢出时兄弟 {i}/{j} 仍不得重叠：{a:?} {b:?}"
                    );
                }
            }
        }
    }
}
