//! X11 请求的**字节序编码**（只组装字节，不发送）。
//!
//! 每个函数都在注释里标注了字段偏移与长度计算方式，这是后续对照规范审查的唯一依据。

use crate::{opcode, pad4, put_u16, put_u32, push_padded, PROTOCOL_MAJOR, PROTOCOL_MINOR};

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
