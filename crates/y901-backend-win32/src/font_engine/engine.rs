//! `MyFontEngine`：自研引擎的图集层（Milestone 2）。
//!
//! 职责：把 M1 的覆盖度位图写进图集，管 LRU、管脏区协议、管度量。
//!
//! **三个与 legacy 一致的硬约束**：
//! 1. `cell_span` 直接调 `y901_core::Grid::width_of_char`（与 DirectWrite 路径同一来源）
//!    ——这是"格契约"，不是按字体 advance 反推；
//! 2. 图集像素格式：RGBA，`rgb` = 覆盖度，`a` = 255（与 DirectWrite 路径一致，两个后端都直接读）；
//! 3. `take_dirty`：`reset` 表示"整张图集重建"，消耗式（取走后置 false）。

use std::collections::HashMap;

use y901_ui::Size;

use super::super::backend::atlas::SkylinePacker;
use super::super::backend::font::{DirtyRect, FontEngine, GlyphInfo};
use super::{rasterize_glyph_phased, GlyphBitmap};

/// 图集边长（与 legacy 一致）。
const ATLAS_SIZE: u32 = 1024;
/// 字形之间的间隙（与 legacy 一致）。
const PADDING: u32 = 1;
/// 热集保护：最近 N 个字形不参与驱逐。
const PIN_LAST: usize = 512;
/// 驱逐目标占用率（每次驱逐降到这个值以下）。
const EVICT_TARGET: f32 = 0.85;

/// 自研字形引擎（图集 + LRU）。
pub struct MyFontEngine {
    /// 字体字节常驻（`Face` 每次临时构造）。
    data: Vec<u8>,
    /// 逻辑字号（像素）。
    size: f32,
    /// 格尺寸（与 legacy 一致）。
    cell: Size,
    /// 基线高度（从字体 `ascender` 算）。
    ascent: f32,
    /// 图集：RGBA。
    atlas: Vec<u8>,
    atlas_size: u32,
    packer: SkylinePacker,
    /// `(char, phase)` → 字形信息。
    cache: HashMap<(char, u8), GlyphInfo>,
    /// 当前帧号（LRU 用）。
    frame: u64,
    /// 待上传的脏区。
    dirty: Vec<DirtyRect>,
    /// 图集是否需要整张重建。
    reset_pending: bool,
    /// 上一帧驱逐了多少个字形（验收用）。
    evicted_last_frame: usize,
}

impl MyFontEngine {
    /// 创建引擎。
    ///
    /// 输入：`data`（TTF/OTF 字节，必须常驻）、`size_px`、`cell`。
    /// 返回：`Err` = 字体解析失败。副作用：无。
    pub fn new(data: Vec<u8>, size_px: f32, cell: Size) -> Result<Self, String> {
        let face = ttf_parser::Face::parse(&data, 0)
            .map_err(|e| format!("字体解析失败: {e}"))?;
        let units = face.units_per_em() as f32;
        let ascent = face.ascender() as f32 * size_px / units.max(1.0);
        Ok(Self {
            data,
            size: size_px,
            cell,
            ascent,
            atlas: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            atlas_size: ATLAS_SIZE,
            packer: SkylinePacker::new(ATLAS_SIZE),
            cache: HashMap::new(),
            frame: 0,
            dirty: Vec::new(),
            reset_pending: true,
            evicted_last_frame: 0,
        })
    }

    /// 字形占用的格数。
    ///
    /// **与 legacy 同一来源**：`y901_core::Grid::width_of_char`。
    /// 这是格契约，不能按字体 advance 反推——否则排版会与 DirectWrite 路径不一致。
    pub fn cell_span(&self, ch: char) -> u32 {
        y901_core::Grid::width_of_char(ch) as u32
    }

    /// 取字形（不命中则现场光栅化并入图集）。
    pub fn glyph(&mut self, ch: char) -> Option<&GlyphInfo> {
        self.glyph_phased(ch, 0)
    }

    /// 带亚像素相位取字形（`phase` ∈ 0..=3）。
    ///
    /// 缓存 key 是 `(char, phase)`——同一字的 4 个相位是 4 张不同位图。
    pub fn glyph_phased(&mut self, ch: char, phase: u8) -> Option<&GlyphInfo> {
        let key = (ch, phase & 3);
        if !self.cache.contains_key(&key) {
            let bitmap = rasterize_glyph_phased(&self.data, ch, self.size, phase)?;
            let info = self.insert_into_atlas(ch, &bitmap)?;
            self.cache.insert(key, info);
        }
        if let Some(g) = self.cache.get_mut(&key) {
            g.last_used = self.frame;
        }
        self.cache.get(&key)
    }

    /// 把一张覆盖度位图写进图集，返回字形信息。
    fn insert_into_atlas(&mut self, _ch: char, bmp: &GlyphBitmap) -> Option<GlyphInfo> {
        let (w, h) = (bmp.width.max(1), bmp.height.max(1));
        let mut pos = self.packer.insert(w + PADDING, h + PADDING);
        if pos.is_none() {
            // 满了：驱逐到目标占用率以下再试一次
            self.evict_until(EVICT_TARGET);
            pos = self.packer.insert(w + PADDING, h + PADDING);
        }
        let (ax, ay) = pos?;

        // 写像素：RGB = 覆盖度，A = 255（与 DirectWrite 路径格式一致）
        let asz = self.atlas_size as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let cov = bmp.coverage.get(y * w as usize + x).copied().unwrap_or(0);
                let i = ((ay as usize + y) * asz + (ax as usize + x)) * 4;
                self.atlas[i] = cov;
                self.atlas[i + 1] = cov;
                self.atlas[i + 2] = cov;
                self.atlas[i + 3] = 255;
            }
        }
        self.dirty.push(DirtyRect {
            x: ax,
            y: ay,
            width: w,
            height: h,
        });
        Some(GlyphInfo {
            width: w,
            height: h,
            bearing_x: bmp.bearing_x,
            bearing_y: bmp.bearing_y,
            advance: bmp.advance,
            atlas_x: ax,
            atlas_y: ay,
            last_used: self.frame,
        })
    }

    /// 按 LRU 驱逐到占用率低于 `target`。
    ///
    /// 策略：最近 `PIN_LAST` 个不动（热集保护）；其余按 `last_used` 升序驱逐。
    fn evict_until(&mut self, target: f32) {
        let total = (self.atlas_size * self.atlas_size) as f32;
        let mut evicted = 0usize;
        while self.occupancy() > target {
            // 选一个候选：不在热集中、`last_used` 最小
            let cutoff = self.frame.saturating_sub(PIN_LAST as u64);
            let victim = self
                .cache
                .iter()
                .filter(|(_, g)| g.last_used < cutoff)
                .min_by_key(|(_, g)| g.last_used)
                .map(|(c, _)| *c);
            let Some(victim) = victim else {
                break; // 全是热集，无法再驱逐
            };
            if let Some(g) = self.cache.remove(&victim) {
                // 图集里的像素清零（避免残影），并标脏该区域
                let asz = self.atlas_size as usize;
                for y in g.atlas_y..g.atlas_y + g.height {
                    for x in g.atlas_x..g.atlas_x + g.width {
                        let i = ((y as usize) * asz + x as usize) * 4;
                        self.atlas[i..i + 4].fill(0);
                    }
                }
                self.dirty.push(DirtyRect {
                    x: g.atlas_x,
                    y: g.atlas_y,
                    width: g.width,
                    height: g.height,
                });
                // 把死了的区域还给 packer：重建 packer 会丢失其他字形布局，
                // 所以这里不重建（M2 接受一定的空洞浪费；M3 再做紧凑）。
                evicted += 1;
            }
            if evicted > 4096 {
                break;
            }
        }
        self.evicted_last_frame += evicted;
        let _ = total;
    }

    /// 图集占用率（0..1）。
    pub fn occupancy(&self) -> f32 {
        let used: u32 = self
            .cache
            .values()
            .map(|g| (g.width + PADDING) * (g.height + PADDING))
            .sum();
        used as f32 / (self.atlas_size * self.atlas_size) as f32
    }

    /// 上一帧驱逐的字形数（验收指标）。
    pub fn evicted_last_frame(&self) -> usize {
        self.evicted_last_frame
    }

    /// 清空图集（验收用：清空后必须能全量恢复）。
    pub fn clear_atlas(&mut self) {
        self.cache.clear();
        self.packer = SkylinePacker::new(ATLAS_SIZE);
        self.atlas.fill(0);
        self.reset_pending = true;
    }

    /// 开始一帧（推进帧号，重置驱逐计数）。
    pub fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        self.evicted_last_frame = 0;
    }

    /// 取走脏区：`reset` 为 true 表示整张图集需重建（消耗式）。
    pub fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>) {
        let reset = std::mem::replace(&mut self.reset_pending, false);
        let dirty = std::mem::take(&mut self.dirty);
        (reset, dirty)
    }

    /// 图集像素（RGBA）。
    pub fn atlas(&self) -> &[u8] {
        &self.atlas
    }

    /// 图集边长。
    pub fn atlas_size(&self) -> u32 {
        self.atlas_size
    }

    /// 基线高度。
    pub fn ascent(&self) -> f32 {
        self.ascent
    }

    /// 格宽度。
    pub fn cell_width(&self) -> f32 {
        self.cell.w
    }

    /// 格高度。
    pub fn cell_height(&self) -> f32 {
        self.cell.h
    }

    /// 已缓存字形数。
    pub fn glyph_count(&self) -> usize {
        self.cache.len()
    }
}

impl FontEngine for MyFontEngine {
    fn glyph(&mut self, ch: char) -> Option<&GlyphInfo> {
        MyFontEngine::glyph(self, ch)
    }
    fn cell_span(&self, ch: char) -> u32 {
        MyFontEngine::cell_span(self, ch)
    }
    fn atlas(&self) -> &[u8] {
        &self.atlas
    }
    fn atlas_size(&self) -> u32 {
        self.atlas_size
    }
    fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>) {
        MyFontEngine::take_dirty(self)
    }
    fn begin_frame(&mut self) {
        MyFontEngine::begin_frame(self)
    }
    fn ascent(&self) -> f32 {
        self.ascent
    }
    fn cell_width(&self) -> f32 {
        self.cell.w
    }
    fn cell_height(&self) -> f32 {
        self.cell.h
    }
}
