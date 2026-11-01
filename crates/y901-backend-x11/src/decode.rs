//! X11 事件的**字节层解码**。
//!
//! 事件固定 32 字节；第 0 字节是码。超过 32 字节的事件（如 `GenericEvent`）本轮不支持。

use crate::event_code;
use crate::{get_u16, get_u32};

/// 解码出的事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X11Event {
    /// 需要重绘的区域（字节 4-7 窗口，8-15 矩形，16-17 待重绘个数）
    Expose {
        /// 目标窗口
        window: u32,
        /// 区域 x
        x: u16,
        /// 区域 y
        y: u16,
        /// 区域宽
        width: u16,
        /// 区域高
        height: u16,
        /// 后续还有多少个 Expose 待处理
        count: u16,
    },
    /// 窗口尺寸/位置变化
    ConfigureNotify {
        /// 事件接收窗口
        event: u32,
        /// 发生变化的窗口
        window: u32,
        /// 上方兄弟窗口（0 = 顶层）
        above_sibling: u32,
        /// x
        x: i16,
        /// y
        y: i16,
        /// 宽
        width: u16,
        /// 高
        height: u16,
        /// 边框宽
        border_width: u16,
        /// 是否绕过窗口管理器
        override_redirect: bool,
    },
    /// 按键（含释放）
    Key {
        /// `true` = 按下，`false` = 释放
        pressed: bool,
        /// keycode（需要后续 `GetKeyboardMapping` 才能变成 keysym）
        keycode: u8,
        /// 事件窗口
        event: u32,
        /// 修饰键位掩码（Shift=1, Lock=2, Ctrl=4, Mod1=8 …）
        state: u16,
        /// 窗口内 x
        event_x: i16,
        /// 窗口内 y
        event_y: i16,
    },
    /// 鼠标按键（含释放）
    Button {
        /// `true` = 按下，`false` = 释放
        pressed: bool,
        /// 1=左 2=中 3=右 4/5=滚轮 8/9=侧键
        button: u8,
        /// 事件窗口
        event: u32,
        /// 修饰键位掩码
        state: u16,
        /// 窗口内 x
        event_x: i16,
        /// 窗口内 y
        event_y: i16,
    },
    /// 鼠标移动
    Motion {
        /// 事件窗口
        event: u32,
        /// 修饰键位掩码
        state: u16,
        /// 窗口内 x
        event_x: i16,
        /// 窗口内 y
        event_y: i16,
    },
    /// 窗口关闭（可能来自 `WM_DELETE_WINDOW` 的 ClientMessage 或 DestroyNotify）
    Destroy {
        /// 事件窗口
        event: u32,
        /// 被销毁的窗口
        window: u32,
    },
    /// 客户端消息（窗口管理器通知，如 `WM_DELETE_WINDOW`）
    ClientMessage {
        /// 事件窗口
        window: u32,
        /// 消息类型原子
        message_type: u32,
        /// `format`（8/16/32）
        format: u8,
        /// 数据（按 `format` 解释）
        data: [u32; 5],
    },
    /// 本轮不关心的事件（保留码与原始字节，方便调试）
    Unknown {
        /// 事件码
        code: u8,
    },
}

/// 解码失败原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// 不足 32 字节
    TooShort {
        /// 实际长度
        got: usize,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort { got } => {
                write!(f, "X11 事件需要 32 字节，实际 {got}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// 把 32 字节的事件字节解码成 [`X11Event`]。
///
/// 返回：`Err` 当不足 32 字节。
/// 副作用：无。
///
/// 为什么不做"事件序列号" 校验：序列号在 `XFlush` 时才分配，
/// 编码层拿不到它；放在 S2 第二步的 socket 层更合适。
pub fn decode_event(bytes: &[u8]) -> Result<X11Event, DecodeError> {
    if bytes.len() < 32 {
        return Err(DecodeError::TooShort { got: bytes.len() });
    }
    let code = bytes[0] & 0x7F; // 最高位是"synthetic"标志，必须屏蔽掉
    let ev = |off: usize| get_u32(bytes, off).unwrap_or(0);
    let ev16 = |off: usize| get_u16(bytes, off).unwrap_or(0);

    Ok(match code {
        event_code::EXPOSE => X11Event::Expose {
            window: ev(4),
            x: ev16(8),
            y: ev16(10),
            width: ev16(12),
            height: ev16(14),
            count: ev16(16),
        },
        event_code::CONFIGURE_NOTIFY => X11Event::ConfigureNotify {
            event: ev(4),
            window: ev(8),
            above_sibling: ev(12),
            x: ev16(16) as i16,
            y: ev16(18) as i16,
            width: ev16(20),
            height: ev16(22),
            border_width: ev16(24),
            override_redirect: bytes[26] != 0,
        },
        event_code::KEY_PRESS | event_code::KEY_RELEASE => X11Event::Key {
            pressed: code == event_code::KEY_PRESS,
            keycode: bytes[1],
            event: ev(12),
            state: ev16(28),
            event_x: ev16(24) as i16,
            event_y: ev16(26) as i16,
        },
        event_code::BUTTON_PRESS | event_code::BUTTON_RELEASE => X11Event::Button {
            pressed: code == event_code::BUTTON_PRESS,
            button: bytes[1],
            event: ev(12),
            state: ev16(28),
            event_x: ev16(24) as i16,
            event_y: ev16(26) as i16,
        },
        event_code::MOTION_NOTIFY => X11Event::Motion {
            event: ev(12),
            state: ev16(28),
            event_x: ev16(24) as i16,
            event_y: ev16(26) as i16,
        },
        event_code::DESTROY_NOTIFY => X11Event::Destroy {
            event: ev(4),
            window: ev(8),
        },
        event_code::CLIENT_MESSAGE => X11Event::ClientMessage {
            window: ev(4),
            message_type: ev(8),
            format: bytes[1],
            data: [ev(12), ev(16), ev(20), ev(24), ev(28)],
        },
        _ => X11Event::Unknown { code },
    })
}
