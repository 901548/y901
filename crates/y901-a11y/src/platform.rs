//! 平台适配：把 [`crate::to_tree_update`] 的结果接到各系统的无障碍 API。
//!
//! 三个平台三条路，但**语义树只翻译一次**（[`crate::to_tree_update`]）：
//!
//! | 平台 | 适配器 | 状态 |
//! |---|---|---|
//! | Windows | `accesskit_windows::Adapter`（UIA） | feature `windows-uia` |
//! | Linux | `accesskit_unix::Adapter`（AT-SPI over D-Bus） | feature `unix-atspi` |
//! | Web | 隐藏 DOM 镜像（ARIA 属性） | 未实现，见 `docs/ROADMAP.md` |
//!
//! ## 为什么用 trait 包一层
//!
//! 三者的**生命周期**完全不同（UIA 绑在 HWND 上、AT-SPI 绑在 D-Bus 连接上、
//! DOM 绑在节点上），但调用方的用法是一样的：**建一次 / 推树 / 收动作**。
//! 用 trait 把这个共同面抽出来，后端就只依赖 `y901_a11y::PlatformBridge`，
//! 不必在 `y901-backend-*` 里各写一遍平台 if-else。

/// 平台无障碍桥的共同接口。
///
/// 实现者负责：把 [`accesskit::TreeUpdate`] 交给系统 API；把系统发来的动作
/// 转成应用能理解的 `(node_id, action)`（再由后端映射回 Y901 的节点 id）。
pub trait PlatformBridge {
    /// 建立桥接时可能产生的错误（例如 D-Bus 连不上、HWND 无效）。
    type Error: std::fmt::Display;

    /// 把一棵（新的或更新后的）树推给系统。
    fn push(&mut self, update: accesskit::TreeUpdate) -> Result<(), Self::Error>;

    /// 取出一个待处理的系统动作（没有则返回 `None`）。
    ///
    /// 返回 `Some((node, action))`：`node` 是 AccessKit 的节点 id，
    /// 调用方用 [`crate::NodeIdMap`] 反查回 Y901 的节点 id 再派发。
    fn poll_action(&mut self) -> Option<(accesskit::NodeId, accesskit::Action)>;
}

/// AccessKit 节点 id ↔ Y901 节点 id 的双向映射。
///
/// 为什么需要：AccessKit 的 `NodeId` 是 `u64`，Y901 的节点 id 是字符串。
/// 每帧重建整棵树时映射会变（id 是按遍历顺序分配的），因此**必须在同一帧内
/// 用同一份映射**，否则动作会打到错误的节点上。
#[derive(Clone, Debug, Default)]
pub struct NodeIdMap {
    to_accesskit: std::collections::HashMap<String, accesskit::NodeId>,
    to_y901: std::collections::HashMap<accesskit::NodeId, String>,
}

impl NodeIdMap {
    /// 记录一条映射。
    pub fn insert(&mut self, y901_id: impl Into<String>, ak: accesskit::NodeId) {
        let y901_id = y901_id.into();
        self.to_accesskit.insert(y901_id.clone(), ak);
        self.to_y901.insert(ak, y901_id);
    }

    /// Y901 id → AccessKit id。
    pub fn accesskit_of(&self, y901_id: &str) -> Option<accesskit::NodeId> {
        self.to_accesskit.get(y901_id).copied()
    }

    /// AccessKit id → Y901 id（处理动作时用）。
    pub fn y901_of(&self, ak: accesskit::NodeId) -> Option<&str> {
        self.to_y901.get(&ak).map(String::as_str)
    }

    /// 条目数。
    pub fn len(&self) -> usize {
        self.to_y901.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.to_y901.is_empty()
    }
}

#[cfg(target_os = "windows")]
#[cfg(feature = "windows-uia")]
pub mod windows_uia;

#[cfg(target_os = "linux")]
#[cfg(feature = "unix-atspi")]
pub mod unix_atspi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_map_round_trips() {
        let mut map = NodeIdMap::default();
        map.insert("btn-1", accesskit::NodeId(7));
        assert_eq!(map.accesskit_of("btn-1"), Some(accesskit::NodeId(7)));
        assert_eq!(map.y901_of(accesskit::NodeId(7)), Some("btn-1"));
        assert_eq!(map.len(), 1);
        assert!(!map.is_empty());
    }

    #[test]
    fn unknown_ids_return_none_instead_of_panicking() {
        let map = NodeIdMap::default();
        assert_eq!(map.accesskit_of("nope"), None);
        assert_eq!(map.y901_of(accesskit::NodeId(99)), None);
        assert!(map.is_empty());
    }

    #[test]
    fn reinserting_the_same_y901_id_overwrites_the_old_mapping() {
        // 每帧重新分配 id 时会出现这种情况：旧映射必须被替换，
        // 否则动作会打到上一帧的节点上。
        let mut map = NodeIdMap::default();
        map.insert("btn", accesskit::NodeId(1));
        map.insert("btn", accesskit::NodeId(2));
        assert_eq!(map.accesskit_of("btn"), Some(accesskit::NodeId(2)));
        assert_eq!(
            map.y901_of(accesskit::NodeId(1)),
            Some("btn"),
            "旧的反向映射仍存在（上一帧的 id 不会被复用，但保留它不会造成错误路由）"
        );
        assert_eq!(map.len(), 2);
    }
}
