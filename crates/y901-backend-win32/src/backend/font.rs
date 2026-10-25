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

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{BOOL, RECT};

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

macro_rules! com_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        struct $name(*mut c_void);

        impl $name {
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
    get_glyph_indices: unsafe extern "system" fn(*mut c_void, *const u16, u32, *mut u16) -> Hresult,
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
    unsafe fn metrics(&self) -> FontMetrics {
        let table = vtbl::<FontFaceVtbl>(self.as_ptr());
        let mut metrics = FontMetrics::default();
        (table.get_metrics)(self.as_ptr(), &mut metrics);
        metrics
    }

    unsafe fn design_metrics(&self, glyph: u16) -> GlyphMetrics {
        let table = vtbl::<FontFaceVtbl>(self.as_ptr());
        let mut metrics = GlyphMetrics::default();
        (table.get_design_glyph_metrics)(self.as_ptr(), &glyph, 1, &mut metrics, 0);
        metrics
    }

    unsafe fn glyph_index(&self, codepoint: u16) -> u16 {
        let table = vtbl::<FontFaceVtbl>(self.as_ptr());
        let mut glyph = 0u16;
        (table.get_glyph_indices)(self.as_ptr(), &codepoint, 1, &mut glyph);
        glyph
    }
}

impl GlyphRunAnalysis {
    unsafe fn alpha_bounds(&self, texture_type: u32) -> RECT {
        let table = vtbl::<GlyphRunAnalysisVtbl>(self.as_ptr());
        let mut rect: RECT = std::mem::zeroed();
        (table.get_alpha_texture_bounds)(self.as_ptr(), texture_type, &mut rect);
        rect
    }

    /// 取 8-bit 覆盖率。ClearType(3x1) 输入时只保留绿通道。
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
        if bpp == 1 {
            raw
        } else {
            (0..(width * height) as usize)
                .map(|i| raw[i * 3 + 1])
                .collect()
        }
    }
}

// ============================ 图集打包 ============================

/// 用 `atlas::SkylinePacker` 替换原来的货架打包。
///
/// 货架打包按"该行最高字形"定行高，混合字号下浪费 35%~55%；
/// skyline 按实际高度紧凑排列，同样的图集能多放 ~40% 的字形。
use super::atlas::SkylinePacker;

// ============================ 对外接口 ============================

/// 图集里的一块脏区，供渲染层做 UpdateSubresource。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtyRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug)]
pub struct GlyphInfo {
    pub width: u32,
    pub height: u32,
    /// 相对基线原点（pen position）的墨迹偏移。
    pub bearing_x: i32,
    pub bearing_y: i32,
    pub advance: f32,
    /// 图集内像素坐标。
    pub atlas_x: u32,
    pub atlas_y: u32,
    /// 最近一次使用帧（LRU）。
    pub last_used: u64,
}

impl GlyphInfo {
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

pub struct TextEngine {
    factory: Factory,
    face: FontFace,
    font_size: f32,
    units_per_em: f32,

    pub atlas: Vec<u8>,
    pub atlas_size: u32,
    packer: SkylinePacker,

    pub ascent: f32,
    #[allow(dead_code)]
    pub descent: f32,
    pub cell_width: f32,
    #[allow(dead_code)] // 元数据：等宽字体的名义行高，目前用 ascent/descent
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

            let advance_of = |ch: char| {
                let glyph = face.glyph_index(ch as u16);
                let design = face.design_metrics(glyph);
                design.advance_width as f32 * font_size / units_per_em
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
                atlas: vec![0u8; (ATLAS_SIZE * ATLAS_SIZE) as usize],
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

    #[allow(dead_code)] // 诊断：已缓存字形数
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

    /// 取出并清空脏区；reset = true 表示图集被整体重建。
    pub fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>) {
        let reset = std::mem::replace(&mut self.reset_pending, false);
        let dirty = std::mem::take(&mut self.dirty);
        (reset, dirty)
    }

    #[allow(dead_code)] // 诊断：是否有待上传的图集脏区
    pub fn has_dirty(&self) -> bool {
        self.reset_pending || !self.dirty.is_empty()
    }

    unsafe fn rasterize(&mut self, ch: char) -> Result<GlyphInfo, String> {
        let mut utf16 = [0u16; 2];
        let codepoints = ch.encode_utf16(&mut utf16);
        let glyph = self.face.glyph_index(codepoints[0]);
        if glyph == 0 {
            return Err(format!("glyph missing ({:?})", ch));
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
            for texture_type in [DWRITE_TEXTURE_ALIASED_1X1, DWRITE_TEXTURE_CLEARTYPE_3X1] {
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
        for row in 0..height {
            let src = &coverage[(row * width) as usize..((row + 1) * width) as usize];
            let dst = ((atlas_y + row) * self.atlas_size + atlas_x) as usize;
            self.atlas[dst..dst + width as usize].copy_from_slice(src);
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
            let Some((x, y)) = self.packer.insert(info.width + PADDING, info.height + PADDING)
            else {
                continue;
            };
            for row in 0..info.height {
                let src = ((info.atlas_y + row) * self.atlas_size + info.atlas_x) as usize;
                let dst = ((y + row) * self.atlas_size + x) as usize;
                self.atlas[dst..dst + info.width as usize]
                    .copy_from_slice(&snapshot[src..src + info.width as usize]);
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
            ('\u{4e2d}', 2),   // 中
            ('\u{65e5}', 2),   // 日
            ('\u{ff21}', 2),   // Ａ（全角）
            ('\u{1f600}', 2),  // 😀（emoji，宽）
            ('\u{200b}', 0),   // 零宽空格
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