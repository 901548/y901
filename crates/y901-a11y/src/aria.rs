//! **ARIA 镜像**：把语义树映射成 DOM 可用的角色与 `aria-*` 属性。
//!
//! ## 为什么 Web 端不能直接用 AccessKit
//!
//! Windows 走 UIA、Linux 走 AT-SPI，两者都能接收一棵"无障碍树"对象。浏览器不一样：
//! 读屏器只认 **DOM**。canvas 里画出来的东西对 DOM 是不存在的，所以必须把语义树
//! **镜像**成一份隐藏的 DOM —— 属性名与取值规则就是 WAI-ARIA。
//!
//! 本模块只做**纯映射**（角色名 + 属性列表 + 树结构），不碰 `web_sys`/`bindings`：
//! 这样它能在任何平台上单测，浏览器后端只负责"把结果写进 DOM"。
//!
//! ## 明确的边界
//!
//! - 只输出**静态**属性，不处理焦点移动、事件转发、live region 的播报调度（DOM 属性
//!   `aria-live` 交给浏览器）；
//! - canvas 缺少"文本光标位置"的语义，输入框只能暴露 `aria-valuetext` 而不是真实光标；
//! - 不做 `aria-owns` 等跨树引用（我们的语义树本身就是一棵树）。

use y901_ui::{LiveRegion, NodeInfo, Semantics};

/// 一个镜像到 DOM 的节点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AriaNode {
    /// Y901 节点 id（写进 DOM 的 `data-y901-id`，便于反查与调试）。
    pub id: String,
    /// WAI-ARIA 角色名（`button` / `list` / `dialog` …）。
    pub role: &'static str,
    /// 要写到该元素上的属性（名, 值）。已按固定顺序排列，便于快照比对。
    pub attrs: Vec<(String, String)>,
    /// 子节点（顺序即 DOM 顺序）。
    pub children: Vec<AriaNode>,
}

impl AriaNode {
    /// 取某个属性的值（测试与调试用）。
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Y901 角色 → WAI-ARIA 角色。
///
/// 与 AccessKit 映射（[`crate::map_role`]）**分开维护**：两边角色集合并不相同
/// （ARIA 没有 `window`，AccessKit 没有 `application` 之外的等价物），
/// 硬合并会让一边永远用错角色。
pub fn aria_role(role: y901_ui::Role) -> &'static str {
    match role.as_str() {
        "window" => "application",
        "dialog" => "dialog",
        "panel" | "split_view" => "group",
        "scroll_view" => "group",
        "text" => "text",
        "button" => "button",
        "text_input" => "textbox",
        "list" => "list",
        "list_item" => "listitem",
        "scrollbar" => "scrollbar",
        "menu" => "menu",
        "menu_bar" => "menubar",
        "menu_item" => "menuitem",
        "table" => "table",
        "row" => "row",
        "cell" => "cell",
        "tree" => "tree",
        "tree_item" => "treeitem",
        "tab" => "tab",
        "tab_panel" => "tabpanel",
        "checkbox" => "checkbox",
        "switch" => "switch",
        "radio" => "radio",
        "slider" => "slider",
        "progress" => "progressbar",
        "combobox" => "combobox",
        "image" => "img",
        "alert" => "alert",
        // 未知角色落到 "generic"：ARIA 里没有 "unknown"，generic 是最保守的选择。
        _ => "generic",
    }
}

/// 把语义状态翻译成 `aria-*` 属性。
fn aria_attrs(sem: &Semantics) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if !sem.label.is_empty() {
        out.push(("aria-label".into(), sem.label.clone()));
    }
    if !sem.description.is_empty() {
        out.push(("aria-description".into(), sem.description.clone()));
    }
    if !sem.value.is_empty() {
        out.push(("aria-valuetext".into(), sem.value.clone()));
    }
    if sem.disabled {
        out.push(("aria-disabled".into(), "true".into()));
    }
    if let Some(checked) = sem.checked {
        out.push(("aria-checked".into(), checked.to_string()));
    }
    if let Some(expanded) = sem.expanded {
        out.push(("aria-expanded".into(), expanded.to_string()));
    }
    if let Some(selected) = sem.selected {
        out.push(("aria-selected".into(), selected.to_string()));
    }
    if let Some(level) = sem.level {
        out.push(("aria-level".into(), level.to_string()));
    }
    if let Some((min, max)) = sem.value_range {
        out.push(("aria-valuemin".into(), min.to_string()));
        out.push(("aria-valuemax".into(), max.to_string()));
    }
    match sem.live {
        LiveRegion::Off => {}
        LiveRegion::Polite => out.push(("aria-live".into(), "polite".into())),
        LiveRegion::Assertive => out.push(("aria-live".into(), "assertive".into())),
    }
    if sem.sensitive {
        out.push(("data-y901-sensitive".into(), "true".into()));
    }
    out
}

/// 递归把语义树映射成 ARIA 节点树。
///
/// 跳过规则与 AccessKit 路径一致：`exposed_to_ai == false` 的节点**连同子树**都不出现
/// （读屏器不该念出隐藏的对话框）。若根节点被跳过，返回 `None`。
pub fn to_aria_tree(root: &NodeInfo) -> Option<AriaNode> {
    if !root.semantics.exposed_to_ai {
        return None;
    }
    let mut attrs = aria_attrs(&root.semantics);
    if root.focused {
        // DOM 的焦点用属性标记；真正的 element.focus() 由后端决定（见平台说明）。
        attrs.push(("data-y901-focused".into(), "true".into()));
    }
    Some(AriaNode {
        id: root.id.clone(),
        role: aria_role(root.role),
        attrs,
        children: root.children.iter().filter_map(to_aria_tree).collect(),
    })
}

/// 统计镜像树里的节点数与最大深度（验收用：确认整棵树都在 DOM 里）。
pub fn stats(node: &AriaNode) -> (usize, usize) {
    let mut count = 1;
    let mut max_depth = 1;
    for child in &node.children {
        let (c, d) = stats(child);
        count += c;
        max_depth = max_depth.max(d + 1);
    }
    (count, max_depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_ui::{Pos, Rect, Role, Semantics};

    fn node(id: &str, role: &'static str) -> NodeInfo {
        NodeInfo {
            id: id.to_string(),
            role: Role(role),
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            focused: false,
            semantics: Semantics::default().exposed(true),
            children: Vec::new(),
        }
    }

    #[test]
    fn roles_map_to_aria_names() {
        assert_eq!(aria_role(Role("button")), "button");
        assert_eq!(aria_role(Role("text_input")), "textbox");
        assert_eq!(aria_role(Role("window")), "application");
        assert_eq!(aria_role(Role("dialog")), "dialog");
        assert_eq!(aria_role(Role("alert")), "alert");
    }

    #[test]
    fn unknown_roles_fall_back_to_generic() {
        assert_eq!(aria_role(Role("terminal")), "generic");
        assert_eq!(aria_role(Role("")), "generic");
    }

    #[test]
    fn label_and_description_become_aria_attributes() {
        let mut n = node("btn", "button");
        n.semantics = n
            .semantics
            .label("Save")
            .description("Write the file to disk");
        let a = to_aria_tree(&n).expect("tree");
        assert_eq!(a.attr("aria-label"), Some("Save"));
        assert_eq!(a.attr("aria-description"), Some("Write the file to disk"));
        assert_eq!(a.role, "button");
    }

    #[test]
    fn state_flags_map_to_the_right_aria_attributes() {
        let mut n = node("c", "checkbox");
        n.semantics = n
            .semantics
            .checked(true)
            .expanded(false)
            .selected(true)
            .level(2)
            .live(LiveRegion::Polite);
        let a = to_aria_tree(&n).expect("tree");
        assert_eq!(a.attr("aria-checked"), Some("true"));
        assert_eq!(a.attr("aria-expanded"), Some("false"));
        assert_eq!(a.attr("aria-selected"), Some("true"));
        assert_eq!(a.attr("aria-level"), Some("2"));
        assert_eq!(a.attr("aria-live"), Some("polite"));
    }

    #[test]
    fn absent_state_produces_no_attributes() {
        // 不设就不写：`aria-checked="false"` 和不写是两种语义，不能凭空补。
        let n = node("x", "button");
        let a = to_aria_tree(&n).expect("tree");
        assert_eq!(a.attr("aria-checked"), None);
        assert_eq!(a.attr("aria-expanded"), None);
        assert_eq!(a.attr("aria-live"), None);
    }

    #[test]
    fn numeric_range_is_exported() {
        let mut n = node("s", "slider");
        n.semantics = n.semantics.value("0.5").value_range(0.0, 1.0);
        let a = to_aria_tree(&n).expect("tree");
        assert_eq!(a.attr("aria-valuemin"), Some("0"));
        assert_eq!(a.attr("aria-valuemax"), Some("1"));
        assert_eq!(a.attr("aria-valuetext"), Some("0.5"));
    }

    #[test]
    fn disabled_and_focus_are_marked() {
        let mut n = node("b", "button");
        n.focused = true;
        n.semantics = n.semantics.disabled(true);
        let a = to_aria_tree(&n).expect("tree");
        assert_eq!(a.attr("aria-disabled"), Some("true"));
        assert_eq!(a.attr("data-y901-focused"), Some("true"));
    }

    #[test]
    fn unexposed_subtree_is_skipped_entirely() {
        let mut root = node("root", "window");
        let mut hidden = node("h", "panel");
        hidden.semantics = Semantics::default(); // exposed_to_ai = false
        hidden.children.push(node("deep", "button"));
        root.children.push(hidden);
        root.children.push(node("visible", "button"));
        let a = to_aria_tree(&root).expect("tree");
        assert_eq!(a.children.len(), 1, "隐藏子树不该进 DOM");
        assert_eq!(a.children[0].id, "visible");
    }

    #[test]
    fn unexposed_root_yields_no_tree() {
        let mut n = node("root", "window");
        n.semantics = Semantics::default();
        assert!(to_aria_tree(&n).is_none(), "根都不可见时不该建 DOM 镜像");
    }

    #[test]
    fn stats_counts_nodes_and_depth() {
        let mut root = node("root", "window");
        let mut a = node("a", "panel");
        a.children.push(node("a1", "button"));
        root.children.push(a);
        root.children.push(node("b", "button"));
        let tree = to_aria_tree(&root).expect("tree");
        assert_eq!(stats(&tree), (4, 3), "4 个节点、深度 3");
    }

    #[test]
    fn sensitive_nodes_are_marked_for_masking() {
        let mut n = node("pw", "text_input");
        n.semantics = n.semantics.sensitive(true);
        let a = to_aria_tree(&n).expect("tree");
        assert_eq!(a.attr("data-y901-sensitive"), Some("true"));
        let _ = Pos::new(0.0, 0.0);
    }
}
