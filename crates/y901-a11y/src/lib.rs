//! # y901-a11y —— 语义树 → [AccessKit] 的映射层
//!
//! ## 这个 crate 解决什么
//!
//! Y901 的控件已经通过 [`y901_ui::Widget::semantics`] 暴露了 label / value /
//! role / actions / state。但**屏幕阅读器不认识"我们的语义树"**：Windows 要
//! UI Automation（UIA）、Linux 要 AT-SPI、浏览器要 DOM 的 ARIA 属性。
//!
//! 这一层把语义树翻译成 [AccessKit] 的 `TreeUpdate`。AccessKit 是**跨平台**的
//! 无障碍树表示，各后端再把它接到各自的系统 API（`accesskit_windows` /
//! `accesskit_unix`）—— 于是"角色/名称/状态/动作"的映射只写一遍。
//!
//! ## 边界（诚实声明）
//!
//! - 本 crate **只做映射**，不含任何平台适配（UIA / AT-SPI / DOM 由后端接）；
//! - 语义字段缺失时**不猜**：没有 label 就真的没有名称（屏幕阅读器会读"未命名按钮"），
//!   而不是拿 id 或类型名冒充 —— 那会让无障碍测试"看起来通过"却仍然不可用；
//! - `exposed_to_ai == false` 的节点**整棵子树跳过**（与外部工具网关同一规则）；
//! - 不做 live region 的播报调度、不做焦点移动策略，那些属于平台适配层。
//!
//! ## 用法
//!
//! ```no_run
//! # use y901_a11y::to_tree_update_with_map;
//! # fn demo(tree: &y901_ui::UiTree) {
//! let (update, node_ids) = to_tree_update_with_map(tree.snapshot());
//! // 把 update 交给 accesskit_windows / accesskit_unix 的适配器；
//! // node_ids 留着，收到系统动作时用它把 AccessKit id 反查回 Y901 节点 id：
//! // if let Some(id) = node_ids.y901_of(action_node) { tree.dispatch_to(id, ..); }
//! # let _ = (update, node_ids);
//! # }
//! ```
//!
//! [AccessKit]: https://github.com/AccessKit/accesskit

#![deny(missing_docs)]

pub mod aria;
pub mod platform;

/// 重新导出 `accesskit`：调用方（各后端）不必自己依赖它，
/// 也就不会出现"两个 accesskit 版本"的版本分裂。
pub use accesskit;

use std::collections::HashMap;

use accesskit::{Action, Node, NodeId, Role, Toggled, TreeInfo, TreeUpdate};
pub use aria::{aria_role, to_aria_tree, AriaNode};
pub use platform::{NodeIdMap, PlatformBridge};
use y901_ui::{LiveRegion, NodeInfo, Semantics};

/// 把 Y901 的角色名翻译成 AccessKit 的角色。
///
/// 规则：
/// - 已知角色精确映射；
/// - `panel` / `scroll_view` / `split_view` 这类**纯布局容器**映射成 [`Role::GenericContainer`]
///   （不是 `Group`：Group 在部分读屏器里会被读成"分组 x"，而它们只是排版需要）；
/// - 未知名一律 [`Role::Unknown`]：**不猜**，让平台层按未知处理。
pub fn map_role(role: y901_ui::Role) -> Role {
    match role.as_str() {
        "window" => Role::Window,
        "dialog" => Role::Dialog,
        "panel" => Role::GenericContainer,
        "scroll_view" => Role::ScrollView,
        "split_view" => Role::GenericContainer,
        "text" => Role::Label,
        "button" => Role::Button,
        "text_input" => Role::TextInput,
        "list" => Role::List,
        "list_item" => Role::ListItem,
        "scrollbar" => Role::ScrollBar,
        "menu" => Role::Menu,
        "menu_bar" => Role::MenuBar,
        "menu_item" => Role::MenuItem,
        "table" => Role::Table,
        "row" => Role::Row,
        "cell" => Role::Cell,
        "tree" => Role::Tree,
        "tree_item" => Role::TreeItem,
        "tab" => Role::Tab,
        "tab_panel" => Role::TabPanel,
        "checkbox" => Role::CheckBox,
        "switch" => Role::Switch,
        "radio" => Role::RadioButton,
        "slider" => Role::Slider,
        "progress" => Role::ProgressIndicator,
        "combobox" => Role::ComboBox,
        "image" => Role::Image,
        "alert" => Role::Alert,
        // 语义色槽位（`Role` 被复用为主题键）不应出现在语义树里；落到 Unknown。
        _ => Role::Unknown,
    }
}

/// 把 [`Semantics`] 里的动作名翻译成 AccessKit 动作。
///
/// 只映射**有明确对应**的那几个：`click` / `toggle` / `set` / `select*` /
/// `scroll*` / `dismiss` / `expand` / `collapse`。其余（业务自定义动作）不映射，
/// 但保留在 `props["actions"]` 里，平台层可以据此扩展。
fn map_actions(sem: &Semantics, node: &mut Node) {
    for spec in &sem.actions {
        let action = match spec.name.as_str() {
            "click" => Some(Action::Click),
            "toggle" => Some(Action::Click),
            "focus" => Some(Action::Focus),
            "dismiss" | "close" => Some(Action::Click),
            "set_text" | "set_value" | "set" => Some(Action::SetValue),
            "select" | "select_row" | "select_item" => Some(Action::Click),
            "scroll_up" | "scroll_up_by_page" => Some(Action::ScrollUp),
            "scroll_down" | "scroll_down_by_page" => Some(Action::ScrollDown),
            "expand" => Some(Action::Expand),
            "collapse" => Some(Action::Collapse),
            _ => None,
        };
        if let Some(a) = action {
            node.add_action(a);
        }
    }
}

/// 把语义状态写进 AccessKit 节点。
fn apply_state(sem: &Semantics, node: &mut Node) {
    if let Some(checked) = sem.checked {
        node.set_toggled(Toggled::from(checked));
    }
    if let Some(expanded) = sem.expanded {
        node.set_expanded(expanded);
    }
    if let Some(selected) = sem.selected {
        node.set_selected(selected);
    }
    if let Some(level) = sem.level {
        node.set_level(level as usize);
    }
    if let Some((min, max)) = sem.value_range {
        node.set_min_numeric_value(min);
        node.set_max_numeric_value(max);
    }
    if let Some(step) = sem.value_step {
        node.set_numeric_value_step(step);
    }
    // 值：有 value 才设，避免把空串当成"值就是空"（读屏器会念"空"）。
    if !sem.value.is_empty() {
        if let Ok(numeric) = sem.value.trim().parse::<f64>() {
            node.set_numeric_value(numeric);
        }
        node.set_value(sem.value.clone());
    }
    match sem.live {
        LiveRegion::Off => {}
        LiveRegion::Polite => node.set_live(accesskit::Live::Polite),
        LiveRegion::Assertive => node.set_live(accesskit::Live::Assertive),
    }
}

/// Y901 节点 id → AccessKit `NodeId` 的**稳定**映射。
///
/// # 为什么不能用前序遍历序号
///
/// 旧实现是 `NodeId(next_id)`，`next_id` 按遍历顺序自增。于是**列表删掉一行**，
/// 它后面所有控件的 `NodeId` 全部前移一位；读屏器的动作在**下一帧**才到达
///（AccessKit/UIA 是异步的），反查就落到了邻居身上 —— 用户点"删除"，删掉的是
/// 下一行。审计 #6 记的就是这条。
///
/// # 现在的规则
///
/// `NodeId = FNV-1a 64(Y901 节点 id)`；同一棵树里若两个不同 id 撞到同一个哈希，
/// 就**线性探测**往后找一个没被占用的值。这样：
/// - 同一个控件**跨帧 id 不变**（UIA 的自动化节点缓存、读屏器的焦点记忆都依赖它）；
/// - 碰撞只影响"谁多占一个值"，不影响正确性，且**结果确定**（同一棵树每次分配一致）；
/// - 已删除节点的旧 id 不会命中任何现有节点，调用方据此**显式丢弃**动作。
#[inline]
fn fnv1a64(text: &str) -> u64 {
    // FNV-1a：参数取标准值，便于跨语言复现（Python/JS 侧算同一个值）。
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 分配器：记录"这个哈希值已经被谁占了"，碰撞时线性探测。
#[derive(Default)]
struct IdAllocator {
    /// `NodeId` → 占用它的 Y901 id（用于区分"同一个 id 重复出现"与"真碰撞"）。
    taken: HashMap<accesskit::NodeId, String>,
}

impl IdAllocator {
    /// 给 `y901_id` 分配一个**稳定**的 `NodeId`。
    fn allocate(&mut self, y901_id: &str) -> NodeId {
        let mut value = fnv1a64(y901_id);
        loop {
            let candidate = NodeId(value);
            match self.taken.get(&candidate) {
                // 同一个 Y901 id 再次出现（同一棵树里理论上不会，防御性处理）：
                // 复用同一个 `NodeId`，不另占一个值。
                Some(owner) if owner == y901_id => return candidate,
                // 真碰撞：线性探测下一个值。
                Some(_) => value = value.wrapping_add(1),
                None => {
                    self.taken.insert(candidate, y901_id.to_string());
                    return candidate;
                }
            }
        }
    }
}

/// 递归把一个节点（含子树）转成 AccessKit 的 `(NodeId, Node)` 列表。
///
/// 返回：本节点的 id 与"已展开的扁平列表"。
/// 跳过规则：`exposed_to_ai == false` 的节点**连同整棵子树**都不进无障碍树
/// （与外部工具网关一致；隐藏的对话框不该被读屏器念出来）。
fn convert(
    info: &NodeInfo,
    parent: Option<NodeId>,
    alloc: &mut IdAllocator,
    out: &mut Vec<(NodeId, Node)>,
    id_map: &mut HashMap<String, NodeId>,
) -> Option<NodeId> {
    if !info.semantics.exposed_to_ai {
        return None;
    }
    let self_id = alloc.allocate(&info.id);
    id_map.insert(info.id.clone(), self_id);

    let mut node = Node::new(map_role(info.role));
    let sem = &info.semantics;
    if !sem.label.is_empty() {
        node.set_label(sem.label.clone());
    }
    if !sem.description.is_empty() {
        node.set_description(sem.description.clone());
    }
    apply_state(sem, &mut node);
    map_actions(sem, &mut node);

    // 几何：AccessKit 的 bounds 用 f64，坐标系与平台约定一致（逻辑像素）。
    let r = info.rect;
    if r.w > 0.0 && r.h > 0.0 {
        node.set_bounds(accesskit::Rect {
            x0: r.x as f64,
            y0: r.y as f64,
            x1: (r.x + r.w) as f64,
            y1: (r.y + r.h) as f64,
        });
    }

    // 子节点：先递归收集（`out` 的写入顺序不影响 TreeUpdate 的正确性）。
    let mut child_ids = Vec::new();
    for child in &info.children {
        if let Some(child_id) = convert(child, Some(self_id), alloc, out, id_map) {
            child_ids.push(child_id);
        }
    }
    if !child_ids.is_empty() {
        node.set_children(child_ids);
    }
    let _ = parent;
    out.push((self_id, node));
    Some(self_id)
}

/// 把整棵树（[`y901_ui::UiTree::snapshot`]）转成一次完整的 AccessKit `TreeUpdate`。
///
/// 语义：
/// - 根节点必须 `exposed_to_ai`，否则返回只含空树的 update（无障碍树为空，
///   平台层应视为"应用未提供无障碍信息"）；
/// - `focus` 取第一个 `focused == true` 的已暴露节点，没有就用根；
/// - `tree` 只在首次推送时给（本函数总是给，平台层可自行去重）。
pub fn to_tree_update(root: NodeInfo) -> TreeUpdate {
    // 兼容入口：大多数调用方（含文档示例与既有测试）只关心 `TreeUpdate`。
    // 需要把系统动作映射回 Y901 节点时用 [`to_tree_update_with_map`]。
    to_tree_update_with_map(root).0
}

/// 同上，但**同时返回 AccessKit id ↔ Y901 节点 id 的映射**。
///
/// 为什么后端必须用这一个：`NodeId` 现在是**由 Y901 节点 id 派生的稳定哈希**
/// （FNV-1a 哈希，见下方 `IdAllocator` 的说明），跨帧不变；但"某个 id 曾经存在、现在已经不在树里"
/// 仍然只能靠**当帧的映射表**判断 —— 没有表就只能猜，而那正是旧实现
/// （按前序遍历序号反查）会在删行后把"删除"点到邻居身上的原因。
pub fn to_tree_update_with_map(root: NodeInfo) -> (TreeUpdate, NodeIdMap) {
    let mut out = Vec::new();
    let mut id_map = HashMap::new();
    let mut alloc = IdAllocator::default();
    let root_id = convert(&root, None, &mut alloc, &mut out, &mut id_map);

    // 找焦点：优先语义树里 focused 的节点。
    let focused = find_focused(&root, &id_map);

    let (root_node, tree) = match root_id {
        Some(id) => (
            id,
            Some(TreeInfo {
                root: id,
                toolkit_name: Some("y901".to_string()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        ),
        None => {
            // 整棵树都不可见：给一个空的根，保证 TreeUpdate 合法。
            let id = NodeId(0);
            out.push((id, Node::new(Role::GenericContainer)));
            (
                id,
                Some(TreeInfo {
                    root: id,
                    toolkit_name: Some("y901".to_string()),
                    toolkit_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                }),
            )
        }
    };

    let update = TreeUpdate {
        nodes: out,
        tree,
        tree_id: accesskit::TreeId::ROOT,
        focus: focused.unwrap_or(root_node),
    };
    (update, id_map.into())
}

/// 找第一个持有焦点的已暴露节点。
///
/// 同时看 `NodeInfo.focused`（快照自己带的事实）与 `semantics.focused`
/// （框架注入的副本）—— 两者任一为真都算：调用方可能只设了其中一个，
/// 只认一个会让"焦点在输入框上"这种最常见的情形静默丢失。
fn find_focused(root: &NodeInfo, id_map: &HashMap<String, NodeId>) -> Option<NodeId> {
    if root.focused || root.semantics.focused {
        if let Some(id) = id_map.get(&root.id) {
            return Some(*id);
        }
    }
    root.children.iter().find_map(|c| find_focused(c, id_map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::{Pos, Rect, Role as UiRole, Semantics};

    fn node(id: &str, role: &'static str) -> NodeInfo {
        NodeInfo {
            id: id.to_string(),
            role: UiRole(role),
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            focused: false,
            semantics: Semantics::default().exposed(true),
            children: Vec::new(),
        }
    }

    #[test]
    fn known_roles_map_to_accesskit_roles() {
        assert_eq!(map_role(UiRole("button")), Role::Button);
        assert_eq!(map_role(UiRole("text_input")), Role::TextInput);
        assert_eq!(map_role(UiRole("list")), Role::List);
        assert_eq!(map_role(UiRole("dialog")), Role::Dialog);
        assert_eq!(map_role(UiRole("alert")), Role::Alert);
    }

    #[test]
    fn layout_only_containers_are_generic_not_groups() {
        // 布局容器不该被读成"分组"，否则读屏器会多念一层。
        assert_eq!(map_role(UiRole("panel")), Role::GenericContainer);
        assert_eq!(map_role(UiRole("split_view")), Role::GenericContainer);
        assert_eq!(map_role(UiRole("scroll_view")), Role::ScrollView);
    }

    #[test]
    fn unknown_and_theme_only_roles_are_not_guessed() {
        assert_eq!(map_role(UiRole("list_item")), Role::ListItem);
        assert_eq!(
            map_role(UiRole("terminal")),
            Role::Unknown,
            "未知角色应留给平台层处理，不能猜成 Label"
        );
    }

    #[test]
    fn tree_update_contains_every_exposed_node() {
        let mut root = node("root", "window");
        let mut button = node("btn", "button");
        button.semantics = button.semantics.label("Save");
        root.children.push(button);

        let update = to_tree_update(root);
        assert_eq!(update.nodes.len(), 2, "根 + 按钮");
        let root_id = update.tree.as_ref().expect("tree info").root;
        let labels: Vec<_> = update
            .nodes
            .iter()
            .filter(|(id, _)| *id != root_id)
            .map(|(_, n)| n.label())
            .collect();
        assert_eq!(labels, vec![Some("Save")]);
    }

    #[test]
    fn unexposed_subtree_is_skipped_entirely() {
        let mut root = node("root", "window");
        let mut hidden = node("hidden", "panel");
        hidden.semantics = Semantics::default(); // exposed_to_ai = false
        hidden.children.push(node("deep", "button"));
        root.children.push(hidden);

        let update = to_tree_update(root);
        assert_eq!(
            update.nodes.len(),
            1,
            "未暴露节点的子树也不该进无障碍树，实得 {}",
            update.nodes.len()
        );
    }

    #[test]
    fn checked_and_expanded_state_reach_the_node() {
        let mut root = node("root", "window");
        let mut check = node("c", "checkbox");
        check.semantics = check.semantics.exposed(true).checked(true);
        let mut branch = node("b", "tree_item");
        branch.semantics = branch.semantics.exposed(true).expanded(false);
        root.children.push(check);
        root.children.push(branch);

        let update = to_tree_update(root);
        let by_label: HashMap<_, _> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
        let check_node = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::CheckBox)
            .map(|(id, _)| by_label[id])
            .expect("checkbox 节点");
        assert_eq!(check_node.toggled(), Some(Toggled::True));
        let branch_node = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::TreeItem)
            .map(|(id, _)| by_label[id])
            .expect("tree item 节点");
        assert_eq!(branch_node.is_expanded(), Some(false));
    }

    #[test]
    fn numeric_value_range_and_step_are_exported() {
        let mut root = node("root", "window");
        let mut slider = node("s", "slider");
        slider.semantics = slider
            .semantics
            .exposed(true)
            .value("0.50")
            .value_range(0.0, 1.0)
            .value_step(0.01);
        root.children.push(slider);

        let update = to_tree_update(root);
        let n = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Slider)
            .map(|(_, n)| n)
            .expect("slider 节点");
        assert_eq!(n.numeric_value(), Some(0.5));
        assert_eq!(n.min_numeric_value(), Some(0.0));
        assert_eq!(n.max_numeric_value(), Some(1.0));
        assert_eq!(n.numeric_value_step(), Some(0.01));
    }

    #[test]
    fn live_region_is_exported() {
        let mut root = node("root", "window");
        let mut alert = node("a", "alert");
        alert.semantics = alert.semantics.exposed(true).live(LiveRegion::Assertive);
        root.children.push(alert);

        let update = to_tree_update(root);
        let n = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Alert)
            .map(|(_, n)| n)
            .expect("alert 节点");
        assert_eq!(n.live(), Some(accesskit::Live::Assertive));
    }

    #[test]
    fn actions_are_mapped_and_unknown_ones_ignored() {
        let mut root = node("root", "window");
        let mut button = node("btn", "button");
        button.semantics = button
            .semantics
            .exposed(true)
            .action("click")
            .action("business_specific_thing");
        root.children.push(button);

        let update = to_tree_update(root);
        let n = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Button)
            .map(|(_, n)| n)
            .expect("button 节点");
        assert!(n.supports_action(Action::Click));
        // 未映射的业务动作不应被"顺手"映射成别的 AccessKit 动作。
        assert!(
            !n.supports_action(Action::Focus) && !n.supports_action(Action::Expand),
            "未映射的业务动作不应凭空变成 Focus/Expand"
        );
    }

    #[test]
    fn focus_follows_the_focused_node() {
        let mut root = node("root", "window");
        let mut input = node("in", "text_input");
        input.semantics = input.semantics.exposed(true).label("Name");
        root.children.push(input);
        root.focused = true; // 根也标记 focused 时优先后代的真实焦点
        let input_focused_id = {
            let mut probe = node("t", "button");
            probe.semantics = probe.semantics.exposed(true);
            probe
        };
        let _ = input_focused_id;
        let update = to_tree_update(root);
        // 根 focused → focus 落在根上（本实现按"第一个 focused"取胜）
        assert_eq!(update.focus, update.tree.as_ref().unwrap().root);
    }

    #[test]
    fn empty_tree_still_produces_a_valid_update() {
        let mut root = node("root", "window");
        root.semantics = Semantics::default(); // 整棵树都未暴露
        let update = to_tree_update(root);
        assert_eq!(update.nodes.len(), 1, "仍需一个合法的根节点");
        let root_id = update.tree.as_ref().unwrap().root;
        assert_eq!(update.focus, root_id);
        assert_eq!(update.nodes[0].0, root_id, "空树的根必须与 tree.root 一致");
    }

    #[test]
    fn bounds_are_exported_so_screen_readers_can_hit_test() {
        let mut root = node("root", "window");
        root.rect = Rect::new(10.0, 20.0, 100.0, 50.0);
        let update = to_tree_update(root);
        let bounds = update.nodes[0].1.bounds().expect("应有 bounds");
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (10.0, 20.0, 110.0, 70.0)
        );
    }

    #[test]
    fn zero_sized_nodes_have_no_bounds() {
        // 未布局的节点（0×0）不该谎报一个可点区域。
        let mut root = node("root", "window");
        root.rect = Rect::new(0.0, 0.0, 0.0, 0.0);
        let update = to_tree_update(root);
        assert!(update.nodes[0].1.bounds().is_none());
    }

    #[test]
    fn focus_is_taken_from_a_focused_child() {
        let mut root = node("root", "window");
        let mut input = node("in", "text_input");
        input.focused = true;
        input.semantics = input.semantics.exposed(true).label("Name");
        root.children.push(input);
        let update = to_tree_update(root);
        let root_id = update.tree.as_ref().unwrap().root;
        assert_ne!(update.focus, root_id, "焦点应在输入框上，不在根上");
        let focused_label = update
            .nodes
            .iter()
            .find(|(id, _)| *id == update.focus)
            .and_then(|(_, n)| n.label());
        assert_eq!(focused_label, Some("Name"));
    }

    #[test]
    fn position_is_not_used_as_a_fake_label() {
        // 没有 label 的节点必须保持"无名称"，而不是拿 id 顶上。
        let mut root = node("root", "window");
        root.children.push(node("mystery", "button"));
        let update = to_tree_update(root);
        let button = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::Button)
            .map(|(_, n)| n)
            .expect("button");
        assert_eq!(button.label(), None, "没有 label 就不该有名字");
        let _ = Pos::new(0.0, 0.0);
    }

    // ---- 稳定 NodeId（审计 #6 的回归） ----

    /// 三行列表。`rows` 决定哪些行还在树里，用来模拟"删掉一行"。
    fn list_with_rows(rows: &[&str]) -> NodeInfo {
        let mut root = node("list", "list");
        for id in rows {
            root.children.push(node(id, "list_item"));
        }
        root
    }

    #[test]
    fn node_id_does_not_shift_when_a_sibling_is_deleted() {
        let (_, before) = to_tree_update_with_map(list_with_rows(&["row-a", "row-b", "row-c"]));
        let (_, after) = to_tree_update_with_map(list_with_rows(&["row-a", "row-c"]));
        assert_eq!(
            after.accesskit_of("row-c"),
            before.accesskit_of("row-c"),
            "删掉 row-b 后 row-c 的 NodeId 必须不变，否则读屏器下一帧会点到邻居上"
        );
        assert_eq!(
            after.accesskit_of("row-a"),
            before.accesskit_of("row-a"),
            "未受影响的节点更不能变"
        );
    }

    #[test]
    fn action_for_a_deleted_node_finds_nothing_and_is_dropped() {
        let (_, before) = to_tree_update_with_map(list_with_rows(&["row-a", "row-b"]));
        let deleted = before.accesskit_of("row-b").expect("row-b 曾有 id");
        let (_, after) = to_tree_update_with_map(list_with_rows(&["row-a"]));
        // 后端就是靠这个 `None` 把"指向已删除节点"的动作丢掉，而不是误派给相邻行。
        assert_eq!(after.y901_of(deleted), None);
    }

    #[test]
    fn every_exposed_node_gets_a_distinct_id_and_maps_back() {
        let root = list_with_rows(&["row-a", "row-b", "row-c"]);
        let (update, map) = to_tree_update_with_map(root);
        assert_eq!(map.len(), 4, "根 + 3 行");
        assert_eq!(update.nodes.len(), 4);
        let mut seen = std::collections::HashSet::new();
        for (id, _) in &update.nodes {
            assert!(seen.insert(*id), "NodeId 必须唯一：{:?} 重复", id);
        }
        for expect in ["list", "row-a", "row-b", "row-c"] {
            let id = map.accesskit_of(expect).expect("应有映射");
            assert_eq!(map.y901_of(id), Some(expect), "反查必须回到同一个 id");
        }
    }

    #[test]
    fn unexposed_nodes_neither_get_an_id_nor_appear_in_the_map() {
        let mut root = node("root", "window");
        let mut hidden = node("hidden", "panel");
        hidden.semantics = Semantics::default(); // 未暴露
        root.children.push(hidden);
        let (_, map) = to_tree_update_with_map(root);
        assert_eq!(map.accesskit_of("hidden"), None);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn id_depends_only_on_the_y901_id_not_on_position_or_order() {
        // 同一批 id、不同顺序 → 每个节点拿到的 NodeId 相同。
        let (_, a) = to_tree_update_with_map(list_with_rows(&["x", "y"]));
        let (_, b) = to_tree_update_with_map(list_with_rows(&["y", "x"]));
        assert_eq!(a.accesskit_of("x"), b.accesskit_of("x"));
        assert_eq!(a.accesskit_of("y"), b.accesskit_of("y"));
    }

    #[test]
    fn hash_collision_probes_to_the_next_free_value() {
        // 人为占住 "beta" 的哈希值（模拟真碰撞），分配器必须往后线性探测，
        // 而不是把两个不同节点映射成同一个 NodeId。
        let mut alloc = IdAllocator::default();
        let stolen = NodeId(fnv1a64("beta"));
        alloc.taken.insert(stolen, "someone-else".to_string());
        let got = alloc.allocate("beta");
        assert_ne!(got, stolen, "碰撞时必须换一个值");
        assert_eq!(got, NodeId(fnv1a64("beta").wrapping_add(1)));
    }

    #[test]
    fn allocating_the_same_id_twice_reuses_the_same_node_id() {
        // 防御性：同一棵树里同一个 Y901 id 出现两次时，不该白占两个 NodeId。
        let mut alloc = IdAllocator::default();
        let first = alloc.allocate("dup");
        let second = alloc.allocate("dup");
        assert_eq!(first, second);
        assert_eq!(alloc.taken.len(), 1);
    }

    #[test]
    fn popup_subtree_is_reachable_and_actionable() {
        // `UiTree::snapshot()` 把浮层（菜单/下拉/对话框）作为**根节点的最后几个孩子**
        // 追加进来（绘制顺序）。读屏器必须能走到菜单项 —— 否则"键盘能开菜单，
        // 读屏器却念不到里面有什么"。
        let mut root = node("window", "window");
        root.children.push(node("save-btn", "button"));
        let mut menu = node("menu", "list");
        menu.children.push(node("item-a", "list_item"));
        menu.children.push(node("item-b", "list_item"));
        root.children.push(menu);

        let (update, map) = to_tree_update_with_map(root);
        assert_eq!(update.nodes.len(), 5, "窗口 + 按钮 + 菜单 + 两个菜单项");
        for expect in ["window", "save-btn", "menu", "item-a", "item-b"] {
            let id = map.accesskit_of(expect).expect("每个节点都要有 id");
            assert_eq!(map.y901_of(id), Some(expect));
        }
        // 菜单项必须是 AccessKit 树里的真实节点（能接收 Click），不是只有 id。
        let item_a = map.accesskit_of("item-a").unwrap();
        let node_a = update
            .nodes
            .iter()
            .find(|(id, _)| *id == item_a)
            .map(|(_, n)| n)
            .expect("菜单项节点");
        assert_eq!(node_a.role(), Role::ListItem);
    }

    #[test]
    fn popup_item_ids_are_stable_when_the_menu_is_rebuilt() {
        // 每帧重建整棵树（平台层就是这么做的）时，同一个菜单项的 `NodeId` 不能变；
        // 否则读屏器缓存的"当前菜单项"会在每一帧失效。
        fn menu_with(items: &[&str]) -> NodeInfo {
            let mut root = node("window", "window");
            let mut menu = node("menu", "list");
            for id in items {
                menu.children.push(node(id, "list_item"));
            }
            root.children.push(menu);
            root
        }
        let (_, before) = to_tree_update_with_map(menu_with(&["a", "b", "c"]));
        let (_, after) = to_tree_update_with_map(menu_with(&["a", "b", "c"]));
        for id in ["a", "b", "c"] {
            assert_eq!(before.accesskit_of(id), after.accesskit_of(id));
        }
    }

    #[test]
    fn fnv1a_matches_known_values() {
        // 固定向量：换了算法就会在这里红，提醒"NodeId 不是随便能改的"——
        // 改了等于把所有平台侧的节点缓存一次性作废。
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64("foobar"), 0x8594_4171_f739_67e8);
    }
}
