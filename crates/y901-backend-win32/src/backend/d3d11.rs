//! ## ID3D11Device
//! |-----|------|
//! | 3 | CreateBuffer |
//! | 9 | CreateRenderTargetView |
//! | 11 | CreateInputLayout |
//! | 12 | CreateVertexShader |
//! | 15 | CreatePixelShader |
//! | 20 | CreateBlendState |
//!
//! ## ID3D11DeviceContext
//! |-----|------|
//! | 7 | VSSetConstantBuffers |
//! | 9 | PSSetShader |
//! | 11 | VSSetShader |
//! | 14 | Map |
//! | 15 | Unmap |
//! | 17 | IASetInputLayout |
//! | 18 | IASetVertexBuffers |
//! | 21 | DrawInstanced |
//! | 24 | IASetPrimitiveTopology |
//! | 33 | OMSetRenderTargets |
//! | 35 | OMSetBlendState |
//! | 44 | RSSetViewports |
//! | 48 | UpdateSubresource |
//! | 50 | ClearRenderTargetView |
//!
//! ## IDXGISwapChain
//! |-----|------|
//! | 8 | Present |
//! | 9 | GetBuffer |
//! | 13 | ResizeBuffers |
//!
//! ## ID3DBlob
//! |-----|------|
//! | 3 | GetBufferPointer |
//! | 4 | GetBufferSize |
//!
//! ## Vtable 契约（本文件所有 `unsafe extern "system"` 声明与 `*Vtbl` 结构共用）
//!
//! 这里的 `*Vtbl` 必须与 Windows SDK 的 COM 接口定义**逐字段一致**：
//!
//! - `#[repr(C)]` 保证字段顺序与 C ABI 布局（去掉它，字段会被重排，槽位全错）；
//! - 槽位顺序必须与 SDK 头文件中的方法声明顺序**完全相同**（含继承自 IUnknown 的
//!   `QueryInterface` / `AddRef` / `Release` 三个前置槽）；
//! - 每个函数签名的参数与返回类型必须与 SDK 完全一致（`extern "system"` = `__stdcall`，
//!   换成 `extern "C"` 在 x64 上恰好相同、在 x86 上会让栈失衡）。
//!
//! **违反后果**：调用会从错误的槽位取函数指针并跳转 —— 表现为随机崩溃、错误的 HRESULT
//! 或静默的错误行为，且堆栈回溯通常指向无辜的调用点。
//!
//! 为什么这里不给每个声明单独写 SAFETY：这些行是**类型声明**，不是调用点；
//! 逐条复制同一段话会让真正的调用契约淹没在模板里。调用点的契约写在各 `unsafe { }` 上方。
//!
//! ## Context 契约（本文件所有 immediate context 调用共用三段式）
//!
//! 1. **单线程**：`ID3D11DeviceContext` 不是线程安全的，同一个 context 只能在本线程调用；
//!    跨线程调用会让命令流交错，驱动层表现为随机崩溃或静默丢帧。
//! 2. **同设备**：所有传给 context 的资源（buffer/texture/view/state）必须由**创建本 context 的
//!    那个 device** 创建；跨设备资源会被 D3D 调试层报错，运行期则可能直接花屏。
//! 3. **Map 配对**：`Map` 返回的指针只在配对 `Unmap` 之前有效；重复 Unmap 或使用已 Unmap 的指针
//!    属未定义行为，驱动内部状态错乱后往往在很远的位置崩溃。
//!
//! 各调用点只写「它自己额外要求什么、违反会怎样」；上面三条不再逐条复述。


use std::ffi::{c_void, CString};
use std::ptr::{null, null_mut, slice_from_raw_parts};

// # 安全契约（本文件所有 `unsafe` 代码共同依赖）
//
// 这个文件手写 D3D11/DXGI 的 COM vtable（不引入 `windows` crate），
// 所以每一次调用都是腔本的 FFI。全文件的 `unsafe` 都建立在四条不变量上：
//
// 1. **对象指针非空且未释放**：所有 COM 对象都被 `com_type!` 包成拥有型别名，
//    释放只发生在 `Drop`（调 `Release`）；因此在 `self` 存活期间 vtable 指针恒为有效。
// 2. **vtable 布局与头文件一致**：`vtbl::<T>()` 把对象开头的 vtable 指针当作 `&T`。
//    每个 `*Vtbl` 结构都按 Windows SDK 的**声明顺序**逐个对齐，错一个方法就会调到错的函数。
// 3. **鼓面数据只在有效期内使用**：`Map` 返回的指针只能用到对应 `Unmap` 为止；
//    回读 staging 纹理时按 `row_pitch` 逐行取，不假设它等于 `width * 4`。
// 4. **调用线程**：immediate context 不线程安全，所有 `unsafe fn` 必须在**拥有该 HWND 的线程**上调用。
//
// 每个 `unsafe fn` 的 `# Safety` 只写"该函数**额外**要求什么"，上面四条不重复。

use windows_sys::Win32::Foundation::HWND;

/// HRESULT；负值表示失败。
pub type Hresult = i32;

/// 单实例字节数，**必须与 `create_pipeline` 里的 InputLayout 完全一致**：改一个字段而忘了改布局，整帧会变成乱码。
pub const INSTANCE_SIZE: usize = 96;
/// 单帧实例上限。超出时按此截断而不是扩缓冲——每帧重分配缓冲比少画几个元素更糟。
pub const INSTANCE_CAPACITY: usize = 10_000;

/// 一个绘制实例（轴对齐四边形）。
///
/// 布局（与 GPU InputLayout 逐字节对应，改字段必须同步改 `create_pipeline`）：
/// | 偏移 | 字段 | 语义 |
/// |------|------|------|
/// | 0  | pos        | 左上角像素坐标 |
/// | 8  | size       | 像素尺寸 |
/// | 16 | uv_rect    | 图集 UV（`xy == zw` 表示纯色，不采样） |
/// | 32 | color      | 纯色 / 渐变起点色 |
/// | 48 | color_end  | 渐变终点色（纯色时等于 `color`） |
/// | 64 | gradient   | `(dir.x, dir.y, mode, offset)`；mode=1 表示线性渐变 |
/// | 80 | params     | `(radius, softness, 0, 0)`；radius>0 时走 SDF 圆角 |
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Instance {
    /// 左上角像素坐标。
    pub pos: [f32; 2],
    /// 像素尺寸。
    pub size: [f32; 2],
    /// 图集 UV。`xy == zw` 表示纯色/渐变（不采样），这是纯色矩形走同一管线的关键：少一个分支就少一次状态切换。
    pub uv_rect: [f32; 4],
    /// 主色（渐变起点、字形颜色）。
    pub color: [f32; 4],
    /// 渐变终点色；纯色实例与 `color` 相同。
    pub color_end: [f32; 4],
    /// `(方向 x, 方向 y, 是否渐变, 偏移)`。
    pub gradient: [f32; 4],
    /// `(radius, softness, reserved, reserved)`；`radius > 0` 时像素着色器
    /// 用 SDF 计算圆角覆盖率（`softness` 默认 1.0 = 1px 过渡）。
    pub params: [f32; 4],
}

/// 一个绘制批次：实例区间 + 用哪张纹理。
///
/// 为什么用"区间"而不是重新排序：图集实例的 Z 序就是数组顺序，
/// 排序会破坏重叠元素的叠放关系。切段只能"就近合并"。
#[derive(Clone, Copy, Debug)]
pub struct Batch {
    /// 起始实例下标
    pub start: u32,
    /// 实例数量
    pub count: u32,
    /// `None` = 图集；`Some(id)` = 图片纹理
    pub image: Option<u64>,
}

impl Instance {
    /// 纯色/字形实例：`color_end` 与 `color` 相同，渐变模式 0。
    ///
    /// 输入：位置、尺寸、UV（`xy == zw` → 纯色）、颜色。
    /// 返回：可直接推入实例流的实例。
    pub fn solid(pos: [f32; 2], size: [f32; 2], uv_rect: [f32; 4], color: [f32; 4]) -> Self {
        Self {
            pos,
            size,
            uv_rect,
            color,
            color_end: color,
            gradient: [0.0, 0.0, 0.0, 0.0],
            params: [0.0, 1.0, 0.0, 0.0],
        }
    }

    /// 圆角矩形实例：像素着色器用 SDF 算覆盖率（GPU 解析式抗锯齿）。
    ///
    /// 输入：位置、尺寸、颜色、半径（像素，自动夹到半宽/半高）。
    pub fn round_rect(pos: [f32; 2], size: [f32; 2], color: [f32; 4], radius: f32) -> Self {
        let r = radius.clamp(0.0, size[0].min(size[1]) * 0.5);
        let mut inst = Self::solid(pos, size, [0.0, 0.0, 0.0, 0.0], color);
        inst.params = [r, 1.0, 0.0, 0.0];
        inst
    }

    /// 圆角 + 线性渐变实例：先算 SDF 覆盖率，再乘渐变颜色。
    ///
    /// 输入：位置、尺寸、起点色、终点色、方向、圆角半径。
    pub fn gradient_round_rect(
        pos: [f32; 2],
        size: [f32; 2],
        from: [f32; 4],
        to: [f32; 4],
        dir: [f32; 2],
        radius: f32,
    ) -> Self {
        let mut inst = Self::gradient_rect(pos, size, from, to, dir);
        let r = radius.clamp(0.0, size[0].min(size[1]) * 0.5);
        inst.params = [r, 1.0, 0.0, 0.0];
        inst
    }

    /// 圆角描边实例：只画"轮廓带"（`|d| <= width/2`），内部与外部都 discard。
    ///
    /// 输入：位置、尺寸、颜色、描边宽度（像素）、圆角半径。
    pub fn round_stroke(
        pos: [f32; 2],
        size: [f32; 2],
        color: [f32; 4],
        width: f32,
        radius: f32,
    ) -> Self {
        let r = radius.clamp(0.0, size[0].min(size[1]) * 0.5);
        let mut inst = Self::solid(pos, size, [0.0, 0.0, 0.0, 0.0], color);
        inst.params = [r, 1.0, width.max(0.0), 0.0];
        inst
    }

    /// 线性渐变实例。
    ///
    /// 输入：矩形（像素）、起点色、终点色、`dir`（像素空间方向向量，无需归一化）。
    /// 说明：CPU 侧把 `dir` 归一化成"每像素 t 增量"并算出 `offset`，
    /// 着色器里 `t = saturate(dot(local_px, dir) + offset)`。
    pub fn gradient_rect(
        pos: [f32; 2],
        size: [f32; 2],
        from: [f32; 4],
        to: [f32; 4],
        dir: [f32; 2],
    ) -> Self {
        let len2 = dir[0] * dir[0] + dir[1] * dir[1];
        if len2 <= f32::EPSILON || size[0] <= 0.0 || size[1] <= 0.0 {
            return Self::solid(pos, size, [0.0, 0.0, 0.0, 0.0], from);
        }
        // 四角投影求 t 的区间，保证 t 恰好铺满 [0,1]
        let mut t_min = f32::MAX;
        let mut t_max = f32::MIN;
        for (cx, cy) in [
            (0.0, 0.0),
            (size[0], 0.0),
            (0.0, size[1]),
            (size[0], size[1]),
        ] {
            let t = (cx * dir[0] + cy * dir[1]) / len2;
            t_min = t_min.min(t);
            t_max = t_max.max(t);
        }
        let span = t_max - t_min;
        if span <= f32::EPSILON {
            return Self::solid(pos, size, [0.0, 0.0, 0.0, 0.0], from);
        }
        Self {
            pos,
            size,
            // xy == zw → 纯色分支（不采样图集）
            uv_rect: [0.0, 0.0, 0.0, 0.0],
            color: from,
            color_end: to,
            gradient: [
                dir[0] / (len2 * span),
                dir[1] / (len2 * span),
                1.0,
                -t_min / span,
            ],
            params: [0.0, 1.0, 0.0, 0.0],
        }
    }
}

const _: () = assert!(std::mem::size_of::<Instance>() == INSTANCE_SIZE);

/// 逐字节核对 CPU 结构体与 GPU InputLayout 的 stride / offset。
///
/// 任何一项不匹配，整个画面都会变成乱码，所以启动时直接断言。
pub fn verify_instance_layout() -> Result<(), String> {
    let size = std::mem::size_of::<Instance>();
    if size != INSTANCE_SIZE {
        return Err(format!(
            "size_of::<Instance>() = {size}, INSTANCE_SIZE = {INSTANCE_SIZE}"
        ));
    }

    let offsets = [
        ("pos", std::mem::offset_of!(Instance, pos), 0usize),
        ("size", std::mem::offset_of!(Instance, size), 8),
        ("uv_rect", std::mem::offset_of!(Instance, uv_rect), 16),
        ("color", std::mem::offset_of!(Instance, color), 32),
        ("color_end", std::mem::offset_of!(Instance, color_end), 48),
        ("gradient", std::mem::offset_of!(Instance, gradient), 64),
        ("params", std::mem::offset_of!(Instance, params), 80),
    ];
    for (name, actual, expected) in offsets {
        if actual != expected {
            return Err(format!(
                "{name} offset = {actual}, InputLayout expects {expected}"
            ));
        }
    }

    println!(
        "[y901] Instance layout ok: size={size} stride={INSTANCE_SIZE} offsets=pos:{} size:{} uv:{} color:{} end:{} grad:{} params:{}",
        std::mem::offset_of!(Instance, pos),
        std::mem::offset_of!(Instance, size),
        std::mem::offset_of!(Instance, uv_rect),
        std::mem::offset_of!(Instance, color),
        std::mem::offset_of!(Instance, color_end),
        std::mem::offset_of!(Instance, gradient),
        std::mem::offset_of!(Instance, params),
    );
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const IID_ID3D11TEXTURE2D: Guid = Guid {
    data1: 0x6f15_aaf2,
    data2: 0xd208,
    data3: 0x4e89,
    data4: [0x9a, 0xb4, 0x48, 0x95, 0x35, 0xd3, 0x4f, 0x9c],
};

const D3D11_SDK_VERSION: u32 = 7;
const D3D11_CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;
const D3D_DRIVER_TYPE_HARDWARE: u32 = 1;
const D3D_DRIVER_TYPE_WARP: u32 = 5;
const D3D_FEATURE_LEVEL_11_0: u32 = 0xb000;
const D3D_FEATURE_LEVEL_10_1: u32 = 0xa100;
const D3D_FEATURE_LEVEL_10_0: u32 = 0xa000;

const DXGI_FORMAT_R32G32_FLOAT: u32 = 16;
const DXGI_FORMAT_R32G32B32A32_FLOAT: u32 = 2;
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;
const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;

const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
const DXGI_SWAP_EFFECT_FLIP_DISCARD: u32 = 4;

const D3D11_USAGE_DYNAMIC: u32 = 2;
const D3D11_USAGE_DEFAULT: u32 = 0;
const D3D11_BIND_VERTEX_BUFFER: u32 = 0x8;
const D3D11_BIND_RENDER_TARGET: u32 = 0x20;
const D3D11_BIND_CONSTANT_BUFFER: u32 = 0x4;
const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
const D3D11_CPU_ACCESS_WRITE: u32 = 0x10000;
const D3D11_MAP_WRITE_DISCARD: u32 = 4;
/// `D3D11_MAP_READ`
const D3D11_MAP_READ: u32 = 1;
/// `D3D11_USAGE_STAGING`
const D3D11_USAGE_STAGING: u32 = 3;
/// `D3D11_CPU_ACCESS_READ`
const D3D11_CPU_ACCESS_READ: u32 = 0x20000;

const D3D11_FILTER_MIN_MAG_MIP_LINEAR: u32 = 0x15;
const D3D11_TEXTURE_ADDRESS_CLAMP: u32 = 3;
const D3D11_COMPARISON_NEVER: u32 = 1;

const D3D11_INPUT_PER_INSTANCE_DATA: u32 = 1;
const D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST: u32 = 4;

const D3D11_BLEND_ONE: u32 = 2;
const D3D11_BLEND_INV_SRC_ALPHA: u32 = 6;
const D3D11_BLEND_OP_ADD: u32 = 1;
const D3D11_COLOR_WRITE_ENABLE_ALL: u8 = 0xF;

const DXGI_STATUS_OCCLUDED: u32 = 0x087A_0001;
const DXGI_ERROR_DEVICE_REMOVED: u32 = 0x887A_0005;
const DXGI_ERROR_DEVICE_RESET: u32 = 0x887A_0007;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rational {
    numerator: u32,
    denominator: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ModeDesc {
    width: u32,
    height: u32,
    refresh_rate: Rational,
    format: u32,
    scanline_ordering: u32,
    scaling: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SampleDesc {
    count: u32,
    quality: u32,
}

#[repr(C)]
struct SwapChainDesc {
    buffer_desc: ModeDesc,
    sample_desc: SampleDesc,
    buffer_usage: u32,
    buffer_count: u32,
    output_window: HWND,
    windowed: i32,
    swap_effect: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BufferDesc {
    byte_width: u32,
    usage: u32,
    bind_flags: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
    structure_byte_stride: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputElementDesc {
    semantic_name: *const u8,
    semantic_index: u32,
    format: u32,
    input_slot: u32,
    aligned_byte_offset: u32,
    input_slot_class: u32,
    instance_data_step_rate: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RenderTargetBlendDesc {
    blend_enable: i32,
    src_blend: u32,
    dest_blend: u32,
    blend_op: u32,
    src_blend_alpha: u32,
    dest_blend_alpha: u32,
    blend_op_alpha: u32,
    render_target_write_mask: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BlendDesc {
    alpha_to_coverage_enable: i32,
    independent_blend_enable: i32,
    render_target: [RenderTargetBlendDesc; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RasterizerDesc {
    fill_mode: u32,
    cull_mode: u32,
    front_counter_clockwise: i32,
    depth_bias: i32,
    depth_bias_clamp: f32,
    slope_scaled_depth_bias: f32,
    depth_clip_enable: i32,
    scissor_enable: i32,
    multisample_enable: i32,
    antialiased_line_enable: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MappedSubresource {
    data: *mut c_void,
    row_pitch: u32,
    depth_pitch: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Viewport {
    top_left_x: f32,
    top_left_y: f32,
    width: f32,
    height: f32,
    min_depth: f32,
    max_depth: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Texture2DDesc {
    width: u32,
    height: u32,
    mip_levels: u32,
    array_size: u32,
    format: u32,
    sample_desc: SampleDesc,
    usage: u32,
    bind_flags: u32,
    cpu_access_flags: u32,
    misc_flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SubresourceData {
    data: *const c_void,
    row_pitch: u32,
    slice_pitch: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Box3d {
    left: u32,
    top: u32,
    front: u32,
    right: u32,
    bottom: u32,
    back: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SamplerDesc {
    filter: u32,
    address_u: u32,
    address_v: u32,
    address_w: u32,
    mip_lod_bias: f32,
    max_anisotropy: u32,
    comparison_func: u32,
    border_color: [f32; 4],
    min_lod: f32,
    max_lod: f32,
}

#[link(name = "d3d11")]
unsafe extern "system" {
    fn D3D11CreateDeviceAndSwapChain(
        adapter: *mut c_void,
        driver_type: u32,
        software: *mut c_void,
        flags: u32,
        feature_levels: *const u32,
        feature_level_count: u32,
        sdk_version: u32,
        swap_chain_desc: *const SwapChainDesc,
        swap_chain: *mut *mut c_void,
        device: *mut *mut c_void,
        feature_level: *mut u32,
        context: *mut *mut c_void,
    ) -> Hresult;
}

#[link(name = "d3dcompiler")]
unsafe extern "system" {
    fn D3DCompile(
        src_data: *const c_void,
        src_data_size: usize,
        source_name: *const u8,
        defines: *const c_void,
        include: *mut c_void,
        entrypoint: *const u8,
        target: *const u8,
        flags1: u32,
        flags2: u32,
        code: *mut *mut c_void,
        error_msgs: *mut *mut c_void,
    ) -> Hresult;
}

type QueryInterfaceFn =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult;
type AddRefFn = unsafe extern "system" fn(*mut c_void) -> u32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

    /// # Safety
    ///
    /// `object` 必须是**未释放的 COM 接口指针**，且它的 vtable 布局与 `T` 逐字段一致。
    /// 传错类型不会崩（仍是合法指针），但会调到错的方法——那是本文件最危险的一类 bug。
unsafe fn vtbl<T>(object: *mut c_void) -> &'static T {
    &*(*(object as *mut *const T))
}

/// # Safety
///
/// `object` 必须是一个**本模块拥有所有权**的合法 COM 接口指针（来自 `Create*`/`Query*`），或为空。
/// 违反的后果是不可诊断的堆损坏：本函数从虚表第 3 个槽取 `Release` 并调用，
/// 指针失效或虚表错位都会跳到随机地址；重复释放会把引用计数减到 0 以下。
/// 调用后不得再触碰 `object`：所有权已交回 COM。
unsafe fn release(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    #[repr(C)]
    struct Unknown {
        lp_vtbl: *const UnknownVtbl,
    }
    #[repr(C)]
    struct UnknownVtbl {
        query_interface: QueryInterfaceFn,
        add_ref: AddRefFn,
        release: ReleaseFn,
    }
    let unknown = object as *mut Unknown;
    ((*(*unknown).lp_vtbl).release)(object);
}

#[cfg(debug_assertions)]
static LIVE_OBJECTS: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

macro_rules! com_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        pub struct $name(*mut c_void);

        impl $name {
            /// # Safety
            ///
            /// `raw` 必须是某次 `Create*`/`Query*` 返回、且**所有权尚未转交他人**的接口指针；
            /// 本函数接管所有权（`Drop` 时释放）。传 null 返回 `None`，不会 panic。
            ///
            /// 违反的后果：若 `raw` 已被别处释放（例如从借用指针构造），本类会在 drop 时
            /// **二次释放**；若 `raw` 不是该接口类型，`Release` 会从错位的虚表槽跳转。
            unsafe fn from_raw(ptr: *mut c_void) -> Option<Self> {
                if ptr.is_null() {
                    return None;
                }
                #[cfg(debug_assertions)]
                LIVE_OBJECTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Some(Self(ptr))
            }

            fn as_ptr(&self) -> *mut c_void {
                self.0
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY：`self.0` 只能由 `from_raw` 写入（字段私有，外部无法构造非法值），
                // 所以本类持有它的所有权；本类不实现 `Clone`、也不向外传播 `self.0`，
                // 因此这一次是对该接口的**唯一一次** `Release`。多一次会让引用计数下降到负、堆损坏。
                unsafe { release(self.0) }
                #[cfg(debug_assertions)]
                LIVE_OBJECTS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    };
}

com_type!(Device, "ID3D11Device");
com_type!(Context, "ID3D11DeviceContext");
com_type!(SwapChain, "IDXGISwapChain");
com_type!(Texture, "ID3D11Texture2D");
com_type!(RenderTarget, "ID3D11RenderTargetView");
com_type!(Buffer, "ID3D11Buffer");
com_type!(VertexShader, "ID3D11VertexShader");
com_type!(PixelShader, "ID3D11PixelShader");
com_type!(InputLayout, "ID3D11InputLayout");
com_type!(BlendState, "ID3D11BlendState");
com_type!(RasterizerState, "ID3D11RasterizerState");
com_type!(ShaderResourceView, "ID3D11ShaderResourceView");
com_type!(SamplerState, "ID3D11SamplerState");
com_type!(Blob, "ID3DBlob");

// ============================ vtable ============================

#[repr(C)]
struct DeviceVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    create_buffer: unsafe extern "system" fn(
        *mut c_void,
        *const BufferDesc,
        *const c_void,
        *mut *mut c_void,
    ) -> Hresult,
    // 4
    reserved_0: *const c_void,
    create_texture_2d: unsafe extern "system" fn(
        *mut c_void,
        *const Texture2DDesc,
        *const SubresourceData,
        *mut *mut c_void,
    ) -> Hresult,
    // 6
    reserved_1: *const c_void,
    create_shader_resource_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const c_void,
        *mut *mut c_void,
    ) -> Hresult,
    // 8
    reserved_1b: *const c_void,
    create_render_target_view: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const c_void,
        *mut *mut c_void,
    ) -> Hresult,
    // 10
    reserved_2: *const c_void,
    create_input_layout: unsafe extern "system" fn(
        *mut c_void,
        *const InputElementDesc,
        u32,
        *const c_void,
        usize,
        *mut *mut c_void,
    ) -> Hresult,
    create_vertex_shader: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        usize,
        *mut c_void,
        *mut *mut c_void,
    ) -> Hresult,
    // 13..=14
    reserved_3: [*const c_void; 2],
    create_pixel_shader: unsafe extern "system" fn(
        *mut c_void,
        *const c_void,
        usize,
        *mut c_void,
        *mut *mut c_void,
    ) -> Hresult,
    // 16..=19
    reserved_4: [*const c_void; 4],
    create_blend_state:
        unsafe extern "system" fn(*mut c_void, *const BlendDesc, *mut *mut c_void) -> Hresult,
    /// 12：PSSetConstantBuffers（PS 也要读 cbuffer：padding.x 是 ClearType gamma）
    psset_constant_buffers: unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    create_rasterizer_state:
        unsafe extern "system" fn(*mut c_void, *const RasterizerDesc, *mut *mut c_void) -> Hresult,
    create_sampler_state:
        unsafe extern "system" fn(*mut c_void, *const SamplerDesc, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
struct ContextVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    reserved_1: [*const c_void; 4],
    vsset_constant_buffers: unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    psset_shader_resources: unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    psset_shader: unsafe extern "system" fn(*mut c_void, *mut c_void, *const *mut c_void, u32),
    psset_samplers: unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    vsset_shader: unsafe extern "system" fn(*mut c_void, *mut c_void, *const *mut c_void, u32),
    reserved_4: [*const c_void; 2],
    map: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        u32,
        u32,
        *mut MappedSubresource,
    ) -> Hresult,
    unmap: unsafe extern "system" fn(*mut c_void, *mut c_void, u32),
    /// 12：PSSetConstantBuffers（PS 也要读 cbuffer：padding.x 是 ClearType gamma）
    psset_constant_buffers: unsafe extern "system" fn(*mut c_void, u32, u32, *const *mut c_void),
    iaset_input_layout: unsafe extern "system" fn(*mut c_void, *mut c_void),
    iaset_vertex_buffers: unsafe extern "system" fn(
        *mut c_void,
        u32,
        u32,
        *const *mut c_void,
        *const u32,
        *const u32,
    ),
    reserved_6: *const c_void,
    reserved_7: *const c_void,
    draw_instanced: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32),
    reserved_8: [*const c_void; 2],
    iaset_primitive_topology: unsafe extern "system" fn(*mut c_void, u32),
    // 25..=32
    reserved_9: [*const c_void; 8],
    omset_render_targets:
        unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void, *mut c_void),
    reserved_10: *const c_void,
    omset_blend_state: unsafe extern "system" fn(*mut c_void, *mut c_void, *const f32, u32),
    // 36..=42
    reserved_11: [*const c_void; 7],
    rsset_state: unsafe extern "system" fn(*mut c_void, *mut c_void),
    rsset_viewports: unsafe extern "system" fn(*mut c_void, u32, *const Viewport),
    /// 41：RSSetScissorRects（vtable 顺序见 winapi 0.3.9 的 ID3D11DeviceContextVtbl）
    rsset_scissor_rects: unsafe extern "system" fn(*mut c_void, u32, *const RectI32),
    /// 42：CopySubresourceRegion（暂未使用，占位保持后续偏移不变）
    reserved_12: *const c_void,
    /// 43：CopyResource（快照回读：把后备缓冲拷进 staging 纹理）
    copy_resource: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void),
    update_subresource: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        u32,
        *const c_void,
        *const c_void,
        u32,
        u32,
    ),
    reserved_13: *const c_void,
    clear_render_target_view: unsafe extern "system" fn(*mut c_void, *mut c_void, *const f32),
}

/// 裁剪矩形（`RSSetScissorRects` 用；D3D11_RECT = 4 个 LONG）。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RectI32 {
    /// 矩形左边界（含）。
    pub left: i32,
    /// 矩形上边界（含）。
    pub top: i32,
    /// 矩形右边界（不含）——半开区间避免相邻脏区重复绘制同一列像素。
    pub right: i32,
    /// 矩形下边界（不含）。
    pub bottom: i32,
}

#[repr(C)]
struct SwapChainVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3..=7
    reserved_1: [*const c_void; 5],
    present: unsafe extern "system" fn(*mut c_void, u32, u32) -> Hresult,
    get_buffer:
        unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> Hresult,
    // 10..=12
    reserved_2: [*const c_void; 3],
    resize_buffers: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32, u32) -> Hresult,
}

#[repr(C)]
struct BlobVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    get_buffer_pointer: unsafe extern "system" fn(*mut c_void) -> *mut c_void,
    get_buffer_size: unsafe extern "system" fn(*mut c_void) -> usize,
}

impl Blob {
    /// # Safety
    ///
    /// `self` 必须是 `D3DCompile` 成功返回的 blob；blob 存活期间其缓冲只读有效，
    /// 返回的切片不得越过 blob 的生命周期使用。
    unsafe fn bytes(&self) -> &[u8] {
        let table = vtbl::<BlobVtbl>(self.as_ptr());
        let ptr = (table.get_buffer_pointer)(self.as_ptr()) as *const u8;
        let len = (table.get_buffer_size)(self.as_ptr());
        if ptr.is_null() || len == 0 {
            &[]
        } else {
            &*slice_from_raw_parts(ptr, len)
        }
    }
}

impl Device {
    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `desc.ByteWidth` 必须与对应 InputLayout 的 **stride 整除**（`ByteWidth % stride == 0`）；
    /// `D3D11_USAGE_DYNAMIC` 时 `CPUAccessFlags` 必须包含 `CPU_ACCESS_WRITE`，否则 Map 失败。
    /// 违反后果：调用本身可能成功（D3D11 不校验这一点），但**绘制时读到错位的顶点**——画面错乱，严重时越界读取。
    unsafe fn create_buffer(&self, desc: &BufferDesc) -> Result<Buffer, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_buffer)(self.as_ptr(), desc, null(), &mut raw);
        if hr < 0 {
            return Err(format!("CreateBuffer failed: 0x{:08X}", hr as u32));
        }
        Buffer::from_raw(raw).ok_or_else(|| "CreateBuffer returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `texture` 必须由**同一个 device** 创建，且 `BindFlags` 含 `RENDER_TARGET`。
    /// 违反后果：跨设备创建视图 → `E_INVALIDARG`；缺标志时视图可能创建成功但渲染结果未定义。
    unsafe fn create_render_target(&self, texture: &Texture) -> Result<RenderTarget, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr =
            (table.create_render_target_view)(self.as_ptr(), texture.as_ptr(), null(), &mut raw);
        if hr < 0 {
            return Err(format!(
                "CreateRenderTargetView failed: 0x{:08X}",
                hr as u32
            ));
        }
        RenderTarget::from_raw(raw).ok_or_else(|| "CreateRenderTargetView returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `elements` 的字段序、偏移与类型必须与 vertex shader 的输入签名**逐项一致**；结尾需有 `SemanticName = null` 的终止项。
    /// 违反后果：调用可能成功，但 IA 会按**错误的布局解析顶点数据**——坐标错乱或越界。
    unsafe fn create_input_layout(
        &self,
        elements: &[InputElementDesc],
        signature: &[u8],
    ) -> Result<InputLayout, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_input_layout)(
            self.as_ptr(),
            elements.as_ptr(),
            elements.len() as u32,
            signature.as_ptr() as *const c_void,
            signature.len(),
            &mut raw,
        );
        if hr < 0 {
            return Err(format!("CreateInputLayout failed: 0x{:08X}", hr as u32));
        }
        InputLayout::from_raw(raw).ok_or_else(|| "CreateInputLayout returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `code` 必须是面向 `vs_4_0`/`vs_5_0` 编译出的 DXBC 字节；传入错误 profile 会在创建时报 `E_INVALIDARG`。
    /// 违反后果：编译产物与 profile 不匹配时此处拒绝；若签名与 InputLayout 不匹配，错误会**延后到绘制时**才暴露。
    unsafe fn create_vertex_shader(&self, code: &[u8]) -> Result<VertexShader, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_vertex_shader)(
            self.as_ptr(),
            code.as_ptr() as *const c_void,
            code.len(),
            null_mut(),
            &mut raw,
        );
        if hr < 0 {
            return Err(format!("CreateVertexShader failed: 0x{:08X}", hr as u32));
        }
        VertexShader::from_raw(raw).ok_or_else(|| "CreateVertexShader returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `code` 必须是面向 `ps_4_0`/`ps_5_0` 编译出的 DXBC，且与所用 RTV 的格式兼容。
    /// 违反后果：格式不兼容时绘制会被调试层报错，否则输出颜色错位。
    unsafe fn create_pixel_shader(&self, code: &[u8]) -> Result<PixelShader, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_pixel_shader)(
            self.as_ptr(),
            code.as_ptr() as *const c_void,
            code.len(),
            null_mut(),
            &mut raw,
        );
        if hr < 0 {
            return Err(format!("CreatePixelShader failed: 0x{:08X}", hr as u32));
        }
        PixelShader::from_raw(raw).ok_or_else(|| "CreatePixelShader returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `desc` 的枚举字段（混合因子/运算）必须在 SDK 定义范围内，否则返回 `E_INVALIDARG`。
    /// 违反后果：超出范围时创建失败（本函数转为 `Err`）；合法但语义错误的值会“正常”渲染出错色。
    unsafe fn create_blend_state(&self, desc: &BlendDesc) -> Result<BlendState, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_blend_state)(self.as_ptr(), desc, &mut raw);
        if hr < 0 {
            return Err(format!("CreateBlendState failed: 0x{:08X}", hr as u32));
        }
        BlendState::from_raw(raw).ok_or_else(|| "CreateBlendState returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `desc` 中的 `FillMode`/`CullMode`/`DepthBias` 必须在 SDK 定义范围内；`DepthBias` 过大会让图元整体偏离。
    /// 违反后果：超范围时 `E_INVALIDARG`；合法但极端的值会让图元被锯掉或消失。
    unsafe fn create_rasterizer_state(
        &self,
        desc: &RasterizerDesc,
    ) -> Result<RasterizerState, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_rasterizer_state)(self.as_ptr(), desc, &mut raw);
        if hr < 0 {
            return Err(format!("CreateRasterizerState failed: 0x{:08X}", hr as u32));
        }
        RasterizerState::from_raw(raw).ok_or_else(|| "CreateRasterizerState returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// 尺寸不得超过 `D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION`（16384）；需要 SRV 时 `BindFlags` 必须含 `SHADER_RESOURCE`。
    /// `pInitialData` 非空时，行距必须 ≥ `width × 每像素字节数`，否则创建返回 `E_INVALIDARG`。
    /// 违反后果：超尺寸/缺 BindFlags 会在**后续创建资源视图时**失败（错误点距离真正原因很远，难排查）。
    unsafe fn create_texture_2d(
        &self,
        desc: &Texture2DDesc,
        data: &SubresourceData,
    ) -> Result<Texture, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_texture_2d)(self.as_ptr(), desc, data, &mut raw);
        if hr < 0 {
            return Err(format!("CreateTexture2D failed: 0x{:08X}", hr as u32));
        }
        Texture::from_raw(raw).ok_or_else(|| "CreateTexture2D returned null".into())
    }

    /// 创建一张**没有初始数据**的纹理（渲染目标用）。
    ///
    /// 与 [`Device::create_texture_2d`] 的唯一区别：`pInitialData = null`。
    /// 自持 RT 每帧会被清屏覆盖，因此不需要初始内容。
    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// 同 `create_texture_2d`，但**不提供初始数据**（`pInitialData = null`）；因此内容在首次写入前是未定义的。
    /// 违反后果：若以为它已清零，首帧会出现**未初始化素材**（花屏）。
    unsafe fn create_texture_2d_empty(&self, desc: &Texture2DDesc) -> Result<Texture, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_texture_2d)(self.as_ptr(), desc, null(), &mut raw);
        if hr < 0 {
            return Err(format!("CreateTexture2D failed: 0x{:08X}", hr as u32));
        }
        Texture::from_raw(raw).ok_or_else(|| "CreateTexture2D returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// `texture` 必须由**同一个 device** 创建，且创建时 `BindFlags` 必须含 `SHADER_RESOURCE`。
    /// 违反后果：跨设备 → `E_INVALIDARG`；缺标志 → 创建失败，且错误信息指向 SRV 而不是真正的原因（贴图步骤）。
    unsafe fn create_shader_resource_view(
        &self,
        texture: &Texture,
    ) -> Result<ShaderResourceView, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr =
            (table.create_shader_resource_view)(self.as_ptr(), texture.as_ptr(), null(), &mut raw);
        if hr < 0 {
            return Err(format!(
                "CreateShaderResourceView failed: 0x{:08X}",
                hr as u32
            ));
        }
        ShaderResourceView::from_raw(raw)
            .ok_or_else(|| "CreateShaderResourceView returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程且未释放**的 ID3D11Device；出参指针指向的结构在调用期间保持有效。
    /// 成功时返回的 COM 接口**所有权转交调用方**（本类 `Drop` 负责 Release 一次）。
    /// 本函数不接受描述结构，内部固定为双线性 + Clamp；若需其他采样方式必须改本函数而非传参。
    /// 违反后果：无（内部枚举已固定）；但同一 device 上多个 sampler 共存时，PS 引用的槽位必须与绑定一致。
    unsafe fn create_sampler_state(&self) -> Result<SamplerState, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let desc = SamplerDesc {
            filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            address_u: D3D11_TEXTURE_ADDRESS_CLAMP,
            address_v: D3D11_TEXTURE_ADDRESS_CLAMP,
            address_w: D3D11_TEXTURE_ADDRESS_CLAMP,
            mip_lod_bias: 0.0,
            max_anisotropy: 1,
            comparison_func: D3D11_COMPARISON_NEVER,
            border_color: [0.0; 4],
            min_lod: 0.0,
            max_lod: f32::MAX,
        };
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_sampler_state)(self.as_ptr(), &desc, &mut raw);
        if hr < 0 {
            return Err(format!("CreateSamplerState failed: 0x{:08X}", hr as u32));
        }
        SamplerState::from_raw(raw).ok_or_else(|| "CreateSamplerState returned null".into())
    }
}

impl Context {
    /// # Safety
    ///
    /// `target` 的视图必须由**创建本 context 的 device** 产生；传 `None` 是合法的（解绑）。
    /// 违反后果：跨设备 RTV 在 `Present` 时表现为花屏或整帧被丢弃（不报错）。
    unsafe fn set_render_target(&self, target: Option<&RenderTarget>) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        match target {
            Some(target) => {
                let views = [target.as_ptr()];
                (table.omset_render_targets)(self.as_ptr(), 1, views.as_ptr(), null_mut());
            }
            None => (table.omset_render_targets)(self.as_ptr(), 0, null(), null_mut()),
        }
    }

    /// # Safety
    ///
    /// `target` 的 RTV 必须由同一 device 创建且当前已绑定为渲染目标。
    /// 违反后果：清一个未绑定的目标会**清掉别的目标**或什么都不做（取决于驱动），画面残留上一帧。
    unsafe fn clear(&self, target: &RenderTarget, color: [f32; 4]) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.clear_render_target_view)(self.as_ptr(), target.as_ptr(), color.as_ptr());
    }

    /// # Safety
    ///
    /// `width`/`height` 必须 > 0 且 ≤ `D3D11_VIEWPORT_BOUNDS_MAX`；它们决定 NDC 到像素的映射。
    /// 违反后果：为 0 时**整个 draw 被静默丢弃**（没有错误码，画面只是空着）；
    /// 超出上限时视口被驱动 clamp，几何会画到错误的像素区域。
    unsafe fn set_viewport(&self, width: f32, height: f32) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let viewport = Viewport {
            top_left_x: 0.0,
            top_left_y: 0.0,
            width,
            height,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        (table.rsset_viewports)(self.as_ptr(), 1, &viewport);
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程独占**的 immediate context（D3D11 context 非线程安全，不得跨线程调用）；
    /// 传入的资源必须由同一设备创建且尚未释放；`Map` 得到的指针只在配对的 `Unmap` 之前有效。
    unsafe fn update_constant_buffer(&self, buffer: &Buffer, data: *const c_void) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.update_subresource)(self.as_ptr(), buffer.as_ptr(), 0, null(), data, 0, 0);
    }

    /// 整张纹理更新（mip 0）。
    /// # Safety
    ///
    /// 目标纹理不能是 `D3D11_USAGE_DYNAMIC`；`data` 的行距必须 ≥ `width × 每像素字节数`。
    /// 违反后果：两者都返回 `void`（**没有 HRESULT 可查**）；行距不足时会读到 `data` 之外的越界内存。
    unsafe fn update_subresource(&self, texture: &Texture, data: &SubresourceData) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.update_subresource)(
            self.as_ptr(),
            texture.as_ptr(),
            0,
            null(),
            data.data,
            data.row_pitch,
            data.slice_pitch,
        );
    }

    /// 局部区域更新：用 D3D11_BOX 限定写入范围。
    #[allow(clippy::too_many_arguments)]
    /// # Safety
    ///
    /// `self` 必须是**本线程独占**的 immediate context（D3D11 context 非线程安全，不得跨线程调用）；
    /// 传入的资源必须由同一设备创建且尚未释放；`Map` 得到的指针只在配对的 `Unmap` 之前有效。
    unsafe fn update_subresource_region(
        &self,
        texture: &Texture,
        data: &SubresourceData,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let bbox = Box3d {
            left: x,
            top: y,
            front: 0,
            right: x + width,
            bottom: y + height,
            back: 1,
        };
        (table.update_subresource)(
            self.as_ptr(),
            texture.as_ptr(),
            0,
            &bbox as *const Box3d as *const c_void,
            data.data,
            data.row_pitch,
            data.slice_pitch,
        );
    }

    /// # Safety
    ///
    /// `buffer` 必须是 `D3D11_USAGE_DYNAMIC` 且带 `CPU_ACCESS_WRITE`；
    /// **`D3D11_MAP_WRITE_DISCARD` 之后旧内容视为未定义**，读到的将是垃圾。
    /// 违反后果：对非 DYNAMIC buffer 调用会失败并返回 `None`（本函数已处理）；
    /// 若把 `Some` 里的指针当成「只写了前半段、后半段仍是旧数据」来用，会读到任意内容。
unsafe fn map_discard(&self, buffer: &Buffer) -> Option<MappedSubresource> {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let mut mapped: MappedSubresource = std::mem::zeroed();
        let hr = (table.map)(
            self.as_ptr(),
            buffer.as_ptr(),
            0,
            D3D11_MAP_WRITE_DISCARD,
            0,
            &mut mapped,
        );
        (hr >= 0 && !mapped.data.is_null()).then_some(mapped)
    }

    /// # Safety
    ///
    /// 必须与同一次 `map_discard` 严格配对，且**只能调用一次**。
    /// 违反后果：重复 Unmap 是未定义行为 —— 驱动内部状态错乱，可能立即崩溃或静默损坏后续帧。
unsafe fn unmap(&self, buffer: &Buffer) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.unmap)(self.as_ptr(), buffer.as_ptr(), 0);
    }

    /// 解除对一个**纹理**资源的 Map（`Unmap` 接受任意 `ID3D11Resource`）。
    /// # Safety
    ///
    /// 必须与同一次 `map_read` 严格配对；`texture` 必须与 Map 时是同一个对象。
    /// 违反后果：对未 Map 的纹理 Unmap → 驱动未定义行为；对另一个纹理 Unmap 会解错绑定。
    unsafe fn unmap_texture(&self, texture: &Texture) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.unmap)(self.as_ptr(), texture.as_ptr(), 0);
    }

    /// # Safety
    ///
    /// `pipeline` 内的各个 state/shader/InputLayout 必须与该 pipeline 的 VS 签名**互相匹配**。
    /// 违反后果：不匹配的组合在绑定时不报错，绘制时表现为顶点错位或三角形消失。
    unsafe fn set_pipeline(&self, pipeline: &Pipeline) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());

        (table.iaset_input_layout)(self.as_ptr(), pipeline.input_layout.as_ptr());

        (table.vsset_shader)(self.as_ptr(), pipeline.vertex_shader.as_ptr(), null(), 0);
        (table.psset_shader)(self.as_ptr(), pipeline.pixel_shader.as_ptr(), null(), 0);

        let constants = [pipeline.constant_buffer.as_ptr()];
        (table.vsset_constant_buffers)(self.as_ptr(), 0, 1, constants.as_ptr());
        // PS 也要绑：`padding.x`（gamma）是在像素着色器里读的
        (table.psset_constant_buffers)(self.as_ptr(), 0, 1, constants.as_ptr());

        let buffers = [pipeline.instance_buffer.as_ptr()];
        let strides = [INSTANCE_SIZE as u32];
        let offsets = [0u32];
        (table.iaset_vertex_buffers)(
            self.as_ptr(),
            0,
            1,
            buffers.as_ptr(),
            strides.as_ptr(),
            offsets.as_ptr(),
        );

        (table.iaset_primitive_topology)(self.as_ptr(), D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

        (table.omset_blend_state)(self.as_ptr(), pipeline.blend.as_ptr(), null(), u32::MAX);
        (table.rsset_state)(self.as_ptr(), pipeline.rasterizer.as_ptr());
    }

    /// # Safety
    ///
    /// `count` 个实例必须已被实例化 buffer 覆盖；InputLayout 与 VS 必须已绑定。
    /// 违反后果：实例数超出 buffer 容量 → GPU 侧**越界读取**（通常不崩溃，但画面错乱且难定位）。
    unsafe fn draw_instances(&self, count: u32, start_offset: u32) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.draw_instanced)(self.as_ptr(), 6, count, start_offset, 0);
    }

    /// 把图片 SRV 绑到 t1（图集在 t0，互不影响）。
    /// # Safety
    ///
    /// `self` 必须是**本线程独占**的 immediate context（D3D11 context 非线程安全，不得跨线程调用）；
    /// 传入的资源必须由同一设备创建且尚未释放；`Map` 得到的指针只在配对的 `Unmap` 之前有效。
    unsafe fn set_ps_image_texture(&self, srv: &ShaderResourceView) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let views = [srv.as_ptr()];
        (table.psset_shader_resources)(self.as_ptr(), 1, 1, views.as_ptr());
    }

    /// # Safety
    ///
    /// `srv` 与 `sampler` 必须同属本 device，且 SRV 对应的纹理创建时带 `SHADER_RESOURCE`。
    /// 违反后果：绑定不匹配时 PS 采样到未定义内容 —— 贴图变黑/错位，且不会产生任何错误码。
    unsafe fn set_ps_texture(&self, srv: &ShaderResourceView, sampler: &SamplerState) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let views = [srv.as_ptr()];
        (table.psset_shader_resources)(self.as_ptr(), 0, 1, views.as_ptr());
        let samplers = [sampler.as_ptr()];
        (table.psset_samplers)(self.as_ptr(), 0, 1, samplers.as_ptr());
    }

    /// 设置裁剪矩形（`RSSetScissorRects`，需要光栅化状态里 `scissor_enable = 1`）。
    ///
    /// 输入：`rect`（像素，右/下为**开区间**）。
    /// 副作用：之后所有 draw 只影响该矩形内的像素。
    // 诊断 API：com_type! 生成的接口别名只在内部与测试里具名使用；删除会削弱类型检查
// 归类：诊断 API —— `set_scissor` 只在 `Y901_SCISSOR_TEST` 打开时调用
    #[allow(dead_code)]
    /// # Safety
    ///
    /// 矩形必须非负且不超出当前 RTV 尺寸；调用前 `scissor_enable` 必须为 1，否则此设置不生效。
    /// 违反后果：超出 RTV 的部分被驱动 clamp —— 不报错，但裁剪结果与预期不同（隐蔽的裁切错误）。
unsafe fn set_scissor(&self, rect: RectI32) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.rsset_scissor_rects)(self.as_ptr(), 1, &rect);
    }

    /// 整资源拷贝（`CopyResource`）：要求同格式、同尺寸、同 mip 数。
    /// # Safety
    ///
    /// `src`/`dst` 必须是同一 device 创建、**同尺寸同格式**；参数是裸指针，类型信息已丢失。
    /// 违反后果：`CopyResource` 返回 `void`，尺寸/格式不匹配时**错误被吞掉**，画面出现错位或花屏。
    unsafe fn copy_resource(&self, dst: *mut c_void, src: *mut c_void) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.copy_resource)(self.as_ptr(), dst, src);
    }

    /// 以只读方式 Map 一个 staging 纹理。
    ///
    /// 返回：映射指针与行跨度（`row_pitch`）；失败返回 `None`。
    /// # Safety
    ///
    /// `texture` 必须是 `D3D11_USAGE_STAGING` 且带 `CPU_ACCESS_READ`，否则 `Map` 失败。
    /// 违反后果：返回 `None`（本函数已处理）；若忽略 `None` 并解引用空指针 → 段错误。
    unsafe fn map_read(&self, texture: &Texture) -> Option<MappedSubresource> {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let mut mapped: MappedSubresource = std::mem::zeroed();
        let hr = (table.map)(
            self.as_ptr(),
            texture.as_ptr(),
            0,
            D3D11_MAP_READ,
            0,
            &mut mapped,
        );
        if hr < 0 {
            None
        } else {
            Some(mapped)
        }
    }

    /// 只换像素着色器（保留 VS / InputLayout / 混合 / 光栅化状态）。
    ///
    /// 用途：blit 阶段复用同一套顶点管线，只把 `ps_main` 换成 `ps_blit`。
    /// # Safety
    ///
    /// `ps` 必须由本 device 创建，且其资源绑定布局与当前绑定的 SRV/sampler 槽位一致。
    /// 违反后果：槽位不一致时 shader 读到别的资源 —— 采样结果错乱，但调用本身成功。
    unsafe fn set_ps_shader(&self, ps: &PixelShader) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.psset_shader)(self.as_ptr(), ps.as_ptr(), null(), 0);
    }
}

impl SwapChain {
    /// # Safety
    ///
    /// `self` 必须是**本线程独占**的 immediate context（D3D11 context 非线程安全，不得跨线程调用）；
    /// 传入的资源必须由同一设备创建且尚未释放；`Map` 得到的指针只在配对的 `Unmap` 之前有效。
    unsafe fn back_buffer(&self, index: u32) -> Result<Texture, String> {
        let table = vtbl::<SwapChainVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.get_buffer)(self.as_ptr(), index, &IID_ID3D11TEXTURE2D, &mut raw);
        if hr < 0 {
            return Err(format!("GetBuffer failed: 0x{:08X}", hr as u32));
        }
        Texture::from_raw(raw).ok_or_else(|| "GetBuffer returned null".into())
    }

    /// # Safety
    ///
    /// `self` 必须是**本线程独占**的 immediate context（D3D11 context 非线程安全，不得跨线程调用）；
    /// 传入的资源必须由同一设备创建且尚未释放；`Map` 得到的指针只在配对的 `Unmap` 之前有效。
    unsafe fn resize_buffers(&self, width: u32, height: u32) -> Result<(), String> {
        let table = vtbl::<SwapChainVtbl>(self.as_ptr());
        let hr = (table.resize_buffers)(self.as_ptr(), 0, width, height, 0, 0);
        if hr < 0 {
            return Err(format!("ResizeBuffers failed: 0x{:08X}", hr as u32));
        }
        Ok(())
    }

        /// # Safety
    ///
    /// 必须在**拥有该交换链窗口的线程**上调用；`sync_interval` 为 0 时不等待帧同步。
unsafe fn present(&self, sync_interval: u32) -> Hresult {
        let table = vtbl::<SwapChainVtbl>(self.as_ptr());
        (table.present)(self.as_ptr(), sync_interval, 0)
    }
}

const SHADER_SOURCE: &str = r#"
cbuffer Globals : register(b0)
{
    float2 screen_size;
    float2 padding;
};

Texture2D atlas_texture : register(t0);
// 图片纹理（第 2 个描述符槽）。与图集分开绑定，
// 因为两者的用法不同：图集的 RGB 是子像素覆盖度，图片的 RGB 是真实颜色。
Texture2D image_texture : register(t1);
SamplerState atlas_sampler : register(s0);

struct VSInput
{
    float2 pos      : TEXCOORD0;
    float2 size     : TEXCOORD1;
    float4 uv       : TEXCOORD2;
    float4 color    : TEXCOORD3;
    float4 color_end: TEXCOORD4;
    float4 gradient : TEXCOORD5;
    float4 params   : TEXCOORD6;
};

struct VSOutput
{
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
    float4 color    : TEXCOORD1;
    float  solid    : TEXCOORD2;
    float2 local    : TEXCOORD3;
    float4 color_end: TEXCOORD4;
    float4 gradient : TEXCOORD5;
    float2 size     : TEXCOORD6;
    float4 params   : TEXCOORD7;
};

VSOutput vs_main(uint vertex_id : SV_VertexID, VSInput input)
{
    // 两个三角形：(0,0)(1,0)(0,1) 与 (1,0)(1,1)(0,1)。
    // 必须查表，不能再用 (vid>>1)&1 —— 那样 6 个顶点只有 4 个不同角，
    // 第二个三角形退化，字形被削掉一半。
    static const float2 kCorners[6] = {
        float2(0.0, 0.0),
        float2(1.0, 0.0),
        float2(0.0, 1.0),
        float2(1.0, 0.0),
        float2(1.0, 1.0),
        float2(0.0, 1.0),
    };
    float2 corner = kCorners[vertex_id % 6];
    float2 pixel = input.pos + corner * input.size;
    float2 ndc = float2(
        pixel.x / screen_size.x * 2.0 - 1.0,
        1.0 - pixel.y / screen_size.y * 2.0
    );

    VSOutput output;
    output.position = float4(ndc, 0.0, 1.0);
    output.uv = input.uv.xy + corner * (input.uv.zw - input.uv.xy);
    output.color = input.color;
    output.color_end = input.color_end;
    output.gradient = input.gradient;
    output.params = input.params;
    output.size = input.size;
    output.local = corner;
    // uv.zw == uv.xy 表示"纯色矩形"（背景、光标），不需要采样图集。
    output.solid = (input.uv.x == input.uv.z && input.uv.y == input.uv.w) ? 1.0 : 0.0;
    return output;
}

float4 ps_main(VSOutput input) : SV_Target
{
    // 混合状态是 SrcBlend=ONE（预乘 Alpha），所以输出必须已经乘过 alpha。
    //
    // 三种图元：
    //   1. 渐变（gradient.z > 0.5）：不采样图集，按局部坐标投影插值
    //   2. 纯色（uv.xy == uv.zw）：直接输出顶点色
    //   3. 字形：采样图集覆盖率
    // 圆角：SDF（有向距离场）覆盖率。d<0 在内，d>0 在外。
    // 覆盖率 = saturate(0.5 - d / softness)，softness=1 时就是 1px 过渡带（完美 AA）。
    float sdf_cov = 1.0;
    // params = (radius, softness, stroke_width, 0)
    //   radius > 0 或 stroke_width > 0 时走 SDF；
    //   stroke_width > 0 时只保留"轮廓带" |d| <= width/2（圆角描边）。
    if (input.params.x > 0.0 || input.params.z > 0.0)
    {
        float2 half_size = input.size * 0.5;
        float2 p = input.local * input.size - half_size;
        float r = clamp(input.params.x, 0.0, min(half_size.x, half_size.y));
        float2 q = abs(p) - half_size + r;
        float d = min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
        float soft = max(input.params.y, 0.001);
        if (input.params.z > 0.0)
        {
            // 描边：距离"轮廓中线" |d| 与半宽比较
            float w = input.params.z;
            sdf_cov = saturate(0.5 - (abs(d) - w * 0.5) / soft);
        }
        else
        {
            sdf_cov = saturate(0.5 - d / soft);
        }
        if (sdf_cov <= 0.0)
        {
            discard;
        }
    }

    if (input.gradient.z > 0.5)
    {
        float2 px = input.local * input.size;
        float t = saturate(dot(px, input.gradient.xy) + input.gradient.w);
        float4 c = lerp(input.color, input.color_end, t);
        return float4(c.rgb * sdf_cov, c.a * sdf_cov);
    }
    if (input.solid > 0.5)
    {
        return float4(input.color.rgb * sdf_cov, input.color.a * sdf_cov);
    }
    // 子像素（ClearType）：R/G/B 三个通道是各自的覆盖率，分别调制颜色通道。
    float3 cov = saturate(atlas_texture.Sample(atlas_sampler, input.uv).rgb);
    // DWrite 的 ClearType 覆盖率在 gamma 空间，直接乘颜色会偏暗（发灰）。
    // 用 padding.x 传 gamma 指数（默认 1/2.2），把它拉回感知线性。
    float g = padding.x > 0.001 ? padding.x : 1.0;
    cov = pow(cov, g);
    // 预乘 Alpha：RGB 和 **alpha 都**要乘覆盖率。
    // 漏乘 alpha 会让覆盖率=0 的像素变成"不透明黑块"（字形命中框内的
    // 空白区会被涂黑）——这是真实遇到过的 bug。
    float a = input.color.a * dot(cov, float3(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0));
    return float4(input.color.rgb * cov, a * sdf_cov);
}

// 图片纹理采样 + 着色（tint）。
//
// 注意预乘 alpha：混合状态是 SrcBlend=ONE，输出必须已乘过 alpha。
// 漏乘会让图片在透明处发白。
float4 ps_image(VSOutput input) : SV_Target
{
    float4 tex = image_texture.Sample(atlas_sampler, input.uv);
    float a = input.color.a * tex.a;
    return float4(input.color.rgb * tex.rgb * a, a);
}

// 全屏 blit：把自持 RT 原样搬到交换链后备缓冲（1:1，线性采样精确）。
// 与 ps_main 分开，是因为后者把采样值当"覆盖率"乘到顶点色上。
float4 ps_blit(VSOutput input) : SV_Target
{
    return atlas_texture.Sample(atlas_sampler, input.uv);
}
"#;

    /// # Safety
    ///
    /// 内部编排：`entry` 必须是 `source` 里真实存在的入口点，`target` 必须是合法的 profile 串，
    /// 且 HLSL 源码由本模块内置（合法 UTF-8）；这些前置条件外部无法伪造。
    /// 违反后果：编译失败会返回 `Err`（已处理）；但入口点拼错时错误来自 D3DCompile 的消息而不是
    /// 我们的类型 —— 排查时容易误以为是着色器逻辑问题。
unsafe fn compile_shader(entry: &str, target: &str) -> Result<Blob, String> {
    let entry = CString::new(entry).map_err(|_| "invalid entry point".to_string())?;
    let target = CString::new(target).map_err(|_| "invalid target".to_string())?;

    let mut code: *mut c_void = null_mut();
    let mut errors: *mut c_void = null_mut();

    let hr = D3DCompile(
        SHADER_SOURCE.as_ptr() as *const c_void,
        SHADER_SOURCE.len(),
        null(),
        null(),
        null_mut(),
        entry.as_ptr() as *const u8,
        target.as_ptr() as *const u8,
        0,
        0,
        &mut code,
        &mut errors,
    );

    if hr < 0 {
        let message = Blob::from_raw(errors)
            .map(|blob| String::from_utf8_lossy(blob.bytes()).into_owned())
            .unwrap_or_else(|| format!("D3DCompile failed: 0x{:08X}", hr as u32));
        return Err(message);
    }
    if !errors.is_null() {
        drop(Blob::from_raw(errors));
    }
    Blob::from_raw(code).ok_or_else(|| "D3DCompile returned empty bytecode".to_string())
}

struct Pipeline {
    vertex_shader: VertexShader,
    pixel_shader: PixelShader,
    /// 全屏 blit 用的像素着色器（把自持 RT 搬到后备缓冲）。
    blit_ps: PixelShader,
    /// 图片纹理着色器（`ps_image`）
    image_ps: PixelShader,
    input_layout: InputLayout,
    blend: BlendState,
    rasterizer: RasterizerState,
    constant_buffer: Buffer,
    instance_buffer: Buffer,
    /// blit 专用的小缓冲（1 个实例）。
    ///
    /// **不能复用 `instance_buffer`**：那会 `Map(DISCARD)` 掉 UI 实例，
    /// 而 GPU 可能还没执行完上一次 draw —— 实测表现为"画面只剩背景色"。
    blit_buffer: Buffer,
}

/// 自持渲染目标：UI 先画到它上面，再整张 blit 到交换链后备缓冲。
///
/// 为什么需要：交换链是 `FLIP_DISCARD`，后备缓冲内容在 Present 后失效。
/// 有了自持 RT，才有可能做"只更新脏区"（后续用 scissor + 局部 clear）。
struct CachedTarget {
    /// 保留纹理本体，确保 RTV/SRV 背后的资源不被释放。
    _texture: Texture,
    rtv: RenderTarget,
    srv: ShaderResourceView,
}

struct Atlas {
    texture: Texture,
    srv: ShaderResourceView,
    sampler: SamplerState,
}

// Win32 的 InputElementDesc.semantic_name 需要 nul 结尾的字节串。
// 这里保留手写 b"…\0" 而不是 c"…"：
//   1. 项目 MSRV 未强制到引入 c-string 字面量的版本（1.77）；
//   2. 语义名将来要按顶点属性表运行时拼接时，b"" 形式更容易与 fmt 组合。
#[allow(clippy::manual_c_str_literals)]
    /// # Safety
    ///
    /// 内部编排：所有组件（VS/PS/InputLayout/BlendState/RasterizerState）都由**同一个 `device`** 创建，
    /// 且 InputLayout 的字段序与 VS 输入签名逐项匹配 —— 这两条由下面的构建代码保证，不在类型里。
    /// 违反后果：签名不匹配时 D3D11 **不校验**，Draw 时静默丢帧（画面空着但没有错误码）。
unsafe fn create_pipeline(device: &Device) -> Result<Pipeline, String> {
    let vs_blob = compile_shader("vs_main", "vs_5_0")?;
    let ps_blob = compile_shader("ps_main", "ps_5_0")?;
    let blit_blob = compile_shader("ps_blit", "ps_5_0")?;
    let image_blob = compile_shader("ps_image", "ps_5_0")?;

    let vertex_shader = device.create_vertex_shader(vs_blob.bytes())?;
    let pixel_shader = device.create_pixel_shader(ps_blob.bytes())?;
    let blit_ps = device.create_pixel_shader(blit_blob.bytes())?;
    let image_ps = device.create_pixel_shader(image_blob.bytes())?;

    let elements = [
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 0,
            format: DXGI_FORMAT_R32G32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 0,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 1,
            format: DXGI_FORMAT_R32G32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 8,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 2,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 16,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 3,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 32,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
        // 渐变终点色
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 4,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 48,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
        // (dir.x, dir.y, mode, offset)
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 5,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 64,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
        // (radius, softness, 0, 0)
        InputElementDesc {
            semantic_name: b"TEXCOORD\0".as_ptr(),
            semantic_index: 6,
            format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            input_slot: 0,
            aligned_byte_offset: 80,
            input_slot_class: D3D11_INPUT_PER_INSTANCE_DATA,
            instance_data_step_rate: 1,
        },
    ];
    let input_layout = device.create_input_layout(&elements, vs_blob.bytes())?;

    let constant_buffer = device.create_buffer(&BufferDesc {
        byte_width: 16,
        usage: 0,
        bind_flags: D3D11_BIND_CONSTANT_BUFFER,
        cpu_access_flags: 0,
        misc_flags: 0,
        structure_byte_stride: 0,
    })?;

    let instance_buffer = device.create_buffer(&BufferDesc {
        byte_width: (INSTANCE_CAPACITY * INSTANCE_SIZE) as u32,
        usage: D3D11_USAGE_DYNAMIC,
        bind_flags: D3D11_BIND_VERTEX_BUFFER,
        cpu_access_flags: D3D11_CPU_ACCESS_WRITE,
        misc_flags: 0,
        structure_byte_stride: 0,
    })?;

    let blit_buffer = device.create_buffer(&BufferDesc {
        byte_width: INSTANCE_SIZE as u32,
        usage: D3D11_USAGE_DYNAMIC,
        bind_flags: D3D11_BIND_VERTEX_BUFFER,
        cpu_access_flags: D3D11_CPU_ACCESS_WRITE,
        misc_flags: 0,
        structure_byte_stride: 0,
    })?;

    let mut blend_desc = BlendDesc::default();
    let target = &mut blend_desc.render_target[0];
    target.blend_enable = 1;
    target.src_blend = D3D11_BLEND_ONE;
    target.dest_blend = D3D11_BLEND_INV_SRC_ALPHA;
    target.blend_op = D3D11_BLEND_OP_ADD;
    target.src_blend_alpha = D3D11_BLEND_ONE;
    target.dest_blend_alpha = D3D11_BLEND_INV_SRC_ALPHA;
    target.blend_op_alpha = D3D11_BLEND_OP_ADD;
    target.render_target_write_mask = D3D11_COLOR_WRITE_ENABLE_ALL;
    let blend = device.create_blend_state(&blend_desc)?;

    let rasterizer = device.create_rasterizer_state(&RasterizerDesc {
        fill_mode: 3,
        cull_mode: 1,
        front_counter_clockwise: 0,
        depth_bias: 0,
        depth_bias_clamp: 0.0,
        slope_scaled_depth_bias: 0.0,
        depth_clip_enable: 1,
        // 开启 scissor：局部重绘靠它把 draw 限制在脏区内
        scissor_enable: 1,
        multisample_enable: 0,
        antialiased_line_enable: 0,
    })?;

    Ok(Pipeline {
        vertex_shader,
        pixel_shader,
        blit_ps,
        image_ps,
        input_layout,
        blend,
        rasterizer,
        constant_buffer,
        instance_buffer,
        blit_buffer,
    })
}

// ============================ Gfx ============================

/// 把一帧 UI 画进自持 RT，只更新 `rect` 指定区域。
///
/// 输入：上下文/管线/图集/目标、实例、脏矩形、是否整窗。
/// 副作用：**只**在 `rect` 内清屏（整窗时用 `ClearRenderTargetView`，
/// 局部时画一个同色背景 quad），并用 scissor 限制后续 draw。
/// 失败：无返回值；scissor 必须提前开启（`scissor_enable = 1`）。
#[allow(clippy::too_many_arguments)] // 纯内部函数，参数就是渲染所需的最小集合
    /// # Safety
    ///
    /// 这是内部编排，不是 COM 调用：只能在 **`Gfx` 已完成初始化、且当前线程持有 HWND** 时调用。
    /// 违反后果：在设备丢失或初始化未完成时进入，会在内部取空指针或提交半成品管线 —— 崩溃点
    /// 落在很深的绘制循环里，而不是调用者。
unsafe fn draw_scene(
    context: &Context,
    pipeline: &Pipeline,
    atlas: &Atlas,
    cached: &CachedTarget,
    instances: &[Instance],
    count: usize,
    batches: &[Batch],
    images: &std::collections::HashMap<u64, (Texture, ShaderResourceView, u32, u32, u32)>,
    rect: RectI32,
    is_full: bool,
    w: f32,
    h: f32,
) {
    context.set_render_target(Some(&cached.rtv));
    context.set_viewport(w, h);
    context.set_pipeline(pipeline);
    context.set_ps_texture(&atlas.srv, &atlas.sampler);
    // scissor 必须在 draw 之前设置；`scissor_enable = 1` 时未设置 = 0 面积 = 什么都不画
    context.set_scissor(rect);

    if is_full {
        context.clear(&cached.rtv, [0.02, 0.02, 0.03, 1.0]);
    } else {
        // 局部清屏：画一个和清屏色相同的实心 quad，正好覆盖 scissor 区域。
        // （ClearRenderTargetView 只能整张清，无法只清一块。）
        let bg = Instance::solid(
            [rect.left as f32, rect.top as f32],
            [
                (rect.right - rect.left) as f32,
                (rect.bottom - rect.top) as f32,
            ],
            // uv 全等 ⇒ shader 里的 solid 分支：直接输出顶点色
            [0.0, 0.0, 0.0, 0.0],
            [0.02, 0.02, 0.03, 1.0],
        );
        if let Some(mapped) = context.map_discard(&pipeline.blit_buffer) {
            std::ptr::copy_nonoverlapping(
                &bg as *const Instance as *const u8,
                mapped.data as *mut u8,
                INSTANCE_SIZE,
            );
            context.unmap(&pipeline.blit_buffer);
        }
        let buffers = [pipeline.blit_buffer.as_ptr()];
        let strides = [INSTANCE_SIZE as u32];
        let offsets = [0u32];
        let table = vtbl::<ContextVtbl>(context.as_ptr());
        (table.iaset_vertex_buffers)(
            context.as_ptr(),
            0,
            1,
            buffers.as_ptr(),
            strides.as_ptr(),
            offsets.as_ptr(),
        );
        context.draw_instances(1, 0);
        // 切回 UI 实例缓冲
        let buffers = [pipeline.instance_buffer.as_ptr()];
        (table.iaset_vertex_buffers)(
            context.as_ptr(),
            0,
            1,
            buffers.as_ptr(),
            strides.as_ptr(),
            offsets.as_ptr(),
        );
    }

    // 批次循环：没有批次信息时行为与改动前**完全一致**（单次 draw）。
    if batches.is_empty() {
        context.draw_instances(count as u32, 0);
    } else {
        for b in batches {
            let n = b.count.min((count as u32).saturating_sub(b.start));
            if n == 0 {
                continue;
            }
            match b.image.and_then(|id| images.get(&id)) {
                Some((_, srv, _, _, _)) => {
                    context.set_ps_shader(&pipeline.image_ps);
                    context.set_ps_image_texture(srv);
                    context.draw_instances(n, b.start);
                    // 恢复图集着色器：后续批次可能还是图集
                    context.set_ps_shader(&pipeline.pixel_shader);
                }
                None => context.draw_instances(n, b.start),
            }
        }
    }
    let _ = instances;
}

/// 把自持 RT 整张 blit 到交换链后备缓冲。
///
/// 输入：上下文/管线/源/目标与尺寸。副作用：切换 PS 与顶点缓冲到 blit 专用资源。
/// 说明：用**独立的** `blit_buffer`——复用 `instance_buffer` 会 `Map(DISCARD)`
/// 掉 UI 实例，而 GPU 可能还没执行完 UI 那次 draw（实测表现为只剩背景色）。
#[allow(clippy::too_many_arguments)]
    /// # Safety
    ///
    /// 内部编排：`cached.rtv` 与 swapchain 后备缓冲的**尺寸必须一致**，且调用时 `cached` 已完整绘制过一帧。
    /// 违反后果：尺寸不一致时只有重叠区域被绘制，其余区域保持上一帧内容 —— 表现为“画面撕裂/残留”，
    /// 且不会有任何错误码。
unsafe fn blit_cached(
    context: &Context,
    pipeline: &Pipeline,
    cached: &CachedTarget,
    atlas: &Atlas,
    rtv: &RenderTarget,
    w: f32,
    h: f32,
) {
    context.set_render_target(Some(rtv));
    context.set_viewport(w, h);
    // **必须把 scissor 复位成整窗**：局部重绘时上一阶段把 scissor 限制在脏区内，
    // 若不复位，blit 也只会覆盖脏区，后备缓冲其余部分保持未定义（FLIP_DISCARD 每帧失效）
    // → 屏幕上出现大片黑/雪花。这个 bug 是 `invalidate_rect` 的逐像素对拍测试抓到的。
    context.set_scissor(RectI32 {
        left: 0,
        top: 0,
        right: w as i32,
        bottom: h as i32,
    });
    if let Some(mapped) = context.map_discard(&pipeline.blit_buffer) {
        let blit = Instance::solid(
            [0.0, 0.0],
            [w, h],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        );
        std::ptr::copy_nonoverlapping(
            &blit as *const Instance as *const u8,
            mapped.data as *mut u8,
            INSTANCE_SIZE,
        );
        context.unmap(&pipeline.blit_buffer);
    }
    let buffers = [pipeline.blit_buffer.as_ptr()];
    let strides = [INSTANCE_SIZE as u32];
    let offsets = [0u32];
    let table = vtbl::<ContextVtbl>(context.as_ptr());
    (table.iaset_vertex_buffers)(
        context.as_ptr(),
        0,
        1,
        buffers.as_ptr(),
        strides.as_ptr(),
        offsets.as_ptr(),
    );
    context.set_ps_texture(&cached.srv, &atlas.sampler);
    context.set_ps_shader(&pipeline.blit_ps);
    context.draw_instances(1, 0);
}

/// D3D11 管线集合：设备、上下文、交换链、自持渲染目标与字形图集。
pub struct Gfx {
    atlas: Option<Atlas>,
    atlas_data: Vec<u8>,
    atlas_size: u32,
    pipeline: Option<Pipeline>,
    rtv: Option<RenderTarget>,
    swap_chain: Option<SwapChain>,
    context: Option<Context>,
    device: Option<Device>,
    width: u32,
    height: u32,
    /// 自持渲染目标（尺寸随窗口，重建/改变尺寸时丢弃重建）。
    cached: Option<CachedTarget>,
    /// 本帧需要更新的矩形列表（`None`/空 = 整窗）。
    ///
    /// 为什么是列表而不是包围盒：两块不相邻的脏区若按包围盒清屏，
    /// 中间空隙会被清掉，而 CPU 侧剪枝已跳过了那些节点→空隙变黑。
    dirty: Option<Vec<RectI32>>,
    /// 已渲染帧数（快照自检只在第二帧之后跑，避免拿未初始化内容比较）。
    frames: u64,
    /// 下一帧必须整帧清屏（自持 RT 刚被重建，内容不可信）。
    force_full: bool,
    /// 下一帧是否需要回读像素（必须在 present 之前拿）。
    snapshot_requested: bool,
    /// 最近一次快照的像素（BGRA）与尺寸。
    snapshot: Option<(Vec<u8>, u32, u32)>,
    /// 图片纹理表（按 `TextureId` 索引）。
    images: std::collections::HashMap<u64, (Texture, ShaderResourceView, u32, u32, u32)>,
    /// 下一个图片索引（从 1 开始，0 = 无效）。
    next_image: u64,
}

impl Gfx {
    /// 创建 D3D11 设备、交换链与管线，并绑定到给定窗口。
    ///
    /// # Safety
    ///
    /// 内部编排：`hwnd` 必须属于本 GUI 线程，且 `width`/`height` 是当前客户区尺寸。
    /// 违反后果：跨线程创建交换链会在 `Present` 时失败或黑屏；尺寸与客户区不符会让渲染区域错位。
    /// 设备创建失败时本函数返回 `Err`（不 panic，调用方可重试或降级到 WARP）。
    pub unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self, String> {
        let mut gfx = Self {
            atlas: None,
            atlas_data: Vec::new(),
            atlas_size: 0,
            pipeline: None,
            rtv: None,
            swap_chain: None,
            context: None,
            device: None,
            width,
            height,
            cached: None,
            dirty: None,
            frames: 0,
            force_full: true,
            snapshot_requested: false,
            snapshot: None,
            images: std::collections::HashMap::new(),
            next_image: 0,
        };
        gfx.rebuild(hwnd, width, height)?;
        Ok(gfx)
    }

    /// 丢弃现有 COM 对象并整套重建（设备丢失或窗口重建后调用）。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `hwnd` 与 [`Gfx::new`] 的要求相同（有效、同线程、未销毁）；
    /// - 重建期间没有外部代码持有本 `Gfx` 内部 COM 对象的裸指针副本
    ///   （本方法会无条件释放 device / context / swap_chain / atlas）。
    ///
    /// 违反会导致：旧指针悬垂，后续提交命令时访问已释放的 COM 对象（未定义行为）。
    pub unsafe fn rebuild(&mut self, hwnd: HWND, width: u32, height: u32) -> Result<(), String> {
        self.atlas = None;
        self.pipeline = None;
        self.cached = None;
        self.rtv = None;
        self.swap_chain = None;
        self.context = None;
        self.device = None;

        let (device, context, swap_chain) = create_device_and_swap_chain(hwnd, width, height)?;
        let pipeline = create_pipeline(&device)?;
        let texture = swap_chain.back_buffer(0)?;
        let rtv = device.create_render_target(&texture)?;

        self.pipeline = Some(pipeline);
        self.rtv = Some(rtv);
        self.swap_chain = Some(swap_chain);
        self.context = Some(context);
        self.device = Some(device);
        self.width = width;
        self.height = height;

        self.create_atlas()?;
        Ok(())
    }

    /// # Safety
    ///
    /// 只在**拥有该 HWND 的线程**上、且 `Gfx` 已完成初始化后调用：
    /// 内部直接解引用 vtable 与裸指针（交换链/设备/上下文），外部无法伪造这些前置条件。
    unsafe fn create_atlas(&mut self) -> Result<(), String> {
        self.atlas = None;
        if self.atlas_data.is_empty() || self.atlas_size == 0 {
            return Ok(());
        }
        let Some(device) = self.device.as_ref() else {
            return Ok(());
        };

        let desc = Texture2DDesc {
            width: self.atlas_size,
            height: self.atlas_size,
            mip_levels: 1,
            array_size: 1,
            // RGBA：ClearType 3 通道覆盖率 + 1 字节对齐
            format: DXGI_FORMAT_R8G8B8A8_UNORM,
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            // DEFAULT + UpdateSubresource：图集运行期会变。
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            cpu_access_flags: 0,
            misc_flags: 0,
        };
        let data = SubresourceData {
            data: self.atlas_data.as_ptr() as *const c_void,
            row_pitch: self.atlas_size * 4,
            slice_pitch: self.atlas_size * self.atlas_size * 4,
        };

        let texture = device.create_texture_2d(&desc, &data)?;
        let srv = device.create_shader_resource_view(&texture)?;
        let sampler = device.create_sampler_state()?;
        self.atlas = Some(Atlas {
            texture,
            srv,
            sampler,
        });
        Ok(())
    }

    /// 把 CPU 端的脏矩形拷进纹理。`reset = true` 时整张重传。
    ///
    /// # Safety
    ///
    /// 内部编排：图集尺寸必须与创建时一致（本函数不重建纹理），行距必须 ≥ `width × bpp`。
    /// 违反后果：尺寸不符会让 `UpdateSubresource` 写入越界或只更新一部分（贴图残影）；
    /// 行距不足会读到 `data` 之外的内存 —— 图集出现随机杂点。
    pub unsafe fn update_atlas(
        &mut self,
        data: &[u8],
        size: u32,
        reset: bool,
        dirty: &[(u32, u32, u32, u32)],
    ) -> Result<(), String> {
        self.atlas_data = data.to_vec();
        self.atlas_size = size;

        // 首次调用（或设备重建后）纹理还不存在：必须先创建。
        // 少了这一步，PS 会采样未绑定的 SRV，得到随机值 —— 满屏白噪点。
        if self.atlas.is_none() {
            return self.create_atlas();
        }

        let (Some(atlas), Some(context)) = (self.atlas.as_ref(), self.context.as_ref()) else {
            return Ok(());
        };

        if reset {
            let subresource = SubresourceData {
                data: data.as_ptr() as *const c_void,
                row_pitch: size,
                slice_pitch: size * size,
            };
            context.update_subresource(&atlas.texture, &subresource);
            return Ok(());
        }

        for &(x, y, width, height) in dirty {
            // 图集是 RGBA：每像素 4 字节，行距与切片跨度都要 ×4
            let row_pitch = width as usize * 4;
            let mut patch = vec![0u8; row_pitch * height as usize];
            for row in 0..height as usize {
                let src = (((y as usize + row) * size as usize + x as usize) * 4)..;
                patch[row * row_pitch..(row + 1) * row_pitch]
                    .copy_from_slice(&data[src.start..src.start + row_pitch]);
            }
            let subresource = SubresourceData {
                data: patch.as_ptr() as *const c_void,
                row_pitch: row_pitch as u32,
                slice_pitch: (row_pitch * height as usize) as u32,
            };
            context.update_subresource_region(&atlas.texture, &subresource, x, y, width, height);
        }
        Ok(())
    }

    /// 调整交换链缓冲尺寸，并重新绑定渲染目标。
    ///
    /// # Safety
    ///
    /// 内部编排：新尺寸不得超过设备最大纹理尺寸（`D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION`），
    /// 且重建前所有依赖旧后备缓冲的资源（RTV/staging）必须**先 Release**。
    /// 违反后果：超尺寸重建失败（返回 `Err`）；旧资源未释放就重建会让它们指向已失效的缓冲，
    /// 表现为设备丢失恢复后花屏或 `Present` 持续失败。
    pub unsafe fn resize(&mut self, width: u32, height: u32) -> Result<bool, String> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(false);
        }
        let (Some(swap_chain), Some(context), Some(device)) = (
            self.swap_chain.as_ref(),
            self.context.as_ref(),
            self.device.as_ref(),
        ) else {
            return Err("gfx is not initialized".to_string());
        };

        context.set_render_target(None);
        self.rtv = None;

        swap_chain.resize_buffers(width, height)?;
        let texture = swap_chain.back_buffer(0)?;
        self.rtv = Some(device.create_render_target(&texture)?);

        self.width = width;
        self.height = height;
        // 尺寸变了：自持 RT 必须重建（旧尺寸 blit 会错位/越界）。
        self.cached = None;
        // 新 RT 是**未初始化**的。下一帧必须整帧清屏，
        // 否则没被脏区覆盖的区域保持 alpha=0 → blit 出纯黑。
        self.force_full = true;
        Ok(true)
    }

    /// 上传一张图片纹理，返回句柄（`0` = 失败）。
    ///
    /// # Safety
    ///
    /// 同 [`Gfx::create_image_texture`]。
    pub unsafe fn upload_image(&mut self, width: u32, height: u32, rgba: &[u8]) -> u64 {
        match self.create_image_texture(width, height, rgba) {
            Ok((texture, srv)) => {
                self.next_image += 1;
                let id = self.next_image;
                // 上传即拥有一次引用
                self.images.insert(id, (texture, srv, width, height, 1));
                id
            }
            Err(err) => {
                eprintln!("[y901] 图片纹理上传失败: {err}");
                0
            }
        }
    }

    /// 释放一张图片纹理（未知 id 忽略）。
    ///
    /// 当前**没有调用点**：`RenderCtx` 没有 `release_texture`，
    /// 控件也没有 unmount 钩子（拿不到 ctx）。界面先留着，
    /// 等框架有了"组件销毁通知"再接。
    pub fn release_image(&mut self, id: u64) {
        // 引用计数归零才释放：同一张纹理可能被多个控件共享。
        let free = match self.images.get_mut(&id) {
            Some(entry) => {
                entry.4 = entry.4.saturating_sub(1);
                entry.4 == 0
            }
            None => false,
        };
        if free {
            self.images.remove(&id);
        }
    }


    /// 增加一次引用计数（共享纹理用）。
    pub fn retain_image(&mut self, id: u64) {
        if let Some(entry) = self.images.get_mut(&id) {
            entry.4 = entry.4.saturating_add(1);
        }
    }

    /// 当前纹理表大小（测试/诊断用）。
    pub fn image_count(&self) -> usize {
        self.images.len()
    }

    /// 某张纹理的引用计数（测试用）。
    pub fn image_refcount(&self, id: u64) -> Option<u32> {
        self.images.get(&id).map(|e| e.4)
    }

    /// 取图片纹理的 SRV 与尺寸（供批次切换时绑定）。
    pub fn image(&self, id: u64) -> Option<(&ShaderResourceView, u32, u32)> {
        self.images.get(&id).map(|(_, srv, w, h, _)| (srv, *w, *h))
    }

    /// 创建一张图片纹理（`R8G8B8A8_UNORM`）并返回它与 SRV。
    ///
    /// 输入：尺寸与 RGBA 字节（长度必须 ≥ `w*h*4`）。
    /// 输出：`(Texture, ShaderResourceView)`；Texture 必须保留，否则 SRV 背后的资源会被释放。
    ///
    /// 为什么 `row_pitch = width * 4` 就够：`R8G8B8A8` 每像素 4 字节，
    /// D3D11 对 SubresourceData 的 row pitch 没有 "256 字节对齐"要求（那是 UpdateSubresource 的目标纹理层面的传说），
    /// 实测上直接用 `w*4` 上传即可。
    ///
    /// # Safety
    ///
    /// 需要有效且未销毁的 D3D11 设备（即 `Gfx` 已初始化），且在创建窗口的线程上调用。
    pub unsafe fn create_image_texture(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(Texture, ShaderResourceView), String> {
        let Some(device) = self.device.as_ref() else {
            return Err("gfx is not initialized".to_string());
        };
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            return Err("invalid texture size or data".to_string());
        }
        let desc = Texture2DDesc {
            width,
            height,
            mip_levels: 1,
            array_size: 1,
            format: 28, // DXGI_FORMAT_R8G8B8A8_UNORM
            sample_desc: SampleDesc { count: 1, quality: 0 },
            usage: 0,                          // D3D11_USAGE_DEFAULT
            bind_flags: D3D11_BIND_SHADER_RESOURCE,
            cpu_access_flags: 0,
            misc_flags: 0,
        };
        let data = SubresourceData {
            data: rgba.as_ptr() as *const c_void,
            row_pitch: width * 4,
            slice_pitch: width * height * 4,
        };
        let texture = device.create_texture_2d(&desc, &data)?;
        let srv = device.create_shader_resource_view(&texture)?;
        Ok((texture, srv))
    }

    /// 提交一帧：上传实例、画一次 `DrawInstanced`、`Present`。
    ///
    /// # Safety
    ///
    /// 内部编排：所有 pipeline / buffer / 图集都必须**已就绪**，且当前不在设备丢失状态。
    /// 违反后果：设备丢失期间提交命令会被静默丢弃；pipeline 未就绪时会绑定空指针 ——
    /// 崩溃点在绘制调用内部，而不是调用者。
    pub unsafe fn render(&mut self, instances: &[Instance], sync_interval: u32) -> Hresult {
        // 没有图片时走这条：等价于改动前的单次 draw。
        self.render_batched(instances, &[], sync_interval)
    }

    /// 带批次的提交：同一帧内按 `batches` 逐段绑纹理并绘制。
    ///
    /// # Safety
    ///
    /// 内部编排：`batches` 里每个 `start + count` 都必须落在 `instances.len()` 之内；
    /// 各批次共用同一 pipeline 与实例 buffer。
    /// 违反后果：越界会让 GPU 读到 buffer 之外 —— 可能挂起设备（TDR）或画出随机三角形。
    pub unsafe fn render_batched(
        &mut self,
        instances: &[Instance],
        batches: &[Batch],
        sync_interval: u32,
    ) -> Hresult {
        // 把 # Safety 里"图集已同步"的承诺在 debug 构建下变成可执行断言。
        debug_assert!(
            render_preconditions_ok(
                self.atlas.is_some() || self.atlas_data.is_empty(),
                self.width,
                self.height,
            ),
            "render 前置条件不满足：图集未同步（有数据但无纹理）或尺寸为 0"
        );
        // 自持 RT：UI 先画到它上面。没有它就无法做"只更新脏区"
        // （`FLIP_DISCARD` 的后备缓冲内容在 Present 后失效）。
        if let Err(err) = self.ensure_cached() {
            eprintln!("[y901] cached RT 创建失败: {err}");
            return 0;
        }
        let (Some(pipeline), Some(rtv), Some(context), Some(swap_chain), Some(cached)) = (
            self.pipeline.as_ref(),
            self.rtv.as_ref(),
            self.context.as_ref(),
            self.swap_chain.as_ref(),
            self.cached.as_ref(),
        ) else {
            return 0;
        };
        let Some(atlas) = self.atlas.as_ref() else {
            return 0;
        };

        let (w, h) = (self.width as f32, self.height as f32);
        // padding.x 用作 ClearType gamma 指数（可用 Y901_GAMMA 覆盖，便于 A/B）
        let gamma = std::env::var("Y901_GAMMA")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0 / 2.2);
        let globals = [w, h, gamma, 0.0];
        context
            .update_constant_buffer(&pipeline.constant_buffer, globals.as_ptr() as *const c_void);

        let count = instances.len().min(INSTANCE_CAPACITY);
        if count > 0 {
            match context.map_discard(&pipeline.instance_buffer) {
                Some(mapped) => {
                    std::ptr::copy_nonoverlapping(
                        instances.as_ptr() as *const u8,
                        mapped.data as *mut u8,
                        count * INSTANCE_SIZE,
                    );
                    context.unmap(&pipeline.instance_buffer);
                }
                None => eprintln!("[y901] map_discard 失败：实例缓冲不可写（设备可能丢失，下一帧将重建）"),
            }
        }

        // ---- 阶段 A：把 UI 画进自持 RT（scissor 限制在脏区） ----
        let full = RectI32 {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        // 逐脏区清屏 + 逐脏区画。
        // 单块时走快路径；多块时逐个 scissor。CPU 侧剪枝已经把
        // "命中任一脏区"的节点都放进了实例流，所以每块自己清 + 画就不会错。
        let force = self.force_full;
        self.force_full = false;
        let rects: Vec<RectI32> = if force {
            vec![full]
        } else {
            match &self.dirty {
                Some(list) if !list.is_empty() => list.clone(),
                _ => vec![full],
            }
        };
        let full_frame = force || self.dirty.as_ref().is_none_or(|v| v.is_empty());
        for rect in rects {
            let is_full = rect.left == 0
                && rect.top == 0
                && rect.right >= w as i32
                && rect.bottom >= h as i32;
            draw_scene(
                context,
                pipeline,
                atlas,
                cached,
                instances,
                count,
                batches,
                &self.images, rect, is_full, w, h,
            );
        }

        // ---- 阶段 B：自持 RT → 后备缓冲 ----
        blit_cached(context, pipeline, cached, atlas, rtv, w, h);

        self.frames = self.frames.saturating_add(1);

        // ---- gamma A/B 自检（`Y901_GAMMA_AB=1`） ----
        //
        // 同一批实例、同一帧，分别用 gamma=1.0（不校正）与 gamma=1/2.2 画一次，
        // 打印平均亮度与亮像素占比 —— 用数据判断"发灰"是否修好，不依赖肉眼。
        if std::env::var_os("Y901_GAMMA_AB").is_some() && self.frames >= 2 {
            for g in [1.0f32, 1.0 / 2.2] {
                let globals = [w, h, g, 0.0];
                context.update_constant_buffer(
                    &pipeline.constant_buffer,
                    globals.as_ptr() as *const c_void,
                );
                draw_scene(
                    context, pipeline, atlas, cached, instances, count, batches, &self.images, full, true, w, h,
                );
                blit_cached(context, pipeline, cached, atlas, rtv, w, h);
                if let Some((mean, bright)) = self.read_back_stats() {
                    eprintln!("[gamma] gamma={g:.4} mean_luma={mean:.4} bright={bright:.2}%");
                }
            }
            // 恢复本帧使用的 gamma
            let globals = [w, h, gamma, 0.0];
            context.update_constant_buffer(
                &pipeline.constant_buffer,
                globals.as_ptr() as *const c_void,
            );
            draw_scene(
                context,
                pipeline,
                atlas,
                cached,
                instances,
                count,
                batches,
                &self.images,
                full,
                true,
                w,
                h,
            );
            blit_cached(context, pipeline, cached, atlas, rtv, w, h);
        }

        // ---- scissor 等价性自检（`Y901_SCISSOR_TEST=1`） ----
        //
        // 前提：上一帧已经是完整画面。此后用**同一批实例**只重画一个小矩形，
        // 结果应当与整窗重绘逐像素一致 —— 这验证"局部清屏 + scissor"没有破坏画面。
        if std::env::var_os("Y901_SCISSOR_TEST").is_some() && self.frames >= 2 && full_frame {
            let before = self.read_back_hash();
            let small = RectI32 {
                left: 0,
                top: 0,
                right: (w as i32 / 4).max(1),
                bottom: (h as i32 / 4).max(1),
            };
            draw_scene(
                context,
                pipeline,
                atlas,
                cached,
                instances,
                count,
                batches,
                &self.images, small, false, w, h,
            );
            blit_cached(context, pipeline, cached, atlas, rtv, w, h);
            let after = self.read_back_hash();
            match (before, after) {
                (Some(a), Some(b)) => eprintln!(
                    "[scissor] instances={count} full=0x{a:016x} partial=0x{b:016x} equal={}",
                    a == b
                ),
                _ => eprintln!("[scissor] 回读失败"),
            }
            // 自检会破坏画面：立刻用整帧重画一次恢复
            draw_scene(
                context,
                pipeline,
                atlas,
                cached,
                instances,
                count,
                batches,
                &self.images, full, true, w, h,
            );
            blit_cached(context, pipeline, cached, atlas, rtv, w, h);
        }

        // ---- 快照验证（`Y901_SNAPSHOT=1` 时启用） ----
        //
        // 用**同一批实例**再画一遍，比较两次回读的像素哈希：
        // 相等 ⇒ 渲染路径确定（没有未初始化内存、没有时间依赖）。
        // 这是后续 scissor / ClearType 改动的回归探针。
        if std::env::var_os("Y901_SNAPSHOT").is_some() {
            let h1 = self.read_back_hash();
            // 重画一遍（同一份实例数据）
            context.set_render_target(Some(&cached.rtv));
            context.clear(&cached.rtv, [0.02, 0.02, 0.03, 1.0]);
            context.set_pipeline(pipeline);
            context.set_ps_texture(&atlas.srv, &atlas.sampler);
            context.draw_instances(count as u32, 0);
            context.set_render_target(Some(rtv));
            if let Some(mapped) = context.map_discard(&pipeline.blit_buffer) {
                let blit = Instance::solid(
                    [0.0, 0.0],
                    [w, h],
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0, 1.0],
                );
                std::ptr::copy_nonoverlapping(
                    &blit as *const Instance as *const u8,
                    mapped.data as *mut u8,
                    INSTANCE_SIZE,
                );
                context.unmap(&pipeline.blit_buffer);
            }
            let buffers = [pipeline.blit_buffer.as_ptr()];
            let strides = [INSTANCE_SIZE as u32];
            let offsets = [0u32];
            let table = vtbl::<ContextVtbl>(context.as_ptr());
            (table.iaset_vertex_buffers)(
                context.as_ptr(),
                0,
                1,
                buffers.as_ptr(),
                strides.as_ptr(),
                offsets.as_ptr(),
            );
            context.set_ps_texture(&cached.srv, &atlas.sampler);
            context.set_ps_shader(&pipeline.blit_ps);
            context.draw_instances(1, 0);
            let h2 = self.read_back_hash();
            match (h1, h2) {
                (Some(a), Some(b)) => eprintln!(
                    "[snapshot] instances={count} hash1=0x{a:016x} hash2=0x{b:016x} equal={}",
                    a == b
                ),
                _ => eprintln!("[snapshot] 回读失败（staging/Map 返回 None）"),
            }
        }

        // 快照：必须在 present 之前回读（FLIP_DISCARD 之后后备缓冲内容未定义）。
        if self.snapshot_requested {
            self.snapshot = self.read_back_frame();
            self.snapshot_requested = false;
        }

        swap_chain.present(sync_interval)
    }

    /// 请求在**下一帧 present 之前**回读像素。
    pub fn request_snapshot(&mut self) {
        self.snapshot_requested = true;
    }

    /// 取走上次快照的像素（BGRA）。
    pub fn take_snapshot(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        self.snapshot.take()
    }

    /// 把后备缓冲回读成紧凑的 BGRA 像素（快照/亮度统计用）。
    ///
    /// 返回：`(pixels, width, height)`；staging 或 Map 失败时 `None`。
    /// 把后备缓冲回读成紧凑的 BGRA 像素（快照/像素数学验证用）。
    ///
    /// 返回：`(pixels, width, height)`；staging 或 Map 失败时 `None`。
    ///
    /// # Safety
    ///
    /// 必须在拥有窗口的线程上调用，且 `Gfx` 已初始化。
    pub unsafe fn read_back_pixels(&self) -> Option<(Vec<u8>, u32, u32)> {
        self.read_back_frame()
    }

    /// # Safety
    ///
    /// 内部编排：必须在 `Present` **之前**调用（后备缓冲仍有效），且内部使用的 staging texture 必须是
    /// `D3D11_USAGE_STAGING` + `CPU_ACCESS_READ`。
    /// 违反后果：`Present` 之后回读拿到空帧或上一帧；staging 配置错误时 `Map` 返回 null（本函数返回 `None`）。
    unsafe fn read_back_frame(&self) -> Option<(Vec<u8>, u32, u32)> {
        let (Some(device), Some(context), Some(swap_chain)) = (
            self.device.as_ref(),
            self.context.as_ref(),
            self.swap_chain.as_ref(),
        ) else {
            return None;
        };
        let (w, h) = (self.width.max(1), self.height.max(1));
        let backbuffer = swap_chain.back_buffer(0).ok()?;
        let desc = Texture2DDesc {
            width: w,
            height: h,
            mip_levels: 1,
            array_size: 1,
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            usage: D3D11_USAGE_STAGING,
            bind_flags: 0,
            cpu_access_flags: D3D11_CPU_ACCESS_READ,
            misc_flags: 0,
        };
        let staging = device.create_texture_2d_empty(&desc).ok()?;
        context.copy_resource(staging.as_ptr(), backbuffer.as_ptr());
        let mapped = context.map_read(&staging)?;
        let pitch = mapped.row_pitch as usize;
        let row_bytes = (w as usize) * 4;
        let base = mapped.data as *const u8;
        if base.is_null() {
            return None;
        }
        let mut out = Vec::with_capacity(row_bytes * h as usize);
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts(base.add(y * pitch), row_bytes);
            out.extend_from_slice(row);
        }
        context.unmap_texture(&staging);
        Some((out, w, h))
    }

    /// 画面统计：`(平均亮度 0..1, 亮像素占比 %)`。
    ///
    /// 用途：客观比较 gamma 校正前后的字形浓淡（不需要肉眼）。
    ///
    /// # Safety
    ///
    /// 必须在拥有窗口的线程上调用，且 `Gfx` 已初始化；
    /// 内部会创建 staging 纹理并 `Map` 后备缓冲。
    pub unsafe fn read_back_stats(&self) -> Option<(f64, f64)> {
        let (px, w, h) = self.read_back_frame()?;
        if px.is_empty() {
            return None;
        }
        let mut sum = 0u64;
        let mut bright = 0u64;
        let mut n = 0u64;
        for chunk in px.chunks_exact(4) {
            let (b, g, r) = (chunk[0] as u64, chunk[1] as u64, chunk[2] as u64);
            let luma = (r * 299 + g * 587 + b * 114) / 1000;
            sum += luma;
            if r + g + b > 300 {
                bright += 1;
            }
            n += 1;
        }
        let _ = (w, h);
        Some((
            sum as f64 / n as f64 / 255.0,
            bright as f64 / n as f64 * 100.0,
        ))
    }

    /// 把交换链后备缓冲回读到 CPU，算 FNV-1a 哈希（快照验证与回归对比用）。
    ///
    /// 返回：`Some(hash)`；设备/staging 创建或 Map 失败时 `None`。
    /// 副作用：创建一个 staging 纹理并立即释放；只在开启快照时调用。
    ///
    /// # Safety
    ///
    /// 必须在拥有该窗口的线程上调用，且 `Gfx` 已初始化。
    unsafe fn read_back_hash(&self) -> Option<u64> {
        let (Some(device), Some(context), Some(swap_chain)) = (
            self.device.as_ref(),
            self.context.as_ref(),
            self.swap_chain.as_ref(),
        ) else {
            return None;
        };
        let (w, h) = (self.width.max(1), self.height.max(1));
        let backbuffer = swap_chain.back_buffer(0).ok()?;
        let desc = Texture2DDesc {
            width: w,
            height: h,
            mip_levels: 1,
            array_size: 1,
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            usage: D3D11_USAGE_STAGING,
            bind_flags: 0,
            cpu_access_flags: D3D11_CPU_ACCESS_READ,
            misc_flags: 0,
        };
        let staging = device.create_texture_2d_empty(&desc).ok()?;
        context.copy_resource(staging.as_ptr(), backbuffer.as_ptr());

        let mapped = context.map_read(&staging)?;
        let pitch = mapped.row_pitch as usize;
        let row_bytes = (w as usize) * 4;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let base = mapped.data as *const u8;
        if base.is_null() {
            return None;
        }
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts(base.add(y * pitch), row_bytes);
            for b in row {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        context.unmap_texture(&staging);
        Some(hash)
    }

    /// 设置本帧需要更新的矩形（`None` = 整窗）。
    ///
    /// 输入：`rect = (left, top, right, bottom)`，像素坐标，右/下开区间。
    /// 副作用：下一帧 `render` 会用它做 scissor + 局部清屏；`None` 时退化为整窗。
    pub fn set_dirty_bounds(&mut self, rects: Option<Vec<(i32, i32, i32, i32)>>) {
        // 过滤空矩形；列表为空时归为"整窗"（呼叫方已通过 dirty.bounds 判断）
        self.dirty = rects.map(|v| {
            v.into_iter()
                .filter(|(l, t, r, b)| r > l && b > t)
                .map(|(l, t, r, b)| RectI32 {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                })
                .collect::<Vec<_>>()
        });
    }

    /// 确保自持 RT 存在且尺寸与窗口一致。
    ///
    /// 返回：`Ok(())` = 可用；`Err` = 设备未就绪或创建失败（调用方应放弃本帧）。
    /// 副作用：首次调用创建纹理 + RTV + SRV；`rebuild`/`resize` 会把它置空，下次自动重建。
    /// # Safety
    ///
    /// 只在**拥有该 HWND 的线程**上、且 `Gfx` 已完成初始化后调用：
    /// 内部直接解引用 vtable 与裸指针（交换链/设备/上下文），外部无法伪造这些前置条件。
    unsafe fn ensure_cached(&mut self) -> Result<(), String> {
        if self.cached.is_some() {
            return Ok(());
        }
        let (Some(device), Some(swap_chain)) = (self.device.as_ref(), self.swap_chain.as_ref())
        else {
            return Err("gfx is not initialized".to_string());
        };
        let (w, h) = (self.width.max(1), self.height.max(1));
        // 格式必须与交换链一致（B8G8R8A8），否则 blit 会出现通道错位。
        let desc = Texture2DDesc {
            width: w,
            height: h,
            mip_levels: 1,
            array_size: 1,
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            sample_desc: SampleDesc {
                count: 1,
                quality: 0,
            },
            usage: D3D11_USAGE_DEFAULT,
            bind_flags: D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE,
            cpu_access_flags: 0,
            misc_flags: 0,
        };
        let _ = swap_chain;
        let texture = device.create_texture_2d_empty(&desc)?;
        let rtv = device.create_render_target(&texture)?;
        let srv = device.create_shader_resource_view(&texture)?;
        self.cached = Some(CachedTarget {
            _texture: texture,
            rtv,
            srv,
        });
        Ok(())
    }

    /// 是否尚未初始化（创建失败或设备丢失后为 true，调用方据此走重建路径）。
    pub fn is_empty(&self) -> bool {
        self.device.is_none()
    }

    /// GPU 侧图集纹理是否已创建。
    ///
    /// `rebuild()` 会释放纹理，必须重新 `update_atlas(.., reset=true, ..)` 才能恢复；
    /// `present` 前用 `debug_assert!` 校验这一点（否则采样到空纹理 = 整屏缺字）。
    pub fn has_atlas(&self) -> bool {
        self.atlas.is_some()
    }

    /// 失败的 HRESULT 是否表示设备被移除/重置。这类失败是**可恢复**的：重建后继续画；其它失败则应当上报。
    pub fn device_lost(hr: Hresult) -> bool {
        matches!(
            hr as u32,
            DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET
        )
    }

    /// `Present` 的返回是否表示窗口被遮挡。被遮挡时不必重画——这正是空闲时省 CPU 的一半来源。
    pub fn is_occluded(hr: Hresult) -> bool {
        hr as u32 == DXGI_STATUS_OCCLUDED
    }

    /// 调试诊断：当前存活的 COM 对象数（Release 构建恒为 0）。
    // 诊断 API：COM 泄漏计数，仅 `live_objects()` 测试与人工排查调用
// 归类：诊断 API —— `live_objects` 供泄漏排查（debug 构建计数）
    #[allow(dead_code)] // 留给调试/诊断使用，主循环不调用
    pub fn live_objects() -> isize {
        #[cfg(debug_assertions)]
        {
            LIVE_OBJECTS.load(std::sync::atomic::Ordering::Relaxed)
        }
        #[cfg(not(debug_assertions))]
        {
            0
        }
    }
}

/// # Safety
///
/// 只在**拥有该 HWND 的线程**上、且 `Gfx` 已完成初始化后调用：
/// 内部直接解引用 vtable 与裸指针（交换链/设备/上下文），外部无法伪造这些前置条件。
unsafe fn create_device_and_swap_chain(
    hwnd: HWND,
    width: u32,
    height: u32,
) -> Result<(Device, Context, SwapChain), String> {
    let desc = SwapChainDesc {
        buffer_desc: ModeDesc {
            width,
            height,
            refresh_rate: Rational {
                numerator: 0,
                denominator: 0,
            },
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            scanline_ordering: 0,
            scaling: 0,
        },
        sample_desc: SampleDesc {
            count: 1,
            quality: 0,
        },
        buffer_usage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        buffer_count: 2,
        output_window: hwnd,
        windowed: 1,
        swap_effect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        flags: 0,
    };

    let levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
        D3D_FEATURE_LEVEL_10_0,
    ];

    for driver_type in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: *mut c_void = null_mut();
        let mut context: *mut c_void = null_mut();
        let mut swap_chain: *mut c_void = null_mut();
        let mut level = 0u32;

        let hr = D3D11CreateDeviceAndSwapChain(
            null_mut(),
            driver_type,
            null_mut(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            levels.as_ptr(),
            levels.len() as u32,
            D3D11_SDK_VERSION,
            &desc,
            &mut swap_chain,
            &mut device,
            &mut level,
            &mut context,
        );

        if hr >= 0 {
            if let (Some(device), Some(context), Some(swap_chain)) = (
                Device::from_raw(device),
                Context::from_raw(context),
                SwapChain::from_raw(swap_chain),
            ) {
                return Ok((device, context, swap_chain));
            }
            return Err("D3D11CreateDeviceAndSwapChain returned null objects".to_string());
        }

        if driver_type == D3D_DRIVER_TYPE_WARP {
            return Err(format!(
                "D3D11CreateDeviceAndSwapChain failed: 0x{:08X}",
                hr as u32
            ));
        }
    }

    // 代码上永远到不了这里：循环体内每条路径要么 `return`、要么以
    // `driver_type == WARP` 为前提写入威胁；两轮循环后 `driver_type` 必为 WARP。
    // 保留一个**可报错的**出口，而不是 `unreachable!()`：一旦上面的循环被改动，
    // 这里会返回一个可诊断的 Err，而不是直接 panic 掉整个进程。
    debug_assert!(false, "device creation loop should have returned");
    Err("D3D11 device creation loop exited without a result".to_string())
}

// ---------------------------------------------------------------------------
// render 前置条件（纯函数，便于单测）
// ---------------------------------------------------------------------------

/// `render` 依赖的两条不变量：图集可用、尺寸非零。
///
/// * `atlas_ready`：纹理已创建，**或** 根本没有待上传的数据。
/// * `width` / `height`：交换链尺寸，必须 > 0。
///
/// 返回 `false` 表示 [`Gfx::render`] 的 `# Safety` 承诺被违反。
fn render_preconditions_ok(atlas_ready: bool, width: u32, height: u32) -> bool {
    atlas_ready && width > 0 && height > 0
}

#[cfg(test)]
mod render_guard_tests {
    use super::render_preconditions_ok;

    #[test]
    fn accepts_ready_atlas_with_nonzero_size() {
        assert!(render_preconditions_ok(true, 800, 600));
    }

    #[test]
    fn rejects_unsynced_atlas() {
        assert!(!render_preconditions_ok(false, 800, 600));
    }

    #[test]
    fn rejects_zero_size() {
        assert!(!render_preconditions_ok(true, 0, 600));
        assert!(!render_preconditions_ok(true, 800, 0));
    }

    #[test]
    fn rejects_everything_at_once() {
        assert!(!render_preconditions_ok(false, 0, 0));
    }
}
