//! 语义树：把 Widget 树变成"可查询、可注入、可命中"的结构。
//!
//! 这层是 Agent-Native 的核心：外部只能通过 id 操作节点，
//! 拿到的矩形与语义与实际渲染完全一致（同一棵树算出来的）。

use std::collections::HashMap;

use crate::event::{EventResult, UiEvent};
use crate::geometry::{Pos, Rect, Size};
use crate::event::{EventPhase, PointerCapture};
use crate::widget::{EventCtx, Layer, RenderCtx, Role, Semantics, TextMetrics, Widget};

/// 对外暴露的节点快照。RPC 的 `get_tree` 直接序列化它。
#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub id: String,
    pub role: Role,
    pub rect: Rect,
    pub focused: bool,
    pub semantics: Semantics,
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
    pub fn full() -> Self {
        Self {
            full: true,
            rects: Vec::new(),
        }
    }

    pub fn add(&mut self, rect: Rect) {
        if self.full || rect.is_empty() {
            return;
        }
        // 合并：已被现有区域包含就不重复添加（小区块很多的场景很常见）
        for r in &self.rects {
            if r.x <= rect.x
                && r.y <= rect.y
                && r.right() >= rect.right()
                && r.bottom() >= rect.bottom()
            {
                return;
            }
        }
        self.rects.push(rect);
    }

    pub fn is_full(&self) -> bool {
        self.full
    }

    pub fn is_empty(&self) -> bool {
        !self.full && self.rects.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rects.len()
    }

    /// 矩形是否与脏区相交。
    pub fn intersects(&self, rect: Rect) -> bool {
        if self.full {
            return true;
        }
        self.rects
            .iter()
            .any(|r| !r.intersect(rect).is_empty())
    }

    pub fn clear(&mut self) {
        self.full = false;
        self.rects.clear();
    }
}

pub struct UiTree {
    root: Box<dyn Widget>,
    focus: Option<String>,
    viewport: Size,
    /// id -> rect 的缓存，避免每次 RPC 都走一遍树。
    rects: HashMap<String, Rect>,
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
    /// 各层是否出现过。
    layer_present: [bool; 3],
    /// 最上层 Modal 的路径（焦点作用域 + 点击吞噬判定）。
    modal_root: Option<Vec<usize>>,
    /// 焦点节点路径缓存（键盘事件零分配）。
    focus_path: Option<Vec<usize>>,
    /// 捕获节点的路径缓存。
    captured_path: Option<Vec<usize>>,
    /// 复用的事件路径缓冲，避免每次鼠标移动都分配。
    path_buf: Vec<usize>,
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
}

impl UiTree {
    pub fn new(root: Box<dyn Widget>) -> Self {
        let mut this = Self {
            root,
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
            layer_present: [false; 3],
            modal_root: None,
            focus_path: None,
            captured_path: None,
            path_buf: Vec::new(),
            pending_ime: None,
            in_focus_change: false,
            focus_order: Vec::new(),
            subscriptions: Vec::new(),
            invalidation_queue: crate::signal::Invalidation::new(),
        };

        // 初始树也要触发 on_mount：bind() 的 Signal 才能自动订阅
        let invalidation = this.invalidation_queue.clone();
        mount_recursive(this.root.as_mut(), &invalidation);
        this
    }

    pub fn hovered(&self) -> Option<&str> {
        self.hovered.as_deref()
    }

    pub fn dirty(&self) -> &DirtyRegion {
        &self.dirty
    }

    /// 把某个节点的矩形加入脏区。
    pub fn invalidate(&mut self, id: &str) {
        if let Some(rect) = self.rects.get(id).copied() {
            self.dirty.add(rect);
        }
    }

    /// 整窗重绘。
    pub fn invalidate_all(&mut self) {
        self.dirty = DirtyRegion::full();
    }

    /// 标记某节点的尺寸可能变化（模拟 EventResult::Relayout 的效果）。
    pub fn mark_layout_dirty(&mut self, id: &str) {
        self.cache.mark_dirty(id);
    }

    pub fn layout_stats(&self) -> crate::layout::LayoutStats {
        self.cache.stats()
    }

    pub fn root(&self) -> &dyn Widget {
        self.root.as_ref()
    }

    pub fn root_mut(&mut self) -> &mut dyn Widget {
        self.root.as_mut()
    }

    pub fn viewport(&self) -> Size {
        self.viewport
    }

    pub fn focus(&self) -> Option<&str> {
        self.focus.as_deref()
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
            let Some(path) = self.find_path(target) else {
                return false;
            };
            if let Some(scope) = self.modal_root.as_ref() {
                if !path.starts_with(scope.as_slice()) {
                    return false;
                }
            }
        }
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
        self.focus_path = self
            .focus
            .as_deref()
            .and_then(|f| self.find_path(f));
        true
    }

    /// 把一条 `Focus` 事件沿路径做三阶段派发。返回 false 表示被否决。
    fn send_focus(&mut self, id: &str, focused: bool) -> bool {
        let Some(path) = self.find_path(id) else {
            return true;
        };
        let viewport = self.viewport;
        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
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

        self.cache.mark_dirty(parent);
        self.dirty = DirtyRegion::full();
        true
    }


    /// 在指定位置插入子节点（越界自动 clamp 到末尾）。
    ///
    /// 与 `add_node` 的差别只有位置；生命周期与重建逻辑完全一致。
    pub fn insert_node(&mut self, parent: &str, index: usize, child: Box<dyn Widget>) -> bool {
        if self.subtree_has_duplicate(child.as_ref()) {
            eprintln!(
                "[y901] insert_node: duplicate id `{}` rejected",
                child.id()
            );
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

        self.cache.mark_dirty(parent);
        self.dirty = DirtyRegion::full();
        true
    }
    /// 子树里是否有 id 已存在于当前树中。
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
        unmount_recursive(removed);

        let parent_id = self
            .node_at_ref(&parent_path)
            .map(|n| n.id().to_string())
            .unwrap_or_default();

        // 焦点/捕获若落在被删子树里，就地清空（否则悬空）
        let inside = |p: &Vec<usize>| p.starts_with(&path);
        if self.focus_path.as_ref().map(|p| inside(p)).unwrap_or(false) {
            self.focus = None;
            self.focus_path = None;
        }
        if self.captured_path.as_ref().map(|p| inside(p)).unwrap_or(false) {
            self.captured_pointer = None;
            self.captured_path = None;
        }
        if self.hovered.as_deref() == Some(id) {
            self.hovered = None;
        }

        self.cache.mark_dirty(&parent_id);
        self.dirty = DirtyRegion::full();
        true
    }

    /// 排空响应式失效队列：把这些节点标脏并重算布局。
    pub fn drain_invalidation(&mut self) -> usize {
        let ids = self.invalidation_queue.drain();
        if ids.is_empty() {
            return 0;
        }
        for id in &ids {
            self.cache.mark_dirty(id);
            self.invalidate(id);
        }
        self.dirty = DirtyRegion::full();
        ids.len()
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
        let Some(node) = self.find(id) else {
            return false;
        };
        let sem = node.semantics();
        let state = sem.state.clone();
        let value = sem.value.clone();
        let rev = fingerprint(node);

        if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
            sub.rev = rev;
            sub.last_state = state;
            sub.last_value = value;
            sub.interval = interval;
            sub.next_due = std::time::Instant::now() + interval;
            return true;
        }

        self.subscriptions.push(Subscription {
            id: id.to_string(),
            rev,
            last_state: state,
            last_value: value,
            interval,
            next_due: std::time::Instant::now() + interval,
        });
        true
    }

    pub fn unsubscribe(&mut self, id: &str) -> bool {
        let before = self.subscriptions.len();
        self.subscriptions.retain(|s| s.id != id);
        self.subscriptions.len() != before
    }

    pub fn subscription_count(&self) -> usize {
        self.subscriptions.len()
    }

    /// 收集到期的状态变化，产出 `state_changed` 事件（**差分 + 节流**）。
    ///
    /// - 差分：`patch` 只包含变化的字段；
    /// - 节流：每个订阅有 `interval`，到期才检查一次，窗口内的多次变化合并成一次。
    pub fn poll_subscriptions(&mut self, now: std::time::Instant) -> Vec<crate::json::Json> {
        use crate::json::Json;

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

            let Some(node) = self.find(&id) else {
                continue;
            };
            let rev = fingerprint(node);
            let state = node.semantics().state;
            let last_state = self
                .subscriptions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.last_state.clone())
                .unwrap_or(crate::json::Json::Null);
            let interval = self
                .subscriptions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.interval)
                .unwrap_or(std::time::Duration::from_millis(100));

            // 状态没变：只顺延下次检查时间
            if rev == self
                .subscriptions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.rev)
                .unwrap_or(rev)
            {
                if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    sub.next_due = now + interval;
                }
                continue;
            }

            let mut patch = diff_state(&last_state, &state);

            // 没有结构化 state 的组件（如终端）：value 变化也必须推出去
            let value = node.semantics().value;
            let last_value = self
                .subscriptions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.last_value.clone())
                .unwrap_or_default();
            if value != last_value {
                patch = patch.set("value", value.as_str());
            }

            if let Some(sub) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                sub.rev = rev;
                sub.last_state = state;
                sub.last_value = value;
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
        let mut path = if use_scope { scope.to_vec() } else { Vec::new() };
        walk(start, &mut path)
    }
    /// 整树布局。
    ///
    /// 约束与所有节点的 `layout_version` 都没变时，这次调用是 O(1)：
    /// 直接返回，不进入任何 `measure`。
    pub fn layout(&mut self, viewport: Size, metrics: &dyn TextMetrics) {
        // 响应式：Signal 变过的节点先标脏，再进布局
        self.drain_invalidation();
        let viewport_changed = self.viewport != viewport;
        if viewport_changed {
            self.cache.mark_dirty(self.root.id());
            self.dirty = DirtyRegion::full();
        }
        self.viewport = viewport;
        crate::layout::layout_tree(self.root.as_mut(), viewport, metrics, &mut self.cache);
        if !self.layout_stats().skipped {
            self.rebuild_indices();
            self.enforce_modal_focus();
        }
    }

    /// 布局后重建两套索引：id → rect，以及绘制/命中顺序。
    fn rebuild_indices(&mut self) {
        self.rects.clear();
        self.paint_order.clear();
        self.layer_present = [false; 3];
        self.modal_root = None;

        fn walk(
            node: &dyn Widget,
            path: &mut Vec<usize>,
            inherited: Layer,
            inherited_clip: Option<Rect>,
            rects: &mut HashMap<String, Rect>,
            out: &mut Vec<PaintEntry>,
            layer_present: &mut [bool; 3],
            modal_root: &mut Option<Vec<usize>>,
        ) {
            // 关闭的浮层整棵子树都不进索引（不画、不命中、不可聚焦）
            if !node.is_open() {
                return;
            }
            let own = node.layer();
            let effective = inherited.max(own);
            let rect = node.rect();
            // 可见区 = 自身 rect ∩ 所有祖先 clip（滚动容器靠它约束命中与绘制）
            let clip = match (inherited_clip, node.clip()) {
                (Some(a), Some(b)) => Some(a.intersect(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };

            rects.insert(node.id().to_string(), rect);
            layer_present[effective.rank() as usize] = true;

            if own == Layer::Modal {
                // 后出现的 Modal 在更上层
                *modal_root = Some(path.clone());
            }

            out.push(PaintEntry {
                path: path.clone(),
                rect,
                clip,
                layer: effective,
            });

            for (i, child) in node.children().iter().enumerate() {
                path.push(i);
                walk(
                    child.as_ref(),
                    path,
                    effective,
                    clip,
                    rects,
                    out,
                    layer_present,
                    modal_root,
                );
                path.pop();
            }
        }

        let mut path = Vec::new();
        walk(
            self.root.as_ref(),
            &mut path,
            Layer::Base,
            None,
            &mut self.rects,
            &mut self.paint_order,
            &mut self.layer_present,
            &mut self.modal_root,
        );

        // 稳定排序：同层内保持树序（即绘制先后）
        self.paint_order.sort_by_key(|e| e.layer.rank());

        self.rebuild_focus_order();
    }

    /// 前序遍历一遍，收集所有 focusable 节点；Modal 存在时只看它子树。
    fn rebuild_focus_order(&mut self) {
        fn walk(node: &dyn Widget, path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
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
        if self.dirty.is_empty() {
            return;
        }
        let dirty = self.dirty.clone();

        let mut deferred: Vec<(Layer, Vec<usize>)> = Vec::new();
        {
            let mut path: Vec<usize> = Vec::new();
            render_pass(
                self.root.as_mut(),
                ctx,
                &dirty,
                Layer::Base,
                &mut path,
                &mut deferred,
            );
        }

        // 稳定排序：Base(0) → Overlay(1) → Modal(2)，同层保持树序
        deferred.sort_by_key(|(layer, _)| layer.rank());
        for (_, path) in deferred {
            if let Some(node) = self.node_at_mut(&path) {
                render_subtree(node, ctx, &dirty);
            }
        }

        self.dirty.clear();
    }

    /// 某层是否出现过。
    pub fn has_layer(&self, layer: Layer) -> bool {
        self.layer_present[layer.rank() as usize]
    }

    /// 当前 Modal 作用域（根到 Modal 节点的路径）。
    pub fn modal_scope(&self) -> Option<&[usize]> {
        self.modal_root.as_deref()
    }

    /// 按 id 取矩形（来自最近一次 layout）。
    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.rects.get(id).copied()
    }

    /// 命中位置处最内层的非默认光标形状（应用层据此设置系统光标）。
    pub fn cursor_at(&self, pos: Pos) -> crate::widget::Cursor {
        fn walk(node: &dyn Widget, pos: Pos, inherited: crate::widget::Cursor) -> crate::widget::Cursor {
            let visible = match node.clip() {
                Some(c) => node.rect().intersect(c),
                None => node.rect(),
            };
            if !visible.contains(pos) {
                return inherited;
            }
            let mut found = match node.cursor_at(pos) {
                crate::widget::Cursor::Default => inherited,
                c => c,
            };
            for child in node.children() {
                found = walk(child.as_ref(), pos, found);
            }
            found
        }
        walk(self.root.as_ref(), pos, crate::widget::Cursor::Default)
    }
    /// 命中测试：返回最深的、包含该点的节点 id。
    pub fn hit_test(&self, pos: Pos) -> Option<String> {
        self.hit_index(pos)
            .and_then(|i| self.node_at_ref(&self.paint_order[i].path))
            .map(|n| n.id().to_string())
    }

    /// 命中测试（路径形式，会分配一次 Vec；供 RPC/测试使用）。
    pub fn hit_path(&self, pos: Pos) -> Option<Vec<usize>> {
        self.hit_index(pos)
            .map(|i| self.paint_order[i].path.clone())
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

        self.paint_order
            .iter()
            .rposition(|entry| {
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

    /// 按路径取可变节点引用（O(depth)）。
    fn node_at_mut<'a>(&'a mut self, path: &[usize]) -> Option<&'a mut dyn Widget> {
        let mut node: &mut dyn Widget = self.root.as_mut();
        for &idx in path {
            node = node.children_mut()?.get_mut(idx)?.as_mut();
        }
        Some(node)
    }

    /// 一次遍历求出 id 对应的路径（子索引序列）。
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

    pub fn find(&self, id: &str) -> Option<&dyn Widget> {
        find_in(self.root.as_ref(), id)
    }

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
        // Tab / Shift+Tab：直接走焦点遍历，不再派发给组件
        if let UiEvent::Key { key: crate::event::Key::Tab, modifiers } = event {
            let delta = if modifiers.shift { -1 } else { 1 };
            let moved = self.focus_next(delta);
            return if moved {
                EventResult::Redraw
            } else {
                EventResult::Handled
            };
        }

        // 悬停追踪：只让"旧节点 + 新节点"两块区域变脏。
        if let UiEvent::MouseMove { pos } = event {
            let new = self
                .hit_index(*pos)
                .and_then(|i| self.node_at_ref(&self.paint_order[i].path))
                .map(|n| n.id().to_string());
            if new != self.hovered {
                if let Some(old) = self.hovered.clone() {
                    self.invalidate(&old);
                }
                if let Some(next) = new.clone() {
                    self.invalidate(&next);
                }
                self.hovered = new;
            }
        }

        let positional = matches!(
            event,
            UiEvent::Wheel { .. }
                | UiEvent::MouseDown { .. }
                | UiEvent::MouseUp { .. }
                | UiEvent::MouseMove { .. }
        );

        // 把目标路径搬进复用缓冲：之后不再分配，也不与 &mut self 冲突
        let mut path = std::mem::take(&mut self.path_buf);
        path.clear();

        let hit = match event {
            UiEvent::Wheel { pos, .. }
            | UiEvent::MouseDown { pos, .. }
            | UiEvent::MouseUp { pos, .. }
            | UiEvent::MouseMove { pos, .. } => self.hit_index(*pos),
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
                path.extend_from_slice(&self.paint_order[i].path);
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
            target_id = self
                .node_at_ref(&path)
                .map(|n| n.id().to_string());
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
            if let Some(id) = target_id.as_deref() {
                self.cache.mark_dirty(id);
            }
            self.dirty = DirtyRegion::full();
        } else if result.needs_redraw() {
            match target_id.as_deref() {
                Some(id) => self.invalidate(id),
                None => self.invalidate_all(),
            }
        }

        result
    }
    /// 沿路径做三阶段派发：每个节点只访问一次，O(depth)。
    fn dispatch_path(&mut self, path: &[usize], event: &UiEvent) -> EventResult {
        let viewport = self.viewport;
        let mut ctx = EventCtx::new(self.focus.clone(), viewport);
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
    fn finish_dispatch(&mut self, mut ctx: EventCtx, result: EventResult) -> EventResult {
        let merged = ctx.finish(result);
        self.pending.append(&mut ctx.commands);

        // 组件请求的 IME 候选窗位置：留给应用层下一帧落实
        if ctx.ime_candidate.is_some() {
            self.pending_ime = ctx.ime_candidate;
        }

        if let Some(want) = ctx.focus_request.clone() {
            self.set_focus(Some(want));
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
        ctx.phase = EventPhase::Target;
        let mut acc = EventResult::Ignored;
        broadcast(self.root.as_mut(), &mut ctx, event, &mut acc);
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
                        let _ = self.dispatch_to(
                            &target,
                            &UiEvent::Action {
                                name: action,
                                args,
                            },
                        );
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
    /// actions / props / children，Agent 可以直接消费。
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
                .set("rect", Json::obj()
                    .set("x", rect.x)
                    .set("y", rect.y)
                    .set("w", rect.w)
                    .set("h", rect.h))
                .set("focused", focus == Some(node.id()))
                .set("label", sem.label.clone())
                .set("value", sem.value.clone())
                .set("actions", Json::Arr(sem.actions.iter().map(|a| a.to_json()).collect()))
                .set("props", props)
                .set("state", sem.state.clone())
                .set("revision", node.revision())
                .set("disabled", sem.disabled)
                .set("children", Json::Arr(
                    node.children().iter().map(|c| node_json(c.as_ref(), focus)).collect()
                ))
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
        walk(self.root.as_ref(), self.focus.as_deref())
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
        Some(children) => children.iter_mut().find_map(|c| find_in_mut(c.as_mut(), id)),
        None => None,
    }
}

fn broadcast(
    node: &mut dyn Widget,
    ctx: &mut EventCtx,
    event: &UiEvent,
    acc: &mut EventResult,
) {
    *acc = acc.merge(node.event(ctx, event));
    if let Some(children) = node.children_mut() {
        for child in children.iter_mut() {
            broadcast(child.as_mut(), ctx, event, acc);
        }
    }
}

/// 一个绘制/命中条目：路径 + 包围盒 + 生效层级。
#[derive(Clone, Debug)]
struct PaintEntry {
    path: Vec<usize>,
    rect: Rect,
    /// 屏幕可见区 = rect ∩ 祖先 clip；命中与裁剪共用。
    clip: Option<Rect>,
    layer: Layer,
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
    if child_stopped {
        return (result, true);
    }

    // 返回：Bubble
    ctx.phase = EventPhase::Bubble;
    let r = node.event(ctx, event);
    result = result.merge(r);
    let stop = ctx.stop_propagation || (r.is_handled() && !ctx.propagate);
    (result, stop)
}

/// 单遍渲染：Base 就地画，Overlay/Modal 记路径延后画。
fn render_pass(
    node: &mut dyn Widget,
    ctx: &mut dyn RenderCtx,
    dirty: &DirtyRegion,
    inherited: Layer,
    path: &mut Vec<usize>,
    deferred: &mut Vec<(Layer, Vec<usize>)>,
) {
    let effective = inherited.max(node.layer());

    if effective == Layer::Base {
        if node.needs_paint(dirty) {
            node.render(ctx);
        }
        // 滚动容器：子节点只能画在视口内
        let clip = node.clip();
        if let Some(c) = clip {
            ctx.push_clip(c);
        }
        if let Some(children) = node.children_mut() {
            for (i, child) in children.iter_mut().enumerate() {
                path.push(i);
                render_pass(child.as_mut(), ctx, dirty, effective, path, deferred);
                path.pop();
            }
        }
        if clip.is_some() {
            ctx.pop_clip();
        }
    } else {
        // 整个子树延后：只记路径，稍后按层统一回放
        deferred.push((effective, path.clone()));
    }
}

/// 回放一个延后的子树（层内顺序 = 树序）。
fn render_subtree(node: &mut dyn Widget, ctx: &mut dyn RenderCtx, dirty: &DirtyRegion) {
    if node.needs_paint(dirty) {
        node.render(ctx);
    }
    let clip = node.clip();
    if let Some(c) = clip {
        ctx.push_clip(c);
    }
    if let Some(children) = node.children_mut() {
        for child in children.iter_mut() {
            render_subtree(child.as_mut(), ctx, dirty);
        }
    }
    if clip.is_some() {
        ctx.pop_clip();
    }
}

/// 一条语义订阅。
struct Subscription {
    id: String,
    /// 上次看到的指纹。
    rev: u64,
    /// 上次推送时的状态快照（用于差分）。
    last_state: crate::json::Json,
    /// 上次推送时的 value（没有结构化 state 的组件靠它感知变化）。
    last_value: String,
    /// 最小推送间隔。
    interval: std::time::Duration,
    /// 下次允许检查的时间点。
    next_due: std::time::Instant,
}

/// 节点状态指纹：revision + value + state。用于低成本判断"变没变"。
fn fingerprint(node: &dyn Widget) -> u64 {
    fn fnv1a(bytes: &[u8], mut hash: u64) -> u64 {
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        hash
    }

    let sem = node.semantics();
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    h = fnv1a(&node.revision().to_le_bytes(), h);
    h = fnv1a(sem.value.as_bytes(), h);
    h = fnv1a(sem.state.to_string().as_bytes(), h);
    h
}

fn patch_is_empty(patch: &crate::json::Json) -> bool {
    matches!(patch, crate::json::Json::Obj(map) if map.is_empty())
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
/// 递归调用卸载钩子：先释放子树资源，再释放自己。
fn unmount_recursive(mut node: Box<dyn Widget>) {
    if let Some(children) = node.children_mut() {
        let taken: Vec<Box<dyn Widget>> = children.drain(..).collect();
        for child in taken {
            unmount_recursive(child);
        }
    }
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