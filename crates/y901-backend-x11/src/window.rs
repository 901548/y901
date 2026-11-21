//! 一个可用的 X11 窗口：把 [`Connection`] 包成 `EventSource` / `Presenter`。
//!
//! 它是 S3 “接入框架”的一半：事件与像素呈现都对外就只剩
//! `y901_runtime` 的两个 trait。另一半（`PlatformRuntime`）还需要一个**平台无关的软渲染器**，
//! 而它目前还在 `y901-backend-win32` 里（见报告的阻塞项）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use y901_render::soft::SoftBackend;
use y901_runtime::window::{RenderOutcome, WindowBackend};
use y901_runtime::{EventSource, FontConfig, Presenter, RawEvent, Waker};
use y901_ui::theme::Theme;
use y901_ui::{Key, Modifiers, MouseButton, Size, UiTree};

use crate::conn::Connection;
use crate::encode::{
    encode_create_gc, encode_create_window, encode_map_window, CreateWindowArgs, GcMask, ValueMask,
    WindowClass,
};

use crate::keys::{keysym_to_key, modifiers_from_state, Keymap};
use crate::X11Event;

/// 一个已创建并显示的 X11 窗口。
pub struct X11Window {
    conn: Connection,
    /// 窗口 ID
    pub wid: u32,
    /// GC ID
    gc: u32,
    width: u16,
    height: u16,
    depth: u8,
    keymap: Keymap,
    /// 被 `wake()` 置位的标志（`poll` 的短轮询会检查它）
    woken: Arc<AtomicBool>,
    /// MIT-SHM：有则走共享内存，没有则回退到分块 PutImage
    shm: Option<ShmState>,
    /// 三种光标形状（`None` = 服务器没有 cursor 字体，保持默认）
    cursors: Option<Cursors>,
    /// 当前光标形状（避免每帧重发请求）
    cursor_now: CursorShape,
    /// CPU 光栅器：清屏 + 整树绘制 + 上屏三段都在 [`X11Window::render_tree`] 里。
    soft: SoftBackend,
    /// 上屏用的像素暂存（`Present` 要 `&mut self`，不能同时借用 `soft`）。
    frame: Vec<u8>,
    /// 当前是否处于 fullscreen（状态机，避免重复发 EWMH）。
    fullscreen: bool,
    /// **输入法客户端**（IBus 或降级的 `NoIme`）。
    ///
    /// M1 阶段只持有连接（保证上下文不会随 D-Bus 连接销毁），
    /// 按键转发在 M2 接入 `wait_events`。
    ime: Box<dyn crate::ime::ImeClient>,
    /// 是否已经向 IME 声明过焦点（避免每个按键都发 `FocusIn`）。
    ime_focused: bool,
    /// **AT-SPI 无障碍桥**（feature `a11y`）。
    ///
    /// X11 下没有 HWND 这类句柄，AT-SPI 也不绑窗口，所以直接在 `create` 里建，
    /// 没有"必须在显示之前"的时序约束。
    #[cfg(feature = "a11y")]
    a11y: Option<y901_a11y::platform::unix_atspi::UnixAtspi>,
    /// 全屏下按 Esc：`translate`（`&self`）置位，`wait_events`（`&mut self`）消费。
    ///
    /// **为何不用进程级 `static`**：`static` 它会让两个窗口共享同一个标志 ——
    /// A 窗口的 Esc 可能被 B 窗口的 `wait_events` 消费掉，A 反而退不出全屏。
    want_exit_fs: std::cell::Cell<bool>,
    /// 运行期缩放变化的**重建配方**（`None` = 不跟随，行为与旧版一致）。
    ///
    /// 为什么存一个闭包而不是"字体 + 主题"字段：重建软后端需要**应用级**信息
    /// （字体文件、字号、主题），那是 `runtime` 的选择，窗口层不该知道。
    /// 闭包由 `runtime` 在创建后注入，窗口只负责"何时重建"。
    rescale: Option<RescaleFactory>,
    /// 当前生效的缩放比（用于判断"是否真的变了"）。
    dpi_scale: f32,
    /// 上次检查桌面设置的时间（节流：1 次/秒足够，桌面改缩放不是逐帧事件）。
    last_dpi_check: std::time::Instant,
    /// 缩放刚变时**要忽略一次的旧尺寸**。
    ///
    /// 为什么需要：改缩放会先 `ConfigureWindow` 到新尺寸，但事件队列里可能还排着
    /// 一个**旧尺寸的 `ConfigureNotify`**（用户拖动/启动时 WM 发的）。若照它重排，
    /// 就会把刚按新缩放建好的缓冲 resize 回旧尺寸 —— 字号是新的、缓冲是旧的，画面错位。
    /// 这里记住旧尺寸并吞掉一次匹配的 `ConfigureNotify`（代价是"用户恰好把窗口拖回
    /// 这个尺寸"的一次事件被忽略，无害：下一次 Expose/Configure 会补上）。
    stale_configure: std::cell::Cell<Option<(u16, u16)>>,
    /// 下一次给 MIT-SHM 段用的资源 ID 偏移（基数 | 这个值）。
    ///
    /// 为什么不能一直复用 `0x10`：重建段时必须给**新的** XID（旧的可能还被服务端附着，
    /// 复用同一个 XID 会让服务端回 BadIDChoice）。安全区间是 `0x10..0x18`：
    /// `0x01` 是窗口、`0x02` 是 GC、`0x20` 是光标字体，互不冲突；8 个槽位循环使用，
    /// 而运行期改缩放是低频操作。
    shm_slot: u32,
}

/// MIT-SHM 段可用的 XID 偏移区间（见 [`X11Window::shm_slot`]）。
const SHM_SLOT_FIRST: u32 = 0x10;
const SHM_SLOT_COUNT: u32 = 8;

/// 对外可用的光标形状（与 `y901_ui::Cursor` 一一对应）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// 箭头（XC_left_ptr = 68）
    Default,
    /// I 型（XC_xterm = 152）—— 文本输入区
    Text,
    /// 手型（XC_hand2 = 60）—— 可点击元素
    Pointer,
}

/// 预先创建好的三个光标资源。
struct Cursors {
    /// cursor 字体 ID。
    ///
    /// 为什么暂不 CloseFont：字体在**连接关闭时由服务端自动回收**，
    /// 而光标在窗口关闭前一直要用它（提前关会让光标变成 X 默认）。
    /// 保留字段供将来"字体切换"时复用。
    // 诊断 API：字体 id 由 `XSetFont` 设置后保留，供 `TextItem` 排字使用
    #[allow(dead_code)]
    font: u32,
    /// 三个光标 ID，顺序：Default / Text / Pointer
    ids: [u32; 3],
}

/// 共享内存上屏所需的全部状态。
struct ShmState {
    /// 扩展主 opcode（服务器分配）
    major: u8,
    /// 客户端自己分配的 shmseg ID
    seg_id: u32,
    /// 内存段（Drop 时自动 shmdt + shmctl）
    seg: crate::shm::ShmSegment,
    /// 段对应的像素尺寸（不匹配时需要重建）
    w: u16,
    h: u16,
}

/// [`X11Window::create`] 的参数。
///
/// **为何用结构体而非位置参数**：加字段时所有调用点都会**编译失败**，
/// 不会漏改。之前用 6 个位置参数时，加 `fullscreen` 就漏改了
/// `tests/real_window.rs` 的一个调用点（而它在 `#[cfg(unix)]` 下，
/// Windows 编译机看不到）。
///
/// 生命周期：`display` 与 `title` 是借用的字符串，其余均为值（`soft` 为所有权）。
pub struct CreateArgs<'a> {
    /// 上屏用的 X display（如 `":99"`；空字符串 = 读 `DISPLAY`）。
    pub display: &'a str,
    /// 窗口宽度（像素）。
    pub width: u16,
    /// 窗口高度（像素）。
    pub height: u16,
    /// 窗口标题。
    pub title: &'a str,
    /// 光栅器（由调用方构造；字体/主题是应用级选择）。
    pub soft: SoftBackend,
    /// 是否以 borderless fullscreen 创建。
    pub fullscreen: bool,
}

/// "按新缩放重建软后端"的配方：输入缩放比，输出按新物理尺寸建好的光栅器。
///
/// 抽成别名有两个原因：字段可读（否则一长串 `Box<dyn FnMut(...)>`），
/// 且 clippy 的 `type_complexity` 阈值不再被这一个字段顶穿。
pub type RescaleFactory = Box<dyn FnMut(f32) -> Result<SoftBackend, String>>;

/// 选择 X11 的 DPI 缩放（物理 / 逻辑）。
///
/// **限制**：X11 setup 提供的是 screen-wide 像素与毫米尺寸；真正的每显示器几何/DPI
/// 需要 XRandR（见 [`crate::randr`]）。当前只解析第一块屏（screen 0）。桌面缩放可以从三处读：
/// **XSettings**（`Xft/DPI`、`Gdk/WindowScalingFactor`，活的设置服务）、
/// 根窗口 Xresource `Xft.dpi`（会话启动时的静态快照）与 XRandR 输出的物理 DPI。
/// 都没有时退回整屏毫米估算。无任何配置时坏/缺失 EDID 尺寸仍可能导致偏差，
/// 因此真实桌面可用显式环境变量覆盖。
///
/// 优先级（高 → 低）：
/// 1. `Y901_X11_SCALE`：直接指定缩放（如 `1.5`），clamp 到 `1.0..=4.0`；
/// 2. `Y901_DPI`：指定物理 DPI（如 `144`），换算 `dpi / 96` 后吸附档位；
/// 3. XSettings：`max(Xft/DPI ÷ 96, Gdk/WindowScalingFactor)`（见
///    [`XSettingsValues::scale`]），**不吸附**；
/// 4. Xresource `Xft.dpi`（由 [`read_xft_dpi`] 读取），**不吸附**；仅接受 `72..=384`；
/// 5. **单一** XRandR 活动输出的物理 DPI（见
///    [`dpi_scale_with_xsettings_and_outputs`]；多输出时跳过，避免猜错屏）；
/// 6. `info` 的毫米尺寸估算（**吸附到 1.0 / ≥1.25 的 0.25 档位**）；
/// 7. 兜底 `1.0`。
///
/// 为什么 XSettings 排在 `Xft.dpi` 前面：XSettings 由设置管理器实时维护
/// （GNOME/KDE 改缩放会立即更新），而 `RESOURCE_MANAGER` 是 `xrdb` 在会话启动时
/// 载入的副本，改了缩放常常还是旧值。
///
/// 为什么毫米估算要吸附、而上面两处不吸附：
/// - Xvfb / 部分驱动报的 mm 只是近似（实测 1280×800 报 325mm ≈ 100 DPI），
///   连续值会得到 1.04 这种"半吊子缩放"，把像素哈希与既有截图全部扰动；
/// - 桌面惯例（GNOME/KDE 的缩放档位）就是 1.0 / 1.25 / 1.5 / 2.0；
/// - XSettings / `Xft.dpi` 已经是用户明确写下的缩放意图，再吸附会改写它。
///
/// **运行期变化**：桌面改缩放后，窗口会在下一秒的 `wait_events` 里发现
/// （见 [`X11Window::refresh_scale_now`]）并重建光栅器 + 调整客户区；不注入
/// 重建配方时窗口不查设置，行为与旧版一致。
///
/// 注意：这是**平台层的策略**，`UiTree` 只消费 `metrics.scale()`，不关心 DPI 从哪来。
pub fn dpi_scale(info: &crate::conn::ServerInfo, xft_dpi: Option<f32>) -> f32 {
    dpi_scale_with_xsettings(info, None, xft_dpi)
}

/// 同 [`dpi_scale`]，但把 XSettings（桌面设置服务）也纳入优先级。
///
/// 完整优先级（高 → 低）：环境变量 → **XSettings** → `Xft.dpi` → 屏幕毫米估算 → 1.0。
/// XSettings 与 `Xft.dpi` 都**不吸附** 0.25 档：它们是"用户/桌面写下的意图"。
/// 为什么 XSettings 排在 `Xft.dpi` 前面：XSettings 是活的设置服务（GNOME/KDE 改缩放
/// 会更新它），而 `RESOURCE_MANAGER` 是会话启动时 `xrdb` 载入的静态副本，常常过期。
pub fn dpi_scale_with_xsettings(
    info: &crate::conn::ServerInfo,
    xsettings: Option<XSettingsValues>,
    xft_dpi: Option<f32>,
) -> f32 {
    dpi_scale_with_xsettings_and_outputs(info, xsettings, xft_dpi, None)
}

/// 同 [`dpi_scale_with_xsettings`]，但把 XRandR 的输出物理 DPI 纳入**毫米估算之前**。
///
/// ## 为什么只对"单个活动输出"生效
///
/// X11 没有"每窗口 DPI"的概念；全局缩放只能在建窗前确定一次，而此时还不知道窗口
/// 会落在哪块屏。多显示器 DPI 不同时，任一全局选择都会让另一块屏的字号偏。
/// 因此本函数只在 **恰好一个** XRandR 输出可用时用它（此时它等价于更精确的屏幕毫米
/// 估算），多输出时**保持原来的整屏毫米估算**，并把"按位置选显示器"留给调用方用
/// [`crate::randr::dpi_at`]——那是每显示器 DPI 唯一不会猜错的做法。
///
/// 优先级（高 → 低）：环境变量 → XSettings → `Xft.dpi` → **单一 XRandR 输出物理 DPI**
/// → 屏幕毫米估算 → 1.0。前三级不吸附；后两级吸附到 1.0 / ≥1.25 的 0.25 档位。
pub fn dpi_scale_with_xsettings_and_outputs(
    info: &crate::conn::ServerInfo,
    xsettings: Option<XSettingsValues>,
    xft_dpi: Option<f32>,
    outputs: Option<&[crate::randr::OutputDpi]>,
) -> f32 {
    if let Ok(raw) = std::env::var("Y901_X11_SCALE") {
        if let Ok(v) = raw.trim().parse::<f32>() {
            if v.is_finite() && v > 0.0 {
                return v.clamp(1.0, 4.0);
            }
        }
    }
    if let Ok(raw) = std::env::var("Y901_DPI") {
        if let Ok(v) = raw.trim().parse::<f32>() {
            if v.is_finite() && v > 0.0 {
                return snap_scale(v / 96.0);
            }
        }
    }
    if let Some(scale) = xsettings.and_then(|x| x.scale()) {
        return scale;
    }
    if let Some(dpi) = xft_dpi.filter(|dpi| dpi.is_finite() && (72.0..=384.0).contains(dpi)) {
        return (dpi / 96.0).clamp(1.0, 4.0);
    }
    if let Some(scale) = single_output_scale(outputs) {
        return scale;
    }
    if info.width_mm == 0 {
        return 1.0;
    }
    let dpi = info.width as f32 * 25.4 / info.width_mm as f32;
    snap_scale(dpi / 96.0)
}

/// 只有一个活动输出时，用它自己的物理 DPI 推出缩放；否则 `None`。
fn single_output_scale(outputs: Option<&[crate::randr::OutputDpi]>) -> Option<f32> {
    let [output] = outputs? else {
        return None;
    };
    (output.dpi_x.is_finite() && output.dpi_x > 0.0).then(|| snap_scale(output.dpi_x / 96.0))
}

/// 从根窗口 `RESOURCE_MANAGER` 属性中读取 Xresources 的最后一个 `Xft.dpi` 数值。
pub fn xft_dpi_from_resource_manager(data: &[u8]) -> Option<f32> {
    let text = std::str::from_utf8(data).ok()?;
    text.lines().filter_map(parse_xft_dpi_line).next_back()
}

fn parse_xft_dpi_line(line: &str) -> Option<f32> {
    let (key, value) = line.split_once(':')?;
    if key.trim() != "Xft.dpi" {
        return None;
    }
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let dpi = value.parse::<f32>().ok()?;
    (dpi.is_finite() && (72.0..=384.0).contains(&dpi)).then_some(dpi)
}

/// 查询可选的根窗口 `Xft.dpi`。属性缺失、类型不符或协议错误均回退到毫米估算。
pub fn read_xft_dpi(conn: &mut crate::conn::Connection, root: u32) -> Option<f32> {
    const STRING: u32 = 31; // X11 core atom STRING
    conn.send(&crate::encode_intern_atom(b"RESOURCE_MANAGER", false))
        .ok()?;
    conn.flush().ok()?;
    let property = conn.read_reply_u32().ok()?;
    let (format, data) = conn
        .get_property(root, property, STRING, 0, 4096, false)
        .ok()?;
    (format == 8)
        .then(|| xft_dpi_from_resource_manager(&data))
        .flatten()
}

/// XSettings 里与缩放相关的两项原始值（整数语义由 [`parse_xsettings`] 负责换算）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XSettingsValues {
    /// `Xft/DPI`，单位是 **DPI**（协议里存的是 `dpi × 1024`，这里已除回）。
    pub xft_dpi: Option<f32>,
    /// `Gdk/WindowScalingFactor`（GNOME 的整数缩放档：1 / 2 / 3）。
    pub scaling_factor: Option<f32>,
}

impl XSettingsValues {
    /// 折算成缩放比（`max(Xft/DPI ÷ 96, Gdk/WindowScalingFactor)`，clamp 到 `1.0..=4.0`）。
    ///
    /// 为什么取 **max** 而不是"DPI 优先"：GNOME 在 200% 时同时给
    /// `Gdk/WindowScalingFactor = 2` 与 `Xft/DPI = 192`，两者一致；
    /// 但有些桌面只更新其中一个（例如 KDE 只改 DPI），取 max 才不会把用户
    /// 明确要的 2 倍缩放读成 1 倍。两个值都无效时返回 `None`（交给下一级来源）。
    pub fn scale(&self) -> Option<f32> {
        let by_dpi = self
            .xft_dpi
            .filter(|d| d.is_finite() && (72.0..=384.0).contains(d))
            .map(|d| d / 96.0);
        let by_factor = self
            .scaling_factor
            .filter(|s| s.is_finite() && (1.0..=4.0).contains(s));
        match (by_dpi, by_factor) {
            (Some(a), Some(b)) => Some(a.max(b)).map(|s| s.clamp(1.0, 4.0)),
            (Some(a), None) => Some(a.clamp(1.0, 4.0)),
            (None, Some(b)) => Some(b.clamp(1.0, 4.0)),
            (None, None) => None,
        }
    }
}

/// 解析 XSettings 的 `_XSETTINGS_SETTINGS` 二进制块（freedesktop 规范）。
///
/// 布局（**小端序**，即首字节 `byte_order == 0`）：
/// ```text
/// 0     : byte-order (0 = LSB)
/// 1..3  : unused
/// 4-7   : SERIAL
/// 8-11  : N_SETTINGS
/// 12..  : N 个条目，每个：
///           u8  type (0=Integer, 1=String, 2=Color)
///           u8  unused
///           u16 name_len
///           name[name_len] (补齐到 4)
///           u32 last_change_serial
///           value: Integer → u32；String → u32 len + bytes(pad4)；Color → 4×u16
/// ```
///
/// 返回 `None` 表示"整块不可解析"（字节序不对 / 截断 / 头部不完整）。
/// 单个条目的类型不认识时**跳过该条目**而不是整块失败 —— 桌面可能加新键，
/// 我们只关心 `Xft/DPI` 与 `Gdk/WindowScalingFactor`。
pub fn parse_xsettings(data: &[u8]) -> Option<XSettingsValues> {
    if data.len() < 12 {
        return None;
    }
    // 大端序客户端在本地路线不会出现（x86_64/aarch64 均小端），直接不认。
    if data[0] != 0 {
        return None;
    }
    let rd_u32 = |off: usize| -> Option<u32> {
        data.get(off..off + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let n = rd_u32(8)? as usize;
    let mut out = XSettingsValues {
        xft_dpi: None,
        scaling_factor: None,
    };
    let mut off = 12usize;
    for _ in 0..n {
        let kind = *data.get(off)?;
        let name_len = u16::from_le_bytes([*data.get(off + 2)?, *data.get(off + 3)?]) as usize;
        off += 4;
        let name = data.get(off..off + name_len)?;
        off += pad4_len(name_len);
        off += 4; // last_change_serial
        let value_start = off;
        // 先推进 `off`，再解析值：任何类型都必须正确吃掉自己的字节，
        // 否则后面所有条目都会错位（这比"读错一个键"更危险）。
        match kind {
            0 => off += 4,
            1 => {
                let len = rd_u32(off)? as usize;
                off += 4 + pad4_len(len);
            }
            2 => off += 8,
            // 未知类型：块长度未知，无法安全跳过 → 整个块作废。
            _ => return None,
        }
        if off > data.len() {
            return None;
        }
        match name {
            b"Xft/DPI" if kind == 0 => {
                let raw = rd_u32(value_start)?;
                if raw > 0 {
                    out.xft_dpi = Some(raw as f32 / 1024.0);
                }
            }
            b"Gdk/WindowScalingFactor" if kind == 0 => {
                out.scaling_factor = Some(rd_u32(value_start)? as f32);
            }
            _ => {}
        }
    }
    Some(out)
}

/// 向上取整到 4 的倍数（X11 的对齐单位）。
fn pad4_len(n: usize) -> usize {
    n.div_ceil(4) * 4
}

/// 读桌面设置服务（XSettings）里的缩放相关键。
///
/// 协议路径：`_XSETTINGS_S<screen>` 这个 selection 的 owner 就是设置管理器窗口，
/// 键值二进制块在它的 `_XSETTINGS_SETTINGS` 属性上。
/// 没有设置管理器（裸 Xvfb / 精简 WM）时 owner 为 0 → 返回 `None`，不是错误。
pub fn read_xsettings(conn: &mut crate::conn::Connection, screen: u32) -> Option<XSettingsValues> {
    let selection = intern(conn, format!("_XSETTINGS_S{screen}").as_bytes()).ok()?;
    let owner = conn.get_selection_owner(selection).ok()?;
    if owner == 0 {
        return None;
    }
    let prop = intern(conn, b"_XSETTINGS_SETTINGS").ok()?;
    let (format, data) = conn
        .get_property(owner, prop, prop, 0, 1 << 20, false)
        .ok()?;
    if format != 8 {
        return None;
    }
    parse_xsettings(&data)
}

/// 缩放吸附：`< 1.2` → 1.0；否则最近的 0.25，上限 4.0。
fn snap_scale(raw: f32) -> f32 {
    if !raw.is_finite() || raw < 1.2 {
        return 1.0;
    }
    ((raw * 4.0).round() / 4.0).clamp(1.0, 4.0)
}

impl X11Window {
    /// 连接、建窗、建 GC、显示。
    ///
    /// `soft` 由调用方准备好（字体/主题是应用级选择，窗口不替它决定）。
    /// 失败：任一步服务器报错都会把错误原文带回来（而不是吞掉）。
    /// # 调用点（改动签名时同步更新）
    ///
    /// - `crates/y901-backend-x11/src/runtime.rs`
    /// - `crates/y901-backend-x11/tests/real_window.rs`
    ///
    /// 为何写在这里：本函数在 `#[cfg(unix)]` 下，Windows 编译机**看不到**调用点，
    /// 改签名时极容易漏改测试里的那一处（已发生一次）。
    pub fn create(args: CreateArgs<'_>) -> Result<Self, String> {
        let CreateArgs {
            display,
            width,
            height,
            title,
            soft,
            fullscreen,
        } = args;
        let mut conn = Connection::connect(display)?;
        let info = conn.info().clone();

        // 标题只用 ASCII 最简形式：WM_NAME + UTF8_STRING（中文标题需要 _NET_WM_NAME，本轮不做）
        let name_atom = intern(&mut conn, b"WM_NAME")?;
        let utf8_atom = intern(&mut conn, b"UTF8_STRING")?;
        let base = info.resource_id_base;
        let wid = base | 0x01;
        let gc = base | 0x02;

        const VALUES: &[(ValueMask, u32)] = &[
            (ValueMask::BackPixel, 0x0000_0000),
            // ExposureMask(1<<15=0x8000) | KeyPress(1) | KeyRelease(2) | ButtonPress(4)
            // | ButtonRelease(8) | PointerMotion(64) | StructureNotify(1<<17=0x20000)
            (ValueMask::EventMask, 0x0002_804F),
        ];
        conn.send(&encode_create_window(&CreateWindowArgs {
            wid,
            parent: info.root,
            x: 0,
            y: 0,
            width,
            height,
            border_width: 0,
            class: WindowClass::InputOutput,
            visual: info.root_visual,
            values: VALUES,
        }))?;
        conn.send(&encode_change_title(wid, name_atom, utf8_atom, title))?;
        // **注册 `WM_PROTOCOLS = [WM_DELETE_WINDOW]`**。
        //
        // 为何必需：不注册时，用户点窗口的 × 之后，WM（如 metacity）
        // 不会发 `WM_DELETE_WINDOW` ClientMessage，而是直接 **`XKillClient`** 把客户端杀掉 ——
        // 于是我们只能从 socket EOF 得知，走"兜底退出"而不是干净路径。
        // 注册之后才是 WM 发 ClientMessage → `RawEvent::Close`（见 `map_event` 的 ClientMessage 分支）。
        //
        // **必须在 `MapWindow` 之前设置**：WM 在管理（reparent）窗口时就会读取 `WM_PROTOCOLS`，
        // 映射后才设就晚了 —— 那时 WM 仍会走 `XKillClient`。
        {
            let wm_protocols = intern(&mut conn, b"WM_PROTOCOLS")?;
            let wm_delete_window = intern(&mut conn, b"WM_DELETE_WINDOW")?;
            // 预定义原子 `ATOM = 4`（X11 协议附录 B）：`WM_PROTOCOLS` 的类型就是 ATOM 数组。
            const ATOM_ATOM: u32 = 4;
            conn.send(&crate::encode_change_property(
                wid,
                wm_protocols,
                ATOM_ATOM,
                32,
                crate::encode::PropMode::Replace,
                &wm_delete_window.to_le_bytes(),
            ))?;
            conn.flush()?;
        }

        conn.send(&encode_map_window(wid))?;

        // SPIKE: `Y901_FULLSCREEN=1` 时走 EWMH 全屏。
        // 必须在 Map 之后发：窗口还未映射时部分 WM 会忽略该消息。
        let fullscreen = fullscreen || std::env::var_os("Y901_FULLSCREEN").is_some();
        if fullscreen {
            // best-effort：全屏请求失败（无 WM、属性不受支持、协议序列异常）
            // 不应阻止窗口启动 —— 用户至少能看到一个普通窗口。
            if let Err(e) = fullscreen_atoms(&mut conn, wid, info.root, true) {
                eprintln!("[y901] EWMH fullscreen 请求失败（无 WM 或协议时序问题）：{e}");
            }
        }
        conn.flush()?;
        let errs = conn.sync()?;
        if !errs.is_empty() {
            return Err(format!("建窗失败：{errs:?}"));
        }

        // GC：GXcopy + plane-mask 全 1 + 关掉 GraphicsExpose
        const GC_VALUES: &[(GcMask, u32)] = &[
            (GcMask::Function, 3),
            (GcMask::PlaneMask, 0xFFFF_FFFF),
            (GcMask::GraphicsExposures, 0),
        ];
        conn.send(&encode_create_gc(gc, wid, GC_VALUES))?;
        conn.flush()?;
        let errs = conn.sync()?;
        if !errs.is_empty() {
            return Err(format!("CreateGC 失败：{errs:?}"));
        }

        // 键盘映射表（一次拿全，后面就不用再问服务器）
        let count = info
            .max_keycode
            .saturating_sub(info.min_keycode)
            .saturating_add(1);
        let keymap = conn.get_keyboard_mapping(info.min_keycode, count)?;

        // MIT-SHM 探测：支持就用，不支持（或失败）静默回退
        let shm = match try_enable_shm(&mut conn, base | 0x10, width, height, info.root_depth) {
            Ok(Some(state)) => {
                eprintln!(
                    "[y901] MIT-SHM 已启用（{}x{}，共享内存 {}字节）",
                    width,
                    height,
                    state.seg.size()
                );
                Some(state)
            }
            Ok(None) => {
                eprintln!("[y901] 服务器不支持 MIT-SHM，回退到分块 PutImage");
                None
            }
            Err(e) => {
                eprintln!("[y901] MIT-SHM 启用失败（{e}），回退到分块 PutImage");
                None
            }
        };

        // 光标：用服务器内置 "cursor" 字体的字形创建（零图片资源）
        let cursors = match create_cursors(&mut conn, base) {
            Ok(c) => {
                eprintln!("[y901] 光标形状已就绪：箭头 / I 型 / 手型");
                Some(c)
            }
            Err(e) => {
                eprintln!("[y901] 光标创建失败（{e}），使用默认箭头");
                None
            }
        };

        Ok(Self {
            conn,
            wid,
            gc,
            width,
            height,
            depth: info.root_depth,
            keymap,
            woken: Arc::new(AtomicBool::new(false)),
            shm,
            cursors,
            cursor_now: CursorShape::Default,
            soft,
            frame: Vec::new(),
            fullscreen,
            want_exit_fs: std::cell::Cell::new(false),
            rescale: None,
            dpi_scale: 1.0,
            last_dpi_check: std::time::Instant::now(),
            stale_configure: std::cell::Cell::new(None),
            // `create` 已经用掉 `SHM_SLOT_FIRST`，下一个槽位留给"改缩放后的重建"。
            shm_slot: SHM_SLOT_FIRST + 1,
            ime: Box::new(crate::ime::NoIme),
            ime_focused: false,
            #[cfg(feature = "a11y")]
            a11y: Some(y901_a11y::platform::unix_atspi::UnixAtspi::new()),
        })
    }

    /// 注入"运行期缩放重建配方"，并记录启动时的缩放比。
    ///
    /// 由 `runtime` 在创建窗口后调用：闭包捕获字体路径/字号/主题/逻辑尺寸，
    /// 收到新的缩放比时返回一份**按新物理尺寸**建好的 [`SoftBackend`]。
    ///
    /// 不注入时窗口完全不查桌面设置（行为与旧版一致），
    /// 这也是 `real_window` 等底层测试的默认状态。
    pub fn set_rescale_factory(&mut self, dpi_scale: f32, factory: RescaleFactory) {
        self.dpi_scale = if dpi_scale.is_finite() && dpi_scale > 0.0 {
            dpi_scale
        } else {
            1.0
        };
        self.rescale = Some(factory);
    }

    /// 当前生效的缩放比（诊断/测试用）。
    pub fn dpi_scale(&self) -> f32 {
        self.dpi_scale
    }

    /// 检查桌面设置是否改了缩放；改了就地重建。
    ///
    /// 返回 `Some((w, h))` 表示**窗口客户区已按新缩放调整**，调用方应据此重排
    /// （`wait_events` 会把它并成一个 `RawEvent::Resize`）。
    ///
    /// 为什么是"轮询"而不是 `PropertyNotify`：XSettings 的键值在别人窗口的属性上，
    /// 要订阅它的变化得给陌生窗口选 `PropertyChangeMask` 并新增 `PropertyNotify`
    /// 解码；轮询一次/秒的成本是两个小请求（InternAtom + GetProperty），
    /// 比这条链路的维护成本低得多，也不会因为漏事件而永久停在旧缩放。
    pub fn refresh_scale_now(&mut self) -> Option<(u16, u16)> {
        // 先把闭包**取出来**：否则 `self.rescale` 的可变借用会与 `self.conn` 冲突。
        let mut factory = self.rescale.take()?;
        let info = self.conn.info().clone();
        let xsettings = read_xsettings(&mut self.conn, 0);
        let xft_dpi = read_xft_dpi(&mut self.conn, info.root);
        let new_scale = dpi_scale_with_xsettings(&info, xsettings, xft_dpi);
        if (new_scale - self.dpi_scale).abs() <= 0.01 {
            self.rescale = Some(factory);
            return None;
        }
        // 保持**逻辑尺寸**不变：物理 = round(旧物理 × 新缩放 / 旧缩放)。
        let old_scale = self.dpi_scale;
        let (old_w, old_h) = (self.width, self.height);
        let ratio = new_scale / old_scale;
        let nw = ((self.width as f32 * ratio).round() as i64).clamp(1, u16::MAX as i64) as u16;
        let nh = ((self.height as f32 * ratio).round() as i64).clamp(1, u16::MAX as i64) as u16;
        match factory(new_scale) {
            Ok(soft) => {
                self.soft = soft;
                self.dpi_scale = new_scale;
                // 尺寸变了 → 旧共享内存段与新缓冲不匹配。**先让服务端 detach 再丢弃**，
                // 然后立刻按新尺寸建一段新的：否则后面每一帧都退化成分块 PutImage
                // （画面正确，但改一次缩放就永久丢掉 SHM 快路径，白白慢了）。
                self.rebuild_shm(nw, nh);
                // 记住旧尺寸：接下来可能还会收到**旧尺寸**的 ConfigureNotify，必须忽略一次。
                self.stale_configure.set(Some((old_w, old_h)));
                self.width = nw;
                self.height = nh;
                let pkt = crate::encode::encode_configure_window(
                    self.wid,
                    &[
                        (crate::encode::ConfigureMask::Width, nw as u32),
                        (crate::encode::ConfigureMask::Height, nh as u32),
                    ],
                );
                if let Err(e) = self.conn.send(&pkt).and_then(|()| self.conn.flush()) {
                    eprintln!("[y901] 缩放后调整窗口尺寸失败：{e}（继续用新缓冲）");
                }
                eprintln!(
                    "[y901] X11 缩放变化：{old_scale:.2} → {new_scale:.2}，客户区 → {nw}x{nh}"
                );
                self.rescale = Some(factory);
                Some((nw, nh))
            }
            Err(e) => {
                // 重建失败就**放弃跟随**（保留旧缩放与旧缓冲），而不是每秒重试刷日志。
                eprintln!("[y901] 缩放变化后重建光栅器失败：{e}（保持 {old_scale:.2}）");
                None
            }
        }
    }

    /// 按新尺寸重建 MIT-SHM 段；失败就静默回退到分块 PutImage（不影响正确性）。
    ///
    /// 两步都必要：
    /// 1. 先发 `ShmDetach` 让**服务端**放开旧段 —— 只 drop 本地 `ShmSegment`
    ///    （shmdt + IPC_RMID）会让段在服务端还附着时"标记删除"，直到连接关闭才真正回收；
    /// 2. 用**新的 XID**（`shm_slot` 轮转）建新段，避免与仍被附着的旧 XID 撞车。
    fn rebuild_shm(&mut self, w: u16, h: u16) {
        if let Some(old) = self.shm.take() {
            if let Err(e) = self
                .conn
                .send(&crate::encode::encode_shm_detach(old.major, old.seg_id))
                .and_then(|()| self.conn.flush())
            {
                eprintln!("[y901] 缩放后 ShmDetach 失败：{e}（旧段靠连接关闭回收）");
            } else if let Ok(errs) = self.conn.sync() {
                if !errs.is_empty() {
                    eprintln!("[y901] 缩放后 ShmDetach 报错：{errs:?}");
                }
            }
            drop(old);
        }
        let base = self.conn.info().resource_id_base;
        let slot = SHM_SLOT_FIRST + (self.shm_slot - SHM_SLOT_FIRST) % SHM_SLOT_COUNT;
        self.shm_slot = SHM_SLOT_FIRST + (slot - SHM_SLOT_FIRST + 1) % SHM_SLOT_COUNT;
        match try_enable_shm(&mut self.conn, base | slot, w, h, self.depth) {
            Ok(Some(state)) => self.shm = Some(state),
            Ok(None) => {
                // 服务器不支持 / Y901_NO_SHM：保持 None，present 走 PutImage。
            }
            Err(e) => eprintln!("[y901] 缩放后重建 SHM 段失败：{e}（回退 PutImage）"),
        }
    }

    /// 当前是否在用 MIT-SHM 快路径（诊断 / 测试用）。
    pub fn shm_active(&self) -> bool {
        self.shm.is_some()
    }

    /// 光栅器尺寸与窗口客户区对齐（启动后由 [`WindowBackend::relayout`] 调用）。
    /// **装上输入法客户端**（IBus 连接成功时由 runtime 调用）。
    ///
    /// 传 `NoIme` 即关闭 IME，与没有 IME 的行为完全一致。
    pub fn set_ime(&mut self, ime: Box<dyn crate::ime::ImeClient>) {
        self.ime = ime;
        self.ime_focused = false;
    }

    /// 当前输入法客户端是否真的可用。
    pub fn ime_available(&self) -> bool {
        self.ime.available()
    }

    /// 只读光栅器（命令行工具 / 测试用）。
    pub fn soft(&self) -> &SoftBackend {
        &self.soft
    }

    /// 可变光栅器（启动时按真实字体/主题建树后 `tree.layout` 需要它）。
    pub fn soft_mut(&mut self) -> &mut SoftBackend {
        &mut self.soft
    }

    /// 渲染一帧并提交 —— 先清屏，再画树，最后推送。
    ///
    /// 为何还要自己查**脏区**：`run_loop` 不再查 `tree.dirty()`，
    /// 它只看返回的 `RenderOutcome`；而"该不该跳过"的判定留在这个函数里。
    /// 两条渲染路径都必须有这个守卫，否则空帧也会推一帧 PutImage。
    pub fn render_tree(&mut self, tree: &mut UiTree) -> Result<(), String> {
        if tree.dirty().is_empty() {
            // 画面没变，但读屏器可能刚连上来或发了动作：无障碍这一侧照常推进。
            #[cfg(feature = "a11y")]
            self.pump_a11y(tree);
            return Ok(());
        }
        // 纯 CPU 路径：整窗重绘（与 Windows 的 --force-soft 一致）
        tree.invalidate_all();
        self.soft.clear();
        tree.render(&mut self.soft);
        // Present 需要 `&mut self`，而像素在 `self.soft` 里：先拷到暂存再上屏。
        let (w, h) = (self.soft.width(), self.soft.height());
        self.frame.clear();
        self.frame.extend_from_slice(self.soft.pixels());
        let frame = std::mem::take(&mut self.frame);
        let result = self.present(&frame, w, h);
        self.frame = frame;
        #[cfg(feature = "a11y")]
        self.pump_a11y(tree);
        result
    }

    /// **推无障碍树 + 回派读屏器动作**（feature `a11y`）。
    ///
    /// 为什么每帧全推：`snapshot()` 本来就要遍历整棵树，而 AccessKit 的
    /// `update_if_active` 在**没有读屏器连接时是空操作**，所以代价只有一次快照；
    /// 换来的是永远不会出现"无障碍树与画面不一致"。
    #[cfg(feature = "a11y")]
    fn pump_a11y(&mut self, tree: &mut UiTree) {
        let Some(bridge) = self.a11y.as_mut() else {
            return;
        };
        bridge.set_window_focused(self.ime_focused);
        bridge.push(y901_a11y::to_tree_update(tree.snapshot()));
        while let Some(req) = bridge.poll_action() {
            if matches!(req.action, y901_a11y::accesskit::Action::Click) {
                // 前序遍历序号 → Y901 节点 id（与 `to_tree_update` 的分配规则一致）。
                if let Some(id) = nth_node_id(tree, req.target_node.0 as usize) {
                    tree.dispatch_to(
                        &id,
                        &y901_ui::UiEvent::Action {
                            name: "click".to_string(),
                            args: y901_ui::json::Json::obj(),
                        },
                    );
                }
            }
        }
    }

    /// 窗口尺寸（逻辑像素）。
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// 键盘映射表（验收用）。
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// 把 X11 事件翻译成平台无关事件；不关心的返回 `None`。
    fn map_event(&self, ev: &X11Event) -> Option<RawEvent> {
        Some(match ev {
            // Expose 只告诉你"哪块需要重绘"，不是尺寸变化。
            // 对外没有"局部重绘"事件，所以这里发一个当前窗口尺寸的 Resize 作为"该画一帧"的信号。
            X11Event::Expose { .. } => RawEvent::Resize {
                w: self.width as u32,
                h: self.height as u32,
            },
            X11Event::ConfigureNotify { width, height, .. } => {
                // 缩放刚变时，事件队列里可能还排着**旧尺寸**的 ConfigureNotify：
                // 若照它重排，会把刚按新缩放建好的缓冲 resize 回去（字号新、缓冲旧 → 错位）。
                // 代价：用户恰好把窗口拖回这个尺寸的那一次事件被吞掉（下一次 Expose 会补）。
                if self.stale_configure.get() == Some((*width, *height)) {
                    self.stale_configure.set(None);
                    return None;
                }
                RawEvent::Resize {
                    w: *width as u32,
                    h: *height as u32,
                }
            }
            X11Event::Key {
                pressed,
                keycode,
                state,
                ..
            } => {
                let (plain, shifted) = self.keymap.plain_and_shift(*keycode);
                let shift = state & 1 != 0;
                let ks = if shift { shifted.or(plain) } else { plain }?;
                let key = keysym_to_key(ks)?;
                if *pressed {
                    // borderless fullscreen 没有标题栏与关闭按钮，必须给用户一条出路。
                    // 与 Win32 一致：**只在全屏下**生效；普通窗口的 Esc 仍作为普通按键上报。
                    if self.fullscreen && key == Key::Escape {
                        // 全屏下 Esc = **退出全屏**（标题栏由 WM 恢复），不再关窗。
                        // `translate` 是 `&self`，所以用全局标志，由 `wait_events` 消费。
                        self.want_exit_fs.set(true);
                        return None;
                    }
                    RawEvent::KeyDown {
                        key,
                        mods: modifiers_from_state(*state),
                    }
                } else {
                    // 对外只需 KeyDown；释放事件目前不用
                    return None;
                }
            }
            X11Event::Button {
                pressed,
                button,
                event_x,
                event_y,
                state,
                ..
            } => {
                let (x, y) = (*event_x as f32, *event_y as f32);
                match (*button, *pressed) {
                    (4, true) | (5, true) => RawEvent::Wheel {
                        x,
                        y,
                        delta: if *button == 4 { 1.0 } else { -1.0 },
                        mods: modifiers_from_state(*state),
                    },
                    (b, true) => RawEvent::MouseDown {
                        x,
                        y,
                        button: match b {
                            1 => MouseButton::Left,
                            2 => MouseButton::Middle,
                            3 => MouseButton::Right,
                            _ => MouseButton::Left,
                        },
                    },
                    (b, false) => RawEvent::MouseUp {
                        x,
                        y,
                        button: match b {
                            1 => MouseButton::Left,
                            2 => MouseButton::Middle,
                            3 => MouseButton::Right,
                            _ => MouseButton::Left,
                        },
                    },
                }
            }
            X11Event::Motion {
                event_x, event_y, ..
            } => RawEvent::MouseMove {
                x: *event_x as f32,
                y: *event_y as f32,
            },
            X11Event::Destroy { .. } => RawEvent::Close,
            X11Event::ClientMessage {
                message_type, data, ..
            } => {
                // WM_DELETE_WINDOW 的第一个 data 项是那个原子名，
                // 本轮只要收到 ClientMessage 就当关窗处理（WM_PROTOCOLS 协商已完成）
                if *message_type != 0 && data[0] != 0 {
                    RawEvent::Close
                } else {
                    return None;
                }
            }
            _ => return None,
        })
    }
}

impl X11Window {
    /// 切换窗口光标形状。
    ///
    /// 输入：形状枚举。副作用：向服务器发一个 `ChangeWindowAttributes`。
    /// 失败：`Err` 当服务器不接受（如无 cursor 字体）。
    ///
    /// 为什么要幂等：`poll` 每帧都会告知"鼠标在哪"，如果每帧都发一次请求，
    /// 就是白白浪费带宽与服务端 CPU。
    pub fn set_cursor(&mut self, shape: CursorShape) -> Result<(), String> {
        if self.cursor_now == shape {
            return Ok(());
        }
        let Some(cursors) = self.cursors.as_ref() else {
            self.cursor_now = shape;
            return Ok(());
        };
        let idx = match shape {
            CursorShape::Default => 0,
            CursorShape::Text => 1,
            CursorShape::Pointer => 2,
        };
        let cid = cursors.ids[idx];
        // ChangeWindowAttributes(window, value-mask=Cursor(1<<14), value-list=cid)
        let mut out = Vec::with_capacity(16);
        out.push(crate::opcode::CHANGE_WINDOW_ATTRIBUTES);
        out.push(0);
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&self.wid.to_le_bytes());
        out.extend_from_slice(&(1u32 << 14).to_le_bytes());
        out.extend_from_slice(&cid.to_le_bytes());
        self.conn.send(&out)?;
        self.conn.flush()?;
        self.cursor_now = shape;
        Ok(())
    }
}

impl EventSource for X11Window {
    fn poll(&mut self, timeout_ms: u32) -> Vec<RawEvent> {
        // 为什么用"短轮询 + 标志：没有 `poll(2)` 就无法同时等 socket 与自管管道；
        // 这里用 8ms 超时反复读，CPU 代价极小（空循环 125 次/秒）。
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms as u64);
        let mut out = Vec::new();
        loop {
            let slice = if timeout_ms == 0 { 0 } else { 8 };
            match self.conn.poll_events(slice) {
                Ok((events, _errors)) => {
                    for e in &events {
                        // **输入法优先**：组字中的字母、空格选词都应被 IBus 吃掉，
                        // 不能变成 `KeyDown` 去触发快捷键或插入英文。
                        if let X11Event::Key {
                            pressed: true,
                            keycode,
                            state,
                            ..
                        } = e
                        {
                            // 取 keysym：与 map_event 同一套（Shift 时用第 1 列）
                            let (plain, shifted) = self.keymap.plain_and_shift(*keycode);
                            let shift = state & 1 != 0;
                            if let Some(ks) = if shift { shifted.or(plain) } else { plain } {
                                // 首次收到按键时告知 IME “我们有焦点”（只发一次）
                                if !self.ime_focused {
                                    self.ime.focus_in();
                                    self.ime_focused = true;
                                }
                                let outcome = self.ime.filter_key(
                                    crate::keys::ibus_keyval(ks),
                                    *keycode as u32,
                                    crate::keys::ibus_state(*state),
                                );
                                if outcome == crate::ime::KeyOutcome::Consumed {
                                    continue;
                                }
                            }
                        }
                        if let Some(raw) = self.map_event(e) {
                            out.push(raw);
                        }
                    }
                    // 把 IME 侧已到达的事件（非阻塞 try_recv）并入本轮事件流
                    push_ime_events(&mut self.ime, &mut out);
                    if !out.is_empty() || timeout_ms == 0 || std::time::Instant::now() >= deadline {
                        return out;
                    }
                    if self.woken.swap(false, Ordering::SeqCst) {
                        push_ime_events(&mut self.ime, &mut out);
                        return out;
                    }
                }
                // **错误不能当“没有事件”，也不能一律当“断线”。**
                //
                // `poll_events` 内部已过滤 `WouldBlock`/`TimedOut`（返回空事件）
                // 并对 `Interrupted` 重试，所以能到这里的 `Err` 分两类：
                //
                // 1. **连接已不可用**（EOF / 真实读错 / 不完整包）→ 推 `Close` 让主循环退出；
                // 2. **配置失败**（`set_nonblocking` / `set_read_timeout` 报错，如 EINVAL）→ **不是断线**，
                //    它是我们自己写错的信号；把它当断线会让所有窗口在首帧后集体关闭（已发生）。
                Err(e) => {
                    let msg = e.to_string();
                    if msg.starts_with("set_") {
                        // 配置错误：只报一次，不退出。
                        // 留在这里是为了让 bug 可见（否则又回到"静默空转"）。
                        eprintln!("[y901] X11 轮询配置错误（非断线）：{msg}");
                    } else {
                        eprintln!("[y901] X11 连接已不可用：{msg}（发 Close 让主循环退出）");
                        out.push(RawEvent::Close);
                    }
                    return out;
                }
            }
        }
    }

    fn waker(&self) -> Arc<dyn Waker> {
        struct W(Arc<AtomicBool>);
        impl Waker for W {
            fn wake(&self) {
                // 只置位，不打断系统调用：`poll` 最多 8ms 后就会看到它
                self.0.store(true, Ordering::SeqCst);
            }
        }
        Arc::new(W(self.woken.clone()))
    }
}

/// 把 IME 客户端里已到达的事件转成 `RawEvent`（**非阻塞**）。
///
/// - `Preedit` → `RawEvent::Ime { composition: true }`（只画不插入）
/// - `Commit`  → `RawEvent::Ime { composition: false }`（正式插入）
///
/// 为何放在这里而不在 `map_event`：IME 事件不来自 X11 事件序列，
/// 而来自 IBus 信号线程，它们只能在“每轮轮询结尾”统一并入。
fn push_ime_events(ime: &mut Box<dyn crate::ime::ImeClient>, out: &mut Vec<RawEvent>) {
    for ev in ime.take_events() {
        let (text, composition) = match ev {
            crate::ime::ImeEvent::Preedit(t) => (t, true),
            crate::ime::ImeEvent::Commit(t) => (t, false),
        };
        out.push(RawEvent::Ime { text, composition });
    }
}

impl Presenter for X11Window {
    fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.width = width.max(1).min(u16::MAX as u32) as u16;
        self.height = height.max(1).min(u16::MAX as u32) as u16;
        Ok(())
    }

    fn present(&mut self, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
        let w = width.min(self.width as u32) as u16;
        let h = height.min(self.height as u32) as u16;

        // 优先走共享内存：一次 memcpy + 一个 40 字节请求
        if let Some(state) = self.shm.as_mut() {
            if state.w == w && state.h == h {
                let need = w as usize * h as usize * 4;
                if pixels.len() >= need {
                    state.seg.write(&pixels[..need])?;
                    self.conn.send(&crate::encode::encode_shm_put_image(
                        state.major,
                        &crate::encode::ShmPutImageArgs {
                            drawable: self.wid,
                            gc: self.gc,
                            total_width: w,
                            total_height: h,
                            src_x: 0,
                            src_y: 0,
                            src_width: w,
                            src_height: h,
                            dst_x: 0,
                            dst_y: 0,
                            depth: self.depth,
                            // false：不要求完成事件（我们不做阻塞等待，
                            // 下一帧的 ShmPutImage 会自然排在后面）
                            send_event: false,
                            shmseg: state.seg_id,
                            offset: 0,
                        },
                    ))?;
                    self.conn.flush()?;
                    return Ok(());
                }
            }
        }

        self.conn
            .put_image_bgra(self.wid, self.gc, w, h, pixels, self.depth)?;
        Ok(())
    }
}

impl WindowBackend for X11Window {
    /// 语义：可阻塞至多 `wait_ms`；关闭（`RawEvent::Close`）由 `run_loop` 判定。
    fn wait_events(&mut self, wait_ms: u32) -> Option<Vec<RawEvent>> {
        // 全屏下的 Esc：`translate`（`&self`）只能置标志，在这里（`&mut self`）执行切换。
        if self.want_exit_fs.replace(false) && self.fullscreen {
            if let Err(e) = self.set_fullscreen(false) {
                eprintln!("[y901] 退出全屏失败：{e}");
            }
        }
        // **运行期缩放检查**：只在注入过重建配方时才有成本（默认零开销）。
        // 节流 1 秒：桌面改缩放是低频操作，而这里是每帧入口。
        let mut resized = None;
        if self.rescale.is_some() {
            let interval = std::env::var("Y901_X11_DPI_POLL_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1000);
            if self.last_dpi_check.elapsed().as_millis() as u64 >= interval {
                self.last_dpi_check = std::time::Instant::now();
                resized = self.refresh_scale_now();
            }
        }
        let mut raws = self.poll(wait_ms);
        if let Some((w, h)) = resized {
            // 合成一个 Resize：`run_loop` 只认事件，重排/全量失效都由它统一处理。
            raws.push(RawEvent::Resize {
                w: w as u32,
                h: h as u32,
            });
        }
        Some(raws)
    }

    fn viewport(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    /// 语义：先把光栅器尺寸对齐窗口，再按新视口整树重排并全量失效。
    fn relayout(&mut self, tree: &mut UiTree) {
        let (w, h) = self.size();
        self.soft.resize(w as u32, h as u32);
        tree.layout(Size::new(w as f32, h as f32), &self.soft);
        tree.invalidate_all();
    }

    /// 脏区判定在 `render_tree` 内部（本层是唯一判定点）。
    fn render(&mut self, tree: &mut UiTree) -> Result<RenderOutcome, String> {
        // 这里只生成 outcome；实际 skip 与 Present 在 `render_tree` 内。
        let had_dirty = !tree.dirty().is_empty();
        self.render_tree(tree)?;
        Ok(if had_dirty {
            RenderOutcome::Presented
        } else {
            RenderOutcome::Skipped
        })
    }

    /// 运行时切换全屏（EWMH）。
    ///
    /// 只发 ClientMessage，边框/几何由 WM 恢复。无 WM 时消息被忽略（不报错）。
    fn set_fullscreen(&mut self, on: bool) -> Result<(), String> {
        if self.fullscreen == on {
            return Ok(()); // 状态机：不重复发
        }
        let root = self.conn.info().root;
        fullscreen_atoms(&mut self.conn, self.wid, root, on)?;
        self.fullscreen = on;
        Ok(())
    }

    fn set_theme(&mut self, theme: Theme) -> Result<(), String> {
        self.soft.set_theme(theme)
    }

    /// X11 后端启动时固定字体（与 Win32 的软后端一致：重建引擎需换字体路径，
    /// 本轮不做运行时切换）。保留默认实现即可，这里显式说明能力缺省。
    fn set_font(&mut self, _font: &FontConfig) -> Result<(), String> {
        Err("X11 后端不支持运行时切换字体（启动时固定）".to_string())
    }
}

/// 内部原子化（取回原子 ID）。
fn intern(conn: &mut Connection, name: &[u8]) -> Result<u32, String> {
    conn.send(&crate::encode_intern_atom(name, false))?;
    conn.flush()?;
    // InternAtom 的响应必须单独读：`sync()` 会丢弃非当前序列的响应
    conn.read_reply_u32()
}

/// 编码 `ChangeProperty`（WM_NAME，UTF8）。
fn encode_change_title(wid: u32, name_atom: u32, type_atom: u32, title: &str) -> Vec<u8> {
    crate::encode_change_property(
        wid,
        name_atom,
        type_atom,
        8,
        crate::PropMode::Replace,
        title.as_bytes(),
    )
}

/// `Key` 的调试名（报告与测试用）。
pub fn key_name(k: Key) -> String {
    match k {
        Key::Char(c) => format!("Char({c:?})"),
        Key::Enter => "Enter".into(),
        Key::Escape => "Escape".into(),
        Key::Backspace => "Backspace".into(),
        Key::Tab => "Tab".into(),
        Key::Delete => "Delete".into(),
        Key::Up => "Up".into(),
        Key::Down => "Down".into(),
        Key::Left => "Left".into(),
        Key::Right => "Right".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Function(n) => format!("F{n}"),
        Key::Unknown => "Unknown".into(),
    }
}

/// 修饰键展示（测试用）。
pub fn mods_name(m: &Modifiers) -> String {
    format!(
        "{}{}{}",
        if m.shift { "Shift+" } else { "" },
        if m.ctrl { "Ctrl+" } else { "" },
        if m.alt { "Alt+" } else { "" }
    )
}

/// 尝试启用 MIT-SHM：探测扩展 → 查版本 → 建段 → 挂载。
///
/// 返回：`Ok(None)` = 服务器不支持（正常情况，调用方应回退）；
/// `Err` = 支持但启用过程出错（同样回退，但要打日志）。
fn try_enable_shm(
    conn: &mut Connection,
    seg_id: u32,
    width: u16,
    height: u16,
    _depth: u8,
) -> Result<Option<ShmState>, String> {
    // 强制回退开关：用于验证"没有 SHM 时画面也对"以及性能对比
    if std::env::var_os("Y901_NO_SHM").is_some() {
        return Ok(None);
    }
    let Some(major) = conn.query_extension(b"MIT-SHM")? else {
        return Ok(None);
    };
    let (vmajor, vminor) = conn.shm_query_version(major)?;
    if vmajor == 0 && vminor == 0 {
        return Ok(None);
    }
    let bytes = width as usize * height as usize * 4;
    let seg = crate::shm::ShmSegment::new(bytes)?;
    let shmid = seg.id() as u32;
    conn.send(&crate::encode::encode_shm_attach(
        major, seg_id, shmid, false,
    ))?;
    conn.flush()?;
    let errs = conn.sync()?;
    if !errs.is_empty() {
        return Err(format!("ShmAttach 报错：{errs:?}"));
    }
    Ok(Some(ShmState {
        major,
        seg_id,
        seg,
        w: width,
        h: height,
    }))
}

/// 用服务器内置 `"cursor"` 字体的字形创建三个光标。
///
/// 字形号来自 X11 的 `cursorfont.h`：`XC_left_ptr` = 68、`XC_xterm` = 152、`XC_hand2` = 60。
fn create_cursors(conn: &mut Connection, base: u32) -> Result<Cursors, String> {
    let font = base | 0x20;
    conn.send(&crate::encode::encode_open_font(font, b"cursor"))?;
    conn.flush()?;
    let errs = conn.sync()?;
    if !errs.is_empty() {
        return Err(format!("OpenFont(cursor) 报错：{errs:?}"));
    }
    const SHAPES: [(u16, u32); 3] = [(68, 0x21), (152, 0x22), (60, 0x23)];
    let mut ids = [0u32; 3];
    for (i, (ch, off)) in SHAPES.iter().enumerate() {
        let cid = base | off;
        conn.send(&crate::encode::encode_create_glyph_cursor(
            cid,
            font,
            *ch,
            font,
            *ch,
            (0, 0, 0),
            (0xFFFF, 0xFFFF, 0xFFFF),
        ))?;
        ids[i] = cid;
    }
    conn.flush()?;
    let errs = conn.sync()?;
    if !errs.is_empty() {
        return Err(format!("CreateGlyphCursor 报错：{errs:?}"));
    }
    Ok(Cursors { font, ids })
}

/// 发 EWMH 全屏消息（`_NET_WM_STATE` + `_NET_WM_STATE_FULLSCREEN`）。
///
/// 依据 freedesktop EWMH §7.6：发一个 `ClientMessage` 到 root，
/// `data[0] = 1`（`_NET_WM_STATE_ADD`）、`data[1]` 为要新增的属性 atom。
/// 事件掩码用 `SubstructureRedirect | SubstructureNotify`（EWMH 规定的发送对象是 root）。
///
/// **注意**：若没有 WM（如 Xvfb），本消息会被忽略，窗口不会真的全屏 ——
/// 这是 EWMH 的设计（全屏由 WM 实施），不是实现缺陷。
fn fullscreen_atoms(conn: &mut Connection, wid: u32, root: u32, add: bool) -> Result<(), String> {
    let state_atom = intern(conn, b"_NET_WM_STATE")?;
    let fs_atom = intern(conn, b"_NET_WM_STATE_FULLSCREEN")?;
    // data[0]: 1 = _NET_WM_STATE_ADD，0 = _NET_WM_STATE_REMOVE（EWMH §7.6）
    let action = if add { 1 } else { 0 };
    let ev = crate::encode::client_message_event(wid, state_atom, [action, fs_atom, 0, 0, 0]);
    // SubstructureRedirectMask = 1<<20, SubstructureNotifyMask = 1<<19
    conn.send(&crate::encode::encode_send_event(
        root,
        (1 << 20) | (1 << 19),
        &ev,
    ))?;
    Ok(())
}

/// 按**前序遍历序号**取节点 id（与 `to_tree_update` 的分配顺序一致）。
///
/// `to_tree_update` 会跳过未暴露的子树，前序遍历 + 同一跳过规则即可对上序号，
/// 比让后端长期持有一份会过期的 `NodeIdMap` 简单。
#[cfg(feature = "a11y")]
fn nth_node_id(tree: &UiTree, wanted: usize) -> Option<String> {
    fn walk(node: &y901_ui::NodeInfo, seen: &mut usize, wanted: usize) -> Option<String> {
        if !node.semantics.exposed_to_ai {
            return None;
        }
        if *seen == wanted {
            return Some(node.id.clone());
        }
        *seen += 1;
        node.children.iter().find_map(|c| walk(c, seen, wanted))
    }
    walk(&tree.snapshot(), &mut 0, wanted)
}
#[cfg(test)]
mod dpi_scale_tests {
    use super::{dpi_scale, snap_scale, xft_dpi_from_resource_manager};
    use crate::conn::ServerInfo;

    fn screen(width_mm: u16) -> ServerInfo {
        ServerInfo {
            width: 1280,
            height: 800,
            width_mm,
            height_mm: 0,
            ..ServerInfo::default()
        }
    }

    /// 环境变量会覆盖推断；测试环境里若用户设了它们，跳过这条断言（不算失败）。
    fn env_clean() -> bool {
        std::env::var_os("Y901_X11_SCALE").is_none() && std::env::var_os("Y901_DPI").is_none()
    }

    #[test]
    fn snapping_uses_desktop_scale_steps() {
        assert_eq!(snap_scale(1.0), 1.0);
        assert_eq!(snap_scale(1.04), 1.0); // Xvfb 默认 ≈100 DPI：不能被当成 1.25
        assert_eq!(snap_scale(1.19), 1.0);
        assert_eq!(snap_scale(1.2), 1.25);
        assert_eq!(snap_scale(1.5), 1.5);
        assert_eq!(snap_scale(1.66), 1.75);
        assert_eq!(snap_scale(2.0), 2.0);
        assert_eq!(snap_scale(9.0), 4.0); // 上限，防奇异 EDID
        assert_eq!(snap_scale(f32::NAN), 1.0);
    }

    #[test]
    fn screen_mm_drives_dpi_scale() {
        if !env_clean() {
            eprintln!("[dpi-test] 跳过：Y901_X11_SCALE / Y901_DPI 已设置");
            return;
        }
        // 1280px / 325mm ≈ 100 DPI → 1.0
        assert_eq!(dpi_scale(&screen(325), None), 1.0);
        // 1280px / 217mm ≈ 150 DPI → 1.5
        assert_eq!(dpi_scale(&screen(217), None), 1.5);
        // 1280px / 169mm ≈ 192 DPI → 2.0
        assert_eq!(dpi_scale(&screen(169), None), 2.0);
        // mm 缺失：不猜，按 1.0
        assert_eq!(dpi_scale(&screen(0), None), 1.0);
    }

    #[test]
    fn parses_xft_dpi_from_resource_manager() {
        assert_eq!(
            xft_dpi_from_resource_manager(b"Xft.dpi: 144\n"),
            Some(144.0)
        );
        assert_eq!(
            xft_dpi_from_resource_manager(b"! comment\nXft.dpi : 110.5 \n"),
            Some(110.5)
        );
        assert_eq!(xft_dpi_from_resource_manager(b"Xft.dpi: bad\n"), None);
        assert_eq!(xft_dpi_from_resource_manager(b"Xft.dpi: 600\n"), None);
        assert_eq!(
            xft_dpi_from_resource_manager(b"Xft.dpi: 96\nXft.dpi: 144\n"),
            Some(144.0)
        );
        assert_eq!(xft_dpi_from_resource_manager(b"! no dpi\n"), None);
        assert_eq!(xft_dpi_from_resource_manager(b"\xff"), None);
    }

    #[test]
    fn xft_dpi_precedes_screen_mm_and_retains_fractional_scale() {
        if !env_clean() {
            eprintln!("[dpi-test] 跳过：Y901_X11_SCALE / Y901_DPI 已设置");
            return;
        }
        assert!((dpi_scale(&screen(325), Some(110.0)) - 110.0 / 96.0).abs() < 0.0001);
        assert_eq!(dpi_scale(&screen(325), Some(144.0)), 1.5);
        assert_eq!(dpi_scale(&screen(217), Some(40.0)), 1.5); // 无效 hint 回退 mm
    }

    // ---------------- XSettings ----------------

    use super::{dpi_scale_with_xsettings, parse_xsettings, XSettingsValues};

    /// 拼一个 XSettings 整数条目（名字补齐到 4；值 4 字节）。
    fn int_entry(name: &str, value: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(0u8); // type = Integer
        v.push(0); // unused
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.resize(4 + name.len().div_ceil(4) * 4, 0);
        v.extend_from_slice(&0u32.to_le_bytes()); // last_change_serial
        v.extend_from_slice(&value.to_le_bytes());
        v
    }

    /// 拼一个 XSettings 字符串条目（用于验证"跳过未知键"时**偏移不错位**）。
    fn string_entry(name: &str, value: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.push(1u8); // type = String
        v.push(0);
        v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.resize(4 + name.len().div_ceil(4) * 4, 0);
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&(value.len() as u32).to_le_bytes());
        v.extend_from_slice(value.as_bytes());
        v.resize(v.len() + (4 - value.len() % 4) % 4, 0);
        v
    }

    /// 拼整个 `_XSETTINGS_SETTINGS` 块。
    fn blob(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut v = vec![0u8; 12]; // byte-order(0) + unused(3) + serial + n
        v[8..12].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        for e in entries {
            v.extend_from_slice(e);
        }
        v
    }

    #[test]
    fn parses_xsettings_dpi_and_scaling_factor() {
        // 192 DPI = 192×1024；同时给 Gdk 因子 2（GNOME 200% 的典型组合）
        let data = blob(&[
            string_entry("Net/ThemeName", "Adwaita"),
            int_entry("Xft/DPI", 192 * 1024),
            int_entry("Gdk/WindowScalingFactor", 2),
            int_entry("Gtk/FontName", 0),
        ]);
        let v = parse_xsettings(&data).expect("应能解析");
        assert_eq!(v.xft_dpi, Some(192.0));
        assert_eq!(v.scaling_factor, Some(2.0));
        assert_eq!(v.scale(), Some(2.0));
    }

    /// 字符串条目的长度补齐若算错，后面所有键都会错位 —— 这条专门盯着它。
    #[test]
    fn string_entries_advance_by_padded_length() {
        for len in 1..=9usize {
            let val = "x".repeat(len);
            let data = blob(&[
                string_entry("A", &val),
                int_entry("Gdk/WindowScalingFactor", 3),
            ]);
            let v = parse_xsettings(&data).unwrap_or_else(|| panic!("len={len} 应能解析"));
            assert_eq!(
                v.scaling_factor,
                Some(3.0),
                "字符串长度 {len} 时后续条目错位"
            );
        }
    }

    #[test]
    fn malformed_xsettings_is_rejected() {
        assert_eq!(parse_xsettings(&[]), None, "空块");
        assert_eq!(parse_xsettings(&[0; 11]), None, "头部不足 12 字节");
        // 字节序标记为 1（大端）→ 不支持
        let mut big = blob(&[int_entry("Xft/DPI", 96 * 1024)]);
        big[0] = 1;
        assert_eq!(parse_xsettings(&big), None);
        // n_settings 声称有 5 条但只有 1 条
        let mut short = blob(&[int_entry("Xft/DPI", 96 * 1024)]);
        short[8..12].copy_from_slice(&5u32.to_le_bytes());
        assert_eq!(
            parse_xsettings(&short),
            None,
            "截断必须整块失败，不能返回半截结果"
        );
        // 未知类型：块长度未知，无法安全跳过
        let mut unknown = blob(&[int_entry("Xft/DPI", 96 * 1024)]);
        unknown[12] = 7;
        assert_eq!(parse_xsettings(&unknown), None);
    }

    #[test]
    fn xsettings_scale_takes_max_and_ignores_garbage() {
        let only_dpi = XSettingsValues {
            xft_dpi: Some(144.0),
            scaling_factor: None,
        };
        assert_eq!(only_dpi.scale(), Some(1.5));
        let only_factor = XSettingsValues {
            xft_dpi: None,
            scaling_factor: Some(2.0),
        };
        assert_eq!(only_factor.scale(), Some(2.0));
        // 两者冲突时取 max：桌面只更新了一个值时不会把 2 倍读成 1 倍
        let conflict = XSettingsValues {
            xft_dpi: Some(96.0),
            scaling_factor: Some(2.0),
        };
        assert_eq!(conflict.scale(), Some(2.0));
        // 越界/NaN 全部忽略
        let garbage = XSettingsValues {
            xft_dpi: Some(f32::NAN),
            scaling_factor: Some(99.0),
        };
        assert_eq!(garbage.scale(), None);
    }

    #[test]
    fn xsettings_precedes_xft_and_mm_but_env_wins() {
        if !env_clean() {
            eprintln!("[dpi-test] 跳过：Y901_X11_SCALE / Y901_DPI 已设置");
            return;
        }
        let xs = XSettingsValues {
            xft_dpi: Some(192.0),
            scaling_factor: None,
        };
        // XSettings(2.0) > Xft.dpi(1.5) > mm(1.0)
        assert_eq!(
            dpi_scale_with_xsettings(&screen(325), Some(xs), Some(144.0)),
            2.0
        );
        // 没有 XSettings 就退回 Xft.dpi
        assert_eq!(
            dpi_scale_with_xsettings(&screen(325), None, Some(144.0)),
            1.5
        );
        // 两者都没有 → mm 估算
        assert_eq!(dpi_scale_with_xsettings(&screen(217), None, None), 1.5);
    }

    #[test]
    fn single_randr_output_refines_mm_estimate_but_multi_output_does_not_guess() {
        if !env_clean() {
            eprintln!("[dpi-test] 跳过：Y901_X11_SCALE / Y901_DPI 已设置");
            return;
        }
        use super::dpi_scale_with_xsettings_and_outputs;
        use crate::randr::OutputDpi;
        let laptop = OutputDpi {
            output: 1,
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            dpi_x: 144.0,
            dpi_y: 144.0,
        };
        // 325mm 的整屏毫米估算本来得到 1.0；单个活动输出 144 DPI 修正为 1.5。
        assert_eq!(
            dpi_scale_with_xsettings_and_outputs(&screen(325), None, None, Some(&[laptop])),
            1.5
        );
        // Xft.dpi 是用户明确意图，仍然优先于 RandR。
        assert_eq!(
            dpi_scale_with_xsettings_and_outputs(&screen(325), None, Some(192.0), Some(&[laptop])),
            2.0
        );
        // 多输出（例如笔记本 144 DPI + 外接 96 DPI）不猜屏：回退到整屏毫米估算。
        let external = OutputDpi {
            output: 2,
            x: 2560,
            y: 0,
            width: 1920,
            height: 1080,
            dpi_x: 96.0,
            dpi_y: 96.0,
        };
        assert_eq!(
            dpi_scale_with_xsettings_and_outputs(
                &screen(217),
                None,
                None,
                Some(&[laptop, external])
            ),
            1.5
        );
        // 空输出列表（无 XRandR / 老服务器）= 与旧行为完全一致。
        assert_eq!(
            dpi_scale_with_xsettings_and_outputs(&screen(325), None, None, Some(&[])),
            1.0
        );
        // NaN DPI 被忽略，不污染缩放。
        let broken = OutputDpi {
            dpi_x: f32::NAN,
            ..laptop
        };
        assert_eq!(
            dpi_scale_with_xsettings_and_outputs(&screen(325), None, None, Some(&[broken])),
            1.0
        );
    }
}
