//! 手写 Flex 布局 + 布局缓存。
//!
//! 三个核心保证：
//!
//! 1. **每个节点每次布局最多 measure 一次**。容器必须通过 [`LayoutCache::measure`]
//!    访问子节点，框架在那里去重。
//!
//! 2. **相同约束下重复布局为 O(1)**。`measure` 结果按
//!    `(id, constraints, layout_version)` 记忆；约束与版本都没变时整棵子树不再进入。
//!
//! 3. **无界约束不会产生 inf / NaN**。[`crate::Constraints::unbounded_main`]
//!    只放宽上限，返回的内容尺寸始终有限；滚动容器的子节点排在**内容坐标系**里，
//!    平移由渲染/命中阶段处理。
//!
//! **已做**：wrap（`Flex::wrap(true)`）、align-items（`Flex::align`）、justify-content、per-child min/max。
//!
//! **仍不做**：absolute（定位逃逸）、aspect-ratio。理由：它们与“所有子控件由父容器分配 rect”
//! 的模型直接冲突；当前层叠需求已由 `Stack` + 各控件自算 rect 满足。

use crate::fasthash::{fast_map, fast_set, FastMap, FastSet};

use crate::color::Color;
use crate::event::{EventResult, UiEvent};
use crate::geometry::{Edges, Rect, Size};
use crate::widget::{
    Align, Axis, Constraints, Dimension, EventCtx, FlexItem, Justify, RenderCtx, Role, Semantics,
    TextMetrics, Widget,
};

// ============================ 布局缓存 ============================

/// 一次布局的统计。测试与线上诊断都用它。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    /// 真正进入 `Widget::measure` 的次数（缓存命中不算）。
    pub measured: u32,
    /// 被 arrange 的节点数。
    pub arranged: u32,
    /// 整次布局是否被跳过（约束与版本都没变）。
    pub skipped: bool,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    constraints: Constraints,
    version: u64,
    size: Size,
    /// 父节点的键（脏传播沿它向上走）。没有父 = 根。
    ///
    /// 放在条目里而不是单独的 `parents` 表：每个被测节点因此少一次哈希插入
    /// （11k 节点 × 每帧一次，实测是笔不小的开销），内存也少一张表。
    parent: Option<u64>,
    /// 在哪个"脏世代"被标脏；`<= finish_serial` 表示已在本轮消耗掉。
    ///
    /// 用世代号替代 `dirty: HashSet<u64>`：清理脏标记从"每帧 clear 一个集合"
    /// 变成 `finish_serial = serial` 一次赋值（O(1)），`is_fresh` 也从
    /// "查集合 + 查条目"两次哈希降到**一次查条目**。
    dirty_serial: u64,
}

/// 节点 id → 64 位缓存键（FNV-1a）。
///
/// 为什么不用 `String` 做键：每次 measure 都要 `id.to_string()`（堆分配）+
/// SipHash（比 FNV 慢数倍），10k 节点的全量重排会因此多花毫秒级。
/// FNV-1a 对短 ASCII id 足够分散，且零分配。
/// 把节点 id 哈希成 `u64`（FNV-1a）。公开供 `UiTree` 的索引表复用。
pub fn id_key(id: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// **节点的规范键**（唯一入口）：`cache_key()` 有值就用它，否则退回 `id_key(id)`。
///
/// 为什么必须只有这一个入口：框架里"按 id 查"（`rect_of`/`invalidate`/`mark_dirty`）
/// 与"按节点查"（`measure`/`rebuild_indices`/墨迹边距）如果各自推导键，一旦某个控件
/// 返回了**不是 `id_key(id)`** 的 `cache_key()`，两边就会指向不同的桶 ——
/// `invalidate(id)` 静默失效、跳过布局的判断永远不命中、墨迹边距丢失。
///
/// 因此 `Widget::cache_key()` 的契约被收紧为：**只能是 `id_key(self.id())` 的预算缓存**。
/// 违反时在调试构建里直接断言失败（而不是让线上出现"某些节点不刷新"这种幽灵问题）。
pub(crate) fn key_of(node: &dyn crate::widget::Widget) -> u64 {
    let id_key = id_key(node.id());
    match node.cache_key() {
        Some(k) => {
            debug_assert_eq!(
                k,
                id_key,
                "Widget::cache_key() 必须等于 id_key(id())（id = {:?}）：它只能当作 id 键的\
                 预算缓存，不能当作自定义键。自定义键会让按 id 的查询（rect_of/invalidate）\
                 与按节点的查询（measure/rebuild_indices）指向不同的桶。",
                node.id()
            );
            k
        }
        None => id_key,
    }
}

/// 布局缓存：记忆化 + 脏传播。
pub struct LayoutCache {
    entries: FastMap<u64, Entry>,
    /// **还没有条目**就被标脏的节点（首帧、新挂载）。
    ///
    /// 它们本来就不可能 fresh（无条目 ⇒ 未测量），所以不参与 `is_fresh`；
    /// 只在 `is_dirty()` 查询与 `finish_layout` 里用到。
    unseen_dirty: FastSet<u64>,
    /// 脏世代号（只增不减）。
    serial: u64,
    /// 已消耗到的世代号：`entry.dirty_serial <= finish_serial` ⇒ 干净。
    finish_serial: u64,
    /// 当前正在测量的祖先链（用于填写 `parents`）。
    stack: Vec<u64>,
    stats: LayoutStats,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutCache {
    /// 建立空缓存。
    pub fn new() -> Self {
        Self {
            entries: fast_map(),
            unseen_dirty: fast_set(),
            serial: 1,
            finish_serial: 0,
            stack: Vec::new(),
            stats: LayoutStats::default(),
        }
    }

    /// 统计信息（命中/未命中/节点数），用于评估缓存是否有效。
    pub fn stats(&self) -> LayoutStats {
        self.stats
    }

    /// 标记某节点的尺寸可能变化：它自己和所有祖先都要重测。
    pub fn mark_dirty(&mut self, id: &str) {
        self.mark_dirty_key(id_key(id));
    }

    /// 同 [`Self::mark_dirty`]，但直接收 64 位键（内部走这条，省一次散列）。
    pub fn mark_dirty_key(&mut self, key: u64) {
        let serial = self.serial;
        let mut cursor = Some(key);
        let mut guard = 0usize;
        while let Some(current) = cursor {
            match self.entries.get_mut(&current) {
                Some(entry) => {
                    if entry.dirty_serial > self.finish_serial {
                        // 已经是脏的：祖先在上次标脏时必然也标过（脏传播是整条链）
                        break;
                    }
                    entry.dirty_serial = serial;
                    cursor = entry.parent;
                }
                None => {
                    // 没有条目 ⇒ 本来就无法 fresh；只为 `is_dirty()` 记一笔
                    self.unseen_dirty.insert(current);
                    break;
                }
            }
            guard += 1;
            if guard > 4096 {
                break; // 防御错误数据形成环
            }
        }
    }

    /// 整棵树失效（主题变更、字体变化等）。
    pub fn mark_all_dirty(&mut self) {
        self.entries.clear();
        self.unseen_dirty.clear();
    }

    /// 清空缓存；主题或字体变化后必须调用，否则旧度量会被复用。
    pub fn clear(&mut self) {
        self.entries.clear();
        self.unseen_dirty.clear();
    }

    /// **忘掉一个节点**：子树被卸载时调用（`UiTree::remove_node`）。
    ///
    /// 为什么必须清：缓存键是 `(id_key, constraints, layout_version)`，而"新实例"的
    /// `layout_version()` 又从 0 开始 —— 只要视口与约束没变，`is_fresh_by_key` 就会命中
    /// 旧实例的条目并**直接返回旧尺寸、跳过 `measure`**。
    /// 表现：同一 id 重建的控件保留旧尺寸/旧内部状态（内容变长被截断、
    /// `ScrollView` 没跑过 measure 导致滚不动）。
    ///
    /// `parents` 也要一起删：否则脏传播会沿"已经不存在的父链"走，
    /// id 复用后甚至可能形成环，让 `mark_dirty` 提前停下、根部不被标脏。
    pub fn forget(&mut self, key: u64) {
        self.entries.remove(&key);
        self.unseen_dirty.remove(&key);
    }

    /// 某节点上次测得的尺寸（不管约束）。arrange 阶段复用。
    pub fn last_size(&self, id: &str) -> Option<Size> {
        self.entries.get(&id_key(id)).map(|e| e.size)
    }

    /// 某节点在本次布局里是否需要重新测量。
    pub fn is_dirty(&self, id: &str) -> bool {
        let key = id_key(id);
        match self.entries.get(&key) {
            Some(entry) => entry.dirty_serial > self.finish_serial,
            None => self.unseen_dirty.contains(&key),
        }
    }

    fn is_fresh(&self, id: &str, constraints: Constraints, version: u64) -> bool {
        self.is_fresh_by_key(id_key(id), constraints, version)
    }

    /// 同 [LayoutCache::is_fresh]，但直接收 u64 键（measure 路径少做一次哈希）。
    ///
    /// **只查一次表**（旧实现先查 `dirty` 集合再查 `entries`，每个节点两次哈希）。
    fn is_fresh_by_key(&self, key: u64, constraints: Constraints, version: u64) -> bool {
        match self.entries.get(&key) {
            Some(entry) => {
                entry.dirty_serial <= self.finish_serial
                    && entry.version == version
                    && entry.constraints == constraints
            }
            None => false,
        }
    }

    /// **测量入口**：所有容器都必须用它访问子节点。
    ///
    /// 命中缓存时直接返回，不进入 `Widget::measure`，整棵子树被切断。
    pub fn measure(
        &mut self,
        widget: &mut dyn Widget,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
    ) -> Size {
        // 优先用 widget 自报的键（容器在构造时算好），避免字符串哈希
        let key = key_of(widget);
        let version = widget.layout_version();

        // **一次查表**完成"是否可跳过"判断
        if self.is_fresh_by_key(key, constraints, version) {
            return self.entries.get(&key).map(|e| e.size).unwrap_or(Size::ZERO);
        }

        // 父键取自测量栈（根没有父）；写进条目，省掉单独的 `parents` 表。
        //
        // **深度 0 的测量要保留已记录的父**：有些控件在 `arrange` 里直接调子控件的
        // `Widget::measure`（不经过本缓存，于是测量栈是空的），那条路径会把
        // "父 = None" 写进条目、把原本正确的父链抹掉 —— 之后 `mark_dirty` 沿父链
        // 上溯时在第一层就断了，根永远不失效、布局被错误跳过。
        // 旧实现用独立的 `parents` 表时天然没这个坑（表只在栈非空时才写），
        // 这里显式保留同样的语义。
        let parent = match self.stack.last().copied() {
            Some(p) => Some(p),
            None => self.entries.get(&key).and_then(|e| e.parent),
        };

        self.stats.measured += 1;
        self.stack.push(key);
        let size = widget.measure(metrics, constraints, self);
        self.stack.pop();

        self.entries.insert(
            key,
            Entry {
                constraints,
                version,
                size,
                parent,
                // 0 = 本世代已消耗（刚测完就是干净的）
                dirty_serial: 0,
            },
        );
        size
    }

    /// arrange 阶段取尺寸：优先复用上次测量结果，取不到才补测。
    pub fn size_for_arrange(
        &mut self,
        widget: &mut dyn Widget,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
    ) -> Size {
        if let Some(size) = self.last_size(widget.id()) {
            return size;
        }
        self.measure(widget, metrics, constraints)
    }

    fn begin_layout(&mut self) {
        self.stats = LayoutStats::default();
    }

    fn finish_layout(&mut self) {
        // 把这一轮的所有脏标记**一次性消费掉**：`dirty_serial <= finish_serial` ⇒ 干净。
        //
        // 旧实现在这里 `dirty.clear()`：虽然也是 O(1)，但每个被测节点都要
        // `dirty.remove(&key)` 一次哈希（11k 节点 = 11k 次）。换成世代号后
        // 这两次哈希（remove + is_fresh 的 contains）都消失了。
        self.finish_serial = self.serial;
        self.unseen_dirty.clear();
        self.serial = self.serial.wrapping_add(1);
        if self.serial == 0 {
            // 极端情况（u64 回绕）：清表重来，避免"世代号撞上历史值"导致漏重测
            self.entries.clear();
            self.serial = 1;
            self.finish_serial = 0;
        }
    }
}

// ============================ Flex 算法 ============================

/// 容器测量：返回内容 + padding 后的期望尺寸。
pub fn flex_measure(
    axis: Axis,
    padding: Edges,
    gap: f32,
    children: &mut [Box<dyn Widget>],
    metrics: &dyn TextMetrics,
    constraints: Constraints,
    cache: &mut LayoutCache,
) -> Size {
    let mut measured = Vec::new();
    flex_measure_into(
        axis,
        padding,
        gap,
        children,
        metrics,
        constraints,
        cache,
        &mut measured,
    )
}

/// 把某轴的尺寸 clamp 到 `[min, max]`；若 `avail` 有限则上限也不超过 `avail`
/// （**`min > avail` 时退化为 `avail`**，不溢出父容器）。
fn clamp_axis(v: f32, min: Dimension, max: Dimension, avail: f32) -> f32 {
    let floor = min.resolve(avail, 0.0).max(0.0);
    let ceil = match max {
        Dimension::Auto => f32::INFINITY,
        d => d.resolve(avail, v),
    };
    let mut out = v.max(floor).min(ceil);
    if !out.is_finite() {
        out = v;
    }
    if avail.is_finite() {
        out = out.min(avail.max(0.0));
    }
    out
}

/// 把 `(main, cross)` 按主/交叉轴各自的 min/max clamp（两处共用）。
fn clamp_pair(
    axis: Axis,
    item: FlexItem,
    main_avail: f32,
    cross_avail: f32,
    main: f32,
    cross: f32,
) -> (f32, f32) {
    let (mw, mh, cw, ch) = match axis {
        Axis::Horizontal => (
            item.min_width,
            item.max_width,
            item.min_height,
            item.max_height,
        ),
        Axis::Vertical => (
            item.min_height,
            item.max_height,
            item.min_width,
            item.max_width,
        ),
    };
    (
        clamp_axis(main, mw, mh, main_avail),
        clamp_axis(cross, cw, ch, cross_avail),
    )
}

/// 同 [`flex_measure`]，额外把每个子节点的 `(实测尺寸, flex 参数)` 写进 `measured`。
///
/// 让 `arrange` 能直接复用测量结果（省掉每子节点一次 id 哈希）。
///
/// 详细约束与无界处理见函数体内注释。
#[allow(clippy::too_many_arguments)] // 与 d3d11::draw_scene 一致：参数就是布局所需的最小集合
pub fn flex_measure_into(
    axis: Axis,
    padding: Edges,
    gap: f32,
    children: &mut [Box<dyn Widget>],
    metrics: &dyn TextMetrics,
    constraints: Constraints,
    cache: &mut LayoutCache,
    measured: &mut Vec<(Size, FlexItem)>,
) -> Size {
    let scale = metrics.scale();
    measured.clear();
    // 主轴无界时，子节点在主轴方向不设上限（ScrollView 场景）
    let inner_max = Size::new(
        if constraints.is_unbounded(Axis::Horizontal) {
            f32::INFINITY
        } else {
            (constraints.max.w - padding.horizontal()).max(0.0)
        },
        if constraints.is_unbounded(Axis::Vertical) {
            f32::INFINITY
        } else {
            (constraints.max.h - padding.vertical()).max(0.0)
        },
    );

    let mut main_total = 0.0f32;
    let mut cross_max = 0.0f32;
    let mut visible = 0usize;

    for child in children.iter_mut() {
        let item = child.flex();
        let size = cache.measure(child.as_mut(), metrics, Constraints::loose(inner_max));
        let _ = &size;

        let (main_dim, cross_dim) = match axis {
            Axis::Horizontal => (item.width, item.height),
            Axis::Vertical => (item.height, item.width),
        };
        let (main_content, cross_content) = match axis {
            Axis::Horizontal => (size.w, size.h),
            Axis::Vertical => (size.h, size.w),
        };

        let main = match main_dim {
            Dimension::Px(v) => v * scale,
            Dimension::Percent(p) => {
                let base = match axis {
                    Axis::Horizontal => inner_max.w,
                    Axis::Vertical => inner_max.h,
                };
                if base.is_finite() {
                    base * (p / 100.0)
                } else {
                    main_content // 无界主轴不能算百分比
                }
            }
            Dimension::Flex(_) | Dimension::Auto => main_content,
        };
        let cross = match cross_dim {
            Dimension::Px(v) => v * scale,
            Dimension::Percent(p) => {
                let base = match axis {
                    Axis::Horizontal => inner_max.h,
                    Axis::Vertical => inner_max.w,
                };
                if base.is_finite() {
                    base * (p / 100.0)
                } else {
                    cross_content
                }
            }
            _ => cross_content,
        };

        // **min/max clamp**（在尺寸解析之后、累加之前）。
        // 默认 min=Px(0)/max=Auto 时不改变任何值 —— 与加该能力前逐像素一致。
        let (main, cross) = clamp_pair(
            axis,
            item,
            match axis {
                Axis::Horizontal => inner_max.w,
                Axis::Vertical => inner_max.h,
            },
            match axis {
                Axis::Horizontal => inner_max.h,
                Axis::Vertical => inner_max.w,
            },
            main,
            cross,
        );
        main_total += main;
        cross_max = cross_max.max(cross);
        visible += 1;
        // **存"解析后的尺寸"而不是控件自报的尺寸**：这是 measure 与 arrange
        // 对 `Px`/`Percent` 达成一致的关键。旧实现存自报尺寸，arrange 再按容器的
        // 实际 rect 重算一次百分比 —— 只要容器尺寸是内容驱动的（自动高度 + `Align::Center`），
        // 两处就会得出不同结果：量出 100 高的列，子项摆进去只有 50 高（列里留空洞）。
        measured.push((
            match axis {
                Axis::Horizontal => Size::new(main, cross),
                Axis::Vertical => Size::new(cross, main),
            },
            item,
        ));
    }

    if visible > 1 {
        main_total += gap * (visible - 1) as f32;
    }

    let (w, h) = match axis {
        Axis::Horizontal => (
            main_total + padding.horizontal(),
            cross_max + padding.vertical(),
        ),
        Axis::Vertical => (
            cross_max + padding.horizontal(),
            main_total + padding.vertical(),
        ),
    };

    // 关键：返回的永远是有限的内容尺寸
    constraints.constrain(Size::new(w, h))
}

/// 容器排布：把 `rect` 内的空间分给子组件并逐个 `arrange`。
///
/// 尺寸全部来自缓存，因此 arrange 阶段**不会**触发 `Widget::measure`。
pub fn flex_arrange(
    axis: Axis,
    padding: Edges,
    gap: f32,
    children: &mut [Box<dyn Widget>],
    rect: Rect,
    metrics: &dyn TextMetrics,
    cache: &mut LayoutCache,
) {
    flex_arrange_with(
        axis,
        padding,
        gap,
        Align::Stretch,
        Justify::Start,
        children,
        rect,
        metrics,
        cache,
        &[],
    );
}

/// 同 [`flex_arrange`]，但**优先复用** `measured`（来自 [`flex_measure_into`]）。
///
/// 命中复用条件：长度一致。此时不再查 `LayoutCache`、不再调 `child.flex()`。
#[allow(clippy::too_many_arguments)] // 同 flex_arrange + 一个 measured 复用参数
pub fn flex_arrange_with(
    axis: Axis,
    padding: Edges,
    gap: f32,
    align: Align,
    justify: Justify,
    children: &mut [Box<dyn Widget>],
    rect: Rect,
    metrics: &dyn TextMetrics,
    cache: &mut LayoutCache,
    measured: &[(Size, FlexItem)],
) {
    // 注意：这里**不再**需要 `metrics.scale()` —— `Px` 的缩放已经在 measure 阶段
    // 应用过了，arrange 一律消费解析后的尺寸（见 `flex_measure_into` 的 measured 说明）。
    let _ = metrics;
    if children.is_empty() {
        return;
    }
    let inner = rect.inset(padding);
    let (main_avail, cross_avail) = match axis {
        Axis::Horizontal => (inner.w, inner.h),
        Axis::Vertical => (inner.h, inner.w),
    };

    // 第一遍：固定尺寸 + flex 因子（全部走缓存）
    //
    // **正常路径不再分配 `sizes`**：`measured.len() == children.len()` 时
    // "第 i 个子项解析后的尺寸"就是 `measured[i].0`，没必要再拷一份到新 Vec。
    // 旧实现每个容器都 `Vec::with_capacity(n)` —— 11k 节点的树有 1113 个容器，
    // 一次重排就是 1113 次分配（`perf_alloc` 的 `cold_allocs` 正好也是 1113）。
    // 只有"测量快照与子项数量不一致"（增删子节点的那一帧）才临时建一份。
    let mut fixed_total = 0.0f32;
    let mut flex_total = 0.0f32;
    let reuse = measured.len() == children.len();
    let mut fallback: Vec<Size> = Vec::new();
    if !reuse {
        fallback.reserve(children.len());
    }

    for (idx, child) in children.iter_mut().enumerate() {
        // 复用 measure 阶段的成果：省掉一次 id 哈希 + 一次 flex() 虚调用
        let (size, item) = if reuse {
            measured[idx]
        } else {
            let s =
                cache.size_for_arrange(child.as_mut(), metrics, Constraints::loose(inner.size()));
            fallback.push(s);
            (s, child.flex())
        };
        let main_dim = match axis {
            Axis::Horizontal => item.width,
            Axis::Vertical => item.height,
        };

        if let Some(factor) = main_dim.flex_factor() {
            flex_total += factor;
        } else {
            // 一律用 measure 解析好的尺寸：`Px`/`Percent` 不再在这里二次解析。
            fixed_total += match axis {
                Axis::Horizontal => size.w,
                Axis::Vertical => size.h,
            };
        }
    }
    let size_of = |i: usize| if reuse { measured[i].0 } else { fallback[i] };

    let gaps = if children.len() > 1 {
        gap * (children.len() - 1) as f32
    } else {
        0.0
    };
    let leftover = (main_avail - fixed_total - gaps).max(0.0);

    // **主轴分布**（justify-content）：用"已分配的主轴总长" 算剩余，
    // 再转成"起始偏移 + 额外间距"。
    // **`Justify::Start` 时 offset 与 gap_extra 均为 0** —— 与加该能力前逐像素一致。
    let n = children.len() as f32;
    let main_used = fixed_total + if flex_total > 0.0 { leftover } else { 0.0 } + gaps;
    let free = (main_avail - main_used).max(0.0);
    let free = if free.is_finite() { free } else { 0.0 };
    let (mut cursor, gap_extra) = match justify {
        Justify::Start => (0.0, 0.0),
        Justify::Center => (free * 0.5, 0.0),
        Justify::End => (free, 0.0),
        // `n == 1` 时退化为 Start（避免 0 除法）
        Justify::SpaceBetween => (0.0, if n > 1.0 { free / (n - 1.0) } else { 0.0 }),
        Justify::SpaceAround => (free / n * 0.5, free / n),
    };
    for (idx, child) in children.iter_mut().enumerate() {
        // **复用 measure 阶段的 flex 参数**（有快照时）：下面算 `main` 用的 `item`
        // 必须与上面算 `fixed_total`/`flex_total` 的那一份**同源** ——
        // 否则 `justify` 的偏移来自一次取值、几何来自另一次取值，
        // 对"`flex()` 是计算出来的"控件就会自相矛盾。
        let item = if reuse { measured[idx].1 } else { child.flex() };
        let size = size_of(idx);
        let (main_dim, cross_dim) = match axis {
            Axis::Horizontal => (item.width, item.height),
            Axis::Vertical => (item.height, item.width),
        };

        // **主轴尺寸直接取 measure 解析好的值**（`measured[idx].0`）。
        //
        // 为什么不再按 `inner`/`scale` 重算：measure 与 arrange 各算一次就会分歧 ——
        // 百分比在 measure 里按"父级给的可用空间"解析，在 arrange 里按容器实际 rect
        // 解析；容器尺寸一旦是内容驱动的（自动高度 + `Align::Center`），
        // 就会"量出 100、摆下 50"（列里留空洞）。现在**测量结果就是唯一事实**。
        let main = match main_dim.flex_factor() {
            Some(factor) if flex_total > 0.0 => leftover * factor / flex_total,
            _ => match axis {
                Axis::Horizontal => size.w,
                Axis::Vertical => size.h,
            },
        };

        // **交叉轴尺寸**：
        // * `Stretch`（默认）—— `Auto` 子项拿满交叉轴（**与加该能力前一致**）；
        // * 其余对齐 —— `Auto` 子项用自己的内容尺寸，否则 Center / End 无意义。
        let content_cross = match axis {
            Axis::Horizontal => size.h,
            Axis::Vertical => size.w,
        };
        let cross = match cross_dim {
            // 显式交叉轴尺寸同样取 measure 解析好的值（含 min/max 钳制）
            Dimension::Px(_) | Dimension::Percent(_) => content_cross,
            _ => match align {
                Align::Stretch => cross_avail,
                _ => content_cross,
            },
        };

        let cross = if cross.is_finite() { cross } else { 0.0 };
        // **min/max clamp**（默认不改变任何值）。
        let (main, cross) = clamp_pair(axis, item, main_avail, cross_avail, main, cross);
        let cross = cross.min(cross_avail);
        // 交叉轴偏移（`Stretch` / `Start` 为 0）。
        let cross_off = match align {
            Align::Start | Align::Stretch => 0.0,
            Align::Center => ((cross_avail - cross) * 0.5).max(0.0),
            Align::End => (cross_avail - cross).max(0.0),
        };
        let child_rect = match axis {
            Axis::Horizontal => Rect::new(inner.x + cursor, inner.y + cross_off, main, cross),
            Axis::Vertical => Rect::new(inner.x + cross_off, inner.y + cursor, cross, main),
        };

        cache.stats.arranged += 1;
        child.arrange(child_rect, metrics, cache);
        // `gap_extra` 仅在 SpaceBetween / SpaceAround 下非零。
        cursor += main + gap + gap_extra;
    }
}

// ============================ 内容坐标系 ============================

/// 把内容矩形放到视口原点，再按 `offset` 平移。
///
/// 滚动容器用这个把"内容坐标"映射到"屏幕坐标"：子节点永远在
/// `[0, content_height]` 里排布，屏幕上再减去偏移，因此**不会出现
/// 1e6 像素的浮点坐标**（那是 NaN 的温床）。
pub fn content_rect(viewport: Rect, content: Size, offset: f32) -> Rect {
    let height = if content.h.is_finite() {
        content.h
    } else {
        viewport.h
    };
    let offset = if offset.is_finite() { offset } else { 0.0 };
    Rect::new(viewport.x, viewport.y - offset, viewport.w, height)
}

// ============================ Flex 容器 ============================

/// 一维弹性容器：按主轴排列子控件，支持 padding、gap 与弹性分配。
pub struct Flex {
    id: String,
    axis: Axis,
    /// **内边距基准值（DIP）**：实际值 = `padding × metrics.scale()`，见 `docs/design/dpi-policy.md`。
    padding: Edges,
    /// **旧的显式缩放值**（`.padding_scaled()` 设置的）。保留是为了兼容既有调用点：
    /// 现在 `padding()` 本身就缩放，所以这个字段只在被显式设置时覆盖 `padding`。
    scaled_padding: Option<Edges>,
    /// **逃生舱**：`.padding_fixed()` 设的物理像素值，**不随 DPI 缩放**。
    /// 典型用途：1px 描边内缩、命中阈值这类"必须恰好的像素"。
    fixed_padding: Option<Edges>,
    /// **间距基准值（DIP）**：实际值 = `gap × metrics.scale()`。
    gap: f32,
    /// 旧的显式缩放间距（`.gap_scaled()`）；语义同 `scaled_padding`。
    scaled_gap: Option<f32>,
    /// **逃生舱**：不缩放的物理像素间距。见 `fixed_padding`。
    fixed_gap: Option<f32>,
    /// **是否开启 wrap（换行）**。默认 `false` = 加该能力前的行为（单行、可溢出）。
    ///
    /// `true` 时走**独立分支**（`wrap_measure` / `wrap_arrange`），原路径逐字不动。
    wrap: bool,
    /// wrap 的**分行结果**（每行 = 子项下标列表）。
    ///
    /// **只在 `wrap == true` 时使用**；默认 `false` 时始终为空
    /// （空 `Vec` 不分配堆内存 —— 零开销）。
    /// `measure` 写入；`arrange` 在宽度不变时与它一致（宽度变了则重算，见注释）。
    wrap_lines: Vec<Vec<usize>>,
    /// wrap 的**每行高度**（= 该行子项交叉轴尺寸的最大值）。
    /// 与 `wrap_lines` 同生同死，同样默认为空。
    wrap_line_heights: Vec<f32>,
    /// 交叉轴对齐（`align-items`）。默认 `Stretch` = 加该能力前的行为。
    align_items: Align,
    /// 主轴分布（`justify-content`）。默认 `Start` = 加该能力前的行为。
    justify_content: Justify,
    background: Option<Color>,
    /// **背景圆角半径**。`0.0`（默认）= 直角，
    /// 与加该字段前的行为**逐像素相同**。
    radius: f32,
    /// **描边**：`(color, width)`。`None`（默认）= 不描边。
    border: Option<(Color, f32)>,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    item: FlexItem,
    /// 自身 id 的缓存键（构造时算一次，`measure` 不再碰字符串）。
    key: u64,
    /// `measure` 阶段算出的子节点尺寸与 flex 参数，`arrange` 直接复用。
    ///
    /// 为什么缓存：arrange 若回查 `LayoutCache`，每个子节点都要重算一次 id 哈希；
    /// 若重新调 `child.flex()`，又是一次虚调用。两者在 10k 节点上是主要开销。
    measured: Vec<(Size, FlexItem)>,
}

impl Flex {
    /// 按主轴方向创建。
    pub fn new(id: impl Into<String>, axis: Axis) -> Self {
        let id = id.into();
        let key = id_key(&id);
        Self {
            id,
            key,
            axis,
            padding: Edges::ZERO,
            scaled_padding: None,
            fixed_padding: None,
            gap: 0.0,
            scaled_gap: None,
            fixed_gap: None,
            wrap: false,
            wrap_lines: Vec::new(),
            wrap_line_heights: Vec::new(),
            align_items: Align::Stretch,
            justify_content: Justify::Start,
            background: None,
            radius: 0.0,
            border: None,
            children: Vec::new(),
            rect: Rect::ZERO,
            item: FlexItem::default(),
            measured: Vec::new(),
        }
    }

    /// 水平排列（从左到右）。
    pub fn row(id: impl Into<String>) -> Self {
        Self::new(id, Axis::Horizontal)
    }

    /// 垂直排列（从上到下）。
    pub fn column(id: impl Into<String>) -> Self {
        Self::new(id, Axis::Vertical)
    }

    /// 设置内边距；子控件可用空间相应减少。
    ///
    /// **量的单位是 DIP**：实际值 = `padding × metrics.scale()`
    /// （DPI 策略见 `docs/design/dpi-policy.md`）。这正是 Windows 应用的行为——
    /// 125% 缩放下留白和字号一起放大。若确实需要"不缩放的物理像素"
    /// （1px 描边内缩、命中阈值），用 [`Flex::padding_fixed`]。
    ///
    /// 与 [`Flex::padding_scaled`] **互斥**：后设置者生效。
    pub fn padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self.scaled_padding = None;
        self.fixed_padding = None;
        self
    }

    /// 设置**不随 DPI 缩放的**内边距（物理像素）。
    ///
    /// 用途：那些"必须恰好是 n 个物理像素"的场合——1px 描边内缩、命中区域阈值。
    /// 普通留白**不要**用它，否则 125% 下会比周围控件明显偏窄。
    pub fn padding_fixed(mut self, padding: Edges) -> Self {
        self.fixed_padding = Some(padding);
        // **三个 setter 之间必须两两互斥**：`padding_for` 里 fixed 优先，
        // 若这里不清 `scaled_padding`，`.padding_scaled(8).padding_fixed(6)`
        // 之外的顺序（`.padding_fixed(6).padding_scaled(8)`）会让"后设置者生效"失效。
        self.scaled_padding = None;
        self.padding = Edges::ZERO;
        self
    }

    /// 设置子控件之间的间距。
    ///
    /// **量的单位是 DIP**：实际值 = `gap × metrics.scale()`；
    /// 需要不缩放的物理像素时用 [`Flex::gap_fixed`]。
    ///
    /// 与 [`Flex::gap_scaled`] **互斥**：后设置者生效。
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self.scaled_gap = None;
        self.fixed_gap = None;
        self
    }

    /// 设置**不随 DPI 缩放的**间距（物理像素）。见 [`Flex::padding_fixed`]。
    pub fn gap_fixed(mut self, gap: f32) -> Self {
        self.fixed_gap = Some(gap);
        self.scaled_gap = None;
        self.gap = 0.0;
        self
    }

    /// 设置**按 DPI 缩放的间距**：实际值 = `base × metrics.scale()`。
    ///
    /// **已冗余**：`gap()` 现在默认就是按 DPI 缩放。保留是为了不让既有调用点失效。
    pub fn gap_scaled(mut self, base: f32) -> Self {
        self.scaled_gap = Some(base);
        self.fixed_gap = None;
        self
    }

    /// 设置**按 DPI 缩放的内边距**：每一边实际值 = `base.边 × metrics.scale()`。
    ///
    /// **已冗余**：`padding()` 现在默认就是按 DPI 缩放。保留仅为兼容。
    pub fn padding_scaled(mut self, base: Edges) -> Self {
        self.scaled_padding = Some(base);
        self.fixed_padding = None;
        self
    }

    /// **实际内边距**。
    ///
    /// DPI 策略（见 `docs/design/dpi-policy.md`）：`padding()` 的量是 **DIP**，
    /// 实际值 = `base × metrics.scale()`；需要"就是不缩放的物理像素"（1px 描边、
    /// 命中阈值之类）时用 `padding_fixed()`。
    fn padding_for(&self, scale: f32) -> Edges {
        if let Some(fixed) = self.fixed_padding {
            return fixed;
        }
        let base = self.scaled_padding.unwrap_or(self.padding);
        Edges {
            left: base.left * scale,
            top: base.top * scale,
            right: base.right * scale,
            bottom: base.bottom * scale,
        }
    }

    /// **开启换行**（`flex-wrap: wrap`）。默认不开启。
    ///
    /// 返回：自身。副作用：无。
    /// **开启后走独立分支**，原（单行）路径逐字不动。
    pub fn wrap(mut self, enabled: bool) -> Self {
        self.wrap = enabled;
        self
    }

    /// **实际间距**：`gap()` 是 DIP，实际值 = `base × scale`；
    /// `gap_fixed()` 则不缩放。
    fn gap_for(&self, scale: f32) -> f32 {
        if let Some(fixed) = self.fixed_gap {
            return fixed;
        }
        self.scaled_gap.unwrap_or(self.gap) * scale
    }

    /// **按容器主轴可用宽度分行**（纯函数，不碰 `self`）。
    ///
    /// 返回：`(lines, heights, main_total, cross_total)`。
    /// * `lines`：每行的子项下标；
    /// * `heights`：每行高度 = 该行子项**交叉轴尺寸的最大值**；
    /// * `main_total`：全部行中**最宽那行**的主轴长度；
    /// * `cross_total`：`Σ行高 + gap × (行数 - 1)`。
    ///
    /// **`Dimension::Flex` 子项不吃剩余空间**：分行需要“先知道每个子项宽”，
    /// 而 flex 因子的宽度取决于同行其他子项 —— **互相依赖**。
    /// 因此 wrap 模式下 flex 因子被**忽略**，按内容尺寸分行（已写入文档）。
    fn wrap_lines_of(
        axis: Axis,
        measured: &[(Size, FlexItem)],
        main_max: f32,
        gap: f32,
    ) -> (Vec<Vec<usize>>, Vec<f32>, f32, f32) {
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut heights: Vec<f32> = Vec::new();
        let mut cur: Vec<usize> = Vec::new();
        let mut cur_main = 0.0f32;
        let mut cur_cross = 0.0f32;
        let mut max_main = 0.0f32;

        for (i, (size, _item)) in measured.iter().enumerate() {
            let (m, c) = match axis {
                Axis::Horizontal => (size.w, size.h),
                Axis::Vertical => (size.h, size.w),
            };
            // 行内已有子项时才需要 gap；换行后重新从 0 累加。
            let extra = if cur.is_empty() { 0.0 } else { gap };
            if !cur.is_empty() && cur_main + extra + m > main_max {
                max_main = max_main.max(cur_main);
                lines.push(std::mem::take(&mut cur));
                heights.push(cur_cross);
                cur_main = 0.0;
                cur_cross = 0.0;
            }
            cur_main += if cur.is_empty() { m } else { gap + m };
            cur_cross = cur_cross.max(c);
            cur.push(i);
        }
        if !cur.is_empty() {
            max_main = max_main.max(cur_main);
            lines.push(cur);
            heights.push(cur_cross);
        }
        let cross_total =
            heights.iter().sum::<f32>() + gap * (heights.len().saturating_sub(1)) as f32;
        (lines, heights, max_main, cross_total)
    }

    /// **wrap 的 measure**：按 `constraints.max` 得到的可用宽分行，报**总主轴×总交叉轴**。
    ///
    /// 注意：这里的分行基于 **measure 宽度**；`wrap_arrange` 会用**实际宽度**
    /// 重算 —— 两者不同时，**以 arrange 为准**（与 CSS 一致）。
    fn wrap_measure(
        &mut self,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
        cache: &mut LayoutCache,
    ) -> Size {
        let scale = metrics.scale();
        let padding = self.padding_for(scale);
        let gap = self.gap_for(scale);
        let inner_max = Size::new(
            if constraints.is_unbounded(Axis::Horizontal) {
                f32::INFINITY
            } else {
                (constraints.max.w - padding.horizontal()).max(0.0)
            },
            if constraints.is_unbounded(Axis::Vertical) {
                f32::INFINITY
            } else {
                (constraints.max.h - padding.vertical()).max(0.0)
            },
        );
        let main_max = match self.axis {
            Axis::Horizontal => inner_max.w,
            Axis::Vertical => inner_max.h,
        };

        self.measured.clear();
        for child in self.children.iter_mut() {
            let item = child.flex();
            let size = cache.measure(child.as_mut(), metrics, Constraints::loose(inner_max));
            self.measured.push((size, item));
        }

        let (lines, heights, main_total, cross_total) =
            Self::wrap_lines_of(self.axis, &self.measured, main_max, gap);
        self.wrap_lines = lines;
        self.wrap_line_heights = heights;

        match self.axis {
            Axis::Horizontal => constraints.constrain(Size::new(
                main_total + padding.horizontal(),
                cross_total + padding.vertical(),
            )),
            Axis::Vertical => constraints.constrain(Size::new(
                cross_total + padding.horizontal(),
                main_total + padding.vertical(),
            )),
        }
    }

    /// **wrap 的 arrange**：用**实际可用宽重算分行**，每行用现有
    /// [`flex_arrange_with`] 摆放——因此 `align` / `justify` 在行内**自然生效**，无需重写。
    fn wrap_arrange(&mut self, rect: Rect, metrics: &dyn TextMetrics, cache: &mut LayoutCache) {
        let scale = metrics.scale();
        let padding = self.padding_for(scale);
        let gap = self.gap_for(scale);
        let inner = rect.inset(padding);
        let main_max = match self.axis {
            Axis::Horizontal => inner.w,
            Axis::Vertical => inner.h,
        };

        let (lines, heights, _main_total, _cross_total) =
            Self::wrap_lines_of(self.axis, &self.measured, main_max, gap);

        // **读回 measure 阶段的分行数做一致性断言**：
        // 不等说明 arrange 宽度与 measure 不同 —— 这时**以 arrange 为准**（行高可能与预留高度不符，见文档）。
        let measured_lines = self.wrap_lines.len();
        debug_assert!(
            measured_lines == 0 || measured_lines == lines.len(),
            "wrap: measure 分行数 {} 与 arrange 分行数 {} 不同（宽度变了）",
            measured_lines,
            lines.len()
        );

        // 把最新分行写回字段：下次同宽 measure 若命中缓存，这里保持与 arrange 一致。
        let line_ranges: Vec<(usize, usize, f32)> = lines
            .iter()
            .zip(heights.iter())
            .map(|(l, h)| {
                let start = *l.first().unwrap_or(&0);
                let end = *l.last().unwrap_or(&0) + 1;
                (start, end, *h)
            })
            .collect();
        self.wrap_lines = lines;
        self.wrap_line_heights = heights;

        // 逐行摆放：拆分借用（children 需 &​mut，measured 需 &）。
        let Self {
            children,
            measured,
            align_items,
            justify_content,
            ..
        } = self;
        let mut cursor = match self.axis {
            Axis::Horizontal => inner.y,
            Axis::Vertical => inner.x,
        };
        for (start, end, h) in line_ranges {
            let sub = match self.axis {
                // 横向 wrap：每一行是一条水平带。
                Axis::Horizontal => Rect::new(inner.x, cursor, inner.w, h),
                // 纵向 wrap：每一列是一条竖带。
                Axis::Vertical => Rect::new(cursor, inner.y, h, inner.h),
            };
            // **行内必须忽略 flex 因子**：分行时按内容宽算的行数，
            // 若排列时又让 flex 子项吃剩余空间，它会溢出本行（与文档不符）。
            // 做法：把 flex 维度换成 `Px(实测尺寸)` 的临时副本，**不改 `flex_arrange_with`**。
            let line_measured: Vec<(Size, FlexItem)> = measured[start..end]
                .iter()
                .map(|(size, item)| {
                    let main_flex = match self.axis {
                        Axis::Horizontal => item.width.flex_factor(),
                        Axis::Vertical => item.height.flex_factor(),
                    };
                    let mut item = *item;
                    if main_flex.is_some() {
                        let px = match self.axis {
                            Axis::Horizontal => size.w,
                            Axis::Vertical => size.h,
                        };
                        match self.axis {
                            Axis::Horizontal => item.width = crate::Dimension::Px(px),
                            Axis::Vertical => item.height = crate::Dimension::Px(px),
                        }
                    }
                    (*size, item)
                })
                .collect();
            flex_arrange_with(
                self.axis,
                Edges::ZERO,
                gap,
                *align_items,
                *justify_content,
                &mut children[start..end],
                sub,
                metrics,
                cache,
                &line_measured,
            );
            cursor += h + gap;
        }
    }

    /// 设置**交叉轴对齐**（`align-items`）。
    ///
    /// 默认 `Align::Stretch`（Auto 子项拿满交叉轴）。
    pub fn align(mut self, align: Align) -> Self {
        self.align_items = align;
        self
    }

    /// 设置**主轴分布**（`justify-content`）。
    ///
    /// 默认 `Justify::Start`（从起点紧排）。
    pub fn justify(mut self, justify: Justify) -> Self {
        self.justify_content = justify;
        self
    }

    /// 设置背景色；`None` 时不画，避免不必要的填充。
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// 设置**背景圆角半径**（逻辑像素）。`0.0` = 直角（默认）。
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius.max(0.0);
        self
    }

    /// 设置**描边**（颜色 + 宽度）；默认不描边。
    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.border = Some((color, width.max(0.0)));
        self
    }

    /// 设置自己在父容器里的尺寸约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }

    /// 追加子控件并返回自身（链式构造）。
    pub fn push(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }

    /// 追加子控件（就地修改，用于动态增删）。
    pub fn push_boxed(&mut self, child: Box<dyn Widget>) {
        self.children.push(child);
    }
}

impl Widget for Flex {
    fn id(&self) -> &str {
        &self.id
    }

    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }

    fn role(&self) -> Role {
        Role::PANEL
    }

    fn flex(&self) -> FlexItem {
        self.item
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }

    fn measure(
        &mut self,
        metrics: &dyn TextMetrics,
        constraints: Constraints,
        cache: &mut LayoutCache,
    ) -> Size {
        // **wrap 走独立分支**；下面的原路径逐字未动（默认 `wrap == false`）。
        if self.wrap {
            return self.wrap_measure(metrics, constraints, cache);
        }
        flex_measure_into(
            self.axis,
            self.padding_for(metrics.scale()),
            self.gap_for(metrics.scale()),
            &mut self.children,
            metrics,
            constraints,
            cache,
            &mut self.measured,
        )
    }

    fn arrange(&mut self, rect: Rect, metrics: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        // **wrap 走独立分支**；原路径逐字未动。
        if self.wrap {
            self.wrap_arrange(rect, metrics, cache);
            return;
        }
        flex_arrange_with(
            self.axis,
            self.padding_for(metrics.scale()),
            self.gap_for(metrics.scale()),
            self.align_items,
            self.justify_content,
            &mut self.children,
            rect,
            metrics,
            cache,
            &self.measured,
        );
    }

    fn rect(&self) -> Rect {
        self.rect
    }

    fn padding(&self) -> Edges {
        self.padding
    }

    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 回退路径保持与加 radius/border 之前**逐像素相同**：
        // radius == 0 且 border == None 时走 `fill_rect`（不走 SDF）。
        let plain = self.radius <= 0.0 && self.border.is_none();
        if let Some(bg) = self.background {
            if plain {
                ctx.fill_rect(self.rect, bg);
            } else {
                ctx.fill_round_rect(self.rect, bg, self.radius);
            }
        }
        if let Some((color, width)) = self.border {
            if width > 0.0 {
                ctx.stroke_round_rect(self.rect, color, width, self.radius);
            }
        }
    }

    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

// ============================ 布局入口 ============================

/// 布局入口。
///
/// 根节点约束与版本都没变、且无脏标记时，整次布局跳过（O(1)）。
pub fn layout_tree(
    root: &mut dyn Widget,
    viewport: Size,
    metrics: &dyn TextMetrics,
    cache: &mut LayoutCache,
) -> LayoutStats {
    cache.begin_layout();
    // **清掉可能残留的祖先栈**：`measure` 里 push/pop 成对出现，但控件 panic 会跳过
    // pop，栈里留下半条祖先链。下一次布局若不清理，`parents` 会被写成"上一个崩溃
    // 那轮的祖先"，脏传播随之错乱 —— 这是"panic 之后还能继续用"的最低要求。
    cache.stack.clear();

    let tight = Constraints::tight(viewport);
    if cache.is_fresh(root.id(), tight, root.layout_version()) {
        cache.stats.skipped = true;
        return cache.stats;
    }

    let _ = cache.measure(root, metrics, tight);
    cache.stats.arranged += 1;
    root.arrange(Rect::from_size(viewport), metrics, cache);

    cache.finish_layout();
    cache.stats
}
/// 叠放容器：所有子节点拿到**同一个矩形**。
///
/// 浮层（Menu/Dialog）需要一个"全屏宿主"，否则它们的 rect 会被父容器的
/// flex 槽位限制住，既盖不住全屏，也无法用几何判断"点在外面"。
pub struct Stack {
    id: String,
    children: Vec<Box<dyn Widget>>,
    rect: Rect,
    item: FlexItem,
    /// 自身 id 的缓存键（构造时算一次）。
    key: u64,
}

impl Stack {
    /// 创建一个单子节点容器（用于把任意控件塞进需要容器的位置）。
    pub fn new(id: impl Into<String>) -> Self {
        let id: String = id.into();
        let key = id_key(&id);
        Self {
            id,
            key,
            children: Vec::new(),
            rect: Rect::ZERO,
            item: FlexItem::default(),
        }
    }

    /// 设置唯一子节点。
    pub fn push(mut self, child: Box<dyn Widget>) -> Self {
        self.children.push(child);
        self
    }

    /// 设置自己在父容器里的尺寸约束。
    pub fn flex_item(mut self, item: FlexItem) -> Self {
        self.item = item;
        self
    }
}

impl Widget for Stack {
    fn id(&self) -> &str {
        &self.id
    }

    fn cache_key(&self) -> Option<u64> {
        Some(self.key)
    }
    fn role(&self) -> Role {
        Role::PANEL
    }
    fn flex(&self) -> FlexItem {
        self.item
    }
    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }
    fn children_mut(&mut self) -> Option<&mut Vec<Box<dyn Widget>>> {
        Some(&mut self.children)
    }
    fn measure(&mut self, m: &dyn TextMetrics, c: Constraints, cache: &mut LayoutCache) -> Size {
        let mut content_w = 0.0f32;
        let mut content_h = 0.0f32;
        for child in self.children.iter_mut() {
            let s = cache.measure(child.as_mut(), m, c);
            content_w = content_w.max(s.w);
            content_h = content_h.max(s.h);
        }
        // **有界时仍然"填满"**（叠放容器的语义：所有子节点拿同一个矩形，历来如此），
        // 但**无界时回退到内容**：旧实现直接 `c.constrain(c.max)`，而无界轴的
        // `c.max = ∞` 会被 `Constraints::constrain` 归零 ——
        // 于是放进滚动容器里的 `Stack` 报高度 0，滚不动（内容明明有高度）。
        let w = if c.max.w.is_finite() {
            c.max.w
        } else {
            content_w
        };
        let h = if c.max.h.is_finite() {
            c.max.h
        } else {
            content_h
        };
        c.constrain(Size::new(w, h))
    }
    fn arrange(&mut self, rect: Rect, m: &dyn TextMetrics, cache: &mut LayoutCache) {
        self.rect = rect;
        for child in self.children.iter_mut() {
            child.arrange(rect, m, cache);
        }
    }
    fn rect(&self) -> Rect {
        self.rect
    }
    fn event(&mut self, _ctx: &mut EventCtx, _event: &UiEvent) -> EventResult {
        EventResult::Ignored
    }
    fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
    fn semantics(&self) -> Semantics {
        Semantics::default().exposed(true)
    }
}

// ============================ align / justify 单测 ============================

#[cfg(test)]
mod align_justify_tests {
    use super::*;
    use crate::widget::{Constraints, Widget};
    use crate::{Size, TextStyle};

    /// 固定尺寸的标记子控件（只用于断言 rect）。
    struct Box1 {
        id: &'static str,
        size: Size,
        rect: Rect,
        item: FlexItem,
    }

    impl Box1 {
        fn new(id: &'static str, w: f32, h: f32) -> Self {
            Self {
                id,
                size: Size::new(w, h),
                rect: Rect::ZERO,
                item: FlexItem::default(),
            }
        }
        fn with_item(mut self, item: FlexItem) -> Self {
            self.item = item;
            self
        }
    }

    impl Widget for Box1 {
        fn id(&self) -> &str {
            self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(self.size)
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn flex(&self) -> FlexItem {
            self.item
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn children(&self) -> &[Box<dyn Widget>] {
            &[]
        }
        fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    struct Dummy;
    impl TextMetrics for Dummy {
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

    /// 布一次局，返回每个子件的 rect。
    fn layout(flex: &mut Flex, w: f32, h: f32) -> Vec<Rect> {
        let mut cache = LayoutCache::new();
        let m = Dummy;
        let _ = cache.measure(flex, &m, Constraints::tight(Size::new(w, h)));
        flex.arrange(Rect::from_size(Size::new(w, h)), &m, &mut cache);
        let mut out = Vec::new();
        walk(flex, &mut out);
        out
    }

    fn walk(node: &mut dyn Widget, out: &mut Vec<Rect>) {
        for c in node.children_mut().unwrap_or(&mut Vec::new()).iter_mut() {
            out.push(c.rect());
            walk(c.as_mut(), out);
        }
    }

    #[test]
    fn align_center_offsets_each_child() {
        // 容器 100 高，子件高 10 / 20 / 30（Auto 高度 = 内容）
        let mut flex = Flex::row("row")
            .align(Align::Center)
            .push(Box::new(Box1::new("a", 10.0, 10.0)))
            .push(Box::new(Box1::new("b", 10.0, 20.0)))
            .push(Box::new(Box1::new("c", 10.0, 30.0)));
        let rects = layout(&mut flex, 100.0, 100.0);
        assert_eq!(rects[0].y, 45.0, "高 10 居中 → y=45");
        assert_eq!(rects[1].y, 40.0, "高 20 居中 → y=40");
        assert_eq!(rects[2].y, 35.0, "高 30 居中 → y=35");
        // 居中时子件不被拉伸：高度保持内容值
        assert_eq!(rects[0].h, 10.0);
        assert_eq!(rects[2].h, 30.0);
    }

    #[test]
    fn align_stretch_keeps_old_behaviour() {
        // 默认 Stretch：Auto 高度子件拿满交叉轴（= 加能力前行为）
        let mut flex = Flex::row("row")
            .push(Box::new(Box1::new("a", 10.0, 10.0)))
            .push(Box::new(Box1::new("b", 10.0, 20.0)));
        let rects = layout(&mut flex, 100.0, 100.0);
        assert_eq!(rects[0].y, 0.0);
        assert_eq!(rects[0].h, 100.0, "Stretch: 交叉轴拉满");
        assert_eq!(rects[1].x, 10.0, "主轴从起点紧排");
    }

    #[test]
    fn justify_space_between_spreads_free_space() {
        // 3 个 20 宽子件在 100 宽内：剩余 40 → 间隔各 +20
        let mut flex = Flex::row("row")
            .justify(Justify::SpaceBetween)
            .push(Box::new(Box1::new("a", 20.0, 10.0)))
            .push(Box::new(Box1::new("b", 20.0, 10.0)))
            .push(Box::new(Box1::new("c", 20.0, 10.0)));
        let rects = layout(&mut flex, 100.0, 100.0);
        assert_eq!(rects[0].x, 0.0);
        assert_eq!(rects[1].x, 40.0, "20 + gap(0) + extra(20)");
        assert_eq!(rects[2].x, 80.0);
    }

    #[test]
    fn justify_space_around_puts_half_gap_at_ends() {
        // 2 个 20 宽子件在 100 宽内：剩余 60 → 每份 30，两侧各 15
        let mut flex = Flex::row("row")
            .justify(Justify::SpaceAround)
            .push(Box::new(Box1::new("a", 20.0, 10.0)))
            .push(Box::new(Box1::new("b", 20.0, 10.0)));
        let rects = layout(&mut flex, 100.0, 100.0);
        assert_eq!(rects[0].x, 15.0, "首端留半份间距");
        assert_eq!(rects[1].x, 65.0, "15 + 20 + 30");
    }

    #[test]
    fn align_center_with_justify_center_centers_child() {
        let mut flex = Flex::row("row")
            .align(Align::Center)
            .justify(Justify::Center)
            .push(Box::new(Box1::new("only", 20.0, 20.0)));
        let rects = layout(&mut flex, 100.0, 100.0);
        assert_eq!(rects[0].x, 40.0);
        assert_eq!(rects[0].y, 40.0);
    }
    #[test]
    fn min_width_applies() {
        // 内容宽 20 但 min_width = 100 → 实际 100
        let mut flex = Flex::row("row").push(Box::new(
            Box1::new("a", 20.0, 10.0).with_item(FlexItem::width(20.0).min_width(100.0)),
        ));
        let rects = layout(&mut flex, 300.0, 100.0);
        assert_eq!(rects[0].w, 100.0, "min_width 应把 20 抬到 100");
    }

    #[test]
    fn max_width_applies() {
        // 内容宽 200 但 max_width = 80 → 实际 80
        let mut flex = Flex::row("row").push(Box::new(
            Box1::new("a", 200.0, 10.0).with_item(FlexItem::width(200.0).max_width(80.0)),
        ));
        let rects = layout(&mut flex, 300.0, 100.0);
        assert_eq!(rects[0].w, 80.0, "max_width 应把 200 压到 80");
    }

    #[test]
    fn min_larger_than_available_degrades_to_available() {
        // 容器只有 50，下界 100 → **退化为可用空间 50**（不溢出）
        let mut flex = Flex::row("row").push(Box::new(
            Box1::new("a", 20.0, 10.0).with_item(FlexItem::width(20.0).min_width(100.0)),
        ));
        let rects = layout(&mut flex, 50.0, 100.0);
        assert_eq!(
            rects[0].w, 50.0,
            "min > 可用空间时应 clamp 到可用空间（不溢出父容器）"
        );
    }

    #[test]
    fn default_min_max_does_not_change_layout() {
        // 默认 NO_LIMITS：布局与不设 min/max 时一致
        let mut flex = Flex::row("row")
            .push(Box::new(Box1::new("a", 30.0, 10.0)))
            .push(Box::new(Box1::new("b", 40.0, 10.0)));
        let rects = layout(&mut flex, 200.0, 100.0);
        assert_eq!(rects[0].w, 30.0);
        assert_eq!(rects[1].w, 40.0);
        assert_eq!(rects[1].x, 30.0);
    }
}

#[cfg(test)]
mod gap_scaled_tests {
    use super::*;
    use crate::widget::Constraints;
    use crate::{Size, TextStyle};

    /// 按固定字号回报 `scale` 的 mock metrics（仅测试用）。
    struct ScaledMetrics(f32);
    impl TextMetrics for ScaledMetrics {
        fn measure_text(&self, text: &str, _s: &TextStyle) -> Size {
            Size::new(text.chars().count() as f32 * 8.0, 16.0)
        }
        fn line_height(&self, _s: &TextStyle) -> f32 {
            16.0
        }
        fn scale(&self) -> f32 {
            self.0
        }
    }

    /// 固定尺寸的子控件（只用于断言 rect）。
    struct Box1 {
        id: &'static str,
        size: Size,
        rect: Rect,
        item: FlexItem,
    }
    impl Box1 {
        fn new(id: &'static str, w: f32, h: f32) -> Self {
            Self {
                id,
                size: Size::new(w, h),
                rect: Rect::ZERO,
                item: FlexItem::default(),
            }
        }
    }
    impl Widget for Box1 {
        fn id(&self) -> &str {
            self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(self.size)
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn flex(&self) -> FlexItem {
            self.item
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn children(&self) -> &[Box<dyn Widget>] {
            &[]
        }
        fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    fn layout_with(flex: &mut Flex, scale: f32) -> Vec<Rect> {
        let mut cache = LayoutCache::new();
        let m = ScaledMetrics(scale);
        let _ = cache.measure(flex, &m, Constraints::tight(Size::new(200.0, 100.0)));
        flex.arrange(Rect::from_size(Size::new(200.0, 100.0)), &m, &mut cache);
        let mut out = Vec::new();
        for c in flex.children_mut().unwrap_or(&mut Vec::new()).iter_mut() {
            out.push(c.rect());
        }
        out
    }

    fn three_boxes(gap_fixed: Option<f32>, gap_scaled: Option<f32>) -> Flex {
        let mut f = Flex::row("row");
        if let Some(g) = gap_fixed {
            f = f.gap(g);
        }
        if let Some(g) = gap_scaled {
            f = f.gap_scaled(g);
        }
        f.push(Box::new(Box1::new("a", 20.0, 10.0)))
            .push(Box::new(Box1::new("b", 20.0, 10.0)))
            .push(Box::new(Box1::new("c", 20.0, 10.0)))
    }

    #[test]
    fn gap_scaled_at_scale_one_equals_gap() {
        let mut fixed = three_boxes(Some(8.0), None);
        let mut scaled = three_boxes(None, Some(8.0));
        let rf = layout_with(&mut fixed, 1.0);
        let rs = layout_with(&mut scaled, 1.0);
        assert_eq!(rf, rs, "scale=1.0 时 gap_scaled(8) 必须等价于 gap(8)");
    }

    #[test]
    fn gap_scaled_follows_scale() {
        let mut scaled = three_boxes(None, Some(8.0));
        let rs = layout_with(&mut scaled, 1.25);
        // 20 + (20*1.25=10) -> 第二个子项 x = 20 + 10 = 30
        assert_eq!(rs[1].x, 30.0, "125% 下间距应为 8*1.25=10（而非 8）");
        assert_eq!(rs[2].x, 60.0);
    }

    #[test]
    fn padding_scaled_follows_scale() {
        let mut f = Flex::column("col")
            .padding_scaled(Edges::all(8.0))
            .push(Box::new(Box1::new("a", 20.0, 10.0)));
        let r = layout_with(&mut f, 1.25);
        assert_eq!(r[0].x, 10.0, "左内边距应 8*1.25=10");
        assert_eq!(r[0].y, 10.0, "上内边距应 8*1.25=10");
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;
    use crate::widget::{Constraints, Widget};
    use crate::{Size, TextStyle};

    /// 测试用 metrics：固定字宽/行高，`scale = 1.0`。无字段（免未读字段告警）。
    struct Fixed;
    impl TextMetrics for Fixed {
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

    struct Box1 {
        id: &'static str,
        size: Size,
        rect: Rect,
        item: FlexItem,
    }
    impl Box1 {
        fn new(id: &'static str, w: f32, h: f32) -> Self {
            Self {
                id,
                size: Size::new(w, h),
                rect: Rect::ZERO,
                item: FlexItem::default(),
            }
        }
    }
    impl Widget for Box1 {
        fn id(&self) -> &str {
            self.id
        }
        fn measure(
            &mut self,
            _m: &dyn TextMetrics,
            c: Constraints,
            _cache: &mut LayoutCache,
        ) -> Size {
            c.constrain(self.size)
        }
        fn arrange(&mut self, rect: Rect, _m: &dyn TextMetrics, _cache: &mut LayoutCache) {
            self.rect = rect;
        }
        fn rect(&self) -> Rect {
            self.rect
        }
        fn flex(&self) -> FlexItem {
            self.item
        }
        fn event(&mut self, _c: &mut EventCtx, _e: &UiEvent) -> EventResult {
            EventResult::Ignored
        }
        fn children(&self) -> &[Box<dyn Widget>] {
            &[]
        }
        fn render(&mut self, _ctx: &mut dyn RenderCtx) {}
        fn semantics(&self) -> Semantics {
            Semantics::default()
        }
    }

    /// 布一次局，返回 (measure 得到的尺寸, 子项 rects)。
    fn layout(flex: &mut Flex, w: f32, h: f32) -> (Size, Vec<Rect>) {
        let mut cache = LayoutCache::new();
        let m = Fixed;
        let size = cache.measure(flex, &m, Constraints::tight(Size::new(w, h)));
        flex.arrange(Rect::from_size(Size::new(w, h)), &m, &mut cache);
        let mut out = Vec::new();
        for c in flex.children_mut().unwrap_or(&mut Vec::new()).iter_mut() {
            out.push(c.rect());
        }
        (size, out)
    }

    #[test]
    fn default_no_wrap_still_overflows() {
        // 3 × 20px 子项在 50px 宽内：**默认不换行 → 单行、总宽 60 溢出**。
        let mut f = Flex::row("r")
            .push(Box::new(Box1::new("a", 20.0, 10.0)))
            .push(Box::new(Box1::new("b", 20.0, 10.0)))
            .push(Box::new(Box1::new("c", 20.0, 10.0)));
        let (_size, rects) = layout(&mut f, 50.0, 100.0);
        // 三个子项 y 相同 = 单行
        assert_eq!(rects[0].y, rects[1].y, "默认应单行");
        assert_eq!(rects[1].y, rects[2].y, "默认应单行");
        // 第三个子项越过 50px 宽度 → 溢出（与改前一致）
        assert!(rects[2].right() > 50.0, "默认应允许溢出：{rects:?}");
    }

    #[test]
    fn wrap_breaks_into_multiple_lines() {
        // 3 × 20px 在 50px 内 + wrap → 2+1 两行
        let mut f = Flex::row("r")
            .wrap(true)
            .push(Box::new(Box1::new("a", 20.0, 10.0)))
            .push(Box::new(Box1::new("b", 20.0, 10.0)))
            .push(Box::new(Box1::new("c", 20.0, 10.0)));
        let (_size, rects) = layout(&mut f, 50.0, 100.0);
        assert_eq!(rects[0].y, rects[1].y, "第一行放 a,b");
        assert!(rects[2].y > rects[1].y, "第二行放 c：{rects:?}");
        assert_eq!(rects[2].x, 0.0, "新行从左侧重新开始");
    }

    #[test]
    fn wrap_line_height_is_max_of_row() {
        // 一行内 10px 与 20px 子项 → 行高 = 20（第二行的 y 应 = 20）
        // 宽 15 × 2 = 30 刚好放进 30 宽的第一行（a、b），c 落第二行。
        let mut f = Flex::row("r")
            .wrap(true)
            .push(Box::new(Box1::new("a", 15.0, 10.0)))
            .push(Box::new(Box1::new("b", 15.0, 20.0)))
            .push(Box::new(Box1::new("c", 15.0, 10.0)));
        let (_size, rects) = layout(&mut f, 30.0, 100.0);
        assert_eq!(rects[0].y, rects[1].y, "a、b 应在同一行：{rects:?}");
        assert_eq!(rects[2].y, 20.0, "行高应取该行最大子项高 20：{rects:?}");
    }

    #[test]
    fn wrap_empty_children_no_panic() {
        let mut f = Flex::row("r").wrap(true);
        let (size, rects) = layout(&mut f, 50.0, 100.0);
        assert!(rects.is_empty(), "空子项集不应有 rect");
        assert!(
            size.w.is_finite() && size.h.is_finite(),
            "尺寸必须有限：{size:?}"
        );
        // 分行结果应为空（无行可排）。
        assert!(f.wrap_lines.is_empty(), "空子项集不五有分行结果");
    }

    /// **回归**：wrap + `Dimension::Flex` 子项时，行内不得溢出。
    ///
    /// 修复前：`measure` 忽略 flex 因子（按内容宽分行），但 `arrange` 的
    /// `flex_arrange_with` 仍让 flex 子项吃"本行剩余空间" → 子项右边界超出容器宽。
    #[test]
    fn wrap_with_flex_child_does_not_overflow() {
        let mut flex_child = Box1::new("f", 20.0, 10.0);
        flex_child.item.width = crate::Dimension::Flex(1.0);
        let mut f = Flex::row("r")
            .wrap(true)
            .push(Box::new(Box1::new("a", 20.0, 10.0)))
            .push(Box::new(flex_child))
            .push(Box::new(Box1::new("c", 20.0, 10.0)));
        let (_size, rects) = layout(&mut f, 50.0, 100.0);
        for (i, r) in rects.iter().enumerate() {
            assert!(
                r.right() <= 50.0 + 0.01,
                "子项 {i} 溢出容器：right={} rect={r:?}",
                r.right()
            );
        }
    }

    /// **回归**：wrap + `Align::Center` 时行内子项在行高内居中对齐。
    #[test]
    fn wrap_line_aligns_children_with_center() {
        let mut f = Flex::row("r")
            .wrap(true)
            .align(Align::Center)
            .push(Box::new(Box1::new("a", 15.0, 10.0)))
            .push(Box::new(Box1::new("b", 15.0, 20.0)))
            .push(Box::new(Box1::new("c", 15.0, 10.0)));
        let (_size, rects) = layout(&mut f, 30.0, 100.0);
        // 第一行高 = 20（a:10, b:20）→ a 居中后 y = (20-10)/2 = 5
        assert_eq!(rects[0].y, 5.0, "a 应在行内垂直居中：{rects:?}");
        assert_eq!(rects[1].y, 0.0, "b 撑满行高：{rects:?}");
    }

    /// **回归**：`Stack` 在**无界**轴上必须报内容尺寸，不能报 0。
    ///
    /// 旧实现 `c.constrain(c.max)` 在无界轴（`max = ∞`）会被 `Constraints::constrain`
    /// 归零 —— 放进滚动容器的 `Stack` 高度报 0，于是"滚不动"（内容明明有高度）。
    #[test]
    fn stack_reports_content_on_unbounded_axis() {
        use crate::Axis2;

        let mut stack = Stack::new("s").push(Box::new(Box1::new("a", 10.0, 20.0)));
        let mut cache = LayoutCache::new();
        let m = Fixed;
        // 竖直方向无界（滚动容器内部就是这么量的）
        let c = Constraints {
            min: Size::ZERO,
            max: Size::new(100.0, f32::INFINITY),
            unbounded: Axis2::V,
        };
        let size = cache.measure(&mut stack, &m, c);
        assert_eq!(
            (size.w, size.h),
            (100.0, 20.0),
            "有界轴填满（100），无界轴回退内容（20）"
        );
    }

    /// 有界时 `Stack` 仍然"填满"（保持既有语义，别把叠放容器改成内容自适应）。
    #[test]
    fn stack_still_fills_bounded_constraints() {
        let mut stack = Stack::new("s2").push(Box::new(Box1::new("a", 10.0, 20.0)));
        let mut cache = LayoutCache::new();
        let size = cache.measure(
            &mut stack,
            &Fixed,
            Constraints::loose(Size::new(100.0, 80.0)),
        );
        assert_eq!((size.w, size.h), (100.0, 80.0), "有界时填满父级");
    }
}
