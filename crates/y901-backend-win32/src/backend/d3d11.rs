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

use std::ffi::{c_void, CString};
use std::ptr::{null, null_mut, slice_from_raw_parts};

use windows_sys::Win32::Foundation::HWND;

pub type Hresult = i32;

pub const INSTANCE_SIZE: usize = 48;
pub const INSTANCE_CAPACITY: usize = 10_000;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Instance {
    pub pos: [f32; 2],
    pub size: [f32; 2],
    pub uv_rect: [f32; 4],
    pub color: [f32; 4],
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
    ];
    for (name, actual, expected) in offsets {
        if actual != expected {
            return Err(format!(
                "{name} offset = {actual}, InputLayout expects {expected}"
            ));
        }
    }

    println!(
        "[y901] Instance layout ok: size={size} stride={INSTANCE_SIZE} offsets=pos:{} size:{} uv:{} color:{}",
        std::mem::offset_of!(Instance, pos),
        std::mem::offset_of!(Instance, size),
        std::mem::offset_of!(Instance, uv_rect),
        std::mem::offset_of!(Instance, color),
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
const DXGI_FORMAT_R8_UNORM: u32 = 61;
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;

const DXGI_USAGE_RENDER_TARGET_OUTPUT: u32 = 0x20;
const DXGI_SWAP_EFFECT_FLIP_DISCARD: u32 = 4;

const D3D11_USAGE_DYNAMIC: u32 = 2;
const D3D11_USAGE_DEFAULT: u32 = 0;
const D3D11_BIND_VERTEX_BUFFER: u32 = 0x8;
const D3D11_BIND_CONSTANT_BUFFER: u32 = 0x4;
const D3D11_BIND_SHADER_RESOURCE: u32 = 0x8;
const D3D11_CPU_ACCESS_WRITE: u32 = 0x10000;
const D3D11_MAP_WRITE_DISCARD: u32 = 4;

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

unsafe fn vtbl<T>(object: *mut c_void) -> &'static T {
    &*(*(object as *mut *const T))
}

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
    reserved_5: *const c_void,
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
    reserved_5: *const c_void,
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
    // 45..=47
    reserved_12: [*const c_void; 3],
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
    unsafe fn create_buffer(&self, desc: &BufferDesc) -> Result<Buffer, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_buffer)(self.as_ptr(), desc, null(), &mut raw);
        if hr < 0 {
            return Err(format!("CreateBuffer failed: 0x{:08X}", hr as u32));
        }
        Buffer::from_raw(raw).ok_or_else(|| "CreateBuffer returned null".into())
    }

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

    unsafe fn create_blend_state(&self, desc: &BlendDesc) -> Result<BlendState, String> {
        let table = vtbl::<DeviceVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_blend_state)(self.as_ptr(), desc, &mut raw);
        if hr < 0 {
            return Err(format!("CreateBlendState failed: 0x{:08X}", hr as u32));
        }
        BlendState::from_raw(raw).ok_or_else(|| "CreateBlendState returned null".into())
    }

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

    unsafe fn clear(&self, target: &RenderTarget, color: [f32; 4]) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.clear_render_target_view)(self.as_ptr(), target.as_ptr(), color.as_ptr());
    }

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

    unsafe fn update_constant_buffer(&self, buffer: &Buffer, data: *const c_void) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.update_subresource)(self.as_ptr(), buffer.as_ptr(), 0, null(), data, 0, 0);
    }

    /// 整张纹理更新（mip 0）。
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

    unsafe fn unmap(&self, buffer: &Buffer) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.unmap)(self.as_ptr(), buffer.as_ptr(), 0);
    }

    unsafe fn set_pipeline(&self, pipeline: &Pipeline) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());

        (table.iaset_input_layout)(self.as_ptr(), pipeline.input_layout.as_ptr());

        (table.vsset_shader)(self.as_ptr(), pipeline.vertex_shader.as_ptr(), null(), 0);
        (table.psset_shader)(self.as_ptr(), pipeline.pixel_shader.as_ptr(), null(), 0);

        let constants = [pipeline.constant_buffer.as_ptr()];
        (table.vsset_constant_buffers)(self.as_ptr(), 0, 1, constants.as_ptr());

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

    unsafe fn draw_instances(&self, count: u32) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        (table.draw_instanced)(self.as_ptr(), 6, count, 0, 0);
    }

    unsafe fn set_ps_texture(&self, srv: &ShaderResourceView, sampler: &SamplerState) {
        let table = vtbl::<ContextVtbl>(self.as_ptr());
        let views = [srv.as_ptr()];
        (table.psset_shader_resources)(self.as_ptr(), 0, 1, views.as_ptr());
        let samplers = [sampler.as_ptr()];
        (table.psset_samplers)(self.as_ptr(), 0, 1, samplers.as_ptr());
    }
}

impl SwapChain {
    unsafe fn back_buffer(&self, index: u32) -> Result<Texture, String> {
        let table = vtbl::<SwapChainVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.get_buffer)(self.as_ptr(), index, &IID_ID3D11TEXTURE2D, &mut raw);
        if hr < 0 {
            return Err(format!("GetBuffer failed: 0x{:08X}", hr as u32));
        }
        Texture::from_raw(raw).ok_or_else(|| "GetBuffer returned null".into())
    }

    unsafe fn resize_buffers(&self, width: u32, height: u32) -> Result<(), String> {
        let table = vtbl::<SwapChainVtbl>(self.as_ptr());
        let hr = (table.resize_buffers)(self.as_ptr(), 0, width, height, 0, 0);
        if hr < 0 {
            return Err(format!("ResizeBuffers failed: 0x{:08X}", hr as u32));
        }
        Ok(())
    }

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
SamplerState atlas_sampler : register(s0);

struct VSInput
{
    float2 pos   : TEXCOORD0;
    float2 size  : TEXCOORD1;
    float4 uv    : TEXCOORD2;
    float4 color : TEXCOORD3;
};

struct VSOutput
{
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
    float4 color    : TEXCOORD1;
    float  solid    : TEXCOORD2;
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
    // uv.zw == uv.xy 表示"纯色矩形"（背景、光标），不需要采样图集。
    output.solid = (input.uv.x == input.uv.z && input.uv.y == input.uv.w) ? 1.0 : 0.0;
    return output;
}

float4 ps_main(VSOutput input) : SV_Target
{
    // 混合状态是 SrcBlend=ONE（预乘 Alpha），所以输出必须已经乘过 alpha。
    if (input.solid > 0.5)
    {
        return input.color;
    }
    float coverage = atlas_texture.Sample(atlas_sampler, input.uv).r;
    // 预乘：RGB 和 A 都乘覆盖率。漏掉 RGB 会变成加法混合 → 字形膨胀成实心块。
    return input.color * coverage;
}
"#;

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
    input_layout: InputLayout,
    blend: BlendState,
    rasterizer: RasterizerState,
    constant_buffer: Buffer,
    instance_buffer: Buffer,
}

struct Atlas {
    texture: Texture,
    srv: ShaderResourceView,
    sampler: SamplerState,
}

unsafe fn create_pipeline(device: &Device) -> Result<Pipeline, String> {
    let vs_blob = compile_shader("vs_main", "vs_5_0")?;
    let ps_blob = compile_shader("ps_main", "ps_5_0")?;

    let vertex_shader = device.create_vertex_shader(vs_blob.bytes())?;
    let pixel_shader = device.create_pixel_shader(ps_blob.bytes())?;

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
        scissor_enable: 0,
        multisample_enable: 0,
        antialiased_line_enable: 0,
    })?;

    Ok(Pipeline {
        vertex_shader,
        pixel_shader,
        input_layout,
        blend,
        rasterizer,
        constant_buffer,
        instance_buffer,
    })
}

// ============================ Gfx ============================

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
}

impl Gfx {
    /// 创建 D3D11 设备、交换链与管线，并绑定到给定窗口。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `hwnd` 是**有效且未销毁**的窗口句柄，且调用期间不会被其它线程销毁；
    /// - 调用发生在拥有该窗口的线程上（D3D11 immediate context 非线程安全）；
    /// - `width`、`height` 均大于 0。
    ///
    /// 违反任一条会导致：DXGI 创建失败（返回错误码，可恢复），
    /// 或产生绑定到悬垂 HWND 的交换链，后续 `Present` 访问已释放窗口内存
    /// （未定义行为，通常表现为进程崩溃）。
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
            format: DXGI_FORMAT_R8_UNORM,
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
            row_pitch: self.atlas_size,
            slice_pitch: self.atlas_size * self.atlas_size,
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
    /// 调用者必须保证：
    /// - `data.len() >= (size as usize) * (size as usize)`（图集是 `size × size` 的 R8 单通道）；
    /// - `size > 0` 且不超过设备最大纹理尺寸；
    /// - `dirty` 中每个矩形满足 `x + width <= size`、`y + height <= size`。
    ///
    /// 违反会导致：`UpdateSubresource` 从 `data` 越界读取（未定义行为），
    /// 或写入纹理越界区域（D3D11 报错或破坏相邻数据）。
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
            let row_pitch = width as usize;
            let mut patch = vec![0u8; row_pitch * height as usize];
            for row in 0..height as usize {
                let src = ((y as usize + row) * size as usize + x as usize)..;
                patch[row * row_pitch..(row + 1) * row_pitch]
                    .copy_from_slice(&data[src.start..src.start + row_pitch]);
            }
            let subresource = SubresourceData {
                data: patch.as_ptr() as *const c_void,
                row_pitch: width,
                slice_pitch: width * height,
            };
            context.update_subresource_region(&atlas.texture, &subresource, x, y, width, height);
        }
        Ok(())
    }

    /// 调整交换链缓冲尺寸，并重新绑定渲染目标。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `width`、`height` 均大于 0；
    /// - 调用时没有任何外部代码仍持有旧的 back buffer / RTV 引用
    ///   （本方法内部会先解绑并释放）。
    ///
    /// 违反会导致：DXGI 返回 `DXGI_ERROR_INVALID_CALL`（可恢复，本次 resize 失败），
    /// 或旧引用与新建缓冲并存，造成画面错乱。
    pub unsafe fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == 0 || height == 0 || (width == self.width && height == self.height) {
            return Ok(());
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
        Ok(())
    }

    /// 提交一帧：上传实例、画一次 `DrawInstanced`、`Present`。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `Gfx` 已通过 [`Gfx::new`] / [`Gfx::rebuild`] 成功初始化；
    /// - 若图集存在，必须在此前调用过 [`Gfx::update_atlas`]（否则 PS 采样未绑定的 SRV）；
    /// - 仍在创建窗口的线程上调用。
    ///
    /// `instances` 长度任意：内部按 `INSTANCE_CAPACITY` 截断，不会越界。
    /// 违反前述任一条不会造成内存不安全，但会得到未定义的画面或设备错误码。
    pub unsafe fn render(&mut self, instances: &[Instance], sync_interval: u32) -> Hresult {
        let (Some(pipeline), Some(rtv), Some(context), Some(swap_chain)) = (
            self.pipeline.as_ref(),
            self.rtv.as_ref(),
            self.context.as_ref(),
            self.swap_chain.as_ref(),
        ) else {
            return 0;
        };

        let globals = [self.width as f32, self.height as f32, 0.0, 0.0];
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
                None => eprintln!("[dbg] map_discard failed"),
            }
        }

        context.set_render_target(Some(rtv));
        context.set_viewport(self.width as f32, self.height as f32);
        context.clear(rtv, [0.02, 0.02, 0.03, 1.0]);
        context.set_pipeline(pipeline);
        if let Some(atlas) = self.atlas.as_ref() {
            context.set_ps_texture(&atlas.srv, &atlas.sampler);
        }
        context.draw_instances(count as u32);

        swap_chain.present(sync_interval)
    }

    pub fn is_empty(&self) -> bool {
        self.device.is_none()
    }

    pub fn device_lost(hr: Hresult) -> bool {
        matches!(
            hr as u32,
            DXGI_ERROR_DEVICE_REMOVED | DXGI_ERROR_DEVICE_RESET
        )
    }

    pub fn is_occluded(hr: Hresult) -> bool {
        hr as u32 == DXGI_STATUS_OCCLUDED
    }

    /// 调试诊断：当前存活的 COM 对象数（Release 构建恒为 0）。
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

    unreachable!("loop always returns")
}
