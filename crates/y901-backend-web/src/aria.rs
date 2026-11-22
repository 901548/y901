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
//! ## 每次推树都重建，但**语义没变就不碰 DOM**
//!
//! 增量 diff（节点增删、顺序变化、属性合并）的复杂度远高于收益：整棵树通常只有
//! 几十到几百个节点，重建一次在毫秒级。**宁可慢一点，也不要"DOM 与画面不一致"**。
//!
//! 但"每帧都重建"并不必要：动画帧里位置/颜色在动、语义（角色/标签/状态）通常不动。
//! 所以先用 [`y901_a11y::aria::tree_fingerprint`] 比对语义树指纹，相同就整帧跳过 ——
//! 否则读屏器每 16ms 看到一棵全新的无障碍树，轻则白做功，重则把当前朗读节点冲掉。
//!
//! ## 已知边界
//!
//! - 不接管焦点：`data-y901-focused` 只是标记，不做 `element.focus()`
//!   （canvas 自己管焦点，两边抢焦点会让输入法错乱）；
//! - 不做 live region 的播报调度（交给浏览器读 `aria-live`）；
//! - 不映射文本光标位置（canvas 无此语义）。

use y901_a11y::aria::{to_aria_tree, tree_fingerprint, AriaNode};
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
    /// 最近一次推入的语义树指纹（0 = 还没推过）。
    ///
    /// 为什么要指纹：动画帧里**语义通常不变**（位置/颜色在变，角色/标签不变），
    /// 而每帧整棵重建 DOM 会让读屏器看到一个全新的无障碍树 —— 轻则白做功，重则
    /// 把读屏器的当前节点冲掉。指纹相同就**一个 DOM 操作都不做**。
    last_fingerprint: u64,
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
            last_fingerprint: 0,
        }
    }

    /// 最近一次推入的节点数。
    pub fn published_nodes(&self) -> usize {
        self.published
    }

    /// 把语义树镜像进 DOM（语义未变时**不碰 DOM**）。
    ///
    /// 返回：镜像里当前的节点数（`0` = 根不可见，容器被清空）。
    pub fn publish(&mut self, doc: &Document, snapshot: &NodeInfo) -> usize {
        let Some(container) = self.container.as_ref() else {
            return 0;
        };
        let Some(tree) = to_aria_tree(snapshot) else {
            // 根不可见：容器里不该留任何东西。只在"之前推过"时才写 DOM。
            if self.published != 0 || self.last_fingerprint != 0 {
                container.set_text_content("");
                self.published = 0;
                self.last_fingerprint = 0;
            }
            return 0;
        };
        let fingerprint = tree_fingerprint(&tree);
        if fingerprint == self.last_fingerprint && self.published > 0 {
            // 语义没变：跳过。（画面可能在动，但读屏器关心的东西没动。）
            return self.published;
        }
        // 清空：清空 `textContent` 会把所有子元素一起移除（比重建容器省事且可靠）。
        container.set_text_content("");
        let count = append_node(doc, container, &tree);
        self.published = count;
        self.last_fingerprint = fingerprint;
        count
    }
}

/// 在浏览器里建立镜像：认领（或创建）`#y901-a11y` 容器，并把 `document` 句柄一并交回。
///
/// 返回 `None` = 当前环境没有 `window` / `document` / `body`（例如脚本在 `<head>` 里跑、
/// 页面还没有 body）。**不 panic、不报错**：无障碍是增强能力，不该让应用起不来。
pub fn init() -> Option<(AriaMirror, Document)> {
    let window = crate::bindings::global_window()?;
    let doc = window.document()?;
    let body = doc.body()?;
    let mirror = AriaMirror::new(&doc, &body);
    Some((mirror, doc))
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
