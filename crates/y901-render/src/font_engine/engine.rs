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

use super::{rasterize_glyph_phased, GlyphBitmap};
use crate::atlas::SkylinePacker;
use crate::font::{DirtyRect, FontEngine, GlyphInfo, MipSlot};

/// 一次 "按缩放选级" 的结果：实际用的级、字形信息、对应的图集坐标。
///
/// `level == 0` 时 `slot` 就是 L0 的图集区域（与 `glyph.atlas_x/...` 一致）。
pub struct MipSelection<'a> {
    /// 实际用到的级：0 = L0，1 = 1/2，2 = 1/4。
    pub level: usize,
    /// 字形信息（含 L0 度量）。
    pub glyph: &'a GlyphInfo,
    /// 该级在图集里的区域。
    pub slot: MipSlot,
}

/// 图集边长。
///
/// 1024² 对 CJK + 4 相位**不够**：1200 字 × 4 相位 = 4800 槽，
/// 而 1024² 只能装 ~2,300 槽（Skyline ~70% 利用率）→ 每帧抽动。
/// 2048² 约 9,000 槽，够用（16 MiB）。
const ATLAS_SIZE: u32 = 2048;
/// 字形之间的间隙（与 legacy 一致）。
const PADDING: u32 = 1;
/// 热集保护：最近 N 个字形不参与驱逐。
const PIN_LAST: usize = 512;

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
    /// `(char, phase, px)` → 字形信息。
    ///
    /// **第三个维度是字号**（px，取整）：同一个字在不同字号下是**不同的位图**，
    /// 必须分开存（否则 18px 的字会拿到 16px 的字形）。
    /// 默认字号用 `px_key()`（= 引擎 `size` 取整），**与加该维度前的行为一致**。
    cache: HashMap<(char, u8, u32), GlyphInfo>,
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
        let face = ttf_parser::Face::parse(&data, 0).map_err(|e| format!("字体解析失败: {e}"))?;
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

    /// 指定图集边长构造（**测试用**：用来制造"装不下"以强制触发驱逐）。
    pub fn with_atlas_size(
        data: Vec<u8>,
        size_px: f32,
        cell: Size,
        atlas_size: u32,
    ) -> Result<Self, String> {
        let mut e = Self::new(data, size_px, cell)?;
        e.atlas_size = atlas_size.max(64);
        e.atlas = vec![0; (e.atlas_size * e.atlas_size * 4) as usize];
        e.packer = SkylinePacker::new(e.atlas_size);
        Ok(e)
    }

    /// 当前图集边长（验收用）。
    pub fn atlas_dim(&self) -> u32 {
        self.atlas_size
    }

    /// 字形占用的格数。
    ///
    /// **与 legacy 同一来源**：`y901_core::Grid::width_of_char`。
    /// 这是格契约，不能按字体 advance 反推——否则排版会与 DirectWrite 路径不一致。
    pub fn cell_span(&self, ch: char) -> u32 {
        y901_core::Grid::width_of_char(ch) as u32
    }

    /// **预取**：把一批字符的 **phase 0** 字形提前光栅化并入图集。
    ///
    /// 用途：滚动前把"即将进入视口的 1~2 行"热好，避免滚动时现场光栅化造成的顿挫。
    /// 为什么只预取 phase 0：亚像素相位依赖真实 pen 位置，预取时还不知道；
    /// 4 相位应由**可见行**在绘制时按需生成（C 方案）。
    ///
    /// 返回：新增的字形数。
    pub fn prefetch(&mut self, chars: &[char]) -> usize {
        let mut added = 0usize;
        for ch in chars {
            if self.cache.contains_key(&(*ch, 0, self.px_key())) {
                continue;
            }
            if self.glyph_phased(*ch, 0).is_some() {
                added += 1;
            }
        }
        added
    }

    /// **引擎基准字号**的整数 key（默认路径用）。
    fn px_key(&self) -> u32 {
        self.size.round().max(1.0) as u32
    }

    /// 取字形（不命中则现场光栅化并入图集）。
    pub fn glyph(&mut self, ch: char) -> Option<&GlyphInfo> {
        self.glyph_phased(ch, 0)
    }

    /// **按字号取字形**（per-text 字号）。
    ///
    /// 输入：`ch`、`px`（逻辑像素，取整）。
    /// 返回：该字号下的字形；不命中则**按该字号现场光栅化**。
    ///
    /// **默认路径（`px` 等于引擎基准字号）直接委托 `glyph()`** ——
    /// 保证与加该能力前**逐像素一致**（同一份缓存、同一条光栅化路径）。
    ///
    /// **字号来源必须与 `cell_metrics_for(px)` 同源**（因为宽度度量用的是同一个 px）。
    pub fn glyph_at_size(&mut self, ch: char, px: f32) -> Option<&GlyphInfo> {
        let k = if px.is_finite() && px > 0.0 {
            px.round().max(1.0) as u32
        } else {
            self.px_key()
        };
        if k == self.px_key() {
            return self.glyph(ch);
        }
        let key = (ch, 0u8, k);
        if !self.cache.contains_key(&key) {
            // 按目标字号光栅化；**不生成 mip**（mip 按基准字号算的级尺寸不适用）。
            let bitmap = rasterize_glyph_phased(&self.data, ch, k as f32, 0)?;
            let info = self.insert_into_atlas(ch, &bitmap)?;
            self.cache.insert(key, info);
        }
        if let Some(g) = self.cache.get_mut(&key) {
            g.last_used = self.frame;
        }
        self.cache.get(&key)
    }

    /// 带亚像素相位取字形（`phase` ∈ 0..=3）。
    ///
    /// 缓存 key 是 `(char, phase)`——同一字的 4 个相位是 4 张不同位图。
    pub fn glyph_phased(&mut self, ch: char, phase: u8) -> Option<&GlyphInfo> {
        let key = (ch, phase & 3, self.px_key());
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
            // 满了：**逐个驱逐直到能装下**。
            //
            // 为什么不能只按"占用率降到 target"循环：
            // Skyline 碎片化后，缓存字形的面积和可能远小于占用率上限（空洞无法利用），
            // 那时 `occupancy() < target` 但 `insert` 仍然失败 → 字形被静默丢弃（实测：256² 下 600 字全丢）。
            let mut attempts = 0;
            while pos.is_none() && attempts < 64 {
                if !self.evict_one() {
                    break; // 全是热集，驱不动了
                }
                attempts += 1;
                pos = self.packer.insert(w + PADDING, h + PADDING);
            }
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
            // 墨迹密度（分母 = 字身框）在这里算一次，渲染期直接读
            ink: (bmp.coverage.iter().filter(|c| **c > 0).count() as f32
                / (self.size * self.size).max(1.0))
            .min(1.0),
            // mip 链在 `insert_mips` 里填充；这里先置空
            mips: [None, None, None],
        })
    }

    /// 把一张覆盖率位图写入图集（RGB = 覆盖率，A = 255）并标脏。
    fn blit_into_atlas(&mut self, bmp: &GlyphBitmap, ax: u32, ay: u32) {
        let (w, h) = (bmp.width.max(1), bmp.height.max(1));
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
    }

    /// 为一个字形生成 L1（1/2）与 L2（1/4）并写入图集。
    ///
    /// 为什么每级**重新光栅化**而不是从 L0 降采样：实测结论（见
    /// `docs/design/text-scaling.md` §4.5）—— 4×4 超采样作为无偏真值时，
    /// 重光栅化的 MAD 为 0.0194，盒式滤波为 0.1404（**5× 更近**）。
    ///
    /// 只用 phase 0：缩小后亚像素差异不可见，存 4 个相位只是浪费图集。
    /// 生成 mip 直到 `max_level`（0 = 仅 L1，1 = L1+L2）。
    fn insert_mips_upto(&mut self, ch: char, info: &mut GlyphInfo, max_level: usize) {
        for level in 0..=max_level.min(1) {
            // 占用率保护：图集接近满时**不生成 L2**（它只服务 s < 0.5 的极端场景）。
            // 为一个很少用到的精度级去挤掉一个真正在用的字形不划算。
            if info.mips.get(level).copied().flatten().is_some() {
                continue; // 该级已有，不重复生成
            }
            if level == 1 && self.packer.stats().occupancy > 0.8 {
                break;
            }
            let size = self.size / (2.0f32.powi(level as i32 + 1));
            if size < 1.0 {
                break;
            }
            let Some(bmp) = rasterize_glyph_phased(&self.data, ch, size, 0) else {
                continue;
            };
            let (w, h) = (bmp.width.max(1), bmp.height.max(1));
            let Some((ax, ay)) = self.packer.insert(w + PADDING, h + PADDING) else {
                // 图集满了：**不驱逐**—— mip 是"有更好，没也能用"的东西，
                // 为它赶走真正需要的字形是不划算的。退回 L0 即可。
                break;
            };
            self.blit_into_atlas(&bmp, ax, ay);
            info.mips[level] = Some(crate::font::MipSlot {
                atlas_x: ax,
                atlas_y: ay,
                width: w,
                height: h,
                bearing_x: bmp.bearing_x as f32,
                bearing_y: bmp.bearing_y as f32,
            });
        }
    }

    /// 按**缩放比**选字形与 mip 级。
    ///
    /// 输入：`ch`、`scale`（变换栈的缩放乘积）。
    /// 输出：[`MipSelection`]；`level` 是**实际用到的**级（不是理论上想用的）。
    /// 副作用：首次对某个字形变小会生成 mip（约 14µs）并产生脏区；后续调用幂等。
    ///
    /// 为什么要把 level **一起返回**而不让渲染器自己算：
    /// 占用率保护可能跳过 L2，此时实际用的是 L1；渲染器若自己重算，
    /// 就会以为"选了 L2"而去读一个不存在的图集坐标。
    pub fn glyph_at_scale(&mut self, ch: char, scale: f32) -> Option<MipSelection<'_>> {
        // 不缩放 / 非法值 / 放大：直接 L0，不生成 mip
        if !(scale.is_finite() && scale > 0.0 && scale < 1.0) {
            self.glyph(ch)?;
            let g = self.cache.get(&(ch, 0u8, self.px_key()))?;
            return Some(MipSelection {
                level: 0,
                glyph: g,
                slot: MipSlot {
                    atlas_x: g.atlas_x,
                    atlas_y: g.atlas_y,
                    width: g.width,
                    height: g.height,
                    bearing_x: g.bearing_x as f32,
                    bearing_y: g.bearing_y as f32,
                },
            });
        }

        // 先取 L0（它是 mip 的源，也提供缓存的 ink）
        self.glyph(ch)?;
        let wanted = {
            let g = self.cache.get(&(ch, 0u8, self.px_key()))?;
            crate::mip::level_for(scale, g.ink, 2)
        };
        // level 0 就是 L0：**不要生成 mip**。否则 0.75 这类"基础级为 0"的调用
        // 会白白付 14µs 并产生脏区，而画面与不生成时完全一致。
        if wanted == 0 {
            let g = self.cache.get(&(ch, 0u8, self.px_key()))?;
            return Some(MipSelection {
                level: 0,
                glyph: g,
                slot: MipSlot {
                    atlas_x: g.atlas_x,
                    atlas_y: g.atlas_y,
                    width: g.width,
                    height: g.height,
                    bearing_x: g.bearing_x as f32,
                    bearing_y: g.bearing_y as f32,
                },
            });
        }
        let _ = self.ensure_mips(ch);

        let g = self.cache.get(&(ch, 0u8, self.px_key()))?;
        // 回退：从 wanted 往下找第一个真正存在的级
        for level in (1..=wanted).rev() {
            if let Some(slot) = g.mip_at(level) {
                return Some(MipSelection {
                    level,
                    glyph: g,
                    slot,
                });
            }
        }
        Some(MipSelection {
            level: 0,
            glyph: g,
            slot: MipSlot {
                atlas_x: g.atlas_x,
                atlas_y: g.atlas_y,
                width: g.width,
                height: g.height,
                bearing_x: g.bearing_x as f32,
                bearing_y: g.bearing_y as f32,
            },
        })
    }

    /// 字形 L0 的**墨迹密度**（墨迹像素 / 字身框面积）。
    ///
    /// 现在是 **O(1)**：值在字形构造时算好并存在 [`GlyphInfo::ink`] 里。
    /// 之前每次调用都遍历整块位图，而渲染每帧都要用它。
    pub fn glyph_ink_ratio(&mut self, ch: char) -> f32 {
        self.glyph(ch).map(|g| g.ink).unwrap_or(0.0)
    }

    /// 按需为已缓存的字形生成 mip 链（幂等：已生成则直接返回）。
    ///
    /// 为什么是**懒生成**而不是安装时生成：
    /// mip 使单字形图集占用从 1× 变成 ~1.6×，直接把
    /// "2048² 装得下 1200 个 CJK × 4 相位"的容量承诺破掉（实测：测试直接取不到字形）。
    /// 而 mip 只在**真的缩小渲染**时才有用，所以把成本推到首次需要它的时候。
    pub fn ensure_mips(&mut self, ch: char) -> bool {
        self.ensure_mips_upto(ch, 2)
    }

    /// 按需生成 mip，**最多到 `max_level` 级**（幂等）。
    ///
    /// 为什么要"有限级"而不只是全部：
    /// L1 的像素面积是 L2 的 4 倍。冷启动时只有 `s ∈ [0.5, 1)`
    /// 需要 L1，把 L2 也一并生成是白付 20% 的时间。
    /// 将来若冷启动超预算，可以改成"首次需要 L2 时才生成 L2"。
    pub fn ensure_mips_upto(&mut self, ch: char, max_level: usize) -> bool {
        let key = (ch, 0u8, self.px_key());
        let Some(info) = self.cache.get(&key) else {
            return false;
        };
        let wanted = max_level.min(2);
        if (0..=wanted).all(|i| info.mips.get(i).copied().flatten().is_some()) {
            return true;
        }
        let mut info = info.clone();
        self.insert_mips_upto(ch, &mut info, wanted);
        self.cache.insert(key, info);
        true
    }

    /// 驱逐**一个**最久未用的字形（保留热集）。
    ///
    /// 返回：`false` = 没有可驱逐的候选（全在热集保护期内）。
    fn evict_one(&mut self) -> bool {
        let cutoff = self.frame.saturating_sub(PIN_LAST as u64);
        let victim = self
            .cache
            .iter()
            .filter(|(_, g)| g.last_used < cutoff)
            .min_by_key(|(_, g)| g.last_used)
            .map(|(c, _)| *c);
        let Some(victim) = victim else {
            return false;
        };
        if let Some(g) = self.cache.remove(&victim) {
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
            // mip 区域同样要清零并标脏，否则阴影会永久留在图集里
            for slot in g.mips.iter().flatten() {
                for y in slot.atlas_y..slot.atlas_y + slot.height {
                    for x in slot.atlas_x..slot.atlas_x + slot.width {
                        let i = ((y as usize) * asz + x as usize) * 4;
                        self.atlas[i..i + 4].fill(0);
                    }
                }
                self.dirty.push(DirtyRect {
                    x: slot.atlas_x,
                    y: slot.atlas_y,
                    width: slot.width,
                    height: slot.height,
                });
            }
            self.evicted_last_frame += 1;
            return true;
        }
        false
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

    /// **仅测试**：把打包器的占用率直接设成某个比例。
    #[cfg(test)]
    pub(crate) fn force_occupancy_for_test(&mut self, ratio: f32) {
        let total = self.packer.size_u64() as f64;
        self.packer
            .set_used_area_for_test((total * ratio as f64) as u64);
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

    fn glyph_at_scale(&mut self, ch: char, scale: f32) -> Option<crate::font::ScaledGlyph> {
        let sel = MyFontEngine::glyph_at_scale(self, ch, scale)?;
        Some(crate::font::ScaledGlyph {
            level: sel.level,
            slot: sel.slot,
            w: sel.glyph.width,
            h: sel.glyph.height,
            advance: sel.glyph.advance,
        })
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

#[cfg(test)]
mod mip_guard_tests {
    use super::*;

    fn engine(atlas: u32) -> MyFontEngine {
        let (cw, ch) = crate::cell_metrics_for(15.0);
        MyFontEngine::with_atlas_size(
            crate::load_font(&crate::default_test_font_path()).expect("font"),
            15.0,
            y901_ui::Size::new(cw, ch),
            atlas,
        )
        .expect("engine")
    }

    // ================= glyph_at_scale（第 2e 步）=================

    fn engine_256() -> MyFontEngine {
        engine(256)
    }

    /// 在 'A'..='Z' 里找一个墨迹密度在某侧的字形。
    ///
    /// 为什么不硬编码字符：墨迹比取决于字体与字号，硬编码会让测试在换字体后失效。
    /// 这里按实测值挑，并在找不到时报错而不是静默跳过。
    fn pick_glyph_with_ink(e: &mut MyFontEngine, below: bool) -> char {
        // 候选集要真的含稀疏字形：大写字母在 15px 下墨迹比普遍 > 0.25（实测），
        // 所以必须把小写与标点也算进来。
        let candidates: Vec<char> = ('A'..='Z')
            .chain('a'..='z')
            .chain('0'..='9')
            .chain(['.', ',', '-', 'l', 'i', 'I', '1'])
            .collect();
        for ch in candidates {
            let ink = e.glyph_ink_ratio(ch);
            if (below && ink < 0.25) || (!below && ink > 0.25) {
                return ch;
            }
        }
        panic!("找不到合适的字形（below={below}），说明墨迹比分布与预期不同");
    }

    #[test]
    fn scale_1_0_returns_l0_without_mips() {
        let mut e = engine_256();
        let sel = e.glyph_at_scale('A', 1.0).expect("A");
        assert_eq!(sel.level, 0, "scale = 1 应选 L0");
        assert_eq!(
            sel.slot.atlas_x, sel.glyph.atlas_x,
            "L0 的 slot 就是字形自身的图集位置"
        );
        assert!(sel.glyph.mips[0].is_none(), "scale = 1 不应生成 mip");
    }

    #[test]
    fn scale_0_75_returns_l0() {
        let mut e = engine_256();
        let sel = e.glyph_at_scale('A', 0.75).expect("A");
        assert_eq!(
            sel.level, 0,
            "0.75 的 LOD 应为 0（floor(log2(1/0.75)) = 0）"
        );
        assert!(sel.glyph.mips[0].is_none(), "L0 时不会生成 mip");
    }

    #[test]
    fn scale_0_5_returns_l1_for_sparse_glyph() {
        let mut e = engine_256();
        let ch = pick_glyph_with_ink(&mut e, true);
        let ink = e.glyph_ink_ratio(ch);
        assert!(ink < 0.25, "sparse glyph ink {ink}");
        let sel = e.glyph_at_scale(ch, 0.5).expect("glyph");
        assert_eq!(sel.level, 1, "稀疏字形在 0.5 下应选 L1");
        assert!(sel.slot.width < sel.glyph.width, "L1 应比 L0 窄");
    }

    #[test]
    fn scale_0_5_returns_l2_for_dense_glyph() {
        let mut e = engine_256();
        let ch = pick_glyph_with_ink(&mut e, false);
        let ink = e.glyph_ink_ratio(ch);
        assert!(ink > 0.25, "dense glyph ink {ink}");
        let sel = e.glyph_at_scale(ch, 0.5).expect("glyph");
        // 密字形：level_for(0.5, >0.25) = 2 → 应取 L2（假如 L2 因占用率被跳过，则退到 L1 并如实报告）
        assert!(sel.level >= 1, "密字形应选更粗的级，实际 {}", sel.level);
        assert!(sel.slot.width < sel.glyph.width, "所选级应比 L0 窄");
    }

    #[test]
    fn scale_0_25_returns_l2() {
        let mut e = engine_256();
        let sel = e.glyph_at_scale('A', 0.25).expect("A");
        assert_eq!(sel.level, 2, "0.25 的 LOD 应为 2");
        assert!(sel.slot.width < sel.glyph.width, "L2 应比 L0 窄");
    }

    #[test]
    fn scale_nan_returns_l0_without_panic() {
        let mut e = engine_256();
        let sel = e.glyph_at_scale('A', f32::NAN).expect("A");
        assert_eq!(sel.level, 0, "NaN 应按不缩放处理");
        assert!(sel.glyph.mips[0].is_none(), "NaN 不应生成 mip");
    }

    #[test]
    fn glyph_after_glyph_at_scale_is_unchanged() {
        let mut e = engine_256();
        let before = e.glyph('A').expect("A").clone();
        let _ = e.glyph_at_scale('A', 0.5);
        let after = e.glyph('A').expect("A").clone();
        assert_eq!(
            (before.width, before.height, before.atlas_x, before.atlas_y),
            (after.width, after.height, after.atlas_x, after.atlas_y),
            "mip 生成不得改动 L0 的尺寸与图集位置"
        );
        assert_eq!(before.advance, after.advance, "advance 不应变");
        assert_eq!(before.ink, after.ink, "ink 不应变");
        assert!(after.mips[0].is_some(), "此时 mip 已生成");
    }

    #[test]
    fn l2_is_skipped_when_the_atlas_is_nearly_full() {
        let mut e = engine(256);
        let _ = e.glyph('A').expect("A");
        e.force_occupancy_for_test(0.85);
        assert!(e.ensure_mips('A'));
        let g = e.glyph('A').expect("A");
        assert!(g.mips[0].is_some(), "占用率高时 L1 仍应生成");
        assert!(
            g.mips[1].is_none(),
            "占用率 > 0.8 时应跳过 L2，实际 {:?}",
            g.mips[1]
        );
    }

    #[test]
    fn l1_and_l2_are_both_generated_at_half_occupancy() {
        let mut e = engine(256);
        let _ = e.glyph('B').expect("B");
        e.force_occupancy_for_test(0.5);
        assert!(e.ensure_mips('B'));
        let g = e.glyph('B').expect("B").clone();
        assert!(g.mips[0].is_some(), "L1 应生成");
        assert!(g.mips[1].is_some(), "占用率 0.5 时 L2 也应生成");
    }
}
