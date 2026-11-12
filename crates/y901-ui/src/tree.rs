//! 语义树：把 Widget 树变成"可查询、可注入、可命中"的结构。
//!
//! 这层是自动化友好的核心：外部只能通过 id 操作节点，
//! 拿到的矩形与语义与实际渲染完全一致（同一棵树算出来的）。

use std::collections::HashMap;

use crate::event::{EventPhase, PointerCapture};
use crate::event::{EventResult, UiEvent};
use crate::geometry::{Pos, Rect, Size};
use crate::widget::{EventCtx, RenderCtx, Role, Semantics, TextMetrics, Widget};

/// 对外暴露的节点快照。RPC 的 `get_tree` 直接序列化它。
#[derive(Clone, Debug)]
pub struct NodeInfo {
    /// 节点 id（树内唯一）。
    pub id: String,
    /// 角色名；主题查表与语义过滤都用它。
    pub role: Role,
    /// 布局后的屏幕矩形。
    pub rect: Rect,
    /// 是否持有键盘焦点。
    pub focused: bool,
    /// 语义信息（标签、值、动作）；这就是外部工具看到的内容。
    pub semantics: Semantics,
    /// 子节点。
    pub children: Vec<NodeInfo>,
}

impl NodeInfo {
    /// 深度优先找节点。
    pub fn find(&self, id: &str) -> Option<&NodeInfo> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    /// 扁平化遍历。
    pub fn walk(&self, f: &mut impl FnMut(&NodeInfo)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }
}

/// 脏区：本帧需要重绘的区域集合。
///
/// 空集合 = 无事发生；`full` = 整窗重绘（首帧、主题变更、设备重建）。
#[derive(Clone, Debug, Default)]
pub struct DirtyRegion {
    full: bool,
    rects: Vec<Rect>,
}

impl DirtyRegion {
    /// 整窗脏区（等价于"全部重画"）。
    ///
    /// 注意：它不代表"一个覆盖全窗的矩形"，而是一个 `full` 标志——后续 `add()`
    /// 会被忽略（已经是全量）。
    pub fn full() -> Self {
        Self {
            full: true,
            rects: Vec::new(),
        }
    }

    /// 把一个矩形并入脏区。
    ///
    /// 已在 `full` 状态或矩形为空时直接返回（无副作用）。
    /// 脏区列表的上限。超过就退化为整窗重绘。
    ///
    /// 为什么在 `add` 里就退化（而不是等到渲染时）：
    /// `add` 是 O(n) 线性扫描，碎片时总体是 O(n²)。
    /// 实测：10k 次追加要 145ms。上限一到就整窗，把最坏情况压回 O(1)。
    pub const MAX_RECTS: usize = 16;

    /// 两块是否**真实重叠**（重叠面积 > 0）。
    ///
    /// 为何不用“边贴边即可合并”：零重叠时 `area(A∪B) − area(A) − area(B)` 恒 ≥ 0，
    /// 而合并只能省一次切换 —— 几何上相邻并不代表合并后面积不增。
    /// 实测反例：1000×1 与 1×1000 在角上相贴 → 合并成 1000×1001（过度覆盖 500×）。
    fn overlaps(a: Rect, b: Rect) -> bool {
        let x = a.x < b.x + b.w && b.x < a.x + a.w;
        let y = a.y < b.y + b.h && b.y < a.y + a.h;
        x && y
    }

    /// 追加一块脏区。为什么不合并成包围盒：两块不相邻的脏区合并后，中间的空白也会被清屏——而 CPU 侧已跳过那些节点，结果就是一块黑。
    pub fn add(&mut self, rect: Rect) {
        if self.full || rect.is_empty() {
            return;
        }
        // 地板保护：已经太碎就不再合并，直接整窗。
        if self.rects.len() >= Self::MAX_RECTS {
            self.full = true;
            self.rects.clear();
            return;
        }
        // 合并策略（两向 + 相交取并）：
        // 1. 新矩形被已有矩形包含  → 什么也不用做；
        // 2. 新矩形包含了已有矩形  → 删掉已有（否则流式增长会无限堆积）；
        // 3. 两者相交                  → 用并集替换（少一个矩形，且不漏画）。
        // 之前只有 (1)，流式输出这种"逐次变大"的脏区会一直追加，
        // 然后撞上 MAX_DIRTY_RECTS 降级成整窗重绘。
        let mut i = 0;
        while i < self.rects.len() {
            let r = self.rects[i];
            if r.contains_rect(rect) {
                return;
            }
            if rect.contains_rect(r) {
                self.rects.swap_remove(i);
                continue;
            }
            if Self::overlaps(r, rect) {
                let merged = r.union(rect);
                self.rects.swap_remove(i);
                // 合并后的矩形可能还能合并其它的：递归一次（数量小，不担心栈深）
                self.add(merged);
                return;
            }
            i += 1;
        }
        self.rects.push(rect);
    }

    /// 是否整窗重绘（此时 `rects()` 无意义）。
    pub fn is_full(&self) -> bool {
        self.full
    }

    /// 是否没有脏区（可以整帧跳过绘制）。
    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }

    /// 脏区块数。
    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// 矩形是否与脏区相交。
    pub fn intersects(&self, rect: Rect) -> bool {
        if self.full {
            return true;
        }
        self.rects.iter().any(|r| !r.intersect(rect).is_empty())
    }

    /// 清空脏区（渲染完成后调用）。
    pub fn clear(&mut self) {
        self.full = false;
        self.rects.clear();
    }

    /// 脏矩形列表（`is_full()` 为真时无意义——那时整窗都要重绘）。
    ///
    /// 返回：只读切片，供渲染后端做 scissor / 局部绘制。
    /// 副作用：无。
    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// 把所有脏矩形合并成一个包围盒（忽略空集）。
    ///
    /// 返回：`None` = 没有脏矩形（且不是 full）；否则是覆盖全部脏块的矩形。
    /// 用途：后端设置单一 scissor 时用它，避免逐矩形多次 draw。
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.rects.iter().filter(|r| !r.is_empty());
        let first = *it.next()?;
        Some(it.fold(first, |acc, r| {
            let x0 = acc.x.min(r.x);
            let y0 = acc.y.min(r.y);
            let x1 = acc.right().max(r.right());
            let y1 = acc.bottom().max(r.bottom());
            Rect::new(x0, y0, x1 - x0, y1 - y0)
        }))
    }
}

/// 挂在 Overlay 层的浮层（菜单/Tooltip/弹窗都用它）。
///
/// 为什么不直接当普通子节点：普通子节点会继承父链的 `push_clip`
/// （滚动容器/面板会裁切），而浮层必须画在窗口最上层、裁切只到视口。
pub struct Popup {
    id: String,
    widget: Box<dyn Widget>,
    /// 触发控件的屏幕矩形（弹层据此摆放，放不下会翻转）。
    anchor: Rect,
    /// 布局阶段算出的真实矩形（绘制与命中共用它）。
    rect: Rect,
    /// 是否模态：模态时底层控件不接收任何事件（点外部也不会关闭）。
    modal: bool,
    /// 弹出前的焦点（关闭时还回去）。
    prev_focus: Option<String>,
}

impl Popup {
    /// 浮层 id（取自内部控件）。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 浮层当前的屏幕矩形（布局后有效）。
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// 浮层里装的控件（只读）。
    pub fn widget(&self) -> &dyn Widget {
        self.widget.as_ref()
    }

    /// 浮层里装的控件（可变）。
    pub fn widget_mut(&mut self) -> &mut dyn Widget {
        self.widget.as_mut()
    }

    /// 是否模态。
    pub fn is_modal(&self) -> bool {
        self.modal
    }
}

/// UI 树：持有根节点、索引、焦点、脏区、浮层与动画。**所有状态变更都从这里发起**，组件本身不持有跨帧状态。
pub struct UiTree {
    /// 待释放的纹理（卸载时收集，下一帧渲染时交给后端）。
    pending_texture_releases: Vec<crate::image::TextureId>,
    /// **DPI 缩放**（最近一次 `layout()` 从 `metrics.scale()` 取）。
    ///
    /// 为何要存：事件分发（`event` / `send_focus`）拿不到 `metrics`，
    /// 但命中测试需要与绘制同一个尺寸。存在这里后拷贝进 `EventCtx`。
    scale: f32,
    /// 活动中的动画（key → 动画）。**同 key 重新设置即打断旧动画**。
    animations: Vec<ActiveAnim>,
    root: Box<dyn Widget>,
    /// Overlay 层的浮层（按开启顺序，后开的在上）。
    popups: Vec<Popup>,
    focus: Option<String>,
    viewport: Size,
    /// id -> rect 的缓存，避免每次 RPC 都走一遍树。
    rects: HashMap<u64, Rect>,
    /// 组件在本轮分发中产生的命令，等待 `drain_commands` 处理。
    pending: Vec<crate::command::Command>,
    /// 布局记忆化 + 脏传播。
    cache: crate::layout::LayoutCache,
    /// 本帧待重绘区域。
    dirty: DirtyRegion,
    /// 框架统一维护的悬停节点。
    hovered: Option<String>,
    /// 指针捕获目标：按住期间所有 MouseMove/MouseUp 都强制给它。
    captured_pointer: Option<String>,
    /// 绘制/命中顺序（按层稳定排序）。
    paint_order: Vec<PaintEntry>,
    /// 最上层 Modal 的路径（焦点作用域 + 点击吞噬判定）。
    modal_root: Option<Vec<usize>>,
    /// 焦点节点路径缓存（键盘事件零分配）。
    focus_path: Option<Vec<usize>>,
    /// 捕获节点的路径缓存。
    captured_path: Option<Vec<usize>>,
    /// 复用的事件路径缓冲，避免每次鼠标移动都分配。
    path_buf: Vec<usize>,
    /// `render_pass` 的临时缓存：子节点矩形。
    ///
    /// **为何要复用**：`render_pass` 是**每帧每节点**都跑的热路径，
    /// 每次都 `Vec::new()` 会产生大量短命分配（churn）。
    render_rects: Vec<Rect>,
    /// `render_pass` 的临时缓存：子节点裁切（每层用一段，用完 truncate）。
    ///
    /// **panic 安全性**：`render_pass` 如果 panic，scratch 不会被放回
    /// （`self.render_rects = rects` 在 `render_pass` 返回后才执行），
    /// 下次 `render` 会重新分配。**这是性能退化，不是正确性问题**。
    render_clips: Vec<Option<Rect>>,
    /// 组件请求的 IME 候选窗位置（应用层取走后清空）。
    pending_ime: Option<Pos>,
    /// 焦点变更重入保护。
    in_focus_change: bool,
    /// Tab 遍历顺序（前序、只含 focusable，受 Modal 作用域限制）。
    focus_order: Vec<Vec<usize>>,
    /// 语义订阅（RPC push）。
    subscriptions: Vec<Subscription>,
    /// 响应式失效队列（Signal 变化 → 节点标脏）。
    invalidation_queue: crate::signal::Invalidation,
    /// 后台线程用的标脏句柄（`Send + Sync`；**不持有树**，因此可跨线程）。
    external: crate::invalidate::InvalidationHandle,
    /// 结构性变化（增删节点）后置位：主循环据此决定这一帧是否要 layout。
    layout_requested: bool,
    /// 广播事件（`Tick` 等）中"要求重绘"节点的矩形（复用缓冲，避免每帧分配）。
    broadcast_rects: Vec<Rect>,
    /// 本次事件里**真正返回 Relayout** 的节点 id。
    ///
    /// 为什么不是 `target`：滚动事件命中的是行/子控件，请求重排的是外层容器。
    /// 只按命中目标标脏会把脏区标到错误的位置（实测：滚动只标了 20px 高的一行）。
    relayout_from: Option<String>,
    /// 每个 id 的实例代号：`add_node` 时递增，用于识别 id 复用。
    generations: HashMap<String, u64>,
    /// 自增代号源。
    next_generation: u64,
    /// 上次观察到的语义面 `(generation, hash)`。
    last_semantic: HashMap<String, (u64, u64)>,
    /// 框架维护的 revision 计数器（与组件自己的 `revision()` 相加）。
    rev: HashMap<String, u64>,
    /// 待检查语义的节点：dispatch 触及的 + Signal 失效的。
    pending_semantic: std::collections::HashSet<String>,
}

/// 一个活动中的动画项。
pub struct ActiveAnim {
    /// 属性标识（同一个 key 只能有一个动画）
    pub key: u64,
    /// 动画本体
    pub anim: crate::animation::Animate,
    /// 需要重绘的节点（`None` = 不标脏任何节点）
    pub node: Option<String>,
    /// 完成回调（只调一次）
    pub on_complete: Option<Box<dyn FnOnce() + Send>>,
}

impl UiTree {
    /// 启动一个动画。
    ///
    /// 输入：`key`（属性标识）、`anim`、`node`（需重绘的节点 id）。
    /// 行为：**同 key 的旧动画被立即取代**（不会两个同时写同一属性）。
    /// 副作用：若节点存在则标脏它。
    pub fn animate(
        &mut self,
        key: u64,
        anim: crate::animation::Animate,
        node: Option<&str>,
    ) -> &mut crate::animation::Animate {
        if let Some(pos) = self.animations.iter().position(|a| a.key == key) {
            self.animations.remove(pos);
        }
        self.animations.push(ActiveAnim {
            key,
            anim,
            node: node.map(|s| s.to_string()),
            on_complete: None,
        });
        if let Some(n) = node {
            self.invalidate(n);
        }
        let last = self.animations.len() - 1;
        &mut self.animations[last].anim
    }

    /// 推进所有动画一帧。
    ///
    /// 返回：`true` = 还有活动动画（主循环应保持快节奏）；
    /// `false` = 已全部完成（**主循环可以回到休眠节奏，不占 CPU**）。
    ///
    /// 副作用：每个未完成动画会标脏它的节点；完成项被移除并触发回调。
    pub fn tick_animations(&mut self, dt: std::time::Duration) -> bool {
        // 先推进所有动画（不碰 self 的其他字段，避免借用冲突）
        let mut dirty_nodes: Vec<String> = Vec::new();
        for a in self.animations.iter_mut() {
            let running = a.anim.tick(dt);
            if let Some(n) = a.node.clone() {
                dirty_nodes.push(n);
            }
            let _ = running;
        }
        // 再把完成的项搬出来（FnOnce 回调需要拥有权）
        let mut completed: Vec<ActiveAnim> = Vec::new();
        let mut kept: Vec<ActiveAnim> = Vec::new();
        for a in std::mem::take(&mut self.animations) {
            if a.anim.is_done() {
                completed.push(a);
            } else {
                kept.push(a);
            }
        }
        let still_running = !kept.is_empty();
        self.animations = kept;

        for n in dirty_nodes {
            self.invalidate(&n);
        }
        for mut a in completed {
            if let Some(n) = a.node.as_deref() {
                self.invalidate(n);
            }
            if let Some(cb) = a.on_complete.take() {
                cb();
            }
        }
        still_running
    }

    /// 挂一个"完成回调"到已存在的动画上。
    ///
    /// 输入：`key`、`cb`（动画完成时调一次）。
    /// 行为：若 key 不存在则**忽略**（回调丢失比 panic 安全）。
    pub fn set_animation_on_complete(&mut self, key: u64, cb: Box<dyn FnOnce() + Send>) {
        if let Some(a) = self.animations.iter_mut().find(|a| a.key == key) {
            a.on_complete = Some(cb);
        }
    }

    /// 某个 key 的动画当前值（`None` = 该属性没有动画）。
    pub fn animation_value(&self, key: u64) -> Option<f32> {
        self.animations
            .iter()
            .find(|a| a.key == key)
            .map(|a| a.anim.value())
    }

    /// 当前活动动画数量（诊断与测试用）。
    pub fn active_animation_count(&self) -> usize {
        self.animations.len()
    }

    /// 用根节点建一棵树。
    ///
    /// **副作用**：递归调用每个节点的 `on_mount`（Signal 的 `bind()` 靠它自动订阅），
    /// 并为整棵树分配初始 generation、登记待检查的语义基线。
    ///
    /// 失败：无。初始状态是"未布局"（`viewport == Size::ZERO`），
    /// 调用方必须随后调 `layout(viewport, metrics)`，否则 `rect_of` 全为空。
    pub fn new(root: Box<dyn Widget>) -> Self {
        let mut this = Self {
            animations: Vec::new(),
            pending_texture_releases: Vec::new(),
            scale: 1.0,
            root,
            popups: Vec::new(),
            focus: None,
            viewport: Size::ZERO,
            rects: HashMap::new(),
            pending: Vec::new(),
            cache: crate::layout::LayoutCache::new(),
            // 首帧必须整窗绘制
            dirty: DirtyRegion::full(),
            hovered: None,
            captured_pointer: None,
            paint_order: Vec::new(),
            modal_root: None,
            focus_path: None,
            captured_path: None,
            path_buf: Vec::new(),
            render_rects: Vec::new(),
            render_clips: Vec::new(),
            pending_ime: None,
            in_focus_change: false,
            focus_order: Vec::new(),
            subscriptions: Vec::new(),
            invalidation_queue: crate::signal::Invalidation::new(),
            external: crate::invalidate::InvalidationHandle::new(),
            layout_requested: false,
            broadcast_rects: Vec::new(),
            relayout_from: None,
            generations: HashMap::new(),
            next_generation: 1,
            last_semantic: HashMap::new(),
            rev: HashMap::new(),
            pending_semantic: std::collections::HashSet::new(),
        };

        // 初始树也要触发 on_mount：bind() 的 Signal 才能自动订阅
        let invalidation = this.invalidation_queue.clone();
        mount_recursive(this.root.as_mut(), &invalidation);
        // 给初始树分配 generation 并登记待建基线（首帧不 bump）
        let gen = this.next_generation;
        this.next_generation += 1;
        let ids = collect_ids(this.root.as_ref());
        for id in ids {
            this.generations.insert(id.clone(), gen);
            this.pending_semantic.insert(id);
        }
        this
    }

    /// 当前鼠标悬停的节点 id（最近一次 `MouseMove` 命中且未被移除的结果）。
    ///
    /// 无副作用；返回 `None` 表示没有悬停节点（指针在窗口外或整棵树都不命中）。
    pub fn hovered(&self) -> Option<&str> {
        self.hovered.as_deref()
    }

    /// 当前帧的脏区快照。
    ///
    /// 只读；`DirtyRegion::is_full()` 为真表示整窗都需要重绘。
    /// 脏区在 `render()` 结束后清空，所以渲染完成后再读一定是空的。
    pub fn dirty(&self) -> &DirtyRegion {
        &self.dirty
    }

    /// 把某个节点的矩形加入脏区（下一帧只重绘这块）。
    ///
    /// 输入：`id`。**必须已经布局过**（`rects` 里找不到就静默什么都不做）。
    /// 返回：无。
    /// 副作用：累加脏区；不影响布局。若该节点已关闭（不在索引里）同样无效——
    /// 关闭浮层的重绘应由 `EventResult::Relayout` 触发全窗重绘。
    pub fn invalidate(&mut self, id: &str) {
        if let Some(rect) = self.rects.get(&crate::layout::id_key(id)).copied() {
            self.dirty.add(rect);
        }
    }

    /// 整窗重绘：把脏区置为 `full`。
    ///
    /// 返回：无。副作用：下一帧会画所有 `needs_paint` 为真的节点。
    /// 用在哪：窗口尺寸变化、主题切换、关闭浮层（旧内容需要被覆盖）等
    /// "无法用矩形表达"的场景。不要每帧调用——它会让脏区剪枝完全失效。
    pub fn invalidate_all(&mut self) {
        self.dirty = DirtyRegion::full();
    }

    /// 浮层内部的可聚焦控件 id（前序，与树内 `rebuild_focus_order` 同一契约）。
    fn popup_focus_ids(popup: &Popup) -> Vec<String> {
        fn walk(node: &dyn Widget, out: &mut Vec<String>) {
            if node.is_open() && node.focusable() && !node.is_disabled() {
                out.push(node.id().to_string());
            }
            for c in node.children() {
                walk(c.as_ref(), out);
            }
        }
        let mut out = Vec::new();
        walk(popup.widget.as_ref(), &mut out);
        out
    }

    /// 某个 id 是否属于浯层（返回浯层下标）。
    fn popup_index_of(&self, id: &str) -> Option<usize> {
        fn contains(node: &dyn Widget, id: &str) -> bool {
            if node.id() == id {
                return true;
            }
            node.children().iter().any(|c| contains(c.as_ref(), id))
        }
        self.popups
            .iter()
            .position(|p| contains(p.widget.as_ref(), id))
    }

    /// 当前最上层的模态浯层（没有则 `None`）。
    ///
    /// 用途：调用方判断"现在是否处于模态态"（例如决定是否屏蔽快捷键）。
    pub fn modal_popup(&self) -> Option<&Popup> {
        self.popups.iter().rev().find(|p| p.modal)
    }

    /// 在模态浯层内循环焦点（Tab / Shift+Tab）。
    ///
    /// 返回：`true` = 已在浯层内移动焦点（上层应吃掉这次 Tab）。
    pub fn focus_next_in_modal(&mut self, delta: i32) -> bool {
        let Some(popup) = self.popups.iter().rev().find(|p| p.modal) else {
            return false;
        };
        let ids = Self::popup_focus_ids(popup);
        if ids.is_empty() {
            return false;
        }
        let cur = self
            .focus
            .as_deref()
            .and_then(|f| ids.iter().position(|i| i == f));
        let len = ids.len() as i32;
        let next = match cur {
            Some(i) => (i as i32 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        } as usize;
        let target = ids[next].clone();
        self.set_focus(Some(target))
    }

    /// 创建一个绑定到 `owner` 的状态：**改值就自动标脏那个控件**。
    ///
    /// 输入：`owner`（控件 id）、`initial`（初值）。
    /// 返回：[`State<T>`]（拿着它的人负责读/写）。
    /// 副作用：向树的失效队列注册订阅（`owner` 变更→ 下一帧标脏）。
    ///
    /// 与 `EventCtx::request_redraw()` 的分工：事件里用 `ctx`；
    /// 非事件途径（后台循环/定时器/回调）用 `State`。
    /// 拿一份失效队列句柄（给 `State::also_invalidate` 用）。
    ///
    /// 轻量 `Clone` 句柄（底层 `Arc<Mutex<..>>`），不持有树。
    pub fn invalidation_queue_clone(&self) -> crate::signal::Invalidation {
        self.invalidation_queue.clone()
    }

    /// 声明一段随树存活的状态，返回可读写的句柄。为什么不用 `Rc<RefCell>`：句柄变更会登记到树，树据此标脏；手写 `Rc` 就绕过了这套通知。
    pub fn use_state<T: Clone + PartialEq + 'static>(
        &mut self,
        owner: impl Into<String>,
        initial: T,
    ) -> crate::state::State<T> {
        crate::state::State::new(owner, initial, self.invalidation_queue.clone())
    }

    /// 把一个控件挂到 Overlay 层（屏幕最上层，不受父级裁切）。
    ///
    /// 输入：`widget`（id 必须树内唯一）、`anchor`（触发控件的屏幕矩形，
    /// 用于摆放：下方放不下会向上翻转）。
    /// 返回：`false` = 同 id 的浮层已存在（本次忽略）。
    /// 副作用：立刻重新布局（下一次 `layout` 会把它摆好）并标脏。
    pub fn open_popup(&mut self, widget: Box<dyn Widget>, anchor: Rect) -> bool {
        self.push_popup(widget, anchor, false)
    }

    /// 打开一个**模态**浮层：底层不接收事件，直到它被关闭。
    pub fn open_modal_popup(&mut self, widget: Box<dyn Widget>, anchor: Rect) -> bool {
        self.push_popup(widget, anchor, true)
    }

    fn push_popup(&mut self, widget: Box<dyn Widget>, anchor: Rect, modal: bool) -> bool {
        let id = widget.id().to_string();
        if self.popups.iter().any(|p| p.id == id) {
            return false;
        }
        // 记下弹出前的焦点：关闭时要还回去（否则键盘用户会"丢失焦点"）。
        let prev_focus = self.focus.clone();
        self.popups.push(Popup {
            id,
            widget,
            anchor,
            rect: Rect::ZERO,
            modal,
            prev_focus,
        });
        self.layout_requested = true;
        self.dirty = DirtyRegion::full();
        // 弹出层开启动画：150ms 缓动。
        // key 取该浮层的 id，于是"同一浮层重开"会打断上一次动画而不是叠加。
        // node 传 None：浮层不在普通节点索引里，靠整窗脏区重绘。
        let key =
            crate::animation::key_of(&self.popups.last().map(|p| p.id.clone()).unwrap_or_default());
        self.animate(
            key,
            crate::animation::Animate::new(
                0.0,
                1.0,
                std::time::Duration::from_millis(crate::animation::POPUP_OPEN_MS),
                crate::animation::Easing::EaseOutCubic,
            ),
            None,
        );
        true
    }

    /// 关闭指定浮层（不存在返回 `false`）。
    /// 取树内节点的可变引用（按具体类型）。
    ///
    /// 与 [`UiTree::popup_as_mut`] 同理：用于测试与诊断驱动具体控件；类型不对返回 `None`。
    pub fn node_as_mut<T: 'static>(&mut self, id: &str) -> Option<&mut T> {
        let path = self.find_path(id)?;
        let node = self.node_at_mut(&path)?;
        node.as_any_mut()?.downcast_mut::<T>()
    }

    /// 取浮层里控件的可变引用（按具体类型）。
    ///
    /// 返回：`None` = 浮层不存在、或类型不对（**不 panic**）。
    /// 用途：浮层不在主树上，测试与诊断需要拿到它才能驱动。
    pub fn popup_as_mut<T: 'static>(&mut self, id: &str) -> Option<&mut T> {
        let popup = self.popups.iter_mut().find(|p| p.id() == id)?;
        popup.widget.as_any_mut()?.downcast_mut::<T>()
    }

    /// 关闭指定浮层；不存在返回 `false`（不 panic）。控件应通过 `EventCtx::request_close_popup` 调用，让关闭发生在分发结束后。
    pub fn close_popup(&mut self, id: &str) -> bool {
        let Some(pos) = self.popups.iter().position(|p| p.id() == id) else {
            return false;
        };
        let rect = self.popups[pos].rect;
        let prev = self.popups[pos].prev_focus.clone();
        self.popups.remove(pos);
        self.restore_focus(prev);
        if !rect.is_empty() {
            self.dirty.add(rect);
        }
        true
    }

    /// 关闭全部浮层，返回关了几个。
    pub fn close_all_popups(&mut self) -> usize {
        let n = self.popups.len();
        // 最早弹出的那个记录了"弹出前的焦点"，它才是要还回去的目标。
        let restore = self.popups.first().and_then(|p| p.prev_focus.clone());
        for p in self.popups.drain(..) {
            if !p.rect.is_empty() {
                self.dirty.add(p.rect);
            }
        }
        self.restore_focus(restore);
        n
    }

    /// 关闭浮层后把焦点还给弹出前的控件（已不存在则不动）。
    fn restore_focus(&mut self, prev: Option<String>) {
        let Some(id) = prev else {
            return;
        };
        if self.focus.as_deref() == Some(id.as_str()) {
            return;
        }
        // 目标还在树里且仍可聚焦才恢复
        if self.find_path(&id).is_some() {
            self.set_focus(Some(id));
        }
    }

    /// 是否有浮层打开。
    pub fn has_popup(&self) -> bool {
        !self.popups.is_empty()
    }

    /// 浮层列表（按开启顺序）。
    pub fn popups(&self) -> &[Popup] {
        &self.popups
    }

    /// 某个浮层的屏幕矩形（布局后有效）。
    pub fn popup_rect(&self, id: &str) -> Option<Rect> {
        self.popups.iter().find(|p| p.id() == id).map(|p| p.rect)
    }

    /// 抉一个浮层内部控件的可变引用（改内容后记得标脏）。
    pub fn with_popup_mut(&mut self, id: &str, f: &mut dyn FnMut(&mut dyn Widget)) -> bool {
        match self.popups.iter_mut().find(|p| p.id() == id) {
            Some(p) => {
                f(p.widget.as_mut());
                true
            }
            None => false,
        }
    }

    /// 按**像素矩形**标脏（不查树、不找节点）。
    ///
    /// 输入：`rect`（屏幕坐标，会被裁到视口内）。
    /// 副作用：并入脏区；空矩形或完全在视口外时什么也不做。
    /// 用在：流式输出、光标闪烁、单行刷新——
    /// 调用方已知那块区域需要重画，不必为了拿到矩形去遍历树。
    /// 注意：它**不会**触发重排；尺寸变了请另外 `mark_layout_dirty(id)`。
    pub fn invalidate_rect(&mut self, rect: Rect) {
        if rect.is_empty() {
            return;
        }
        let window = Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h);
        let clamped = rect.intersect(window);
        if clamped.is_empty() {
            return;
        }
        self.dirty.add(clamped);
    }

    /// 标记某节点的尺寸可能变化（等价于组件返回 `EventResult::Relayout` 的效果）。
    ///
    /// 输入：`id`。不存在的 id 会被忽略（`cache.mark_dirty` 内部处理）。
    /// 返回：无。
    /// 副作用：只标布局失效，**不**标脏区——下一帧 `layout()` 会重算，
    /// 但要在 `layout` 之后再调 `invalidate`/`invalidate_all` 才会重绘。
    pub fn mark_layout_dirty(&mut self, id: &str) {
        self.cache.mark_dirty(id);
    }

    /// 上一次 `layout()` 的统计（测量次数 / arrange 次数 / 是否被整体跳过）。
    ///
    /// 只读、无副作用，用于诊断"缓存是否生效"。
    /// 注意它反映的是**最近一次** `layout()`；从未布局过时各计数为 0、`skipped` 为 false。
    pub fn layout_stats(&self) -> crate::layout::LayoutStats {
        self.cache.stats()
    }

    /// 根节点（只读）。根节点的 `rect` 即整窗。
    ///
    /// 无副作用；仓库里不存在"没有根"的状态，因此调用方不需要判空。
    pub fn root(&self) -> &dyn Widget {
        self.root.as_ref()
    }

    /// 根节点（可变）。
    ///
    /// **危险**：绕过框架直接改根节点不会触发布局失效/索引重建。
    /// 只给测试与"换整棵树"的场景用；正常改结构请用 `add_node` / `remove_node`。
    pub fn root_mut(&mut self) -> &mut dyn Widget {
        self.root.as_mut()
    }

    /// 上一次布局使用的视口尺寸（逻辑像素）。
    ///
    /// 无副作用；首次布局前是 `Size::ZERO`。
    pub fn viewport(&self) -> Size {
        self.viewport
    }

    /// 当前持有键盘焦点的节点 id。
    ///
    /// 无副作用。这是**框架焦点**的权威来源；组件内部的 `focused` 字段只是镜像。
    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// 路径上所有祖先（含根、含自身）是否都处于 open 状态。
    ///
    /// 输入：从根开始的子节点下标路径（`find_path` 的返回值）。
    /// 返回：`true` = 整条路径可见；`false` = 路径越界或任一节点 `is_open() == false`。
    /// 失败：路径非法时返回 `false`（不 panic）。
    ///
    /// 用途：`set_focus` 用它拒绝"把焦点送进关闭子树"——与 `rebuild_indices` /
    /// `render_pass` / `cursor_at` / `rebuild_focus_order` 的 `is_open` 跳过保持一致。
    fn path_is_open(&self, path: &[usize]) -> bool {
        let mut node = self.root.as_ref();
        if !node.is_open() {
            return false;
        }
        for &i in path {
            let Some(child) = node.children().get(i) else {
                return false;
            };
            if !child.is_open() {
                return false;
            }
            node = child.as_ref();
        }
        true
    }

    /// 切换焦点。事件走完整的三阶段路由，因此可以被"焦点守卫"否决。
    ///
    /// - `Focus(false)` 先发给旧节点：它在 Capture/Target 里调 `ctx.veto_focus()`
    ///   就能阻止焦点离开（例如表单未填完）；
    /// - `Focus(true)` 再发给新节点，同样可否决（例如目标暂不可用）。
    ///
    /// Modal 打开时，焦点被限制在 Modal 子树内。
    pub fn set_focus(&mut self, id: Option<String>) -> bool {
        if self.in_focus_change {
            return false; // 焦点事件里再改焦点：直接拒绝，防重入
        }
        if let Some(target) = &id {
            // 浯层内的控件不在树里（不参与 `find_path`）：单独走一条路。
            if self.find_path(target).is_none() {
                let Some(idx) = self.popup_index_of(target) else {
                    return false;
                };
                let Some(node) = find_in(self.popups[idx].widget.as_ref(), target) else {
                    return false;
                };
                if !node.focusable() || node.is_disabled() {
                    return false;
                }
                return self.apply_focus_change(id);
            }
            let Some(path) = self.find_path(target) else {
                return false;
            };
            if let Some(scope) = self.modal_root.as_ref() {
                if !path.starts_with(scope.as_slice()) {
                    return false;
                }
            }
            // 条件 3：路径上所有祖先（含自身）必须 open。
            // 否则焦点会被送进"关闭的 Dialog"里——画不出来、点不到，却能收键盘。
            if !self.path_is_open(&path) {
                return false;
            }
            // 条件 4：目标必须可聚焦且未禁用（与 focus_order 的收集条件一致）。
            let Some(widget) = self.node_at_ref(&path) else {
                return false;
            };
            if !widget.focusable() || widget.is_disabled() {
                return false;
            }
        }
        self.apply_focus_change(id)
    }

    /// 真正改焦点：给旧节点发 `Focus(false)`、给新节点发 `Focus(true)`。
    ///
    /// 输入：目标 id（`None` = 取消焦点）。返回：是否真的改成了。
    fn apply_focus_change(&mut self, id: Option<String>) -> bool {
        if self.focus == id {
            return true;
        }

        self.in_focus_change = true;
        let mut allowed = true;

        if let Some(old) = self.focus.clone() {
            allowed = self.send_focus(&old, false);
        }
        if allowed {
            if let Some(new) = id.clone() {
                allowed = self.send_focus(&new, true);
            }
        }

        self.in_focus_change = false;

        if !allowed {
            return false;
        }

        self.focus = id;
        self.focus_path = self.focus.as_deref().and_then(|f| self.find_path(f));
        true
    }

    /// 把一条 `Focus` 事件沿路径做三阶段派发。返回 false 表示被否决。
    fn send_focus(&mut self, id: &str, focused: bool) -> bool {
        let Some(path) = self.find_path(id) else {
            return true;
        };
        let viewport = self.viewport;
        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
        ctx.scale = self.scale;
        ctx.target = id.to_string();

        let _ = dispatch_rec(
            self.root.as_mut(),
            &path,
            &mut ctx,
            &UiEvent::Focus(focused),
        );

        let vetoed = ctx.focus_vetoed;
        let _ = self.finish_dispatch(ctx, EventResult::Ignored);
        !vetoed
    }

    // ---------- 动态挂载 / 卸载 ----------

    /// 响应式失效队列（组件在 on_mount 里会拿到它）。
    pub fn invalidation(&self) -> crate::signal::Invalidation {
        self.invalidation_queue.clone()
    }

    /// 取一个**线程安全**的标脏句柄，交给后台任务。
    ///
    /// 返回：`Send + Sync` 的句柄。后台线程调用 `invalidate(id)` /
    /// `invalidate_all()` 之后，UI 线程下一帧
    /// [`UiTree::drain_external_invalidations`] 会把它合并进脏区。
    /// 副作用：无（只是 `Arc` 克隆，不借用树）。
    ///
    /// **它只通知"有变化"，不是线程安全的状态容器**：后台线程不能借此直接改组件字段，
    /// 跨线程数据必须由应用自己用 `Arc<Mutex<…>>` 管理，在 UI 线程 drain 后再写进组件。
    /// 组件的重绘请求仍走 [`crate::EventResult`]。
    pub fn invalidation_handle(&self) -> crate::invalidate::InvalidationHandle {
        self.external.clone()
    }

    /// 把后台线程的标脏请求合并进当前脏区（**框架内部接口**）。
    ///
    /// 返回：本次处理的请求数（0 = 没有后台请求）。
    /// 规则：`One(id)` → 该节点布局失效 + 标脏 + 登记语义检查；
    /// 只要出现一条 `All` → 整窗脏区（合并取最强，不会漏块）。
    ///
    /// **必须在 `layout()` 之前调用**（主循环在 `window.resize` 之前 drain），
    /// 否则本帧按旧脏区布局、下一帧才重排。
    /// 注意：与 `UiTree::invalidate(id)` 一样，id 不存在时静默忽略。
    #[doc(hidden)]
    pub fn drain_external_invalidations(&mut self) -> usize {
        let requests = self.external.drain();
        if requests.is_empty() {
            return 0;
        }
        let mut count = 0;
        let mut whole_window = false;
        for request in requests {
            count += 1;
            match request {
                crate::invalidate::PendingInvalidation::One(id) => {
                    self.cache.mark_dirty(&id);
                    self.invalidate(&id);
                    self.pending_semantic.insert(id);
                }
                crate::invalidate::PendingInvalidation::All => whole_window = true,
            }
        }
        // 有请求就一定重绘：`invalidate(id)` 只覆盖单个矩形，但后台任务改的
        // 往往是"看不见的内容"（例如列表里的数据），整窗重绘最稳妥。
        let _ = whole_window;
        self.dirty = DirtyRegion::full();
        count
    }

    /// 取走"需要重新 layout"的请求（主循环在 layout 之前调用）。
    ///
    /// 返回：`true` = 自上次 take 以来发生过**结构性变化**
    /// （`add_node` / `insert_node` / `remove_node` 成功），本帧需要 layout。
    /// 副作用：清零标志（一次只消费一次）。
    ///
    /// 为什么需要它：主循环只在 `need_layout`（来自 `EventResult::Relayout` 或窗口尺寸
    /// 变化）时调 layout。没有这个标志，Host 在 `command()` 里 `add_node` 之后
    /// 这一帧不会重排——新节点会带着旧矩形被画出来，直到下一次窗口 resize。
    #[doc(hidden)]
    pub fn take_layout_request(&mut self) -> bool {
        std::mem::take(&mut self.layout_requested)
    }

    /// 把 parent 挂到一个新子节点下。返回是否成功。
    ///
    /// 会：调用 `on_mount` → 标记父节点布局失效 → 整窗重绘。
    pub fn add_node(&mut self, parent: &str, child: Box<dyn Widget>) -> bool {
        // 重复 id 会让 find/dispatch_to 命中第一个，必须拒绝而不是静默接受
        if self.subtree_has_duplicate(child.as_ref()) {
            eprintln!("[y901] add_node: duplicate id `{}` rejected", child.id());
            return false;
        }
        let Some(node) = self.find_mut(parent) else {
            return false;
        };
        let Some(children) = node.children_mut() else {
            return false;
        };
        children.push(child);

        // 新子树整体挂载（订阅 Signal 等），不只是顶层节点
        let invalidation = self.invalidation_queue.clone();
        if let Some(children) = self.find_mut(parent).and_then(|n| n.children_mut()) {
            if let Some(mounted) = children.last_mut() {
                mount_recursive(mounted.as_mut(), &invalidation);
            }
        }

        self.register_new_subtree(parent, true);
        self.cache.mark_dirty(parent);
        self.dirty = DirtyRegion::full();
        self.layout_requested = true;
        true
    }

    /// 给刚挂上的子树分配 generation 并登记待建基线（首帧不 bump）。
    ///
    /// `last = true` 取最后一个子节点（add_node）；否则需要调用方给定索引。
    fn register_new_subtree(&mut self, parent: &str, last: bool) {
        let ids = {
            let Some(node) = self.find(parent) else {
                return;
            };
            let child = if last { node.children().last() } else { None };
            match child {
                Some(c) => collect_ids(c.as_ref()),
                None => return,
            }
        };
        let gen = self.next_generation;
        self.next_generation += 1;
        for id in ids {
            self.generations.insert(id.clone(), gen);
            self.pending_semantic.insert(id);
        }
    }

    /// 在指定位置插入子节点（越界自动 clamp 到末尾）。
    ///
    /// 与 `add_node` 的差别只有位置；生命周期与重建逻辑完全一致。
    pub fn insert_node(&mut self, parent: &str, index: usize, child: Box<dyn Widget>) -> bool {
        if self.subtree_has_duplicate(child.as_ref()) {
            eprintln!("[y901] insert_node: duplicate id `{}` rejected", child.id());
            return false;
        }
        let Some(node) = self.find_mut(parent) else {
            return false;
        };
        let Some(children) = node.children_mut() else {
            return false;
        };
        let at = index.min(children.len());
        children.insert(at, child);

        let invalidation = self.invalidation_queue.clone();
        if let Some(children) = self.find_mut(parent).and_then(|n| n.children_mut()) {
            if let Some(mounted) = children.get_mut(at) {
                mount_recursive(mounted.as_mut(), &invalidation);
            }
        }

        self.register_new_subtree_at(parent, at);
        self.cache.mark_dirty(parent);
        self.dirty = DirtyRegion::full();
        self.layout_requested = true;
        true
    }

    /// 给插入位置 `at` 上的新子树分配 generation。
    fn register_new_subtree_at(&mut self, parent: &str, at: usize) {
        let ids = {
            let Some(node) = self.find(parent) else {
                return;
            };
            match node.children().get(at) {
                Some(c) => collect_ids(c.as_ref()),
                None => return,
            }
        };
        let gen = self.next_generation;
        self.next_generation += 1;
        for id in ids {
            self.generations.insert(id.clone(), gen);
            self.pending_semantic.insert(id);
        }
    }
    fn subtree_has_duplicate(&self, node: &dyn Widget) -> bool {
        if self.find(node.id()).is_some() {
            return true;
        }
        node.children()
            .iter()
            .any(|c| self.subtree_has_duplicate(c.as_ref()))
    }
    /// 从树上摘掉一个节点（连同子树），返回是否成功。
    ///
    /// 会：调用 `on_unmount`（递归）→ 清理焦点/捕获 → 标记父节点失效。
    /// 把一张纹理排入"下一帧释放"队列。
    ///
    /// 为什么需要公开：卸载路径不只 `remove_node`（整树替换、窗口关闭等），
    /// 而后端取得纹理的时机只在渲染开头。
    pub fn queue_texture_release(&mut self, id: crate::image::TextureId) {
        if id.is_valid() {
            self.pending_texture_releases.push(id);
        }
    }

    /// 移除节点及其子树。返回 `false` 表示根节点或节点不存在。注意：会触发卸载钩子（`release_textures` / `on_unmount`），纹理因此被回收。
    pub fn remove_node(&mut self, id: &str) -> bool {
        if id == self.root.id() {
            return false; // 根节点不可摘
        }
        let Some(path) = self.find_path(id) else {
            return false;
        };
        if path.is_empty() {
            return false;
        }

        let parent_path = path[..path.len() - 1].to_vec();
        let index = path[path.len() - 1];

        let removed = {
            let Some(parent) = self.node_at_mut(&parent_path) else {
                return false;
            };
            let Some(children) = parent.children_mut() else {
                return false;
            };
            if index >= children.len() {
                return false;
            }
            children.remove(index)
        };

        // 卸载钩子（整棵被摘掉的子树）
        // 同 id 将来若被复用，不能继承旧实例的哈希快照（否则会假 bump）
        forget_recursive(removed.as_ref(), self);
        let mut releases = Vec::new();
        unmount_recursive(removed, &mut releases);
        self.pending_texture_releases.extend(releases);

        let parent_id = self
            .node_at_ref(&parent_path)
            .map(|n| n.id().to_string())
            .unwrap_or_default();

        // 焦点/捕获若落在被删子树里，就地清空（否则悬空）
        let inside = |p: &Vec<usize>| p.starts_with(&path);
        if self.focus_path.as_ref().map(inside).unwrap_or(false) {
            self.focus = None;
            self.focus_path = None;
        }
        if self.captured_path.as_ref().map(inside).unwrap_or(false) {
            self.captured_pointer = None;
            self.captured_path = None;
        }
        if self.hovered.as_deref() == Some(id) {
            self.hovered = None;
        }

        self.cache.mark_dirty(&parent_id);
        self.dirty = DirtyRegion::full();
        self.layout_requested = true;
        true
    }

    /// 排空响应式失效队列：把这些节点标脏并重算布局。
    pub fn drain_invalidation(&mut self) -> usize {
        let ids = self.invalidation_queue.drain();
        if ids.is_empty() {
            return 0;
        }
        let mut need_full = false;
        for id in &ids {
            self.cache.mark_dirty(id);
            match self.rects.get(&crate::layout::id_key(id)).copied() {
                // 局部标脏：只加该节点的矩形。
                // 之前这里无条件 `dirty = full()`，把"响应式更新"变成了"整窗重绘"——
                // 那是为了兼容"变值可能带来重排"的保守做法。
                // 重排场景不需要它：布局阶段的 `mark_moved` 会把新旧矩形都并进脏区。
                Some(r) if !r.is_empty() => self.dirty.add(r),
                // 拿不到矩形（尚未布局 / 已删除）：只能整窗
                _ => need_full = true,
            }
            // Signal 驱动的变化不经 dispatch，必须在这里登记语义检查
            self.pending_semantic.insert(id.clone());
        }
        if need_full {
            self.dirty = DirtyRegion::full();
        }
        ids.len()
    }

    // ---------- 语义 revision（方案 1'） ----------

    /// 节点对外部工具的"有效版本"：组件自己的 revision + 框架计数器。
    ///
    /// 输入：节点 id。失败：id 不存在时返回 0（不 panic）。
    ///
    /// **注意复杂度**：内部调 `find(id)`，是 O(n)。不要在遍历中对每个节点调用，
    /// 那会退化成 O(n²)；遍历场景请用 [`UiTree::rev_snapshot`]。
    pub fn effective_revision(&self, id: &str) -> u64 {
        let own = self.find(id).map(|n| n.revision()).unwrap_or(0);
        own + self.rev.get(id).copied().unwrap_or(0)
    }

    /// 一次遍历构建 `id -> effective_revision` 快照，**总成本 O(n)**。
    ///
    /// 用于网关输出整棵树（`public_tree` / `query_result`）：
    /// 这些路径要对每个节点取有效 revision，逐个调 `effective_revision` 就是 O(n²)。
    ///
    /// 失败：无（空树返回空 map）。
    pub fn rev_snapshot(&self) -> HashMap<String, u64> {
        fn walk(
            node: &dyn Widget,
            counters: &HashMap<String, u64>,
            out: &mut HashMap<String, u64>,
        ) {
            let effective = node.revision() + counters.get(node.id()).copied().unwrap_or(0);
            out.insert(node.id().to_string(), effective);
            for child in node.children() {
                walk(child.as_ref(), counters, out);
            }
        }

        let mut out = HashMap::new();
        walk(self.root.as_ref(), &self.rev, &mut out);
        out
    }

    /// 检查待定节点的语义面是否变化，变化则 bump。
    ///
    /// 幂等：没有待定节点时立即返回。失败：id 已不在树上时跳过。
    pub fn refresh_semantics(&mut self) {
        if self.pending_semantic.is_empty() {
            return;
        }
        let ids: Vec<String> = self.pending_semantic.drain().collect();
        for id in ids {
            // 先结束不可变借用，再写 map
            let Some((hash, gen)) = self.find(&id).map(|node| {
                let focused = self.focus.as_deref() == Some(id.as_str());
                (
                    semantic_hash(node, focused),
                    self.generations.get(&id).copied().unwrap_or(0),
                )
            }) else {
                continue;
            };
            match self.last_semantic.get(&id).copied() {
                // 同一实例、语义面变了 → bump
                Some((g, h)) if g == gen && h != hash => {
                    self.last_semantic.insert(id.clone(), (gen, hash));
                    *self.rev.entry(id).or_insert(0) += 1;
                }
                // 同一实例、语义面未变 → 什么都不做（hover 走这条）
                Some((g, _)) if g == gen => {}
                // 新实例（generation 不同）或无记录 → 只记基线，不 bump
                _ => {
                    self.last_semantic.insert(id, (gen, hash));
                }
            }
        }
    }

    /// 取走组件请求的 IME 候选窗位置（应用层每帧调用一次）。
    pub fn take_pending_ime(&mut self) -> Option<Pos> {
        self.pending_ime.take()
    }

    /// Tab 遍历：把焦点移到下一个（delta = ±1）可聚焦节点，边界处循环。
    pub fn focus_next(&mut self, delta: i32) -> bool {
        if self.focus_order.is_empty() {
            return false;
        }
        let cur = self
            .focus_path
            .as_ref()
            .and_then(|p| self.focus_order.iter().position(|q| q == p));

        let len = self.focus_order.len() as i32;
        let next = match cur {
            Some(i) => (i as i32 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        } as usize;

        let path = self.focus_order[next].clone();
        let Some(id) = self.node_at_ref(&path).map(|n| n.id().to_string()) else {
            return false;
        };
        self.set_focus(Some(id))
    }

    /// 当前 Tab 顺序（测试 / 调试用）。
    pub fn focus_order_len(&self) -> usize {
        self.focus_order.len()
    }

    // ---------- 语义订阅（RPC 增量推送） ----------

    /// 订阅某个节点的状态变化。
    pub fn subscribe(&mut self, id: &str, interval: std::time::Duration) -> bool {
        let Some(sem) = self.semantics_with_focus(id) else {
            return false;
        };
        let rev = self.fingerprint_of(id).unwrap_or(0);

        if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
            sub.rev = rev;
            sub.last_sem = sem;
            sub.interval = interval;
            sub.next_due = std::time::Instant::now() + interval;
            return true;
        }

        self.subscriptions.push(Subscription {
            id: id.to_string(),
            rev,
            last_sem: sem,
            interval,
            next_due: std::time::Instant::now() + interval,
        });
        true
    }

    /// 取节点语义并把 `focused` 注入为框架真值。
    fn semantics_with_focus(&self, id: &str) -> Option<crate::widget::Semantics> {
        let node = self.find(id)?;
        let mut sem = node.semantics();
        sem.focused = self.focus.as_deref() == Some(id);
        Some(sem)
    }

    /// 节点的语义指纹（含焦点与组件 revision）。
    fn fingerprint_of(&self, id: &str) -> Option<u64> {
        let node = self.find(id)?;
        let focused = self.focus.as_deref() == Some(id);
        Some(fingerprint(node, focused))
    }

    /// 取消订阅某个节点的 `state_changed` 推送。
    ///
    /// 返回：**之前是否存在**该订阅（`false` 表示本来就没订阅，幂等）。
    /// 失败：无；对不存在的 id 也返回 `false` 而不是错误。
    pub fn unsubscribe(&mut self, id: &str) -> bool {
        let before = self.subscriptions.len();
        self.subscriptions.retain(|s| s.id != id);
        self.subscriptions.len() != before
    }

    /// 当前活跃的订阅数（诊断用）。
    ///
    /// 只读。订阅在 `remove_node` 删除目标节点时**不会立即**清理；
    /// 它们会在**下一次 `poll_subscriptions`** 被剔除（懒清理）。
    /// 因此在两次 poll 之间，本计数可能短暂包含已失效的条目。
    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// 收集到期的状态变化，产出 `state_changed` 事件（**差分 + 节流**）。
    ///
    /// - 差分：`patch` 只包含变化的字段；
    /// - 节流：每个订阅有 `interval`，到期才检查一次，窗口内的多次变化合并成一次。
    pub fn poll_subscriptions(&mut self, now: std::time::Instant) -> Vec<crate::json::Json> {
        use crate::json::Json;

        // **懒清理**：先剔除"目标节点已不存在"的订阅。
        //
        // 为何在这里而不是 `remove_node`：`remove_node` 碎树时只知道"被挂掉的子树根"，
        // 而订阅的 key 是**节点 id**（任意层级）—— 要在那里清就得再遍历一次子树。
        // 而 `poll_subscriptions` 本来就要逐个查节点，顺带判存在性**零额外遍历**。
        //
        // 为何用 `mem::take`：`retain` 的闭包需要 `&self`，而字段又要 `&mut`。
        // 先把 Vec 整个取出（`Vec` 只是移走指针，零成本），筛完放回。
        let subs = std::mem::take(&mut self.subscriptions);
        let mut alive = Vec::with_capacity(subs.len());
        let mut dropped = 0usize;
        for sub in subs {
            // **用 `find_path` 而不是 `find`**：`find` 走的是"上次 layout 建立"的节点索引，
            // 而 `remove_node` 只把节点从父节点摘掉、并不立即重建索引 ——
            // 于是"已删节点"仍会被 `find` 找到，订阅永远清不掉（实测）。
            // `find_path` 直接遍历**活的控件树**，是"节点此刻是否存在"的可靠判据。
            if self.find_path(&sub.id).is_some() {
                alive.push(sub);
            } else {
                dropped += 1;
            }
        }
        self.subscriptions = alive;
        if dropped > 0 && std::env::var_os("Y901_SUB_TRACE").is_some() {
            eprintln!("[y901] 清理了 {dropped} 条已失效订阅");
        }

        let mut out = Vec::new();
        let ids: Vec<String> = self.subscriptions.iter().map(|s| s.id.clone()).collect();

        for id in ids {
            // 先取出到期信息，避免同时持有 &mut subscriptions 和 &self
            let due = match self.subscriptions.iter().find(|s| s.id == id) {
                Some(sub) => sub.next_due,
                None => continue,
            };
            if now < due {
                continue;
            }

            let Some(sem) = self.semantics_with_focus(&id) else {
                continue;
            };
            let rev = self.fingerprint_of(&id).unwrap_or(0);
            let (last_rev, last_sem, interval) =
                match self.subscriptions.iter().find(|s| s.id == id) {
                    Some(sub) => (sub.rev, sub.last_sem.clone(), sub.interval),
                    None => continue,
                };

            // 语义面没变：只顺延下次检查时间
            if rev == last_rev {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    sub.next_due = now + interval;
                }
                continue;
            }

            let patch = semantic_patch(&last_sem, &sem);

            if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                sub.rev = rev;
                sub.last_sem = sem;
                sub.next_due = now + interval;
            }

            if patch_is_empty(&patch) {
                continue;
            }

            out.push(
                Json::obj()
                    .set("event", "state_changed")
                    .set("id", id.as_str())
                    .set("rev", rev)
                    .set("patch", patch),
            );
        }

        out
    }
    /// Modal 打开时，若当前焦点不在作用域内，自动聚焦第一个可聚焦节点。
    fn enforce_modal_focus(&mut self) {
        let Some(scope) = self.modal_root.clone() else {
            return;
        };
        let inside = self
            .focus_path
            .as_ref()
            .map(|p| p.starts_with(scope.as_slice()))
            .unwrap_or(false);
        if inside {
            return;
        }

        if let Some(path) = self.first_focusable_in(&scope, true) {
            if let Some(id) = self.node_at_ref(&path).map(|n| n.id().to_string()) {
                let _ = self.set_focus(Some(id));
            }
        }
    }

    /// 在 `scope` 子树里找第一个 `focusable()` 的节点路径。
    fn first_focusable_in(&self, scope: &[usize], use_scope: bool) -> Option<Vec<usize>> {
        fn walk(node: &dyn Widget, path: &mut Vec<usize>) -> Option<Vec<usize>> {
            if node.focusable() {
                return Some(path.clone());
            }
            for (i, child) in node.children().iter().enumerate() {
                path.push(i);
                if let Some(found) = walk(child.as_ref(), path) {
                    return Some(found);
                }
                path.pop();
            }
            None
        }

        let start = self.node_at_ref(scope)?;
        let mut path = if use_scope {
            scope.to_vec()
        } else {
            Vec::new()
        };
        walk(start, &mut path)
    }
    /// 整树布局。
    ///
    /// 约束与所有节点的 `layout_version` 都没变时，这次调用是 O(1)：
    /// 直接返回，不进入任何 `measure`。
    pub fn layout(&mut self, viewport: Size, metrics: &dyn TextMetrics) {
        // **DPI 缩放源头**：事件分发时拿不到 `metrics`，所以在这里存下。
        self.scale = metrics.scale();
        // 响应式：Signal 变过的节点先标脏，再进布局
        self.drain_invalidation();
        // Signal 与 dispatch 触及的节点在此统一比对语义面（方案 1'，一帧一次）
        self.refresh_semantics();
        let viewport_changed = self.viewport != viewport;
        if viewport_changed {
            self.cache.mark_dirty(self.root.id());
            self.dirty = DirtyRegion::full();
        }
        self.viewport = viewport;
        // 布局前快照：控件收缩/内容变短时，**旧 rect 也要重绘**，
        // 否则旧内容的残影会留在屏幕上（局部重绘下才暴露）。
        // 视口变化时整窗已经是脏的，不需要快照。
        let before = if viewport_changed {
            Vec::new()
        } else {
            self.snapshot_rects()
        };
        crate::layout::layout_tree(self.root.as_mut(), viewport, metrics, &mut self.cache);

        // 浮层摆放：先量尺寸（浮层自己决定内容高），再用 `place_popup`
        // 把它放在锚点旁且不超出视口。绘制与命中都用这个 rect。
        let bounds = Rect::new(0.0, 0.0, viewport.w, viewport.h);
        for popup in self.popups.iter_mut() {
            let size = popup.widget.measure(
                metrics,
                crate::Constraints::loose(viewport),
                &mut self.cache,
            );
            let rect = crate::place_popup(popup.anchor, size, bounds);
            popup.rect = rect;
            popup.widget.arrange(rect, metrics, &mut self.cache);
        }
        if !self.layout_stats().skipped {
            self.rebuild_indices();
            if !before.is_empty() {
                self.mark_moved(&before);
            }
            self.enforce_modal_focus();
        }
    }

    /// 快照当前每个节点的 rect（布局前调用）。
    ///
    /// 返回：`(id, rect)` 列表；顺序无所谓（后面按 id 查）。
    /// 开销：O(n) 次 String clone——只在**非视口变化**的重排里做，
    /// 视口变化（拉伸）走整窗脏区，不需要它。
    fn snapshot_rects(&self) -> Vec<(String, Rect)> {
        fn walk(node: &dyn Widget, out: &mut Vec<(String, Rect)>) {
            out.push((node.id().to_string(), node.rect()));
            if !node.is_open() {
                return;
            }
            for child in node.children() {
                walk(child.as_ref(), out);
            }
        }
        let mut out = Vec::new();
        walk(self.root.as_ref(), &mut out);
        out
    }

    /// 对比布局前后的 rect：变了就标记 `旧 ∪ 新`。
    ///
    /// 输入：布局前的快照。副作用：往 `dirty` 里加并集矩形。
    /// 为什么是并集：只标新区域会把"原来那块"留在屏幕上（残影）。
    fn mark_moved(&mut self, before: &[(String, Rect)]) {
        for (id, old) in before {
            let Some(new) = self.rects.get(&crate::layout::id_key(id)).copied() else {
                // 节点消失了（弹窗关闭、列表删行、子树被隐藏）：
                // **旧矩形必须重绘**，否则原来的内容留在屏幕上（弹窗残影）。
                self.dirty.add(*old);
                continue;
            };
            if new != *old {
                self.dirty.add(old.union(new));
            }
        }
    }

    /// 布局后重建两套索引：id → rect，以及绘制/命中顺序。
    fn rebuild_indices(&mut self) {
        self.rects.clear();
        self.paint_order.clear();
        self.modal_root = None;

        // 把四个可变的输出目标打包：既避免 8 参数，也明确"它们是一起写的"
        struct Outputs<'a> {
            rects: &'a mut HashMap<u64, Rect>,
            order: &'a mut Vec<PaintEntry>,
            modal_root: &'a mut Option<Vec<usize>>,
        }

        /// 前序遍历建索引：关闭的子树整棵跳过；clip 沿祖先累积。
        fn walk(
            node: &dyn Widget,
            path: &mut Vec<usize>,
            parent_entry: Option<usize>,
            inherited_clip: Option<Rect>,
            out: &mut Outputs<'_>,
        ) {
            // 关闭的浮层整棵子树都不进索引（不画、不命中、不可聚焦）
            if !node.is_open() {
                return;
            }
            let rect = node.rect();
            // 可见区 = 自身 rect ∩ 所有祖先 clip（滚动容器靠它约束命中与绘制）
            let clip = match (inherited_clip, node.clip()) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            // 优先用组件预算好的 key（热路径组件都缓存了一份），
            // 没缓存的才现场哈希字符串。
            let key = node
                .cache_key()
                .unwrap_or_else(|| crate::layout::id_key(node.id()));
            out.rects.insert(key, rect);
            // 自己在 `order` 里的下标：子节点要用它当 parent
            let me = out.order.len();

            // 模态作用域由 `Widget::is_modal()` 宣言：
            // "画在哪层"与"是否阻塞底层"是两件事，不再混为一谈。
            if node.is_modal() {
                *out.modal_root = Some(path.clone());
            }

            out.order.push(PaintEntry {
                parent: parent_entry,
                slot: *path.last().unwrap_or(&0),
                rect,
                clip,
            });

            for (i, child) in node.children().iter().enumerate() {
                path.push(i);
                walk(child.as_ref(), path, Some(me), clip, out);
                path.pop();
            }
        }

        let mut path = Vec::new();
        let mut outputs = Outputs {
            rects: &mut self.rects,
            order: &mut self.paint_order,
            modal_root: &mut self.modal_root,
        };
        walk(self.root.as_ref(), &mut path, None, None, &mut outputs);

        // 稳定排序：同层内保持树序（即绘制先后）

        self.rebuild_focus_order();
    }

    /// 前序遍历一遍，收集所有 focusable 节点；Modal 存在时只看它子树。
    fn rebuild_focus_order(&mut self) {
        fn walk(node: &dyn Widget, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
            // 关闭节点整棵子树跳过：与 `rebuild_indices` / `render_pass` / `cursor_at`
            // 完全同构。少这一条会让"关闭的 Dialog 里的 TextInput"进入 Tab 遍历——
            // 表现为焦点跑到看不见的控件上（点不到、画不出，却能拿到键盘）。
            if !node.is_open() {
                return;
            }
            if node.focusable() && !node.is_disabled() {
                out.push(path.clone());
            }
            for (i, child) in node.children().iter().enumerate() {
                path.push(i);
                walk(child.as_ref(), path, out);
                path.pop();
            }
        }

        let mut out: Vec<Vec<usize>> = Vec::new();
        if let Some(scope) = self.modal_root.clone() {
            if let Some(node) = self.node_at_ref(&scope) {
                let mut path = scope.clone();
                walk(node, &mut path, &mut out);
            }
        } else {
            let mut path = Vec::new();
            walk(self.root.as_ref(), &mut path, &mut out);
        }
        self.focus_order = out;
    }

    /// 出图：**单遍遍历**。
    ///
    /// Base 节点边遍历边画；Overlay / Modal 的子树先把路径记下来，
    /// 遍历结束后按层顺序回放。没有浮层时就是一次 O(n) 遍历。
    pub fn render(&mut self, ctx: &mut dyn RenderCtx) {
        // 卸载时收集到的纹理在这里交给后端。
        //
        // 为什么不在卸载当定就释放：那时手里没有 `RenderCtx`，
        // 而资源属于后端。延迟一帧释放是最小的语义代价。
        if !self.pending_texture_releases.is_empty() {
            let pending = std::mem::take(&mut self.pending_texture_releases);
            for id in pending {
                ctx.release_texture(id);
            }
        }
        // 防御性 guard：脏区为空时不遍历 Widget、不绘制。
        // 这**不决定 Present** —— Present 语义在 `WindowBackend::render`（见 `RenderOutcome`）。
        // 保留此 guard 是为了直接调 `UiTree::render` 的测试与 Canvas 不画空树。
        if self.dirty.is_empty() {
            return;
        }
        // 把本帧动画快照交给后端：控件在 render 里用 `ctx.animation(key)` 取进度。
        // 为什么不给每个控件塞一个 `Animate`：动画是「展示层事实」，控件只该问"我现在该是几"，
        // 不该自己管时间；而这正是控件拿不到 dt 的唯一解法。
        // 悬停快照：控件在 render 里用 `ctx.is_hovered(id)` 判断自己是否被悬停。
        ctx.set_hovered(self.hovered.as_deref());
        let snapshot = crate::animation::AnimSnapshot::new(
            self.animations
                .iter()
                .map(|a| (a.key, a.anim.progress()))
                .collect(),
        );
        ctx.set_animations(&snapshot);
        let dirty = self.dirty.clone();

        // 单遍绘制：**树序就是绘制顺序**（后访问的后画，后画的在上）。
        //
        // 之前有 `Layer::{Base,Overlay,Modal}` + `deferred` 回放：子树先记路径、遍历完
        // 再按层回放。那是"画在上层"的局部手段；现在浯层统一走
        // `UiTree::popups`（树外、最后画），树内只需"父先子后、兄弟按序"。
        {
            let mut path: Vec<usize> = Vec::new();
            // scratch 用 `mem::take` 取出（避免 `&mut self` 字段与 `self.root` 同时借用），用完放回。
            let mut rects = std::mem::take(&mut self.render_rects);
            let mut clips = std::mem::take(&mut self.render_clips);
            render_pass(
                self.root.as_mut(),
                ctx,
                &dirty,
                &mut path,
                &mut rects,
                &mut clips,
            );
            self.render_rects = rects;
            self.render_clips = clips;
        }

        if !self.popups.is_empty() {
            let viewport = Rect::new(0.0, 0.0, self.viewport.w, self.viewport.h);
            ctx.push_clip(viewport);
            // 模态弹层：先铺一层遮罩（与 Dialog 的 0.55 黑一致）
            if self.popups.iter().any(|p| p.modal) {
                ctx.fill_rect(viewport, crate::Color::rgba(0.0, 0.0, 0.0, 0.55));
            }
            for popup in self.popups.iter_mut() {
                if popup.rect.is_empty() {
                    continue;
                }
                if popup.rect.intersect(viewport).is_empty() && !dirty.is_full() {
                    continue;
                }
                // 弹层开启动画：从锚点缩放 + 淡入。
                //
                // 为什么"缩放"和"淡入"要同时做：只缩放会让内容瞬间出现（只是变小），
                // 只淡入则没有"从哪里弹出来"的空间感。两者叠加才是常见弹层观感。
                let progress = ctx.animation(crate::animation::key_of(popup.id()));
                if progress < 1.0 {
                    let anchor = Pos::new(
                        popup.rect.x + popup.rect.w * 0.5,
                        popup.rect.y + popup.rect.h * 0.5,
                    );
                    // scale 下限 0.8：再小会让文字糊成一团，且 rect 可能算成负尺寸
                    let scale = (0.8 + 0.2 * progress).clamp(0.6, 1.0);
                    ctx.push_transform(anchor, scale);
                    popup.widget.render(ctx);
                    ctx.pop_transform();
                    // 淡入：用背景色盖一层 (1-progress) 透明度的膜
                    let bg = ctx.background_color();
                    ctx.fill_rect(
                        popup.rect,
                        crate::Color::rgba(bg.r, bg.g, bg.b, (1.0 - progress).clamp(0.0, 1.0)),
                    );
                } else {
                    popup.widget.render(ctx);
                }
            }
            ctx.pop_clip();
        }

        self.dirty.clear();
    }

    /// 当前 Modal 作用域（根到 Modal 节点的路径）。
    pub fn modal_scope(&self) -> Option<&[usize]> {
        self.modal_root.as_deref()
    }

    /// 按 id 取矩形（来自最近一次 layout）。
    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.rects.get(&crate::layout::id_key(id)).copied()
    }

    /// 命中位置处最内层的非默认光标形状（应用层据此设置系统光标）。
    ///
    /// 语义必须与 `hit_index` 一致，否则光标会指向一个**点不到**的元素：
    /// 1. 关闭节点（`is_open() == false`）整棵子树跳过（与 `rebuild_indices` 同构）；
    /// 2. 祖先 `clip()` 逐层累加，可见区 = `rect ∩ 累计clip`（同上）；
    /// 3. 优先级：`depth` 深者 > 树序靠后者（`>=` 让后访问者胜出）。
    ///
    /// 返回：优先级最高的非 `Default` 光标；没有候选时返回 `Default`。
    /// 副作用：无。复杂度 O(n)（每次 MouseMove 调用一次）。
    pub fn cursor_at(&self, pos: Pos) -> crate::widget::Cursor {
        use crate::widget::Cursor;

        /// 递归收集候选，`best` 保存当前优先级最高的 `(depth, cursor)`。
        fn walk(
            node: &dyn Widget,
            pos: Pos,
            inherited_clip: Option<Rect>,
            depth: usize,
            best: &mut Option<(usize, Cursor)>,
        ) {
            // 1) 关闭节点整棵跳过（与 rebuild_indices 一致）
            if !node.is_open() {
                return;
            }
            // 2) 祖先 clip 累加（与 rebuild_indices 一致）
            let clip = match (inherited_clip, node.clip()) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            // 3) 可见区检查
            let visible = match clip {
                Some(c) => node.rect().intersect(c),
                None => node.rect(),
            };
            if !visible.contains(pos) {
                return;
            }
            // 4) 自己的光标进候选
            let c = node.cursor_at(pos);
            if c != Cursor::Default {
                // `>=` 让同 depth 的**后访问者**胜出（树序靠后者在上）
                let better = best.map(|(bd, _)| depth >= bd).unwrap_or(true);
                if better {
                    *best = Some((depth, c));
                }
            }
            // 5) 递归子节点
            for child in node.children() {
                walk(child.as_ref(), pos, clip, depth + 1, best);
            }
        }

        let mut best: Option<(usize, Cursor)> = None;
        walk(self.root.as_ref(), pos, None, 0, &mut best);
        best.map(|(_, c)| c).unwrap_or(Cursor::Default)
    }
    /// 命中测试：返回最深的、包含该点的节点 id。
    pub fn hit_test(&self, pos: Pos) -> Option<String> {
        self.hit_index(pos)
            .and_then(|i| {
                let p = self.path_of(i);
                self.node_at_ref(&p)
            })
            .map(|n| n.id().to_string())
    }

    /// 命中测试（路径形式，会分配一次 Vec；供 RPC/测试使用）。
    pub fn hit_path(&self, pos: Pos) -> Option<Vec<usize>> {
        self.hit_index(pos).map(|i| self.path_of(i))
    }

    /// 命中测试的**零分配**版本：返回 `paint_order` 的下标。
    ///
    /// 逆序扫描（后画的在上），第一个包含该点的就是目标。
    /// 存在 Modal 时，点在 Modal 子树外 → `None`（事件被吞掉，不穿透）。
    pub fn hit_index(&self, pos: Pos) -> Option<usize> {
        if let Some(root) = self.modal_root.as_ref() {
            let modal_rect = self
                .node_at_ref(root)
                .map(|n| n.rect())
                .unwrap_or(Rect::ZERO);
            if !modal_rect.contains(pos) {
                return None;
            }
        }

        self.paint_order.iter().rposition(|entry| {
            let visible = match entry.clip {
                Some(c) => entry.rect.intersect(c),
                None => entry.rect,
            };
            visible.contains(pos)
        })
    }
    /// 按路径取不可变节点引用（O(depth)，无字符串比较）。
    fn node_at_ref<'a>(&'a self, path: &[usize]) -> Option<&'a dyn Widget> {
        let mut node: &dyn Widget = self.root.as_ref();
        for &idx in path {
            node = node.children().get(idx)?.as_ref();
        }
        Some(node)
    }

    /// 把 `paint_order` 下标还原成树路径（O(depth)，不引用堆上的路径字段）。
    fn path_of(&self, mut idx: usize) -> Vec<usize> {
        let mut slots = Vec::new();
        while let Some(parent) = self.paint_order[idx].parent {
            slots.push(self.paint_order[idx].slot);
            idx = parent;
        }
        slots.reverse();
        slots
    }

    /// 按路径取可变节点引用（O(depth)）。
    fn node_at_mut<'a>(&'a mut self, path: &[usize]) -> Option<&'a mut dyn Widget> {
        let mut node: &mut dyn Widget = self.root.as_mut();
        for &idx in path {
            node = node.children_mut()?.get_mut(idx)?.as_mut();
        }
        Some(node)
    }

    /// 一次遍历求出 id 对应的路径（子索引序列）。
    /// 借出某个节点的可变引用（找不到返回 `false`）。
    ///
    /// 输入：`id`；`f` 拿到 `&mut dyn Widget`（通常再 `as_any_mut().downcast_mut::<T>()`）。
    /// 返回：是否找到该节点。
    /// 用途：应用层改组件状态（关弹窗、设文本）而不必自己遍历树。
    /// 注意：**不会**自动标脏——改完记得 `invalidate(id)` / `mark_layout_dirty(id)`。
    pub fn with_node_mut(&mut self, id: &str, f: &mut dyn FnMut(&mut dyn Widget)) -> bool {
        let Some(path) = self.find_path(id) else {
            return false;
        };
        match self.node_at_mut(&path) {
            Some(node) => {
                f(node);
                true
            }
            None => false,
        }
    }

    /// 按 id 求从根到该节点的子索引路径。为什么用路径而不是指针：路径可跨帧保存，且能在重排后重新定位（指针在树变动后会失效）。
    pub fn find_path(&self, id: &str) -> Option<Vec<usize>> {
        fn walk(node: &dyn Widget, target: &str, acc: &mut Vec<usize>) -> bool {
            if node.id() == target {
                return true;
            }
            for (i, child) in node.children().iter().enumerate() {
                acc.push(i);
                if walk(child.as_ref(), target, acc) {
                    return true;
                }
                acc.pop();
            }
            false
        }

        let mut acc = Vec::new();
        if walk(self.root.as_ref(), id, &mut acc) {
            Some(acc)
        } else {
            None
        }
    }

    /// 按 id 查节点（只读）。
    ///
    /// **复杂度 O(n)**：全树前序递归。遍历场景（对每个节点都要查）请改用
    /// `rev_snapshot` 或直接递归 `children()`，否则会退化成 O(n²)。
    ///
    /// 返回：找到的第一个匹配（正常情况下 id 唯一）；找不到返回 `None`。
    pub fn find(&self, id: &str) -> Option<&dyn Widget> {
        find_in(self.root.as_ref(), id)
    }

    /// 按 id 查节点（可变）。
    ///
    /// 复杂度与 `find` 相同（O(n)）。**绕过框架的字段修改不会触发重绘/重排**，
    /// 组件内部状态请走 `dispatch` 或组件自己的方法。
    pub fn find_mut(&mut self, id: &str) -> Option<&mut dyn Widget> {
        find_in_mut(self.root.as_mut(), id)
    }

    /// 事件入口。
    ///
    /// - **位置类事件**：命中（零分配，拿 paint_order 下标）→ Capture → Target → Bubble。
    /// - **键盘/文本/动作**：只发给焦点节点（单节点 Target）。
    ///
    /// 路径只解析一次，派发沿路径递归一趟，每个节点只访问一次。
    pub fn dispatch(&mut self, event: &UiEvent) -> EventResult {
        // 模态浮层存在时：**键盘事件只走浮层**。
        //   - Tab / Shift+Tab → 在浮层内循环（不能逃到底层）；
        //   - Escape → 关闭浮层（并恢复焦点）；
        //   - 其它键盘/文本 → 只在焦点属于浮层时派发。
        if self.popups.iter().any(|p| p.modal) {
            match event {
                UiEvent::Key {
                    key: crate::event::Key::Tab,
                    modifiers,
                } => {
                    let delta = if modifiers.shift { -1 } else { 1 };
                    let _ = self.focus_next_in_modal(delta);
                    return EventResult::Handled;
                }
                UiEvent::Key {
                    key: crate::event::Key::Escape,
                    ..
                } => {
                    self.close_all_popups();
                    return EventResult::Handled;
                }
                UiEvent::Key { .. } | UiEvent::Text(_) | UiEvent::Compose(_) => {
                    let in_popup = self
                        .focus
                        .as_deref()
                        .map(|f| self.popup_index_of(f).is_some())
                        .unwrap_or(false);
                    if !in_popup {
                        return EventResult::Ignored;
                    }
                    // 继续走下面的浮层命中分支
                }
                _ => {}
            }
        }

        // Overlay 层最高优先级：
        //   - 命中浮层 → 直接派给浮层内部控件（不走树的命中，避免被遮挡）；
        //   - 点在浮层外 → 关闭全部浮层并吃掉这次按下（点外关闭的标准行为）。
        if !self.popups.is_empty() {
            if let Some(pos) = match event {
                UiEvent::MouseDown { pos, .. }
                | UiEvent::MouseUp { pos, .. }
                | UiEvent::MouseMove { pos, .. }
                | UiEvent::Wheel { pos, .. }
                | UiEvent::FileDrop { pos, .. } => Some(*pos),
                _ => None,
            } {
                let hit = self
                    .popups
                    .iter()
                    .rev()
                    .find(|p| p.rect.contains(pos))
                    .map(|p| p.id.clone());
                match hit {
                    Some(id) => {
                        let viewport = self.viewport;
                        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
        ctx.scale = self.scale;
                        ctx.phase = EventPhase::Target;
                        ctx.target = id.clone();
                        let result = match self.popups.iter_mut().find(|p| p.id == id) {
                            Some(p) => p.widget.event(&mut ctx, event),
                            None => EventResult::Ignored,
                        };
                        let merged = ctx.finish(result);
                        self.pending.append(&mut ctx.commands);
                        // 浯层内容自己"关闭"了（`is_open() == false`）就跟着撞掉：
                        // 这是通用机制——Menu 处理"点外部"时会自己 close，
                        // 浯层层不需要认识 Menu，只需要看 `is_open`。
                        if !self
                            .popups
                            .iter()
                            .find(|p| p.id == id)
                            .map(|p| p.widget.is_open())
                            .unwrap_or(false)
                        {
                            self.close_popup(&id);
                        }

                        return merged;
                    }
                    None => {
                        // 模态浮层：点外部**不关闭**（必须显式关），但仍吃掉事件
                        // 以阻塞底层；非模态：点外部关闭。
                        if matches!(event, UiEvent::MouseDown { .. }) {
                            let modal = self.popups.iter().any(|p| p.modal);
                            if !modal {
                                self.close_all_popups();
                            }
                            return EventResult::Handled;
                        }
                    }
                }
            }
        }

        // Tab / Shift+Tab：**焦点组件声明要自己处理时放行**，否则走焦点遍历。
        // 终端用它做 shell 补全；单行输入框不声明（Tab 仍切焦点）。
        if let UiEvent::Key {
            key: crate::event::Key::Tab,
            modifiers,
        } = event
        {
            let wants = self
                .focus
                .as_deref()
                .and_then(|id| self.find(id))
                .map(|n| n.wants_tab())
                .unwrap_or(false);
            if !wants {
                let delta = if modifiers.shift { -1 } else { 1 };
                let moved = self.focus_next(delta);
                return if moved {
                    EventResult::Redraw
                } else {
                    EventResult::Handled
                };
            }
        }

        // 悬停追踪：只让"旧节点 + 新节点"两块区域变脏。
        if let UiEvent::MouseMove { pos } = event {
            // 指针被捕获时不更新悬停：拖拽中鼠标会划过别的控件，
            // 若那时切换悬停目标，工具提示会在拖拽中闪现，且与
            // "按住期间所有移动都给捕获者"的语义矛盾。
            if self.captured_pointer.is_none() {
                let new = self
                    .hit_index(*pos)
                    .and_then(|i| {
                        let p = self.path_of(i);
                        self.node_at_ref(&p)
                    })
                    .map(|n| n.id().to_string());
                if new != self.hovered {
                    let old = self.hovered.clone();
                    if let Some(ref old_id) = old {
                        self.invalidate(old_id);
                    }
                    if let Some(ref next) = new {
                        self.invalidate(next);
                    }
                    self.hovered = new.clone();

                    // 先发 Leave（旧节点）再发 Enter（新节点）。
                    //
                    // 为什么用 `dispatch_to` 而不是在当前遍历里附带：
                    // Enter/Leave 是**定向**事件，它们不应该像 MouseMove 那样走命中链。
                    // **冒泡到祖先**：包装型控件（如 TooltipHost）的命中点在子控件上，
                    // 若只发给命中节点，它永远不知道自己被悬停。
                    // 顺序：目标 → 根（与 DOM 的 mouseover/mouseout 一致）。
                    if let Some(old_id) = old {
                        if let Some(path) = self.find_path(&old_id) {
                            // 从"**目标 → 根**" 逐层派发：`k=0` 就是根，不能省略，
                            // 否则"包装型控件自己就是根"的情形永远收不到事件。
                            for k in (0..=path.len()).rev() {
                                let _ = self.dispatch_target_only(&path[..k], &UiEvent::MouseLeave);
                            }
                        }
                    }
                    if let Some(new_id) = new {
                        if let Some(path) = self.find_path(&new_id) {
                            for k in (0..=path.len()).rev() {
                                let _ = self.dispatch_target_only(&path[..k], &UiEvent::MouseEnter);
                            }
                        }
                    }
                }
            }
        }

        let positional = matches!(
            event,
            UiEvent::Wheel { .. }
                | UiEvent::MouseDown { .. }
                | UiEvent::MouseUp { .. }
                | UiEvent::MouseMove { .. }
                | UiEvent::FileDrop { .. }
        );

        // 把目标路径搬进复用缓冲：之后不再分配，也不与 &mut self 冲突
        let mut path = std::mem::take(&mut self.path_buf);
        path.clear();

        let hit = match event {
            UiEvent::Wheel { pos, .. }
            | UiEvent::MouseDown { pos, .. }
            | UiEvent::MouseUp { pos, .. }
            | UiEvent::MouseMove { pos, .. }
            | UiEvent::FileDrop { pos, .. } => self.hit_index(*pos),
            _ => None,
        };

        // 指针捕获：按住期间的移动/抬起，永远送给捕获者
        let captured = match event {
            UiEvent::MouseMove { .. } | UiEvent::MouseUp { .. } => self.captured_path.clone(),
            _ => None,
        };

        let mut target_id: Option<String> = None;
        let mut has_target = false;

        if positional {
            if let Some(cap) = captured {
                path.extend_from_slice(&cap);
                has_target = true;
            } else if let Some(i) = hit {
                let p = self.path_of(i);
                path.extend_from_slice(&p);
                has_target = true;
            }
        } else if matches!(
            event,
            UiEvent::Key { .. } | UiEvent::Text(_) | UiEvent::Compose(_) | UiEvent::Action { .. }
        ) {
            if let Some(fp) = self.focus_path.clone() {
                path.extend_from_slice(&fp);
                has_target = true;
            }
        }

        if has_target {
            target_id = self.node_at_ref(&path).map(|n| n.id().to_string());
        }

        let result = if has_target {
            if positional {
                self.dispatch_path(&path, event)
            } else {
                self.dispatch_target_only(&path, event)
            }
        } else if matches!(event, UiEvent::Focus(_) | UiEvent::Resize | UiEvent::Tick) {
            self.dispatch_broadcast(event)
        } else {
            EventResult::Ignored
        };

        self.path_buf = path;

        // 鼠标抬起后无论如何都解捕获
        if matches!(event, UiEvent::MouseUp { .. }) {
            self.captured_pointer = None;
            self.captured_path = None;
        }

        // 脏粒度：Relayout 连尺寸都重算，Redraw 只重画目标
        if result.needs_relayout() {
            let who = self.relayout_from.clone().or_else(|| target_id.clone());
            match who.as_deref() {
                Some(id) => {
                    self.cache.mark_dirty(id);
                    // 只标目标矩形（不是整窗）：布局后 `mark_moved` 会把所有
                    // 位置变化的节点用"旧 ∪ 新"补进脏区。
                    // 滚动这类"只有目标子树在动"的场景因此不再触发整窗重绘。
                    // 目标还没有有效矩形（新节点/零尺寸）时退回整窗，避免漏画。
                    if self.rect_of(id).is_none_or(|r| r.is_empty()) {
                        self.dirty = DirtyRegion::full();
                    } else {
                        self.invalidate(id);
                    }
                }
                None => self.dirty = DirtyRegion::full(),
            }
        } else if result.needs_redraw() {
            match target_id.as_deref() {
                Some(id) => self.invalidate(id),
                // 广播事件没有单一 target：用 broadcast 期间收集到的矩形逐块标脏。
                // 之前这里 `invalidate_all()`，于是"终端每帧 Tick 读到 PTY 数据"
                // 会触发整窗重绘（§35 的性能隐患）。
                None => {
                    if self.broadcast_rects.is_empty() {
                        self.invalidate_all();
                    } else {
                        let rects = std::mem::take(&mut self.broadcast_rects);
                        for r in &rects {
                            self.dirty.add(*r);
                        }
                        self.broadcast_rects = rects;
                        self.broadcast_rects.clear();
                    }
                }
            }
        }

        result
    }
    /// 沿路径做三阶段派发：每个节点只访问一次，O(depth)。
    fn dispatch_path(&mut self, path: &[usize], event: &UiEvent) -> EventResult {
        let viewport = self.viewport;
        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
        ctx.scale = self.scale;
        if let Some(node) = self.node_at_ref(path) {
            ctx.target = node.id().to_string();
        }

        let (result, _stopped) = dispatch_rec(self.root.as_mut(), path, &mut ctx, event);
        self.finish_dispatch(ctx, result)
    }

    /// 只把事件投给路径末端（键盘/文本/动作/RPC）。
    fn dispatch_target_only(&mut self, path: &[usize], event: &UiEvent) -> EventResult {
        // 注意：空路径是合法的 —— 它表示根节点本身。
        let viewport = self.viewport;
        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
        ctx.scale = self.scale;
        ctx.phase = EventPhase::Target;
        if let Some(node) = self.node_at_ref(path) {
            ctx.target = node.id().to_string();
        }

        let result = match self.node_at_mut(path) {
            Some(node) => node.event(&mut ctx, event),
            None => EventResult::Ignored,
        };

        self.finish_dispatch(ctx, result)
    }

    /// 把事件直接投递给指定节点（不走命中测试，也不冒泡）。
    ///
    /// RPC、命令路由、测试都走这里。
    pub fn dispatch_to(&mut self, id: &str, event: &UiEvent) -> EventResult {
        match self.find_path(id) {
            Some(path) => self.dispatch_target_only(&path, event),
            None => EventResult::Ignored,
        }
    }

    /// 统一收尾：合并标志、搬运命令、落实焦点与指针捕获。
    ///
    /// 输入：本次派发的 `ctx` 与组件返回的 `result`。
    /// 失败：无（不 panic）。若 `result` 要求 Relayout，会在此处标记布局失效——
    /// 这是**所有入口**（dispatch / dispatch_to / RPC）共用的语义，
    /// 否则 RPC 触发的 Relayout 会变成空操作，索引与布局永远停在旧状态。
    fn finish_dispatch(&mut self, mut ctx: EventCtx, result: EventResult) -> EventResult {
        let target = ctx.target.clone();
        // 记录"谁请求了重排"：Relayout 的脏区要标在它身上（见 dispatch 分支）
        self.relayout_from = ctx.relayout_from.take();
        let merged = ctx.finish(result);
        if merged.needs_relayout() && !target.is_empty() {
            self.cache.mark_dirty(&target);
            // 同上：只标目标矩形，其余交给布局后的 `mark_moved` 补并集。
            if self.rect_of(&target).is_none_or(|r| r.is_empty()) {
                self.dirty = DirtyRegion::full();
            } else {
                self.invalidate(&target);
            }
        }
        // 目标节点的语义面可能在这次事件里变了：登记到待检查集合，
        // 下一次 refresh_semantics 会比对哈希并决定是否 bump。
        if !target.is_empty() {
            self.pending_semantic.insert(target);
        }
        self.pending.append(&mut ctx.commands);

        // 组件请求的 IME 候选窗位置：留给应用层下一帧落实
        if ctx.ime_candidate.is_some() {
            self.pending_ime = ctx.ime_candidate;
        }

        if let Some(want) = ctx.focus_request.clone() {
            self.set_focus(Some(want));
        }

        // 组件在事件里提交的浮层请求：到这里才真正构造 + 挂载。
        // 工厂已经拿到了需要的数据，此时再调用不会与 `&mut tree` 冲突。
        // 先关、后开：同一次分发里同时要求关与开时，
        // 先执行关能避免"新浮层被旧请求关掉"。
        for id in std::mem::take(&mut ctx.close_popup_requests) {
            self.close_popup(&id);
        }
        for req in ctx.popup_requests.drain(..) {
            let widget = (req.factory)();
            if req.modal {
                self.open_modal_popup(widget, req.anchor);
            } else {
                self.open_popup(widget, req.anchor);
            }
        }

        match ctx.pointer_capture.take() {
            Some(PointerCapture::Capture(id)) => {
                // 路径在捕获时就解析好，后续 MouseMove 不再查找、不再分配
                self.captured_path = self.find_path(&id);
                self.captured_pointer = Some(id);
            }
            Some(PointerCapture::Release) => {
                self.captured_pointer = None;
                self.captured_path = None;
            }
            None => {}
        }

        merged
    }

    fn dispatch_broadcast(&mut self, event: &UiEvent) -> EventResult {
        let viewport = self.viewport;
        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
        ctx.scale = self.scale;
        ctx.phase = EventPhase::Target;
        let mut acc = EventResult::Ignored;
        let mut rects = std::mem::take(&mut self.broadcast_rects);
        rects.clear();
        broadcast(self.root.as_mut(), &mut ctx, event, &mut acc, &mut rects);
        self.broadcast_rects = rects;
        self.finish_dispatch(ctx, acc)
    }
    /// 取出并处理命令队列。
    ///
    /// - `Command::Node`：框架自己路由成一条 `UiEvent::Action` 投递给目标节点；
    /// - `Command::App`：原样返回给调用方（应用层），框架不解释。
    ///
    /// 返回的只有 App 级命令。Node 级命令可能级联产生新命令，
    /// 这里用固定轮数上限防止组件互相触发形成死循环。
    pub fn drain_commands(&mut self) -> Vec<crate::command::Command> {
        let mut app_commands = Vec::new();
        for _round in 0..8 {
            if self.pending.is_empty() {
                break;
            }
            let batch = std::mem::take(&mut self.pending);
            let mut deferred = Vec::new();
            for cmd in batch {
                match cmd {
                    crate::command::Command::Node {
                        target,
                        action,
                        args,
                    } => {
                        deferred.push(crate::command::Command::Node {
                            target: target.clone(),
                            action: action.clone(),
                            args: args.clone(),
                        });
                        let _ = self.dispatch_to(&target, &UiEvent::Action { name: action, args });
                    }
                    other => app_commands.push(other),
                }
            }
            // Node 命令已经投递过，不要重复处理
            deferred.clear();
        }
        app_commands
    }
    /// 导出语义树为 JSON（RPC get_tree 的返回体）。
    ///
    /// 形状与 [NodeInfo] 一致：id / role / rect / focused / label / value /
    /// actions / props / children，外部工具可以直接消费。
    pub fn snapshot_json(&self) -> crate::json::Json {
        fn node_json(node: &dyn Widget, focus: Option<&str>) -> crate::json::Json {
            use crate::json::Json;
            let rect = node.rect();
            let sem = node.semantics();

            let mut props = Json::obj();
            for (k, v) in &sem.props {
                props = props.set(k, v.clone());
            }

            Json::obj()
                .set("id", node.id())
                .set("role", node.role().as_str())
                .set(
                    "rect",
                    Json::obj()
                        .set("x", rect.x)
                        .set("y", rect.y)
                        .set("w", rect.w)
                        .set("h", rect.h),
                )
                .set("focused", focus == Some(node.id()))
                .set("label", sem.label.clone())
                .set("value", sem.value.clone())
                .set(
                    "actions",
                    Json::Arr(sem.actions.iter().map(|a| a.to_json()).collect()),
                )
                .set("props", props)
                .set("state", sem.state.clone())
                .set("revision", node.revision())
                .set("disabled", sem.disabled)
                .set(
                    "children",
                    Json::Arr(
                        node.children()
                            .iter()
                            .map(|c| node_json(c.as_ref(), focus))
                            .collect(),
                    ),
                )
        }
        node_json(self.root.as_ref(), self.focus.as_deref())
    }
    /// 导出语义树快照。
    pub fn snapshot(&self) -> NodeInfo {
        fn walk(node: &dyn Widget, focus: Option<&str>) -> NodeInfo {
            NodeInfo {
                id: node.id().to_string(),
                role: node.role(),
                rect: node.rect(),
                focused: focus == Some(node.id()),
                semantics: node.semantics(),
                children: node
                    .children()
                    .iter()
                    .map(|c| walk(c.as_ref(), focus))
                    .collect(),
            }
        }
        let mut root = walk(self.root.as_ref(), self.focus.as_deref());
        // 浮层也要进语义树：否则外部工具/无障碍看不见菜单里的项。
        // 挂在根节点的 `children` 末尾（与绘制顺序一致：浮层最后画）。
        for popup in &self.popups {
            let mut info = walk(popup.widget.as_ref(), self.focus.as_deref());
            // 浮层的布局矩形由 `place_popup` 决定，与 `node.rect()` 可能不一致：
            // 语义树里必须报**真正绘制/命中**的矩形，否则外部工具点不中。
            info.rect = popup.rect;
            root.children.push(info);
        }
        root
    }
}

fn find_in<'a>(node: &'a dyn Widget, id: &str) -> Option<&'a dyn Widget> {
    if node.id() == id {
        return Some(node);
    }
    node.children().iter().find_map(|c| find_in(c.as_ref(), id))
}

fn find_in_mut<'a>(node: &'a mut dyn Widget, id: &str) -> Option<&'a mut dyn Widget> {
    if node.id() == id {
        return Some(node);
    }
    match node.children_mut() {
        Some(children) => children
            .iter_mut()
            .find_map(|c| find_in_mut(c.as_mut(), id)),
        None => None,
    }
}

fn broadcast(
    node: &mut dyn Widget,
    ctx: &mut EventCtx,
    event: &UiEvent,
    acc: &mut EventResult,
    redraw_rects: &mut Vec<Rect>,
) {
    // `Tick` 是每帧心跳：只发给显式订阅的节点（`wants_tick`），
    // 否则每帧 8 轮 × 全树虚调用。其他广播事件（Focus/Resize）照常全员送达。
    let wants = !matches!(event, UiEvent::Tick) || node.wants_tick();
    let r = if wants {
        node.event(ctx, event)
    } else {
        EventResult::Ignored
    };
    if r.needs_redraw() {
        // 广播事件（Tick 等）没有单一 target：记录每个**真的要求重绘**的节点矩形，
        // 稍后逐块标脏，而不是整窗重绘（见 §35）。
        redraw_rects.push(node.rect());
    }
    *acc = acc.merge(r);
    if let Some(children) = node.children_mut() {
        for child in children.iter_mut() {
            broadcast(child.as_mut(), ctx, event, acc, redraw_rects);
        }
    }
}

/// 一个绘制/命中条目：路径 + 包围盒 + 生效层级。
#[derive(Clone, Debug)]
struct PaintEntry {
    /// 父节点在 `paint_order` 里的下标（根 = `None`）。
    ///
    /// 为什么不存 `Vec<usize>` 路径：DFS 下父节点一定先于子节点入表，
    /// 所以只用 `(父下标, 在父中的 slot)` 就能重建路径。
    /// 每节点一次 `path.clone()` 就是一次堆分配（实测：10k 节点 1.9次分配/节点）。
    parent: Option<usize>,
    /// 在父节点 `children` 中的下标。
    slot: usize,
    rect: Rect,
    /// 屏幕可见区 = rect ∩ 祖先 clip；命中与裁剪共用。
    clip: Option<Rect>,
}

/// 沿路径递归派发：进入时 Capture、末端 Target、���回时 Bubble。
///
/// 访问的节点数恰好等于路径长度，与树的规模无关。
fn dispatch_rec(
    node: &mut dyn Widget,
    rest: &[usize],
    ctx: &mut EventCtx,
    event: &UiEvent,
) -> (EventResult, bool) {
    // 末端：Target
    if rest.is_empty() {
        ctx.phase = EventPhase::Target;
        let r = node.event(ctx, event);
        let stop = ctx.stop_propagation || (r.is_handled() && !ctx.propagate);
        if r.needs_relayout() {
            ctx.relayout_from = Some(node.id().to_string());
        }
        return (r, stop);
    }

    // 进入：Capture
    ctx.phase = EventPhase::Capture;
    let mut result = node.event(ctx, event);
    if ctx.stop_propagation {
        return (result, true);
    }

    // 下潜
    let mut child_result = EventResult::Ignored;
    let mut child_stopped = false;
    if let Some(children) = node.children_mut() {
        if let Some(child) = children.get_mut(rest[0]) {
            let (r, s) = dispatch_rec(child.as_mut(), &rest[1..], ctx, event);
            child_result = r;
            child_stopped = s;
        }
    }
    result = result.merge(child_result);
    if result.needs_relayout() && ctx.relayout_from.is_none() {
        // 子节点没记到（例如它只返回 Redraw，是当前节点自己要求重排）
        ctx.relayout_from = Some(node.id().to_string());
    }
    if child_stopped {
        return (result, true);
    }

    // 返回：Bubble
    ctx.phase = EventPhase::Bubble;
    let r = node.event(ctx, event);
    result = result.merge(r);
    // 冒泡阶段的返回也可能要求重排（滚动容器就是在冒泡里处理滚轮的），
    // 这里再记一次"请求者"，否则脏区会标到命中目标（某一行）身上。
    if r.needs_relayout() && ctx.relayout_from.is_none() {
        ctx.relayout_from = Some(node.id().to_string());
    }
    let stop = ctx.stop_propagation || (r.is_handled() && !ctx.propagate);
    (result, stop)
}

/// 单遍渲染：Base 就地画，Overlay/Modal 记路径延后画。
fn render_pass(
    node: &mut dyn Widget,
    ctx: &mut dyn RenderCtx,
    dirty: &DirtyRegion,
    path: &mut Vec<usize>,
    // **复用缓存**（由 `UiTree::render` 持有）：避免每层每帧分配 `Vec`。
    rects: &mut Vec<Rect>,
    clips: &mut Vec<Option<Rect>>,
) {
    // 关闭的浮层：整棵子树都不绘制（与 rebuild_indices 共享同一不变量）。
    if !node.is_open() {
        return;
    }
    if node.needs_paint(dirty) {
        // 诊断开关：按**真实绘制顺序**打印每个节点的 id/矩形/裁切，
        // 用于定位"文字被后画的背景盖掉"这类问题。
        if std::env::var_os("Y901_PAINT_TRACE").is_some() {
            let r = node.rect();
            eprintln!(
                "[paint] {:<24} rect=({:.0},{:.0} {:.0}x{:.0}) clip={:?}",
                node.id(),
                r.x,
                r.y,
                r.w,
                r.h,
                node.clip()
            );
        }
        node.render(ctx);
    }
    // 滚动容器：子节点只能画在视口内
    let clip = node.clip();
    if let Some(c) = clip {
        ctx.push_clip(c);
    }
    // 逐子控件剪裁（如 Grid 的每个格子）。
    //
    // 为什么要先算好：`children_mut()` 已经短临借了 `node`，
    // 循环里再调 `node.children_clip(..)` 会碰借用冲突。
    // **本节点的裁切写在 `clips` 尾部的一段里**，管理方式是"记下 base，用完 truncate"：
    // 递归每层都会往同一个 Vec 里 push，但**各自只读自己那一段**，所以互不覆盖。
    let base = clips.len();
    rects.clear();
    for c in node.children() {
        rects.push(c.rect());
    }
    let node_rect = node.rect();
    for r in rects.iter() {
        clips.push(node.children_clip(*r).map(|c| c.intersect(node_rect)));
    }
    let count = clips.len() - base;

    if let Some(children) = node.children_mut() {
        for (i, child) in children.iter_mut().enumerate().take(count) {
            // 先拷出值，避免递归期间持有 `clips` 的引用。
            let child_clip = clips[base + i];
            if let Some(c) = child_clip {
                ctx.push_clip(c);
            }
            path.push(i);
            render_pass(child.as_mut(), ctx, dirty, path, rects, clips);
            path.pop();
            if child_clip.is_some() {
                ctx.pop_clip();
            }
        }
    }
    // 收回本节点那一段（递归返回时它们已各自收回，这里保险地再收一次）。
    clips.truncate(base);
    if clip.is_some() {
        ctx.pop_clip();
    }
}

/// 一条语义订阅。
struct Subscription {
    id: String,
    /// 上次看到的指纹（语义面哈希 + 组件 revision）。
    rev: u64,
    /// 上次推送时的完整语义快照：差分需要 label/value/state/disabled/focused。
    last_sem: crate::widget::Semantics,
    /// 最小推送间隔。
    interval: std::time::Duration,
    /// 下次允许检查的时间点。
    next_due: std::time::Instant,
}

/// 节点状态指纹：语义面哈希 + 组件 revision。
///
/// 语义面已含 label/value/state/disabled/sensitive/exposed/focused，
/// 因此任何对外可见变化都会改变指纹 —— 这正是"要不要推送"的判据。
fn fingerprint(node: &dyn Widget, focused: bool) -> u64 {
    let mut sem = node.semantics();
    sem.focused = focused;
    crate::tree::hash_semantics(node.role(), &sem)
        ^ node.revision().wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// 语义差分：只保留外部工具关心的、且真的变了的字段。
///
/// 覆盖 value / state（**拍平**成顶层键）/ disabled / focused / label。
/// `role` / `actions` / `sensitive` / `exposed` 很少变，靠 get_tree 兜底。
///
/// 拍平是为了让订阅者拿到的形状与 `get_tree` 的 state 一致（`patch.text` 而不是
/// `patch.state.text`），也保持既有消费者的兼容性。
fn semantic_patch(
    old: &crate::widget::Semantics,
    new: &crate::widget::Semantics,
) -> crate::json::Json {
    let mut out = diff_state(&old.state, &new.state);
    if old.label != new.label {
        out = out.set("label", new.label.as_str());
    }
    if old.value != new.value {
        out = out.set("value", new.value.as_str());
    }
    if old.disabled != new.disabled {
        out = out.set("disabled", new.disabled);
    }
    if old.focused != new.focused {
        out = out.set("focused", new.focused);
    }
    out
}

/// 计算两个扁平状态对象的差分（只保留变化的键）。
fn diff_state(old: &crate::json::Json, new: &crate::json::Json) -> crate::json::Json {
    use crate::json::Json;

    let mut out: Vec<(String, Json)> = Vec::new();
    if let Json::Obj(map) = new {
        for (key, value) in map {
            let changed = match old {
                Json::Obj(old_map) => old_map
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v != value)
                    .unwrap_or(true),
                _ => true,
            };
            if changed {
                out.push((key.clone(), value.clone()));
            }
        }
    }
    Json::Obj(out)
}

fn patch_is_empty(patch: &crate::json::Json) -> bool {
    matches!(patch, crate::json::Json::Obj(map) if map.is_empty())
}

/// 递归调用卸载钩子：先释放子树资源，再释放自己。
fn unmount_recursive(mut node: Box<dyn Widget>, releases: &mut Vec<crate::image::TextureId>) {
    if let Some(children) = node.children_mut() {
        let taken: Vec<Box<dyn Widget>> = std::mem::take(children);
        for child in taken {
            unmount_recursive(child, releases);
        }
    }
    // 先收集纹理所有权，再走卸载钩子（资源回收要在对象消失前记下）
    node.release_textures(releases);
    node.on_unmount();
}

/// 递归触发挂载钩子（初始树用）。
fn mount_recursive(node: &mut dyn Widget, invalidation: &crate::signal::Invalidation) {
    let mut ctx = crate::widget::MountCtx::new(node.id().to_string(), invalidation.clone());
    node.on_mount(&mut ctx);
    if let Some(children) = node.children_mut() {
        for child in children.iter_mut() {
            mount_recursive(child.as_mut(), invalidation);
        }
    }
}

// ---------------------------------------------------------------------------
// revision 辅助：语义面哈希 / generation 登记
// ---------------------------------------------------------------------------

/// 语义面哈希：**对外可见的字段**才参与。
///
/// 进：role / label / value / actions / state / disabled / sensitive / exposed / focused
/// 不进：id（身份）、rect（几何）、props（调试信息）
///
/// `focused` 由调用方传入（框架拥有），组件自报的 `semantics().focused` 会被覆盖。
/// 用 FNV-1a 增量写入，不构造中间 String（比 to_string 快数倍）。
fn semantic_hash(node: &dyn Widget, focused: bool) -> u64 {
    let mut sem = node.semantics();
    sem.focused = focused;
    crate::tree::hash_semantics(node.role(), &sem)
}

/// 纯函数版本：给定 `(role, Semantics)` 计算语义面哈希。
///
/// 公开只是工程妥协：`tests/` 是独立 crate，需要和实现共用同一份哈希，
/// 否则碰撞统计测的就不是产品代码。**不属于稳定 API，不要依赖。**
///
/// 分隔符说明：每个字段结尾都会喂一次 `0xff`，避免 `"ab"+"c"` 与 `"a"+"bc"`
/// 落在同一状态机上。
#[doc(hidden)]
pub fn hash_semantics(role: Role, sem: &crate::widget::Semantics) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x1000_0000_01b3);
    };

    feed(role.as_str().as_bytes());
    feed(sem.label.as_bytes());
    feed(sem.value.as_bytes());
    feed(&u64::from(sem.disabled).to_le_bytes());
    feed(&u64::from(sem.sensitive).to_le_bytes());
    feed(&u64::from(sem.focused).to_le_bytes());
    feed(&u64::from(sem.exposed_to_ai).to_le_bytes());
    feed(sem.state.to_string().as_bytes());
    for action in &sem.actions {
        feed(action.name.as_bytes());
        // InputKind 是 4 值枚举，u8 足够；不是截断
        feed(&[action.input as u8]);
    }
    h
}

/// 收集子树里所有 id（前序）。
fn collect_ids(node: &dyn Widget) -> Vec<String> {
    let mut out = vec![node.id().to_string()];
    for child in node.children() {
        out.extend(collect_ids(child.as_ref()));
    }
    out
}

/// 把子树里每个 id 从所有 revision 相关 map 里清掉。
fn forget_recursive(node: &dyn Widget, tree: &mut UiTree) {
    let id = node.id().to_string();
    tree.generations.remove(&id);
    tree.last_semantic.remove(&id);
    tree.rev.remove(&id);
    tree.pending_semantic.remove(&id);
    for child in node.children() {
        forget_recursive(child.as_ref(), tree);
    }
}
