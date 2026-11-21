//! **隐藏 DOM 的 ARIA 镜像**（feature `a11y`）。
//!
//! canvas 里画出来的东西对读屏器不存在，所以这里把语义树镜像成一份
//! **视觉上不可见、但仍在无障碍树里**的 DOM：
//!
//! ```text
//! <div id="y901-a11y" role="application"
//!      style="position:absolute;width:1px;height:1px;overflow:hidden;
//!             clip:rect(0 0 0 0);clip-path:inset(50%);white-space:nowrap">
//!   <div role="button" data-y901-id="btn" aria-label="Save"></div>
//!   …
//! </div>
//! ```
//!
//! ## 为什么用 `clip` + 1px 而不是 `display:none`
//!
//! `display:none` / `visibility:hidden` 会把节点**从无障碍树里移除** —— 那就等于没做。
//! 上面的写法是"对视觉隐藏但对 AT 可见"的标准做法。
//!
//! ## 每次推树都重建
//!
//! 增量 diff（节点增删、顺序变化、属性合并）的复杂度远高于收益：整棵树通常只有
//! 几十到几百个节点，重建一次在毫秒级。**宁可慢一点，也不要"DOM 与画面不一致"**。
//!
//! ## 已知边界
//!
//! - 不接管焦点：`data-y901-focused` 只是标记，不做 `element.focus()`
//!   （canvas 自己管焦点，两边抢焦点会让输入法错乱）；
//! - 不做 live region 的播报调度（交给浏览器读 `aria-live`）；
//! - 不映射文本光标位置（canvas 无此语义）。

use y901_a11y::aria::{to_aria_tree, AriaNode};
use y901_ui::NodeInfo;

use crate::bindings::{Document, Element};

/// 镜像容器的 DOM id。
pub const CONTAINER_ID: &str = "y901-a11y";

/// 容器的内联样式：对视觉隐藏、对无障碍可见。
const CONTAINER_STYLE: &str = "position:absolute;width:1px;height:1px;overflow:hidden;\
clip:rect(0 0 0 0);clip-path:inset(50%);white-space:nowrap;border:0;padding:0;margin:0";

/// ARIA 镜像：持有容器元素，每次推树时重建内容。
pub struct AriaMirror {
    container: Option<Element>,
    /// 最近一次成功推入镜像的节点数（0 = 空树或尚未推过）。
    published: usize,
}

impl AriaMirror {
    /// 在文档里确保容器存在（不存在就建一个并挂到 `body`）。
    pub fn new(doc: &Document, body: &Element) -> Self {
        let container = match doc.get_element_by_id(CONTAINER_ID) {
            Some(el) => el,
            None => {
                let el = doc.create_element("div");
                el.set_attribute("id", CONTAINER_ID);
                // `aria-label` 给容器一个名字，否则读屏器只念"application"。
                el.set_attribute("aria-label", "Y901 application");
                body.append_child(&el);
                el
            }
        };
        // 容器已在文档里时也要确保角色与样式正确（热重启/重复初始化）。
        container.set_attribute("role", "application");
        container.set_attribute("style", CONTAINER_STYLE);
        Self {
            container: Some(container),
            published: 0,
        }
    }

    /// 最近一次推入的节点数。
    pub fn published_nodes(&self) -> usize {
        self.published
    }

    /// 把语义树镜像进 DOM（整棵重建）。
    ///
    /// 返回：写进去的节点数（`0` = 根不可见，容器被清空）。
    pub fn publish(&mut self, doc: &Document, snapshot: &NodeInfo) -> usize {
        let Some(container) = self.container.as_ref() else {
            return 0;
        };
        // 清空：清空 `textContent` 会把所有子元素一起移除（比重建容器省事且可靠）。
        container.set_text_content("");
        let Some(tree) = to_aria_tree(snapshot) else {
            self.published = 0;
            return 0;
        };
        let count = append_node(doc, container, &tree);
        self.published = count;
        count
    }
}

/// 递归建元素：容器下的每个节点都是一个带 `role` 的 `div`。
///
/// 用 `div` 承载所有角色：**语义由 `role` 属性决定**，元素标签不影响读屏器。
fn append_node(doc: &Document, parent: &Element, node: &AriaNode) -> usize {
    let el = doc.create_element("div");
    el.set_attribute("role", node.role);
    el.set_attribute("data-y901-id", &node.id);
    for (name, value) in &node.attrs {
        el.set_attribute(name, value);
    }
    parent.append_child(&el);
    let mut count = 1;
    for child in &node.children {
        count += append_node(doc, &el, child);
    }
    count
}

#[cfg(test)]
mod tests {
    // 这个模块是 wasm-only：真正的 DOM 行为只能在浏览器里验证。
    // 纯映射逻辑在 `y901_a11y::aria`（跨平台单测覆盖）。
    #[test]
    fn container_id_is_stable() {
        assert_eq!(super::CONTAINER_ID, "y901-a11y");
    }

    #[test]
    fn container_style_hides_visually_but_keeps_it_in_the_a11y_tree() {
        let style = super::CONTAINER_STYLE;
        assert!(style.contains("clip:rect"), "必须用 clip 隐藏：{style}");
        assert!(
            !style.contains("display:none") && !style.contains("visibility:hidden"),
            "display:none / visibility:hidden 会把节点移出无障碍树，等于没做"
        );
        assert!(style.contains("position:absolute"));
    }
}
