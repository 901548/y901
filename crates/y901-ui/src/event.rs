//! 事件模型。
//!
//! 框架定义"发生了什么"，组件决定"要不要处理"，
//! 平台层（Win32）只负责把消息翻译成 `UiEvent`。

use crate::geometry::Pos;

/// 与平台无关的按键码。
///
/// 没有直接用 Windows VK，也没有用 winit 的码表：框架必须能跑在任何后端上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Tab,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Function(u8),
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// 框架事件。
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// 按键。`text` 是已经过 IME/键盘布局翻译的可打印文本（如果有）。
    Key {
        key: Key,
        modifiers: Modifiers,
    },
    /// 文本输入（IME 提交、粘贴、RPC 注入都走这里）。
    Text(String),
    /// OS 级文件拖放（从资源管理器拖进来）。
    ///
    /// `paths` 是绝对路径；`pos` 是松手时的客户区坐标（用于命中目标控件）。
    /// 框架只负责路由，不解释路径含义。
    FileDrop {
        paths: Vec<std::path::PathBuf>,
        pos: Pos,
    },
    /// 滚轮：正数向上。
    Wheel {
        delta: f32,
        pos: Pos,
        /// 滚轮时的修饰键（Ctrl+滚轮常用来缩放）。
        mods: Modifiers,
    },
    MouseDown {
        pos: Pos,
        button: MouseButton,
    },
    /// 鼠标进入节点（由树在命中目标变化时派发）。
    ///
    /// 为什么单独成事件而不是让控件自己比 `is_hovered`：
    /// 控件需要在**进入的瞬间**做一次动作（启动延时、播放动画），
    /// 那是个瞬时事实，不是可轮询的状态。
    MouseEnter,
    /// 鼠标离开节点（由树在命中目标变化时派发）。
    MouseLeave,
    MouseUp {
        pos: Pos,
        button: MouseButton,
    },
    MouseMove {
        pos: Pos,
    },
    /// 获得/失去焦点。
    Focus(bool),
    /// 视口尺寸变化。
    Resize,
    /// 每帧心跳：组件用它做非阻塞轮询（后台数据、动画等）。
    Tick,
    /// 动作：由命令路由或 RPC 投递给某个节点。
    /// `name` 的语义只有组件自己知道，框架不解释。
    Action {
        name: String,
        args: crate::json::Json,
    },
    /// IME 组合串变化（预编辑文字）。空串表示组合结束。
    ///
    /// 组合串**不是**已提交的输入：组件应把它画成临时覆盖层，
    /// 绝不能写进正文。
    Compose(String),
}

/// 事件所处的路由阶段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventPhase {
    /// 捕获：根 → 目标的父节点。容器可以在这里拦截（如 Modal 吞掉外部点击）。
    Capture,
    /// 目标：事件真正命中的节点。
    #[default]
    Target,
    /// 冒泡：目标的父 → 根。
    Bubble,
}

/// 指针捕获请求：组件通过它告诉框架"接下来的移动都给我"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PointerCapture {
    Capture(String),
    Release,
}

/// 事件处理结果。多个组件的结果会按"最强者"合并。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum EventResult {
    /// 没兴趣，继续往上冒泡。
    #[default]
    Ignored,
    /// 已处理，阻止冒泡。
    Handled,
    /// 已处理，且需要重绘。
    Redraw,
    /// 已处理，且需要重新布局（尺寸可能变了）。
    Relayout,
}

impl EventResult {
    /// 合并两个结果：取更强的标志（Handled/Relayout/Redraw 依次累积）。
    pub fn merge(self, other: EventResult) -> EventResult {
        self.max(other)
    }

    /// 是否已被处理（含 Redraw / Relayout）。
    pub fn is_handled(self) -> bool {
        self != EventResult::Ignored
    }

    /// 是否需要重绘。
    pub fn needs_redraw(self) -> bool {
        self >= EventResult::Redraw
    }

    /// 是否需要重新布局（布局失效 + 重绘）。
    pub fn needs_relayout(self) -> bool {
        self == EventResult::Relayout
    }
}
