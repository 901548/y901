//! 平台无关的运行时契约。
//!
//! 为什么单独一个 crate：`Host::policy()` 需要 `y901_agent::Policy`，而
//! `y901-agent` 依赖 `y901-ui`。若把 `Host` 放在 `y901-ui`，就会形成
//! `ui -> agent -> ui` 的循环。放在这里，依赖方向是单向的：
//!
//! ```text
//! y901-runtime -> y901-agent -> y901-ui
//!              -> y901-ui
//! y901-backend-win32 -> y901-runtime
//! y901-demos         -> y901-runtime -> y901-widgets -> y901-ui
//! y901-app           -> y901-demos / y901-backend-win32
//! ```

use y901_agent::Policy;
use y901_ui::{
    Command, EventResult, Key, Modifiers, MouseButton, Size, UiEvent, UiTree,
};

/// 渲染后端抽象（跨平台 v3 阶段 1）。
pub mod gfx;

/// 业务侧回调：App 与 Demo 实现它。
///
/// 运行时不认识任何业务，只在合适的时机回调这些方法。
pub trait Host {
    /// 每帧空闲：抽事件队列、驱动 Tick、推送订阅。
    fn idle(&mut self, tree: &mut UiTree);

    /// 处理一条应用级命令。返回 `true` 表示已处理。
    fn command(&mut self, tree: &mut UiTree, cmd: Command) -> bool;

    /// 事件钩子（在 `tree.dispatch` 之前调用）。
    fn event(&mut self, _tree: &mut UiTree, _ev: &UiEvent) -> EventResult {
        EventResult::Ignored
    }

    /// Agent 网关使用的安全策略（RPC 与 MCP 共用同一份）。
    fn policy(&self) -> Policy {
        Policy::default()
    }

    /// 组件请求把 IME 候选窗钉到某处时调用。
    fn ime_position(&mut self, _tree: &mut UiTree) -> Option<(f32, f32)> {
        None
    }
}

/// 平台侧：每个后端实现一次。
pub trait PlatformRuntime {
    fn run(
        host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
        options: Options,
    ) -> Result<(), String>;
}

/// 窗口参数。
#[derive(Clone, Debug)]
pub struct Options {
    pub title: String,
    pub width: u32,
    pub height: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            title: "Y901".to_string(),
            width: 1200,
            height: 800,
        }
    }
}

/// 平台原始事件：wndproc 只产出它，不认识 `UiTree`。
#[derive(Clone, Debug, PartialEq)]
pub enum RawEvent {
    MouseMove { x: f32, y: f32 },
    MouseDown { x: f32, y: f32, button: MouseButton },
    MouseUp { x: f32, y: f32, button: MouseButton },
    Wheel { x: f32, y: f32, delta: f32 },
    KeyDown { key: Key, mods: Modifiers },
    Char { c: char },
    Ime { text: String, composition: bool },
    Resize { w: u32, h: u32 },
    SetCursor { x: f32, y: f32 },
    Close,
}

/// 纯函数翻译：平台事件 → 框架事件。可单测，不碰窗口。
pub fn translate(raw: &RawEvent, viewport: Size) -> Option<UiEvent> {
    match raw {
        RawEvent::MouseMove { x, y } => Some(UiEvent::MouseMove {
            pos: y901_ui::Pos::new(*x, *y),
        }),
        RawEvent::MouseDown { x, y, button } => Some(UiEvent::MouseDown {
            pos: y901_ui::Pos::new(*x, *y),
            button: *button,
        }),
        RawEvent::MouseUp { x, y, button } => Some(UiEvent::MouseUp {
            pos: y901_ui::Pos::new(*x, *y),
            button: *button,
        }),
        RawEvent::Wheel { x, y, delta } => Some(UiEvent::Wheel {
            pos: y901_ui::Pos::new(*x, *y),
            delta: *delta,
        }),
        RawEvent::KeyDown { key, mods } => Some(UiEvent::Key {
            key: *key,
            modifiers: *mods,
        }),
        RawEvent::Char { c } => Some(UiEvent::Text(c.to_string())),
        // 组合中 → Compose（不插入正文）；提交 → Text（插入正文）
        RawEvent::Ime { text, composition } => {
            if *composition {
                Some(UiEvent::Compose(text.clone()))
            } else {
                Some(UiEvent::Text(text.clone()))
            }
        }
        RawEvent::Resize { w: _, h: _ } => {
            let _ = viewport;
            Some(UiEvent::Resize)
        }
        // 光标位置只用于 SetCursor，不产生框架事件
        RawEvent::SetCursor { .. } => None,
        RawEvent::Close => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_translates_to_positional_event() {
        let ev = translate(
            &RawEvent::MouseDown {
                x: 10.0,
                y: 20.0,
                button: MouseButton::Left,
            },
            Size::new(100.0, 100.0),
        )
        .expect("event");
        match ev {
            UiEvent::MouseDown { pos, .. } => {
                assert_eq!(pos.x, 10.0);
                assert_eq!(pos.y, 20.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn char_becomes_text() {
        let ev = translate(&RawEvent::Char { c: 'A' }, Size::new(1.0, 1.0)).unwrap();
        assert!(matches!(ev, UiEvent::Text(t) if t == "A"));
    }

    #[test]
    fn set_cursor_and_close_emit_nothing() {
        assert!(translate(&RawEvent::SetCursor { x: 1.0, y: 2.0 }, Size::new(1.0, 1.0)).is_none());
        assert!(translate(&RawEvent::Close, Size::new(1.0, 1.0)).is_none());
    }
}
