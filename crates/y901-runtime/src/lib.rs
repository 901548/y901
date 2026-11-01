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
use y901_ui::{Command, EventResult, Key, Modifiers, MouseButton, Size, UiEvent, UiTree};

/// 渲染后端抽象（跨平台 v3 阶段 1）。
pub mod gfx;

/// 业务侧回调：App 与 Demo 实现它。
///
/// 运行时不认识任何业务，只在合适的时机回调这些方法。
/// 跨线程唤醒器：后台线程（PTY/RPC/网络）用它打断主循环的阻塞等待。
///
/// 为什么需要：主循环空闲时阻塞等待 Windows 消息（0% CPU），
/// 此时"管道里来了数据"不产生消息，必须显式唤醒，否则要等下一次输入。
pub trait Waker: Send + Sync + 'static {
    /// 唤醒主循环跑一帧。
    fn wake(&self);
}

pub trait Host {
    /// 每帧空闲：抽事件队列、驱动 Tick、推送订阅。
    fn idle(&mut self, tree: &mut UiTree);

    /// 接收唤醒器（平台层在窗口创建后调用一次）。
    ///
    /// 默认实现：忽略——这样不关心异步唤醒的 Host 无需改动。
    /// 需要它的 Host 应保存起来，交给后台线程在"有数据"时 `wake()`。
    fn attach_waker(&mut self, _waker: std::sync::Arc<dyn Waker>) {}

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

    /// 应用请求切换主题（`Host` 表达意图，平台层执行）。
    ///
    /// 返回：`Some(theme)` = 请求切换，平台层在本帧末尾应用；`None` = 无请求。
    /// 一次请求只生效一次（平台层取走后即清空），避免每帧重复设置。
    /// 默认实现返回 `None`（不支持运行时切换的宿主无需实现）。
    fn take_theme_request(&mut self) -> Option<y901_ui::theme::Theme> {
        None
    }

    /// 应用请求切换字体（族名 + 字号）。
    ///
    /// 返回：`Some(font)` = 请求切换；`None` = 无请求。
    /// 失败语义由平台层决定：字体加载失败时保留旧字体，并在 stderr 报告。
    fn take_font_request(&mut self) -> Option<FontConfig> {
        None
    }

    /// 应用请求一帧真实 backbuffer 快照。
    ///
    /// 返回 `true` 时，平台层会在本帧 present 前 request，并在 render 后
    /// 通过 [`Host::on_snapshot`] 回传 BGRA 像素。应用通常由快捷键（F12）
    /// 或菜单置位，再把 PNG 落盘。
    fn take_snapshot_request(&mut self) -> bool {
        false
    }

    /// 收到真实 backbuffer 像素：`pixels` 为 BGRA，长度 = width*height*4。
    ///
    /// 在 UI 线程、present 之后调用；实现方负责编码 PNG，不要做长耗时操作。
    fn on_snapshot(&mut self, _pixels: &[u8], _width: u32, _height: u32) {}

    /// 组件请求把 IME 候选窗钉到某处时调用。
    fn ime_position(&mut self, _tree: &mut UiTree) -> Option<(f32, f32)> {
        None
    }
}

/// 平台侧：每个后端实现一次。
pub trait PlatformRuntime {
    /// 跑完整主循环（建窗 → 事件 → 重绘 → 直到关闭）。
    ///
    /// ## 为什么是 `&mut self` 而不是静态方法
    ///
    /// 只有取 `&mut self`，才能写成 `Box<dyn PlatformRuntime>` 并在运行时选后端；
    /// 静态方法在 `dyn` 上不可调用（它不在 vtable 里），那就只能编译期选后端——
    /// 业务层就得认识具体类型。
    fn run(
        &mut self,
        host: Box<dyn Host>,
        build: Box<dyn FnOnce() -> UiTree>,
        options: Options,
    ) -> Result<(), String>;
}

/// 不透明的窗口句柄：**业务与组件永远不接触 `HWND`/`XID`/`NSWindow*`**。
///
/// 包含尺寸与 `scale`（HiDPI），因为它们是平台无关的事实；
/// 底层句柄只供后端自己解包（而不是业务层）。
#[derive(Clone, Copy, Debug)]
pub struct WindowHandle {
    raw: gfx::PlatformWindowHandle,
    width: u32,
    height: u32,
    /// 物理像素 / 逻辑像素。Windows 通常 1.0，macOS Retina 2.0，Linux 可能 1.25/1.5。
    scale: f32,
}

impl WindowHandle {
    /// 由后端构造（业务不应调用）。
    pub fn new(raw: gfx::PlatformWindowHandle, width: u32, height: u32, scale: f32) -> Self {
        Self {
            raw,
            width,
            height,
            scale: if scale > 0.0 { scale } else { 1.0 },
        }
    }

    /// 逻辑尺寸。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 比例因子（物理/逻辑）。
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// 底层句柄（**只给后端用**）。
    pub fn raw(&self) -> gfx::PlatformWindowHandle {
        self.raw
    }
}

/// 事件源：把"事件从哪来"藏在实现里（Win32 消息队列 / X11 socket / Cocoa runloop）。
///
/// 和 `Presenter` 分开的理由：桥接两者的时机完全不同——
/// 事件源负责阻塞等待，`Presenter` 负责一帧像素上屏；合并会让每个平台各写一遍重绘策略。
pub trait EventSource {
    /// 取一批事件；`timeout_ms = 0` 表示不阻塞。
    fn poll(&mut self, timeout_ms: u32) -> Vec<RawEvent>;

    /// 跨线程唤醒句柄（后台线程用它打断 `poll` 的阻塞）。
    fn waker(&self) -> std::sync::Arc<dyn Waker>;
}

/// 像素呈现器：**纯 CPU 后端的统一出口**。
///
/// 为什么是 `&[u8]` 而不是 `Vec<u32>`：三个平台的底层都吃字节布局（
/// GDI `StretchDIBits` / X11 `PutImage` / `CGImage`），逐像素类型对接反而多一层转换。
/// 格式约定：**BGRA（预乘 alpha）**，行主序，与 `SoftBackend` 的缓冲区完全一致。
pub trait Presenter {
    /// 调整呈现尺寸（缓冲区重建）。
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String>;

    /// 把一帧 BGRA 像素贴到窗口客户区。
    fn present(&mut self, pixels: &[u8], width: u32, height: u32) -> Result<(), String>;
}

/// 窗口参数。
#[derive(Clone, Debug)]
pub struct Options {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// 主题（`None` = [`y901_ui::theme::DARK_THEME`]）。
    ///
    /// 主题是**渲染属性**，由后端持有；`UiTree` 不保存主题（树不关心颜色）。
    /// 若将来需要"运行时切主题"，再引入 `set_theme`——当前只支持启动时注入。
    pub theme: Option<y901_ui::theme::Theme>,
    /// 字体（`None` = NSimSun / 16px）。
    pub font: Option<FontConfig>,
}

/// 字体配置：族名 + 逻辑像素字号。
///
/// **注意**：字号是**全局**的——文本引擎只用一个字号，
/// 不支持同一文档内不同字号（富文本）。若将来需要富文本，
/// 再给 `TextStyle` 加 `font_size` 字段并让 `measure_text` 读它。
#[derive(Clone, Debug, PartialEq)]
pub struct FontConfig {
    /// 字体族名（DirectWrite family name），例如 `"NSimSun"` / `"JetBrains Mono"`。
    pub family: String,
    /// 字号（逻辑像素）。
    pub size: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "NSimSun".to_string(),
            size: 16.0,
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            title: "Y901".to_string(),
            width: 1200,
            height: 800,
            theme: None,
            font: None,
        }
    }
}

/// 平台原始事件：wndproc 只产出它，不认识 `UiTree`。
#[derive(Clone, Debug, PartialEq)]
pub enum RawEvent {
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseDown {
        x: f32,
        y: f32,
        button: MouseButton,
    },
    MouseUp {
        x: f32,
        y: f32,
        button: MouseButton,
    },
    Wheel {
        x: f32,
        y: f32,
        delta: f32,
        mods: Modifiers,
    },
    /// 系统文件拖放（WM_DROPFILES）。
    FileDrop {
        paths: Vec<std::path::PathBuf>,
        x: f32,
        y: f32,
    },
    KeyDown {
        key: Key,
        mods: Modifiers,
    },
    /// Alt+键（WM_SYSKEYDOWN）：系统键，不与普通 KeyDown 混用。
    SystemKeyDown {
        key: Key,
        mods: Modifiers,
    },
    Char {
        c: char,
    },
    Ime {
        text: String,
        composition: bool,
    },
    Resize {
        w: u32,
        h: u32,
    },
    SetCursor {
        x: f32,
        y: f32,
    },
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
        RawEvent::FileDrop { paths, x, y } => Some(UiEvent::FileDrop {
            paths: paths.clone(),
            pos: y901_ui::Pos::new(*x, *y),
        }),
        RawEvent::Wheel { x, y, delta, mods } => Some(UiEvent::Wheel {
            pos: y901_ui::Pos::new(*x, *y),
            delta: *delta,
            mods: *mods,
        }),
        // Alt+键 折叠成 `UiEvent::Key`：组件不需要区分来源。
        // 修饰键状态由平台层用 `GetKeyState` 实时查询，不靠"松开事件"维护，
        // 因此本层**没有**松开事件（将来若需要松开检测，再单独加并明确语义）。
        RawEvent::SystemKeyDown { key, mods } => Some(UiEvent::Key {
            key: *key,
            modifiers: *mods,
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

    struct SnapshotHost {
        requested: bool,
        pixels: usize,
        size: (u32, u32),
    }

    impl Host for SnapshotHost {
        fn idle(&mut self, _tree: &mut UiTree) {}
        fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
            true
        }
        fn take_snapshot_request(&mut self) -> bool {
            std::mem::take(&mut self.requested)
        }
        fn on_snapshot(&mut self, pixels: &[u8], width: u32, height: u32) {
            self.pixels = pixels.len();
            self.size = (width, height);
        }
    }

    #[test]
    fn snapshot_hooks_are_host_contract() {
        let mut host = SnapshotHost {
            requested: true,
            pixels: 0,
            size: (0, 0),
        };
        assert!(host.take_snapshot_request());
        assert!(!host.take_snapshot_request(), "请求应只消费一次");
        host.on_snapshot(&[1, 2, 3, 4], 1, 1);
        assert_eq!(host.pixels, 4);
        assert_eq!(host.size, (1, 1));
    }
}
