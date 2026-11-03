//! DirectWrite 动态文本引擎（M3b）。
//!
//! 职责：
//! - 持有 IDWriteFactory / FontFace；
//! - 按需光栅化任意字符（含 UTF-16 代理对）；
//! - Shelf 打包进 1024x1024 的 R8 图集；
//! - 缓存 GlyphInfo，并记录图集脏区供 D3D11 做 UpdateSubresource。
//!
//! 布局约定：调用方给出引擎坐标系里的 (base_x, baseline)，
//! 绘制时 pos = (base_x + bearing_x, baseline + bearing_y)。

// # 安全契约
//
// 本文件直接调用 DirectWrite/Direct2D 的 COM 接口，`unsafe` 代码共同依赖三条不变量：
//
// 1. **COM 对象由 `Drop` 释放**：所有工厂/字体/布局对象都被拥有型别名包裹，
//    `Release` 只在 `Drop` 里调用，因此对象存活期内 vtable 恒有效。
// 2. **`*mut c_void` 参数均为存活对象**：`GetGlyphRunOutline`/`CreateTextLayout` 等的输出指针
//    由调用方提供且保证非空、对齐；返回值负（HRESULT 失败）时不再读取该缓冲。
// 3. **字体字节生命周期**：`MyFontEngine` 持有所需的字体数据副本，DirectWrite 侧不保留裸指针。


use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{BOOL, RECT};

/// HRESULT；负值表示失败。
pub type Hresult = i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const IID_IDWRITEFACTORY: Guid = Guid {
    data1: 0xb859_ee5a,
    data2: 0xd838,
    data3: 0x4b5b,
    data4: [0xa2, 0xe8, 0x1a, 0xdc, 0x7d, 0x93, 0xdb, 0x48],
};

const DWRITE_FACTORY_TYPE_SHARED: u32 = 0;
const DWRITE_FONT_WEIGHT_NORMAL: u32 = 400;
const DWRITE_FONT_STRETCH_NORMAL: u32 = 5;
const DWRITE_FONT_STYLE_NORMAL: u32 = 0;
const DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC: u32 = 5;
const DWRITE_MEASURING_MODE_NATURAL: u32 = 0;
const DWRITE_TEXTURE_ALIASED_1X1: u32 = 0;
const DWRITE_TEXTURE_CLEARTYPE_3X1: u32 = 1;

/// 图集边长。1024^2 的 R8 = 1 MB。
pub const ATLAS_SIZE: u32 = 1024;
/// 字形之间留 1px，避免线性采样串色。
const PADDING: u32 = 1;

#[link(name = "dwrite")]
unsafe extern "system" {
    fn DWriteCreateFactory(
        factory_type: u32,
        iid: *const Guid,
        factory: *mut *mut c_void,
    ) -> Hresult;
}

// ============================ COM 基础设施 ============================

type QueryInterfaceFn =
    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult;
type AddRefFn = unsafe extern "system" fn(*mut c_void) -> u32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

/// # Safety
///
/// `object` 必须是**未释放的 DirectWrite COM 接口指针**，且它的 vtable 布局与 `T` 逐字段一致
/// （按 Windows SDK 声明顺序）。传错类型不会崩，但会调到错的方法。
unsafe fn vtbl<T>(object: *mut c_void) -> &'static T {
    &*(*(object as *mut *const T))
}

/// # Safety
///
/// `self.ptr` 要么为空、要么是一个**仍持有引用**的合法 COM 接口指针；本函数按 COM 规则调用
/// `Release` 并把指针置空，因此同一对象不得被并发释放（`&mut self` 已保证串行）。
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

macro_rules! com_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        struct $name(*mut c_void);

        impl $name {
            /// # Safety
            ///
            /// `raw` 必须是某次 `Create*`/`Query*` 返回、且**所有权尚未转交他人**的接口指针；
            /// 本函数接管所有权（`Drop` 时释放）。传 null 返回 `None`，不会 panic。
            unsafe fn from_raw(ptr: *mut c_void) -> Option<Self> {
                (!ptr.is_null()).then_some(Self(ptr))
            }

            fn as_ptr(&self) -> *mut c_void {
                self.0
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                unsafe { release(self.0) }
            }
        }
    };
}

com_type!(Factory, "IDWriteFactory");
com_type!(FontCollection, "IDWriteFontCollection");
com_type!(FontFamily, "IDWriteFontFamily");
com_type!(Font, "IDWriteFont");
com_type!(FontFace, "IDWriteFontFace");
com_type!(GlyphRunAnalysis, "IDWriteGlyphRunAnalysis");

// ============================ 结构 ============================

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FontMetrics {
    design_units_per_em: u16,
    ascent: u16,
    descent: u16,
    line_gap: u16,
    cap_height: u16,
    x_height: u16,
    underline_position: u16,
    underline_thickness: u16,
    strikethrough_position: u16,
    strikethrough_thickness: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct GlyphMetrics {
    left_side_bearing: i32,
    advance_width: i32,
    right_side_bearing: i32,
    top_side_bearing: i32,
    advance_height: i32,
    bottom_side_bearing: i32,
    vertical_origin_y: i32,
}

#[repr(C)]
struct GlyphRun {
    font_face: *mut c_void,
    font_em_size: f32,
    glyph_count: u32,
    glyph_indices: *const u16,
    glyph_advances: *const f32,
    glyph_offsets: *const c_void,
    is_sideways: i32,
    bidi_level: u32,
}

// ============================ vtable ============================

#[repr(C)]
struct FactoryVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3：GetSystemFontCollection
    get_system_font_collection:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void, BOOL) -> Hresult,
    // 4..=22
    reserved_1: [*const c_void; 19],
    // 23：CreateGlyphRunAnalysis
    create_glyph_run_analysis: unsafe extern "system" fn(
        *mut c_void,
        *const GlyphRun,
        f32,
        *const c_void,
        u32,
        u32,
        f32,
        f32,
        *mut *mut c_void,
    ) -> Hresult,
}

#[repr(C)]
struct FontCollectionVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3：GetFontFamilyCount
    reserved_1: *const c_void,
    // 4：GetFontFamily
    get_font_family: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> Hresult,
    // 5：FindFamilyName
    find_family_name:
        unsafe extern "system" fn(*mut c_void, *const u16, *mut u32, *mut BOOL) -> Hresult,
}

#[repr(C)]
struct FontFamilyVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3..=6：IDWriteFontList + GetFamilyNames
    reserved_1: [*const c_void; 4],
    // 7：GetFirstMatchingFont
    get_first_matching_font:
        unsafe extern "system" fn(*mut c_void, u32, u32, u32, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
struct FontVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3..=12
    reserved_1: [*const c_void; 10],
    // 13：CreateFontFace
    create_font_face: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
struct FontFaceVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3..=7
    reserved_1: [*const c_void; 5],
    // 8：GetMetrics
    get_metrics: unsafe extern "system" fn(*mut c_void, *mut FontMetrics),
    // 9：GetGlyphCount
    reserved_2: *const c_void,
    // 10：GetDesignGlyphMetrics
    get_design_glyph_metrics:
        unsafe extern "system" fn(*mut c_void, *const u16, u32, *mut GlyphMetrics, BOOL) -> Hresult,
    // 11：GetGlyphIndices
    /// DWrite 的原型是 GetGlyphIndices(const UINT32 *codePoints, UINT16 *glyphIndices, UINT32 glyphCount)。
    /// **码点必须是 32 位**：声明成 *const u16 会让 DWrite 每次读 4 字节，
    /// Release/LTO 下栈布局变化后高 16 位是垃圾值 → 查不到字形（整屏缺字）。
    get_glyph_indices: unsafe extern "system" fn(*mut c_void, *const u32, u32, *mut u16) -> Hresult,
}

#[repr(C)]
struct GlyphRunAnalysisVtbl {
    query_interface: QueryInterfaceFn,
    add_ref: AddRefFn,
    release: ReleaseFn,
    // 3：GetAlphaTextureBounds
    get_alpha_texture_bounds: unsafe extern "system" fn(*mut c_void, u32, *mut RECT) -> Hresult,
    // 4：CreateAlphaTexture
    create_alpha_texture:
        unsafe extern "system" fn(*mut c_void, u32, *const RECT, *mut u8, u32) -> Hresult,
}

// ============================ DirectWrite 方法 ============================

impl Factory {
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn system_font_collection(&self) -> Result<FontCollection, String> {
        let table = vtbl::<FactoryVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.get_system_font_collection)(self.as_ptr(), &mut raw, 0);
        if hr < 0 {
            return Err(format!(
                "GetSystemFontCollection failed: 0x{:08X}",
                hr as u32
            ));
        }
        FontCollection::from_raw(raw).ok_or_else(|| "font collection is null".to_string())
    }

    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn glyph_run_analysis(
        &self,
        run: &GlyphRun,
        rendering_mode: u32,
    ) -> Result<GlyphRunAnalysis, String> {
        let table = vtbl::<FactoryVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_glyph_run_analysis)(
            self.as_ptr(),
            run,
            1.0,
            null(),
            rendering_mode,
            DWRITE_MEASURING_MODE_NATURAL,
            0.0,
            0.0,
            &mut raw,
        );
        if hr < 0 {
            return Err(format!(
                "CreateGlyphRunAnalysis failed: 0x{:08X}",
                hr as u32
            ));
        }
        GlyphRunAnalysis::from_raw(raw).ok_or_else(|| "glyph run analysis is null".to_string())
    }
}

impl FontCollection {
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn find_family(&self, name: &[u16]) -> Result<FontFamily, String> {
        let table = vtbl::<FontCollectionVtbl>(self.as_ptr());
        let mut index = 0u32;
        let mut exists: BOOL = 0;
        let hr = (table.find_family_name)(self.as_ptr(), name.as_ptr(), &mut index, &mut exists);
        if hr < 0 || exists == 0 {
            return Err("font family not found".to_string());
        }
        let mut raw: *mut c_void = null_mut();
        let hr = (table.get_font_family)(self.as_ptr(), index, &mut raw);
        if hr < 0 {
            return Err(format!("GetFontFamily failed: 0x{:08X}", hr as u32));
        }
        FontFamily::from_raw(raw).ok_or_else(|| "font family is null".to_string())
    }
}

impl FontFamily {
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn first_font(&self) -> Result<Font, String> {
        let table = vtbl::<FontFamilyVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.get_first_matching_font)(
            self.as_ptr(),
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            DWRITE_FONT_STYLE_NORMAL,
            &mut raw,
        );
        if hr < 0 {
            return Err(format!("GetFirstMatchingFont failed: 0x{:08X}", hr as u32));
        }
        Font::from_raw(raw).ok_or_else(|| "font is null".to_string())
    }
}

impl Font {
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn create_face(&self) -> Result<FontFace, String> {
        let table = vtbl::<FontVtbl>(self.as_ptr());
        let mut raw: *mut c_void = null_mut();
        let hr = (table.create_font_face)(self.as_ptr(), &mut raw);
        if hr < 0 {
            return Err(format!("CreateFontFace failed: 0x{:08X}", hr as u32));
        }
        FontFace::from_raw(raw).ok_or_else(|| "font face is null".to_string())
    }
}

impl FontFace {
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn metrics(&self) -> FontMetrics {
        let table = vtbl::<FontFaceVtbl>(self.as_ptr());
        let mut metrics = FontMetrics::default();
        (table.get_metrics)(self.as_ptr(), &mut metrics);
        metrics
    }

    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn design_metrics(&self, glyph: u16) -> GlyphMetrics {
        let table = vtbl::<FontFaceVtbl>(self.as_ptr());
        let mut metrics = GlyphMetrics::default();
        (table.get_design_glyph_metrics)(self.as_ptr(), &glyph, 1, &mut metrics, 0);
        metrics
    }

    /// 查一个码点（**32 位**，不用 UTF-16 代理项）的字形下标。
    ///
    /// 输入：`codepoint` = `char as u32`（非 BMP 直接是完整码点）。
    /// 返回：`Ok(glyph)`；`Ok(0)` 表示字体里没有这个字形（`.notdef`）。
    /// 失败：`GetGlyphIndices` 返回负 HRESULT 时给 `Err`（含码点与 HRESULT），
    /// **不静默返回 0**——否则上层只会看到 "glyph missing"，查不到真正原因。
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn glyph_index(&self, codepoint: u32) -> Result<u16, String> {
        let table = vtbl::<FontFaceVtbl>(self.as_ptr());
        let mut glyph = 0u16;
        let hr = (table.get_glyph_indices)(self.as_ptr(), &codepoint, 1, &mut glyph);
        if hr < 0 {
            return Err(format!(
                "GetGlyphIndices failed: codepoint=U+{codepoint:04X} hr=0x{:08X}",
                hr as u32
            ));
        }
        Ok(glyph)
    }
}

impl GlyphRunAnalysis {
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn alpha_bounds(&self, texture_type: u32) -> RECT {
        let table = vtbl::<GlyphRunAnalysisVtbl>(self.as_ptr());
        let mut rect: RECT = std::mem::zeroed();
        (table.get_alpha_texture_bounds)(self.as_ptr(), texture_type, &mut rect);
        rect
    }

    /// 取 8-bit 覆盖率。ClearType(3x1) 输入时只保留绿通道。
    /// # Safety
    ///
    /// `self` 必须是**未释放**的 DirectWrite COM 对象（工厂/字体集合/字体/字形分析等），
    /// 且通过本文件 `com_type!` 生成的拥有型别名持有；输出指针（`*mut *mut c_void` 或结构指针）
    /// 由调用方提供、非空、按接口要求对齐。返回负 HRESULT 时不再读取输出缓冲。
    unsafe fn coverage(&self, bounds: &RECT, texture_type: u32) -> Vec<u8> {
        let width = (bounds.right - bounds.left).max(0) as u32;
        let height = (bounds.bottom - bounds.top).max(0) as u32;
        let bpp = if texture_type == DWRITE_TEXTURE_CLEARTYPE_3X1 {
            3
        } else {
            1
        };
        let mut raw = vec![0u8; (width * height * bpp) as usize];
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let table = vtbl::<GlyphRunAnalysisVtbl>(self.as_ptr());
        (table.create_alpha_texture)(
            self.as_ptr(),
            texture_type,
            bounds,
            raw.as_mut_ptr(),
            raw.len() as u32,
        );
        // 统一输出 RGBA：ClearType(3x1) 保留 R/G/B 三个通道，灰度(1x1) 复制到三通道。
        // 着色器按子像素采样时就要这三个值；只留绿通道会退化成灰度抗锯齿。
        let px = (width * height) as usize;
        let mut out = vec![0u8; px * 4];
        for i in 0..px {
            let (r, g, b) = if bpp == 3 {
                (raw[i * 3], raw[i * 3 + 1], raw[i * 3 + 2])
            } else {
                (raw[i], raw[i], raw[i])
            };
            out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, 255]);
        }
        out
    }
}

// ============================ 图集打包 ============================

/// 用 `atlas::SkylinePacker` 替换原来的货架打包。
///
/// 货架打包按"该行最高字形"定行高，混合字号下浪费 35%~55%；
/// skyline 按实际高度紧凑排列，同样的图集能多放 ~40% 的字形。
use y901_render::atlas::SkylinePacker;

// ============================ 对外接口 ============================

/// 图集里的一块脏区，供渲染层做 UpdateSubresource。
// 已搬到 `y901-render::font`（平台无关），这里只保留 re-export 以保持旧路径可用。
pub use y901_render::font::{cell_metrics_for, DirtyRect, FontEngine, GlyphInfo};

impl FontEngine for TextEngine {
    fn glyph(&mut self, ch: char) -> Option<&GlyphInfo> {
        TextEngine::glyph(self, ch)
    }
    fn cell_span(&self, ch: char) -> u32 {
        TextEngine::cell_span(self, ch)
    }
    fn atlas(&self) -> &[u8] {
        &self.atlas
    }
    fn atlas_size(&self) -> u32 {
        self.atlas_size
    }
    fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>) {
        TextEngine::take_dirty(self)
    }
    fn begin_frame(&mut self) {
        TextEngine::begin_frame(self)
    }
    fn ascent(&self) -> f32 {
        self.ascent
    }
    fn cell_width(&self) -> f32 {
        self.cell_width
    }
    fn cell_height(&self) -> f32 {
        self.cell_height
    }
}

/// DirectWrite 字形引擎（Windows 原生实现）。与自研 `MyFontEngine` 实现同一个 `FontEngine`，两者可互换而上层一行不改。
pub struct TextEngine {
    factory: Factory,
    face: FontFace,
    font_size: f32,
    units_per_em: f32,

    /// 字形图集像素（RGBA：`rgb` 存覆盖率，`a` 恒为 255）。
    pub atlas: Vec<u8>,
    /// 图集边长（像素）。
    pub atlas_size: u32,
    packer: SkylinePacker,

    /// 基线到行顶的距离；绘制时基线的位置由它决定。
    pub ascent: f32,
    // 诊断 API：`ascent` 字段：自研引擎用它算基线，DirectWrite 路径由 `metrics()` 读
    // 诊断 API：`has_dirty()` 与图集统计：后端据此跳过整张上传，`font_m3_atlas` 测试直接断言
    #[allow(dead_code)]
    /// 基线到行底的距离；与 `ascent` 一起决定行高。
    pub descent: f32,
    /// 等宽单元格宽度（由字号推出，必须与布局用的一致）。
    pub cell_width: f32,
    // 诊断 API：`descent` 字段：同上，行高计算的一部分
    #[allow(dead_code)] // 元数据：等宽字体的名义行高，目前用 ascent/descent
    /// 等宽单元格高度。
    pub cell_height: f32,

    cache: HashMap<char, GlyphInfo>,
    dirty: Vec<DirtyRect>,
    reset_pending: bool,
    /// 当前帧号（LRU 用）。
    frame: u64,
}

impl TextEngine {
    /// font_size 为物理像素高度；cell_width/cell_height 由调用方给出，
    /// 并在此断言 ASCII = 1 格、CJK = 2 格。
    pub fn new(
        family_name: &str,
        font_size: f32,
        cell_width: f32,
        cell_height: f32,
    ) -> Result<Self, String> {
        unsafe {
            let mut raw: *mut c_void = null_mut();
            let hr = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, &IID_IDWRITEFACTORY, &mut raw);
            if hr < 0 {
                return Err(format!("DWriteCreateFactory failed: 0x{:08X}", hr as u32));
            }
            let factory = Factory::from_raw(raw).ok_or_else(|| "factory is null".to_string())?;

            let collection = factory.system_font_collection()?;
            let family = collection.find_family(&wide(family_name))?;
            let font = family.first_font()?;
            let face = font.create_face()?;

            let metrics = face.metrics();
            let units_per_em = metrics.design_units_per_em as f32;
            let ascent = metrics.ascent as f32 * font_size / units_per_em;
            let descent = metrics.descent as f32 * font_size / units_per_em;

            // 用 **32 位码点**（`ch as u32`）查字形：非 BMP 字符不能走 UTF-16 代理项。
            // 查询失败只警告（启动不该因为某个字形查不到就失败），返回 0 让偏差检查跳过。
            let advance_of = |ch: char| match face.glyph_index(ch as u32) {
                Ok(glyph) => {
                    let design = face.design_metrics(glyph);
                    design.advance_width as f32 * font_size / units_per_em
                }
                Err(err) => {
                    eprintln!("[y901] 警告：查字形 advance 失败（{ch:?}）：{err}");
                    0.0
                }
            };

            // 字体度量与"格"的偏差只警告、不拒绝启动：
            // 格数由 unicode-width 决定（`cell_span`），字体 advance 只影响居中偏移。
            // 早期这里是硬 Err，导致非精确 2:1 的字体直接起不来（跨平台阻塞）。
            let ascii = advance_of('M');
            let ascii_dev = (ascii - cell_width).abs();
            if ascii_dev > 0.1 * cell_width && cell_width >= 8.0 {
                eprintln!(
                    "[y901] 警告：ASCII advance ({ascii:.2}px) 偏离 cell_width ({cell_width:.2}px) 超过 10%，字形将按居中补偿摆放"
                );
            }
            let cjk = advance_of('\u{4e2d}');
            let cjk_dev = (cjk - cell_width * 2.0).abs();
            if cjk_dev > 0.1 * cell_width && cell_width >= 8.0 {
                eprintln!(
                    "[y901] 警告：CJK advance ({cjk:.2}px) 偏离 2*cell_width ({:.2}px) 超过 10%，字形将按居中补偿摆放",
                    cell_width * 2.0
                );
            }

            Ok(Self {
                factory,
                face,
                font_size,
                units_per_em,
                // RGBA：ClearType 需要 RGB 三通道覆盖率（4 字节/像素）
                atlas: vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
                atlas_size: ATLAS_SIZE,
                packer: SkylinePacker::new(ATLAS_SIZE),
                ascent,
                descent,
                cell_width,
                cell_height,
                cache: HashMap::new(),
                dirty: Vec::new(),
                reset_pending: true,
                frame: 0,
            })
        }
    }

    // 诊断 API：`glyph_count()`：图集占用诊断，`font_m3_atlas` 测试读取
    #[allow(dead_code)] // 诊断：已缓存字形数
    /// 已缓存字形数（诊断与内存估算用）。
    pub fn glyph_count(&self) -> usize {
        self.cache.len()
    }

    /// 取字形；未缓存则光栅化并写入图集。
    pub fn glyph(&mut self, ch: char) -> Option<&GlyphInfo> {
        if !self.cache.contains_key(&ch) {
            match unsafe { self.rasterize(ch) } {
                Ok(info) => {
                    self.cache.insert(ch, info);
                }
                Err(err) => {
                    eprintln!("[y901] rasterize {:?} failed: {err}", ch);
                    return None;
                }
            }
        }
        self.cache.get(&ch)
    }

    /// 字符占用的格数：**直接来自 `y901_core::width_of_char`（唯一来源）**。
    ///
    /// 早期实现用字体 advance 阈值（`advance > 1.5 * cell_width`）反推，
    /// 与网格存储侧的 unicode-width 判定可能不一致，导致字形错位/重叠。
    /// 字体 advance 现在只用于渲染层的居中偏移（见 `glyph_offset`）。
    pub fn cell_span(&self, ch: char) -> u32 {
        cell_span_of(ch)
    }

    /// 诊断用：查某个字符的字形下标（不做任何缓存与图集写入）。
    ///
    /// 输入：`ch`。返回：`Ok(glyph)`（0 = 字体里没有该字形）；
    /// 失败：`GetGlyphIndices` 返回失败 HRESULT 时给出含码点的错误。
    /// 用途：`font_probe` 测试验证"Debug 与 Release 查字形结果一致"。
    pub fn probe_glyph_index(&self, ch: char) -> Result<u16, String> {
        // SAFETY: face 由 TextEngine 持有且在 self 生命周期内有效。
        unsafe { self.face.glyph_index(ch as u32) }
    }

    /// 取出并清空脏区；reset = true 表示图集被整体重建。
    pub fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>) {
        let reset = std::mem::replace(&mut self.reset_pending, false);
        let dirty = std::mem::take(&mut self.dirty);
        (reset, dirty)
    }

    #[allow(dead_code)] // 诊断：是否有待上传的图集脏区
    /// 是否有待上传的图集脏区；false 时后端可跳过整张图集上传。
    pub fn has_dirty(&self) -> bool {
        self.reset_pending || !self.dirty.is_empty()
    }

    /// # Safety
    ///
    /// `self` 必须是未释放的字形分析器；`advance_pen`/`origin` 必须指向调用方持有的有效内存，
    /// `outline` 的容量必须足以容纳 `GetGlyphRunOutline` 写出的轮廓点数（本文件按 4096 点预留）。
    unsafe fn rasterize(&mut self, ch: char) -> Result<GlyphInfo, String> {
        // **不要用 `encode_utf16()[0]`**：非 BMP 字符（emoji 等）会得到代理项，
        // 查不到字形；DWrite 的 GetGlyphIndices 本来就要 32 位码点。
        let glyph = self.face.glyph_index(ch as u32)?;
        if glyph == 0 {
            return Err(format!("glyph missing ({:?}, U+{:04X})", ch, ch as u32));
        }

        let design = self.face.design_metrics(glyph);
        let advance = design.advance_width as f32 * self.font_size / self.units_per_em;

        let run = GlyphRun {
            font_face: self.face.as_ptr(),
            font_em_size: self.font_size,
            glyph_count: 1,
            glyph_indices: &glyph,
            glyph_advances: &advance,
            glyph_offsets: null(),
            is_sideways: 0,
            bidi_level: 0,
        };

        // 不同 Windows 版本对 rendering mode / texture type 支持不同，
        // 逐个组合试，直到拿到非空墨迹。
        let mut chosen: Option<(GlyphRunAnalysis, RECT, u32)> = None;
        'outer: for mode in [DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, 4, 1] {
            let analysis = self.factory.glyph_run_analysis(&run, mode)?;
            // 先试 ClearType（子像素），实在拿不到再退化成 1x1 灰度
            for texture_type in [DWRITE_TEXTURE_CLEARTYPE_3X1, DWRITE_TEXTURE_ALIASED_1X1] {
                let bounds = analysis.alpha_bounds(texture_type);
                if bounds.right > bounds.left && bounds.bottom > bounds.top {
                    chosen = Some((analysis, bounds, texture_type));
                    break 'outer;
                }
            }
        }

        let Some((analysis, bounds, texture_type)) = chosen else {
            // 无墨迹（空格等）：只贡献 advance。
            return Ok(GlyphInfo {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance,
                atlas_x: 0,
                atlas_y: 0,
                last_used: self.frame,
                mips: [None, None, None],
            });
        };

        let width = (bounds.right - bounds.left) as u32;
        let height = (bounds.bottom - bounds.top) as u32;

        // 图集压力处理顺序：**LRU 淘汰 → 压缩搬移 → 再分配**。
        // 绝不清空重画：清空会让所有字形重新光栅化，是可见卡顿的根源。
        let (atlas_x, atlas_y) = match self.packer.insert(width + PADDING, height + PADDING) {
            Some(spot) => spot,
            None => {
                self.evict_lru(0.70);
                self.compact_atlas();
                self.packer
                    .insert(width + PADDING, height + PADDING)
                    .ok_or_else(|| "glyph larger than atlas".to_string())?
            }
        };

        let coverage = analysis.coverage(&bounds, texture_type);
        let row_bytes = (width * 4) as usize;
        for row in 0..height {
            let src = &coverage[(row * width * 4) as usize..((row + 1) * width * 4) as usize];
            let dst = (((atlas_y + row) * self.atlas_size + atlas_x) * 4) as usize;
            self.atlas[dst..dst + row_bytes].copy_from_slice(src);
        }

        if std::env::var_os("Y901_DUMP_GLYPH").is_some() {
            println!(
                "[glyph] {:?} bounds=({},{}..{},{}) size={}x{} bearing=({},{}) atlas=({},{}) type={} cov_len={}",
                ch, bounds.left, bounds.top, bounds.right, bounds.bottom,
                width, height, bounds.left, bounds.top, atlas_x, atlas_y,
                texture_type, coverage.len()
            );
            for row in 0..height.min(20) {
                let mut line = String::new();
                for col in 0..width.min(24) {
                    let v = coverage[(row * width + col) as usize];
                    line.push(if v > 160 {
                        '#'
                    } else if v > 48 {
                        '+'
                    } else {
                        '.'
                    });
                }
                println!("[glyph]   bmp {row:>2} |{line}|");
            }
            for row in 0..height.min(20) {
                let mut line = String::new();
                for col in 0..width.min(24) {
                    let idx = ((atlas_y + row) * self.atlas_size + atlas_x + col) as usize;
                    let v = self.atlas[idx];
                    line.push(if v > 160 {
                        '#'
                    } else if v > 48 {
                        '+'
                    } else {
                        '.'
                    });
                }
                println!("[glyph]   atl {row:>2} |{line}|");
            }
        }

        self.dirty.push(DirtyRect {
            x: atlas_x,
            y: atlas_y,
            width,
            height,
        });

        Ok(GlyphInfo {
            width,
            height,
            bearing_x: bounds.left,
            bearing_y: bounds.top,
            advance,
            atlas_x,
            atlas_y,
            last_used: self.frame,
            mips: [None, None, None],
        })
    }

    /// 开始新的一帧（LRU 用）。
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// 图集占用率。
    pub fn atlas_occupancy(&self) -> f32 {
        let used: u32 = self
            .cache
            .values()
            .map(|g| (g.width + PADDING) * (g.height + PADDING))
            .sum();
        used as f32 / (self.atlas_size * self.atlas_size) as f32
    }

    /// 按使用时间淘汰，直到占用降到 `target` 以下。
    fn evict_lru(&mut self, target: f32) {
        let mut guard = 0;
        while self.atlas_occupancy() > target && !self.cache.is_empty() && guard < 100_000 {
            guard += 1;
            // 最久未使用的那个（frame 最小）
            let victim = self
                .cache
                .iter()
                .min_by_key(|(_, g)| g.last_used)
                .map(|(k, _)| *k);
            match victim {
                Some(ch) => {
                    self.cache.remove(&ch);
                }
                None => break,
            }
        }
    }

    /// 压缩搬移：把所有存活字形重新紧凑排布，只做一次全量上传。
    ///
    /// 与"清空重画"的本质区别：**不重新光栅化**，只移动像素。
    fn compact_atlas(&mut self) {
        if self.cache.is_empty() {
            self.packer.reset();
            return;
        }

        // 快照旧像素（搬移期间要读旧位置）
        let snapshot = self.atlas.clone();
        let mut entries: Vec<(char, GlyphInfo)> =
            self.cache.iter().map(|(k, v)| (*k, v.clone())).collect();
        entries.sort_by_key(|(_, g)| (g.atlas_y, g.atlas_x));

        self.atlas.fill(0);
        self.packer.reset();

        for (ch, mut info) in entries {
            let Some((x, y)) = self
                .packer
                .insert(info.width + PADDING, info.height + PADDING)
            else {
                continue;
            };
            let bytes = (info.width * 4) as usize;
            for row in 0..info.height {
                let src = (((info.atlas_y + row) * self.atlas_size + info.atlas_x) * 4) as usize;
                let dst = (((y + row) * self.atlas_size + x) * 4) as usize;
                self.atlas[dst..dst + bytes].copy_from_slice(&snapshot[src..src + bytes]);
            }
            info.atlas_x = x;
            info.atlas_y = y;
            self.cache.insert(ch, info);
        }

        self.dirty.clear();
        self.reset_pending = true;
        println!(
            "[y901] atlas compacted: {} glyphs, occupancy {:.1}%",
            self.cache.len(),
            self.atlas_occupancy() * 100.0
        );
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 格数的唯一实现：委托给 `y901_core`。
///
/// 独立成自由函数是为了可测（`TextEngine` 的构造需要 D3D 设备，单元测试起不来）。
pub(crate) fn cell_span_of(ch: char) -> u32 {
    y901_core::Grid::width_of_char(ch) as u32
}

#[cfg(test)]
mod span_tests {
    use super::cell_span_of;

    /// 格契约的字符表：**backend 的格数必须与 core 一致**。
    ///
    /// 这不是端到端渲染测试（那需要 GPU），而是契约测试：
    /// 若有人把 `cell_span_of` 改回"按字体 advance 反推"，这张表会立刻红。
    #[test]
    fn span_matches_core_implementation() {
        let cases: &[(char, u32)] = &[
            ('M', 1),
            ('a', 1),
            (' ', 1),
            ('\u{4e2d}', 2),  // 中
            ('\u{65e5}', 2),  // 日
            ('\u{ff21}', 2),  // Ａ（全角）
            ('\u{1f600}', 2), // 😀（emoji，宽）
            ('\u{200b}', 0),  // 零宽空格
        ];
        for (ch, want) in cases {
            assert_eq!(
                cell_span_of(*ch),
                *want,
                "字符 {ch:?} 的格数与 unicode-width 契约不符"
            );
            assert_eq!(
                cell_span_of(*ch),
                y901_core::Grid::width_of_char(*ch) as u32,
                "backend 与 core 的格数必须同源"
            );
        }
    }
}

/// 由字号推出单元格尺寸（逻辑像素），**纯函数**（不碰 DirectWrite）。
///
/// 输入：`font_size`（逻辑像素，<=0 时按 1 处理）。
/// 返回：`(cell_width, cell_height)`；等宽字体下宽度 = 字号的一半（16 → 8×16）。
/// 说明：真实字形宽度仍由 DirectWrite 测量（`cell_span_of` 决定跨几格），
/// 这里只给"一个格子的名义尺寸"，供网格与图集分配使用。
#[cfg(test)]
mod cell_metrics_tests {
    use super::cell_metrics_for;

    /// 默认 16px → 8×16（与历史常量一致，回归保护）。
    #[test]
    fn default_font_still_16px() {
        assert_eq!(cell_metrics_for(16.0), (8.0, 16.0));
    }

    /// 换字号会改变单元格尺寸（20px → 10×20）。
    #[test]
    fn font_size_changes_cell_size() {
        assert_eq!(cell_metrics_for(20.0), (10.0, 20.0));
        assert_ne!(cell_metrics_for(20.0).0, 8.0);
    }

    /// 非法字号不 panic，且至少 1px。
    #[test]
    fn invalid_font_size_is_clamped() {
        assert_eq!(cell_metrics_for(0.0), (1.0, 1.0));
        assert_eq!(cell_metrics_for(-5.0), (1.0, 1.0));
        assert_eq!(cell_metrics_for(f32::NAN), (1.0, 1.0));
    }
}

#[cfg(test)]
mod options_tests {
    use y901_runtime::{FontConfig, Options};

    /// 默认选项：不指定主题与字体（走 DARK + NSimSun/16）。
    #[test]
    fn defaults_are_none() {
        let o = Options::default();
        assert!(o.theme.is_none(), "默认不注入主题");
        assert!(o.font.is_none(), "默认不注入字体");
    }

    /// 默认字体：NSimSun / 16px（与历史常量一致）。
    #[test]
    fn default_font_is_nsimsun_16() {
        let f = FontConfig::default();
        assert_eq!(f.family, "NSimSun");
        assert_eq!(f.size, 16.0);
    }

    /// 显式指定字体后，cell 尺寸随之变化（P0-1 的核心断言）。
    #[test]
    fn font_config_changes_cell_size() {
        let f = FontConfig {
            family: "JetBrains Mono".to_string(),
            size: 20.0,
        };
        let (w, h) = super::cell_metrics_for(f.size);
        assert_ne!(w, 8.0, "20px 字体的单元格宽度不应还是 8");
        assert_eq!((w, h), (10.0, 20.0));
        assert_eq!(f.family, "JetBrains Mono");
    }
}

/// 最小字体探针：**Debug 与 Release 都必须能查到字形**。
///
/// 这个测试存在的唯一理由：曾经 `FontFaceVtbl::get_glyph_indices` 把码点声明成
/// `*const u16`，而 DWrite 读的是 32 位；Debug 下栈上的相邻字节恰好是 0，
/// 所以一直是对的；Release/LTO 重新布局栈后高 16 位是垃圾 → 所有字形查不到 →
/// 整屏缺字（只在 release 出现，极难定位）。
#[cfg(test)]
mod font_probe {
    use super::TextEngine;

    #[test]
    fn font_probe() {
        let engine = TextEngine::new("NSimSun", 16.0, 8.0, 16.0)
            .expect("TextEngine::new 需要 D3D/DirectWrite 设备");

        // 每个字符都必须拿到非 0 字形
        for ch in ['C', 'n', '.', 'A', '中'] {
            let g = engine
                .probe_glyph_index(ch)
                .unwrap_or_else(|e| panic!("查字形失败 {ch:?}: {e}"));
            assert_ne!(
                g, 0,
                "字符 {ch:?} (U+{:04X}) 的字形下标是 0 —— 字体里没有或查询路径错",
                ch as u32
            );
            eprintln!("[probe] {ch:?} U+{:04X} -> glyph {}", ch as u32, g);
        }

        // 非 BMP（emoji）走完整码点，不能是代理项
        match engine.probe_glyph_index('\u{1F600}') {
            Ok(g) => eprintln!("[probe] emoji -> glyph {g}（字体可能没有该字形，0 也可接受）"),
            Err(e) => eprintln!("[probe] emoji 查询返回错误（可接受）：{e}"),
        }
    }
}
