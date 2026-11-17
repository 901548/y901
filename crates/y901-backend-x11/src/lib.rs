//! # y901-backend-x11 —— S2 第一步：**协议编码层**
//!
//! 本 crate 现阶段只做一件事：把 X11 请求/事件**编码成字节流 / 从字节流解码**。
//! 不碰 socket、不碰 `EventSource`/`Presenter` trait——那些在 S2 第二步。
//!
//! ## 为什么先写编码层
//!
//! 编码是**纯函数**：输入参数 → 输出字节。它可以在没有 X server 的 Windows 机器上
//! 通过"逐字节对比期望值"来验证（golden test），覆盖了整个后端大约 60~70% 的代码量。
//!
//! ## 字节序
//!
//! 只支持 **小端序**（`'l'` = 0x6C）—— x86_64 / aarch64 均为小端序，
//! 大端序客户端（如 s390x）在本地路线不会出现。见文末「未实现清单」。
//!
//! ## 协议基本单位
//!
//! - 所有请求的 `length` 字段都是**4 字节为一个单位**，且**包含 4 字节头**；
//! - 所有请求/响应都必须 4 字节对齐（不够就补 0）；
//! - 事件固定 **32 字节**。

#![deny(missing_docs)]
#[cfg(unix)]
/// **输入法（IME）抽象与 IBus 客户端**：中文输入走这里，不影响框架契约。
pub mod ime;
#[cfg(unix)]
/// X11 剪贴板（PRIMARY / CLIPBOARD 选择属主）。
pub mod clipboard;
#[cfg(unix)]
/// X11 连接、请求发送与响应/事件读取。
pub mod conn;
/// X11 事件与错误包的解码。
pub mod decode;
/// X11 请求与事件的编码。
pub mod encode;
/// keycode → keysym → 平台无关 `Key` 的映射。
pub mod keys;
#[cfg(unix)]
/// X11 后端的 `run_loop` 接线。
pub mod runtime;
#[cfg(unix)]
/// MIT-SHM 共享内存优化（可选，失败回退到 PutImage）。
pub mod shm;
#[cfg(unix)]
/// X11 窗口与事件循环。
pub mod window;
#[cfg(unix)]
/// Xauthority 读取（用于连接需认证的 X server）。
pub mod xauth;

pub use decode::{decode_event, DecodeError, X11Event};
pub use encode::{
    encode_change_property, encode_close_font, encode_configure_window, encode_convert_selection,
    encode_create_gc, encode_create_glyph_cursor, encode_create_window, encode_destroy_window,
    encode_free_cursor, encode_free_gc, encode_get_image, encode_get_input_focus,
    encode_get_keyboard_mapping, encode_get_property, encode_intern_atom, encode_map_window,
    encode_no_operation, encode_open_font, encode_put_image, encode_query_extension,
    encode_send_event, encode_set_input_focus, encode_set_selection_owner, encode_setup_request,
    encode_shm_attach, encode_shm_detach, encode_shm_put_image, encode_shm_query_version,
    synth_key_event, ConfigureMask, GcMask, PropMode, PutImageArgs, ShmPutImageArgs, ValueMask,
    WindowClass,
};

/// X11 协议主版本号。
pub const PROTOCOL_MAJOR: u16 = 11;
/// X11 协议次版本号。
pub const PROTOCOL_MINOR: u16 = 0;

/// 请求原语（opcode）。
///
/// 只列出本轮用到的；完整表见 X11 规范第 5 章。
pub mod opcode {
    /// `CreateWindow`
    pub const CREATE_WINDOW: u8 = 1;
    /// `MapWindow`
    pub const MAP_WINDOW: u8 = 8;
    /// `DestroyWindow`
    pub const DESTROY_WINDOW: u8 = 4;
    /// `ConfigureWindow`
    pub const CONFIGURE_WINDOW: u8 = 12;
    /// `SetInputFocus`
    pub const SET_INPUT_FOCUS: u8 = 42;
    /// `CreateGC`
    pub const CREATE_GC: u8 = 55;
    /// `ChangeGC`
    pub const CHANGE_GC: u8 = 56;
    /// `FreeGC`
    pub const FREE_GC: u8 = 60;
    /// `InternAtom`
    pub const INTERN_ATOM: u8 = 16;
    /// `ChangeProperty`
    pub const CHANGE_PROPERTY: u8 = 18;
    /// `GetInputFocus`（用作"强制一次往返"的探针）
    pub const GET_INPUT_FOCUS: u8 = 43;
    /// `PutImage`
    pub const PUT_IMAGE: u8 = 72;
    /// `GetImage`
    pub const GET_IMAGE: u8 = 73;
    /// `GetKeyboardMapping`
    pub const GET_KEYBOARD_MAPPING: u8 = 101;
    /// `SendEvent`
    pub const SEND_EVENT: u8 = 25;
    /// `QueryExtension`
    pub const QUERY_EXTENSION: u8 = 98;
    /// `ChangeWindowAttributes`
    pub const CHANGE_WINDOW_ATTRIBUTES: u8 = 2;
    /// `GetProperty`
    pub const GET_PROPERTY: u8 = 20;
    /// `SetSelectionOwner`
    pub const SET_SELECTION_OWNER: u8 = 22;
    /// `ConvertSelection`
    pub const CONVERT_SELECTION: u8 = 24;
    /// `OpenFont`
    pub const OPEN_FONT: u8 = 45;
    /// `CloseFont`
    pub const CLOSE_FONT: u8 = 46;
    /// `CreateGlyphCursor`
    pub const CREATE_GLYPH_CURSOR: u8 = 94;
    /// `FreeCursor`
    pub const FREE_CURSOR: u8 = 95;
    /// `NoOperation`
    pub const NO_OPERATION: u8 = 127;
}

/// 事件码（`code`，事件字节 0）。
pub mod event_code {
    /// `KeyPress`
    pub const KEY_PRESS: u8 = 2;
    /// `KeyRelease`
    pub const KEY_RELEASE: u8 = 3;
    /// `ButtonPress`
    pub const BUTTON_PRESS: u8 = 4;
    /// `ButtonRelease`
    pub const BUTTON_RELEASE: u8 = 5;
    /// `MotionNotify`
    pub const MOTION_NOTIFY: u8 = 6;
    /// `Expose`
    pub const EXPOSE: u8 = 12;
    /// `DestroyNotify`
    pub const DESTROY_NOTIFY: u8 = 17;
    /// `ConfigureNotify`
    pub const CONFIGURE_NOTIFY: u8 = 22;
    /// `ClientMessage`
    pub const CLIENT_MESSAGE: u8 = 33;
    /// `SelectionClear`
    pub const SELECTION_CLEAR: u8 = 29;
    /// `SelectionRequest`
    pub const SELECTION_REQUEST: u8 = 30;
    /// `SelectionNotify`
    pub const SELECTION_NOTIFY: u8 = 31;
}

/// 将 `n` 向上对齐到 4 字节的倍数。
///
/// 为什么单独抽一个函数：X11 里"补齐"出错是最常见的实现 bug，
/// 它会让后续所有字段错位。抽出来就能被单独测试。
pub fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

/// 把 `src` 追加到 `out`，并补 0 到 4 字节对齐。
pub fn push_padded(out: &mut Vec<u8>, src: &[u8]) {
    out.extend_from_slice(src);
    let pad = pad4(src.len()) - src.len();
    out.extend(std::iter::repeat_n(0u8, pad));
}

/// 向 `out` 写一个 `u16`（小端序）。
#[inline]
pub fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// 向 `out` 写一个 `u32`（小端序）。
#[inline]
pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// 从字节串读一个 `u16`（小端序）；越界返回 `None`。
#[inline]
pub fn get_u16(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
}

/// 从字节串读一个 `u32`（小端序）；越界返回 `None`。
#[inline]
pub fn get_u32(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
