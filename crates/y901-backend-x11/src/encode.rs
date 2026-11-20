//! X11 请求的**字节序编码**（只组装字节，不发送）。
//!
//! 每个函数都在注释里标注了字段偏移与长度计算方式，这是后续对照规范审查的唯一依据。

use crate::{opcode, pad4, push_padded, put_u16, put_u32, PROTOCOL_MAJOR, PROTOCOL_MINOR};

/// 窗口类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowClass {
    /// 可显示、可画（`InputOutput` = 1）
    InputOutput = 1,
    /// 只接收事件、不可画（`InputOnly` = 2）
    InputOnly = 2,
}

/// `ChangeProperty` 的写入模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropMode {
    /// 替换（= 0）
    Replace = 0,
    /// 前缀插入（= 1）
    Prepend = 1,
    /// 后缀追加（= 2）
    Append = 2,
}

/// `CreateWindow` 的 value-mask 位。
///
/// 为什么用类型而不是 `u32`：位序写错会导致"值被放到错的属性上"，
/// 而这种 bug 在运行时只表现为"窗口没出现"——极难排查。用枚举能在编译期拦住。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueMask {
    /// 背景像素（bit 0）
    BackPixmap = 1 << 0,
    /// 背景像素值（bit 1）
    BackPixel = 1 << 1,
    /// 边框像素（bit 2）
    BorderPixmap = 1 << 2,
    /// 边框像素值（bit 3）
    BorderPixel = 1 << 3,
    /// 位图（bit 4）
    BitGravity = 1 << 4,
    /// 不要窗口管理器干预（bit 9）
    OverrideRedirect = 1 << 9,
    /// 存储屏幕外区域（bit 10）
    SaveUnder = 1 << 10,
    /// 窗口事件掩码（bit 11）—— 易错点：不是 bit 10
    EventMask = 1 << 11,
    /// 不传播的事件（bit 12）
    DontPropagate = 1 << 12,
    /// 色彩表（bit 13）
    Colormap = 1 << 13,
    /// 鼠标光标形状（bit 14）
    Cursor = 1 << 14,
}

/// 编码连接初始化包（Setup request）。
///
/// 布局（小端序）：
/// ```text
/// 0   : byte-order        'l' = 0x6C
/// 1   : unused            0
/// 2-3 : protocol-major    11
/// 4-5 : protocol-minor    0
/// 6-7 : auth-name  length n
/// 8-9 : auth-data  length d
/// 10-11: unused           0
/// 12..: name[n]  (padded to 4)
/// ..  : data[d]  (padded to 4)
/// ```
///
/// 为什么这里没有 `length` 字段：Setup 是"连接建立包"，不属于请求序列，
/// 它的长度由服务端根据 n/d 推算。
pub fn encode_setup_request(auth_name: &[u8], auth_data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + pad4(auth_name.len()) + pad4(auth_data.len()));
    out.push(b'l'); // byte-order：小端序
    out.push(0); // unused
    put_u16(&mut out, PROTOCOL_MAJOR);
    put_u16(&mut out, PROTOCOL_MINOR);
    put_u16(&mut out, auth_name.len() as u16);
    put_u16(&mut out, auth_data.len() as u16);
    put_u16(&mut out, 0); // unused
    push_padded(&mut out, auth_name);
    push_padded(&mut out, auth_data);
    out
}

/// 编码 `InternAtom`（opcode 16）。
///
/// ```text
/// 0   : opcode 16
/// 1   : only-if-exists (0/1)
/// 2-3 : length = 2 + ceil(n/4)
/// 4-5 : name length n
/// 6-7 : unused
/// 8.. : name[n] (padded to 4)
/// ```
pub fn encode_intern_atom(name: &[u8], only_if_exists: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + pad4(name.len()));
    out.push(opcode::INTERN_ATOM);
    out.push(u8::from(only_if_exists));
    put_u16(&mut out, (2 + pad4(name.len()) / 4) as u16);
    put_u16(&mut out, name.len() as u16);
    put_u16(&mut out, 0); // unused
    push_padded(&mut out, name);
    out
}

/// `CreateWindow` 的参数（避免 12 个平铺参数）。
#[derive(Clone, Copy, Debug)]
pub struct CreateWindowArgs {
    /// 新窗口 ID（客户端自己分配，必须在 `XID` 区间）
    pub wid: u32,
    /// 父窗口（通常是 root）
    pub parent: u32,
    /// 相对父窗口的 x
    pub x: i16,
    /// 相对父窗口的 y
    pub y: i16,
    /// 宽
    pub width: u16,
    /// 高
    pub height: u16,
    /// 边框宽
    pub border_width: u16,
    /// 类型
    pub class: WindowClass,
    /// 可视化类型（`CopyFromParent` = 0）
    pub visual: u32,
    /// 属性列表（顺序必须与 value-mask 的位升序一致！）
    pub values: &'static [(ValueMask, u32)],
}

/// 编码 `CreateWindow`（opcode 1）。
///
/// ```text
/// 0   : opcode 1          1   : depth (0 = CopyFromParent)
/// 2-3 : length = 8 + n    4-7 : wid
/// 8-11: parent            12-13: x
/// 14-15: y                16-17: width
/// 18-19: height           20-21: border-width
/// 22-23: class (1/2)      24-27: visual
/// 28-31: value-mask       32.. : value-list（每个 4 字节）
/// ```
///
/// **关键约束**：value-list 的顺序必须与 value-mask 的位**升序**一致，
/// 否则值会被填到错属性上。这里在调用时排序，并在单元测试里验证。
pub fn encode_create_window(a: &CreateWindowArgs) -> Vec<u8> {
    // 按 mask 位升序排序，保证 value-list 顺序合法
    let mut sorted: Vec<(ValueMask, u32)> = a.values.to_vec();
    sorted.sort_by_key(|(m, _)| *m as u32);

    let mut mask: u32 = 0;
    for (m, _) in &sorted {
        mask |= *m as u32;
    }

    let mut out = Vec::with_capacity(32 + sorted.len() * 4);
    out.push(opcode::CREATE_WINDOW);
    out.push(0); // depth = CopyFromParent
    put_u16(&mut out, (8 + sorted.len()) as u16);
    put_u32(&mut out, a.wid);
    put_u32(&mut out, a.parent);
    put_u16(&mut out, a.x as u16);
    put_u16(&mut out, a.y as u16);
    put_u16(&mut out, a.width);
    put_u16(&mut out, a.height);
    put_u16(&mut out, a.border_width);
    put_u16(&mut out, a.class as u16);
    put_u32(&mut out, a.visual);
    put_u32(&mut out, mask);
    for (_, v) in &sorted {
        put_u32(&mut out, *v);
    }
    out
}

/// 编码 `MapWindow`（opcode 8）。固定 8 字节，length = 2。
pub fn encode_map_window(wid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(opcode::MAP_WINDOW);
    out.push(0); // unused
    put_u16(&mut out, 2);
    put_u32(&mut out, wid);
    out
}

/// `PutImage` 的像素格式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// 位图（= 0）
    Bitmap = 0,
    /// XY 平面（= 1）
    XyPixmap = 1,
    /// Z 序像素（= 2）—— 我们用这个
    ZPixmap = 2,
}

/// `PutImage` 的参数（避免 8 个平铺参数，也让调用点自说明）。
#[derive(Clone, Copy, Debug)]
pub struct PutImageArgs<'a> {
    /// 目标 drawable（窗口或 pixmap）
    pub drawable: u32,
    /// 图形上下文（必须先 `CreateGC`）
    pub gc: u32,
    /// 宽（像素）
    pub width: u16,
    /// 高（像素）
    pub height: u16,
    /// 目标 x
    pub dst_x: i16,
    /// 目标 y
    pub dst_y: i16,
    /// 深度（通常 24）
    pub depth: u8,
    /// BGRA 像素字节
    pub data: &'a [u8],
}

/// 编码 `PutImage`（opcode 72）—— **非 SHM 版**（S3 再做 MIT-SHM）。
///
/// ```text
/// 0   : opcode 72         1   : format (2 = ZPixmap)
/// 2-3 : length = 6 + ceil(len(data)/4)
/// 4-7 : drawable          8-11: gc
/// 12-13: width            14-15: height
/// 16-17: dst-x            18-19: dst-y
/// 20  : left-pad          21  : depth
/// 22-23: unused           24.. : data (padded to 4)
/// ```
///
/// **关键**：`data` 的行幅（bytes-per-line）由客户端决定，服务端按
/// `bits-per-pixel × width` 推算，**不会自帮你补齐每一行**。
/// 我们的 BGRA 缓冲区每像素 4 字节，行幅天然就是 `width*4`（已是 4 的倍数），
/// 所以 —— 这里没有额外补齐。
pub fn encode_put_image(a: &PutImageArgs) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + pad4(a.data.len()));
    out.push(opcode::PUT_IMAGE);
    out.push(ImageFormat::ZPixmap as u8);
    put_u16(&mut out, (6 + pad4(a.data.len()) / 4) as u16);
    put_u32(&mut out, a.drawable);
    put_u32(&mut out, a.gc);
    put_u16(&mut out, a.width);
    put_u16(&mut out, a.height);
    put_u16(&mut out, a.dst_x as u16);
    put_u16(&mut out, a.dst_y as u16);
    out.push(0); // left-pad
    out.push(a.depth);
    put_u16(&mut out, 0); // unused
    push_padded(&mut out, a.data);
    out
}

/// 编码 `ChangeProperty`（opcode 18）（用于设置窗口标题 `_NET_WM_NAME` / `WM_NAME`）。
///
/// ```text
/// 0   : opcode 18         1   : mode (0/1/2)
/// 2-3 : length = 6 + ceil(len(data)/4)
/// 4-7 : window            8-11: property atom
/// 12-15: type atom        16  : format (8/16/32)
/// 17-19: unused           20-23: data length（**按项目数，不是字节数**）
/// 24.. : data (padded to 4)
/// ```
///
/// **陷阱**：第 20-23 字节是"项目个数"而非"字节数"。
/// `format = 8` 时两者相等；`format = 32` 时项目数 = 字节数 / 4。弄错会导致标题变乱码。
pub fn encode_change_property(
    window: u32,
    property: u32,
    type_atom: u32,
    format: u8,
    mode: PropMode,
    data: &[u8],
) -> Vec<u8> {
    let items = match format {
        32 => data.len() / 4,
        16 => data.len() / 2,
        _ => data.len(),
    };
    let mut out = Vec::with_capacity(24 + pad4(data.len()));
    out.push(opcode::CHANGE_PROPERTY);
    out.push(mode as u8);
    put_u16(&mut out, (6 + pad4(data.len()) / 4) as u16);
    put_u32(&mut out, window);
    put_u32(&mut out, property);
    put_u32(&mut out, type_atom);
    out.push(format);
    out.extend_from_slice(&[0, 0, 0]); // unused
    put_u32(&mut out, items as u32);
    push_padded(&mut out, data);
    out
}

/// 编码 `GetInputFocus`（opcode 43）。
///
/// 用途：X11 **没有** `poll` 请求——事件是服务端主动推过来的。
/// 但发一个带响应的请求可以**强制一次往返**，把之前缓冲的请求全部刷出去。
/// 这是 `encode_poll_event()` 在 X11 里的**等价做法**（`NoOperation` 也行，但它无响应）。
pub fn encode_get_input_focus() -> Vec<u8> {
    vec![opcode::GET_INPUT_FOCUS, 0, 1, 0]
}

/// 编码 `NoOperation`（opcode 127）。服务端什么也不做，也不回应。
pub fn encode_no_operation() -> Vec<u8> {
    vec![opcode::NO_OPERATION, 0, 1, 0]
}

// ============================ GC / 窗口管理（S2 第二步）============================

/// `CreateGC` 的属性位（value-mask）。
///
/// 只列出本轮用到的；完整表见 X11 规范 `CreateGC` 一节。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GcMask {
    /// 画图函数（bit 0），如 `GXcopy` = 3
    Function = 1 << 0,
    /// 平面掩码（bit 1）
    PlaneMask = 1 << 1,
    /// 前景色（bit 2）
    Foreground = 1 << 2,
    /// 背景色（bit 3）
    Background = 1 << 3,
    /// 线宽（bit 4）
    LineWidth = 1 << 4,
    /// 图形暴露事件（bit 16）—— 置 0 可避免 `GraphicsExpose` 噪音
    GraphicsExposures = 1 << 16,
}

/// 编码 `CreateGC`（opcode 55）。
///
/// ```text
/// 0   : opcode 55         1   : unused
/// 2-3 : length = 4 + n
/// 4-7 : cid (GC ID)       8-11: drawable
/// 12-15: value-mask       16.. : value-list（每个 4 字节，按位升序）
/// ```
pub fn encode_create_gc(gc: u32, drawable: u32, values: &[(GcMask, u32)]) -> Vec<u8> {
    let (mask, list) = gc_mask_and_list(values);
    let mut out = Vec::with_capacity(16 + list.len() * 4);
    out.push(opcode::CREATE_GC);
    out.push(0);
    put_u16(&mut out, (4 + list.len()) as u16);
    put_u32(&mut out, gc);
    put_u32(&mut out, drawable);
    put_u32(&mut out, mask);
    for v in list {
        put_u32(&mut out, v);
    }
    out
}

/// 编码 `ChangeGC`（opcode 56）。布局与 `CreateGC` 相同，但**没有 drawable**，length = 3 + n。
pub fn encode_change_gc(gc: u32, values: &[(GcMask, u32)]) -> Vec<u8> {
    let (mask, list) = gc_mask_and_list(values);
    let mut out = Vec::with_capacity(12 + list.len() * 4);
    out.push(opcode::CHANGE_GC);
    out.push(0);
    put_u16(&mut out, (3 + list.len()) as u16);
    put_u32(&mut out, gc);
    put_u32(&mut out, mask);
    for v in list {
        put_u32(&mut out, v);
    }
    out
}

/// 编码 `FreeGC`（opcode 60）。固定 8 字节。
pub fn encode_free_gc(gc: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(opcode::FREE_GC);
    out.push(0);
    put_u16(&mut out, 2);
    put_u32(&mut out, gc);
    out
}

/// 编码 `DestroyWindow`（opcode 4）。固定 8 字节。
pub fn encode_destroy_window(wid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(opcode::DESTROY_WINDOW);
    out.push(0);
    put_u16(&mut out, 2);
    put_u32(&mut out, wid);
    out
}

/// `ConfigureWindow` 的属性位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigureMask {
    /// x 坐标（bit 0）
    X = 1 << 0,
    /// y 坐标（bit 1）
    Y = 1 << 1,
    /// 宽（bit 2）
    Width = 1 << 2,
    /// 高（bit 3）
    Height = 1 << 3,
    /// 边框宽（bit 4）
    BorderWidth = 1 << 4,
}

/// 编码 `ConfigureWindow`（opcode 12）。length = 3 + n。
pub fn encode_configure_window(wid: u32, values: &[(ConfigureMask, u32)]) -> Vec<u8> {
    let mut sorted: Vec<(ConfigureMask, u32)> = values.to_vec();
    sorted.sort_by_key(|(m, _)| *m as u32);
    let mut mask = 0u32;
    for (m, _) in &sorted {
        mask |= *m as u32;
    }
    let mut out = Vec::with_capacity(12 + sorted.len() * 4);
    out.push(opcode::CONFIGURE_WINDOW);
    out.push(0);
    put_u16(&mut out, (3 + sorted.len()) as u16);
    put_u32(&mut out, wid);
    put_u32(&mut out, mask);
    for (_, v) in &sorted {
        put_u32(&mut out, *v);
    }
    out
}

/// 编码 `SetInputFocus`（opcode 42）。固定 12 字节（length = 3）。
///
/// - `focus`：`PointerRoot` = 1，`None` = 0，或具体窗口 ID
/// - `revert_to`：`RevertToParent` = 2
/// - `time`：`CurrentTime` = 0
pub fn encode_set_input_focus(focus: u32, revert_to: u8, time: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.push(opcode::SET_INPUT_FOCUS);
    out.push(revert_to);
    put_u16(&mut out, 3);
    put_u32(&mut out, focus);
    put_u32(&mut out, time);
    out
}

/// 把属性列表按 mask 位升序排序并合并成 mask。
///
/// 为什么单独抽出来：GC 和 ConfigureWindow 都要求同样的"按位升序"约束，
/// 一处写错就会把值填到错属性上（而且服务端不报错，只是画得不对）。
fn gc_mask_and_list(values: &[(GcMask, u32)]) -> (u32, Vec<u32>) {
    let mut sorted: Vec<(GcMask, u32)> = values.to_vec();
    sorted.sort_by_key(|(m, _)| *m as u32);
    let mut mask = 0u32;
    for (m, _) in &sorted {
        mask |= *m as u32;
    }
    (mask, sorted.into_iter().map(|(_, v)| v).collect())
}

/// 编码 `GetImage`（opcode 73）—— 用于**回读像素验证"真的画上了"。
///
/// ```text
/// 0   : opcode 73         1   : format (2 = ZPixmap)
/// 2-3 : length = 5
/// 4-7 : drawable          8-9 : x
/// 10-11: y                12-13: width
/// 14-15: height           16-19: plane-mask (全 1 = 所有通道)
/// ```
pub fn encode_get_image(
    drawable: u32,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    plane_mask: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.push(opcode::GET_IMAGE);
    out.push(ImageFormat::ZPixmap as u8);
    put_u16(&mut out, 5);
    put_u32(&mut out, drawable);
    put_u16(&mut out, x as u16);
    put_u16(&mut out, y as u16);
    put_u16(&mut out, width);
    put_u16(&mut out, height);
    put_u32(&mut out, plane_mask);
    out
}

/// 编码 `GetKeyboardMapping`（opcode 101）。
///
/// ```text
/// 0   : opcode 101        1   : unused
/// 2-3 : length = 2
/// 4   : first-keycode     5   : count
/// 6-7 : unused
/// ```
///
/// 为什么要单独发它：`KeyPress` 事件只给**keycode**（物理键位），
/// 不给字符。必须先拿到这张表，才能把 keycode 译成 keysym → 字符。
pub fn encode_get_keyboard_mapping(first_keycode: u8, count: u8) -> Vec<u8> {
    vec![
        opcode::GET_KEYBOARD_MAPPING,
        0,
        2,
        0,
        first_keycode,
        count,
        0,
        0,
    ]
}

/// 编码 `SendEvent`（opcode 25）—— 把一个**构造好的 32 字节事件**发给某个窗口。
///
/// ```text
/// 0   : opcode 25         1   : propagate (0/非0)
/// 2-3 : length = 11
/// 4-7 : destination       8-11: event-mask
/// 12..43: event（32 字节）
/// ```
///
/// 用途：测试里模拟"真的有个键被按下"，而不得依赖 `xdotool` 这类外部工具
/// （CI 环境不一定有，安装还要 sudo）。
pub fn encode_send_event(destination: u32, event_mask: u32, event: &[u8; 32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(44);
    out.push(opcode::SEND_EVENT);
    out.push(0); // propagate = False
    put_u16(&mut out, 11);
    put_u32(&mut out, destination);
    put_u32(&mut out, event_mask);
    out.extend_from_slice(event);
    out
}

/// 构造一个 32 字节的 `ClientMessage` 事件（用于 SendEvent 给 root，EWMH 就是这么发的）。
///
/// 格式来源：X11 协议“ClientMessage Events”一节（事件类型 33），
/// EWMH `_NET_WM_STATE` 的发送方式见 freedesktop EWMH 规范 §7.6。
///
/// 字节布局（均为小端）：
/// ```text
/// 0     : type = 33 (ClientMessage)
/// 1     : format = 32
/// 2-3   : sequence（由服务器填充，发送时写 0）
/// 4-7   : window
/// 8-11  : message_type（atom，如 `_NET_WM_STATE`）
/// 12-31 : data[5]（每个 4 字节）
/// ```
pub fn client_message_event(window: u32, message_type: u32, data: [u32; 5]) -> [u8; 32] {
    let mut ev = [0u8; 32];
    ev[0] = 33; // ClientMessage
    ev[1] = 32; // format = 32
                // ev[2..4] 序号：服务器会覆写，保持 0
    ev[4..8].copy_from_slice(&window.to_ne_bytes());
    ev[8..12].copy_from_slice(&message_type.to_ne_bytes());
    for (i, v) in data.iter().enumerate() {
        let o = 12 + i * 4;
        ev[o..o + 4].copy_from_slice(&v.to_ne_bytes());
    }
    ev
}

/// 构造一个 32 字节的 `KeyPress`/`KeyRelease` 事件（用于 SendEvent）。
///
/// 字段布局与真实事件完全一致（见 `decode_event`）；
/// `state` 是修饰键掩码（Shift=1, Ctrl=4, Alt=8）。
pub fn synth_key_event(
    window: u32,
    keycode: u8,
    state: u16,
    x: i16,
    y: i16,
    pressed: bool,
) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0] = if pressed {
        crate::event_code::KEY_PRESS
    } else {
        crate::event_code::KEY_RELEASE
    };
    e[1] = keycode;
    e[4..8].copy_from_slice(&0u32.to_le_bytes()); // time
    e[8..12].copy_from_slice(&window.to_le_bytes()); // root（模拟时这里无所谓）
    e[12..16].copy_from_slice(&window.to_le_bytes()); // event window
    e[24..26].copy_from_slice(&(x as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(y as u16).to_le_bytes());
    e[28..30].copy_from_slice(&state.to_le_bytes());
    e[30] = 1; // same-screen
    e
}

// ============================ MIT-SHM（共享内存扩展）============================

/// 编码 `QueryExtension`（opcode 98）—— 用来探测服务器是否支持 `MIT-SHM`。
///
/// ```text
/// 0   : opcode 98         1   : unused
/// 2-3 : length = 2 + ceil(n/4)
/// 4-5 : name length n     6-7 : unused
/// 8.. : name (padded to 4)
/// ```
pub fn encode_query_extension(name: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + pad4(name.len()));
    out.push(opcode::QUERY_EXTENSION);
    out.push(0);
    put_u16(&mut out, (2 + pad4(name.len()) / 4) as u16);
    put_u16(&mut out, name.len() as u16);
    put_u16(&mut out, 0);
    push_padded(&mut out, name);
    out
}

/// 编码 `MIT-SHM` 的 `QueryVersion`（扩展内 minor opcode 0）。
///
/// 布局：`[主 opcode, 次 opcode, length=1, 0]`。
pub fn encode_shm_query_version(major_opcode: u8) -> Vec<u8> {
    vec![major_opcode, 0, 1, 0]
}

/// 编码 `MIT-SHM` 的 `Attach`（次 opcode 1）—— 把共享内存段告诉服务器。
///
/// ```text
/// 0   : 主 opcode         1   : 次 opcode = 1
/// 2-3 : length = 4
/// 4-7 : shmseg（客户端自己分配的 ID）
/// 8-11: shmid（shmget 返回的段 ID）
/// 12  : read-only (0/非0)
/// 13-15: unused
/// ```
pub fn encode_shm_attach(major_opcode: u8, shmseg: u32, shmid: u32, read_only: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.push(major_opcode);
    out.push(1);
    put_u16(&mut out, 4);
    put_u32(&mut out, shmseg);
    put_u32(&mut out, shmid);
    out.push(u8::from(read_only));
    out.extend_from_slice(&[0, 0, 0]);
    out
}

/// 编码 `MIT-SHM` 的 `Detach`（次 opcode 2）。
pub fn encode_shm_detach(major_opcode: u8, shmseg: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(major_opcode);
    out.push(2);
    put_u16(&mut out, 2);
    put_u32(&mut out, shmseg);
    out
}

/// `ShmPutImage` 的参数。
#[derive(Clone, Copy, Debug)]
pub struct ShmPutImageArgs {
    /// 目标 drawable
    pub drawable: u32,
    /// GC
    pub gc: u32,
    /// 整张图的宽（必须与共享内存里的行幅匹配）
    pub total_width: u16,
    /// 整张图的高
    pub total_height: u16,
    /// 源区域 x
    pub src_x: u16,
    /// 源区域 y
    pub src_y: u16,
    /// 源区域宽
    pub src_width: u16,
    /// 源区域高
    pub src_height: u16,
    /// 目标 x
    pub dst_x: i16,
    /// 目标 y
    pub dst_y: i16,
    /// 深度
    pub depth: u8,
    /// 是否要求完成事件（异步通知）
    pub send_event: bool,
    /// 共享内存段句柄（客户端自己分配）
    pub shmseg: u32,
    /// 像素在段内的字节偏移
    pub offset: u32,
}

/// 编码 `MIT-SHM` 的 `PutImage`（次 opcode 3）。
///
/// 布局（共 40 字节 = 10 个单位）：
/// ```text
/// 0    : 主 opcode        1    : 次 opcode = 3
/// 2-3  : length = 10
/// 4-7  : drawable          8-11 : gc
/// 12-13: total_width       14-15: total_height
/// 16-17: src_x             18-19: src_y
/// 20-21: src_width         22-23: src_height
/// 24-25: dst_x             26-27: dst_y
/// 28   : depth             29   : format (2 = ZPixmap)
/// 30   : send_event        31   : unused
/// 32-35: shmseg            36-39: offset
/// ```
///
/// **注意**：这里的 `send_event` 是"干完发一个事件告诉我"，
/// 与 `SendEvent` 请求的 `propagate` 是两回事。要接异步完成事件，
/// 还需要在 `CreateWindow` 时注册 `ShmCompletion` 事件掩码。
pub fn encode_shm_put_image(major_opcode: u8, a: &ShmPutImageArgs) -> Vec<u8> {
    let mut out = Vec::with_capacity(40);
    out.push(major_opcode);
    out.push(3);
    put_u16(&mut out, 10);
    put_u32(&mut out, a.drawable);
    put_u32(&mut out, a.gc);
    put_u16(&mut out, a.total_width);
    put_u16(&mut out, a.total_height);
    put_u16(&mut out, a.src_x);
    put_u16(&mut out, a.src_y);
    put_u16(&mut out, a.src_width);
    put_u16(&mut out, a.src_height);
    put_u16(&mut out, a.dst_x as u16);
    put_u16(&mut out, a.dst_y as u16);
    out.push(a.depth);
    out.push(ImageFormat::ZPixmap as u8);
    out.push(u8::from(a.send_event));
    out.push(0);
    put_u32(&mut out, a.shmseg);
    put_u32(&mut out, a.offset);
    out
}

// ============================ 光标（cursor 字体 → 光标形状）============================

/// 编码 `OpenFont`（opcode 45）—— 光标形状来自服务器内置的 `"cursor"` 字体。
///
/// ```text
/// 0   : opcode 45         1   : unused
/// 2-3 : length = 3 + ceil(n/4)
/// 4-7 : fid（客户端分配的字体 ID）
/// 8-9 : name length n     10-11: unused
/// 12..: name (padded to 4)
/// ```
pub fn encode_open_font(fid: u32, name: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + pad4(name.len()));
    out.push(opcode::OPEN_FONT);
    out.push(0);
    put_u16(&mut out, (3 + pad4(name.len()) / 4) as u16);
    put_u32(&mut out, fid);
    put_u16(&mut out, name.len() as u16);
    put_u16(&mut out, 0);
    push_padded(&mut out, name);
    out
}

/// 编码 `CreateGlyphCursor`（opcode 94）—— 用字体里的一个字形当光标。
///
/// ```text
/// 0   : opcode 94         1   : unused
/// 2-3 : length = 8
/// 4-7 : cid               8-11: source font
/// 12-13: source char      14-17: mask font（FONT，4 字节）
/// 18-19: mask char        20-21: fore red
/// 22-23: fore green       24-25: fore blue
/// 26-27: back red         28-29: back green
/// 30-31: back blue
/// ```
///
/// 为什么用字形而不是位图：服务器自带 `cursor` 字体，里面就是标准光标形状
/// （XC_left_ptr=68、XC_xterm=152、XC_hand2=60）——零图片资源、零上传。
#[allow(clippy::too_many_arguments)]
pub fn encode_create_glyph_cursor(
    cid: u32,
    source_font: u32,
    source_char: u16,
    mask_font: u32,
    mask_char: u16,
    fore: (u16, u16, u16),
    back: (u16, u16, u16),
) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.push(opcode::CREATE_GLYPH_CURSOR);
    out.push(0);
    put_u16(&mut out, 8);
    put_u32(&mut out, cid);
    put_u32(&mut out, source_font);
    put_u16(&mut out, source_char);
    put_u32(&mut out, mask_font);
    put_u16(&mut out, mask_char);
    put_u16(&mut out, fore.0);
    put_u16(&mut out, fore.1);
    put_u16(&mut out, fore.2);
    put_u16(&mut out, back.0);
    put_u16(&mut out, back.1);
    put_u16(&mut out, back.2);
    // 总长 = 4(头) + 4(cid) + 4(font) + 2(char) + 4(mask font) + 2(mask char) + 12(颜色) = 32 字节 = 8 单位
    out
}

/// 编码 `FreeCursor`（opcode 95）。
pub fn encode_free_cursor(cid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(opcode::FREE_CURSOR);
    out.push(0);
    put_u16(&mut out, 2);
    put_u32(&mut out, cid);
    out
}

/// 编码 `CloseFont`（opcode 46）。
pub fn encode_close_font(fid: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(opcode::CLOSE_FONT);
    out.push(0);
    put_u16(&mut out, 2);
    put_u32(&mut out, fid);
    out
}

// ============================ 剪贴板（Selections）============================

/// 编码 `SetSelectionOwner`（opcode 22）—— 声明"某个 selection 归我"。
///
/// `owner = 0` 表示主动放弃。`time = 0` 是 `CurrentTime`。
pub fn encode_set_selection_owner(owner: u32, selection: u32, time: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.push(opcode::SET_SELECTION_OWNER);
    out.push(0);
    put_u16(&mut out, 4);
    put_u32(&mut out, owner);
    put_u32(&mut out, selection);
    put_u32(&mut out, time);
    out
}

/// 编码 `ConvertSelection`（opcode 24）——"请把 selection 换成我要的格式，写到我的属性里"。
///
/// 应答是异步的：服务器会给 requestor 发一个 `SelectionNotify`。
pub fn encode_convert_selection(
    requestor: u32,
    selection: u32,
    target: u32,
    property: u32,
    time: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    out.push(opcode::CONVERT_SELECTION);
    out.push(0);
    put_u16(&mut out, 6);
    put_u32(&mut out, requestor);
    put_u32(&mut out, selection);
    put_u32(&mut out, target);
    put_u32(&mut out, property);
    put_u32(&mut out, time);
    out
}

/// 编码 `GetProperty`（opcode 20）—— 把属性内容读回来。
///
/// `delete_after`：读完是否删掉该属性（剪贴板场景用 `true`，避免残留旧数据）。
pub fn encode_get_property(
    window: u32,
    property: u32,
    type_atom: u32,
    long_offset: u32,
    long_length: u32,
    delete_after: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    out.push(opcode::GET_PROPERTY);
    out.push(u8::from(delete_after));
    put_u16(&mut out, 6);
    put_u32(&mut out, window);
    put_u32(&mut out, property);
    put_u32(&mut out, type_atom);
    put_u32(&mut out, long_offset);
    put_u32(&mut out, long_length);
    out
}

/// 编码 `GetSelectionOwner`（opcode 23）—— 查某个 selection 当前归谁。
///
/// 用途：XSettings 协议里 `_XSETTINGS_S0` 这个 selection 的 owner 就是
/// “设置管理器”的窗口，真正的键值写在它的 `_XSETTINGS_SETTINGS` 属性上。
/// 没有 owner（无桌面设置服务，如裸 Xvfb）时服务端回 `owner = 0`。
///
/// 布局（小端序）：
/// ```text
/// 0   : opcode 23
/// 1   : unused 0
/// 2-3 : length = 2
/// 4-7 : selection atom
/// ```
pub fn encode_get_selection_owner(selection: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.push(opcode::GET_SELECTION_OWNER);
    out.push(0);
    put_u16(&mut out, 2);
    put_u32(&mut out, selection);
    out
}
