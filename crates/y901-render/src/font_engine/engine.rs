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

use crate::atlas::SkylinePacker;
use crate::font::{DirtyRect, FontEngine, GlyphInfo};
use super::{rasterize_glyph_phased, GlyphBitmap};

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
            if self.cache.contains_key(&(*ch, 0)) {
                continue;
            }
            if self.glyph_phased(*ch, 0).is_some() {
                added += 1;
            }
        }
        added
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
    fn insert_mips(&mut self, ch: char, info: &mut GlyphInfo) {
        for level in 0..2usize {
            // 占用率保护：图集接近满时**不生成 L2**（它只服务 s < 0.5 的极端场景）。
            // 为一个很少用到的精度级去挤掉一个真正在用的字形不划算。
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
            });
        }
    }

    /// 按**缩放比**取字形：`scale < 1` 时选 mip 级，否则就是 L0。
    ///
    /// 输入：`ch`、`scale`（当前变换栈的缩放乘积）。
    /// 输出：字形信息；调用方用 [`GlyphInfo::mip_at`] 按选中的级取图集坐标。
    /// 副作用：**首次对某个字形变小时会生成 mip（约 14µs）并产生脏区**；
    /// 后续调用幂等，无额外开销。
    ///
    /// 为什么 `scale >= 1` 不生成 mip：放大方向没有更精细的源，
    /// 生成也没人用，只会占图集。这也是"s = 1.0 逐像素不变"的实现保证。
    pub fn glyph_at_scale(&mut self, ch: char, scale: f32) -> Option<&GlyphInfo> {
        if !(scale.is_finite() && scale > 0.0 && scale < 1.0) {
            return self.glyph(ch);
        }
        // 先保证 L0 存在（它是 mip 的源）
        self.glyph(ch)?;
        let _ = self.ensure_mips(ch);
        self.glyph(ch)
    }

    /// 字形 L0 的**墨迹密度**（墨迹像素 / 字身框面积），供 LOD 选择使用。
    ///
    /// 为什么除以 `size²` 而不是字形自己的包围盒：
    /// 包围盒是**紧贴墨迹**的，于是一个句点的覆盖率也能高达 0.5——
    /// 这会让"密字多降一级"的规则**几乎恒为真**（实测：全部 Latin 都 > 0.25）。
    /// 用字身框作分母才能区分"同一格子里墨多还是墨少"，这正是设计意图。
    pub fn glyph_ink_ratio(&mut self, ch: char) -> f32 {
        let Some(g) = self.glyph(ch).cloned() else {
            return 0.0;
        };
        if g.width == 0 || g.height == 0 {
            return 0.0;
        }
        let asz = self.atlas_size as usize;
        let mut inked = 0usize;
        for y in 0..g.height as usize {
            for x in 0..g.width as usize {
                let i = ((g.atlas_y as usize + y) * asz + (g.atlas_x as usize + x)) * 4;
                if self.atlas.get(i).copied().unwrap_or(0) > 0 {
                    inked += 1;
                }
            }
        }
        let em = (self.size * self.size).max(1.0);
        (inked as f32 / em).min(1.0)
    }

    /// 按需为已缓存的字形生成 mip 链（幂等：已生成则直接返回）。
    ///
    /// 为什么是**懒生成**而不是安装时生成：
    /// mip 使单字形图集占用从 1× 变成 ~1.6×，直接把
    /// "2048² 装得下 1200 个 CJK × 4 相位"的容量承诺破掉（实测：测试直接取不到字形）。
    /// 而 mip 只在**真的缩小渲染**时才有用，所以把成本推到首次需要它的时候。
    pub fn ensure_mips(&mut self, ch: char) -> bool {
        let key = (ch, 0u8);
        let Some(info) = self.cache.get(&key) else {
            return false;
        };
        if info.mips.iter().all(|m| m.is_some()) {
            return true;
        }
        let mut info = info.clone();
        self.insert_mips(ch, &mut info);
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
        self.packer.set_used_area_for_test((total * ratio as f64) as u64);
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
        let g = e.glyph_at_scale('A', 1.0).expect("A").clone();
        assert!(g.mips[0].is_none(), "scale = 1 不应生成 mip");
        assert!(g.mip_at(1).is_none(), "回退也应为 None（即用 L0）");
    }

    #[test]
    fn scale_0_75_returns_l0() {
        let mut e = engine_256();
        let g = e.glyph_at_scale('A', 0.75).expect("A").clone();
        assert_eq!(
            crate::mip::level_for(0.75, 0.0, 2),
            0,
            "0.75 的 LOD 应为 0（floor(log2(1/0.75)) = 0）"
        );
        assert!(g.mip_at(0).is_none(), "level 0 = L0");
    }

    #[test]
    fn scale_0_5_returns_l1_for_sparse_glyph() {
        let mut e = engine_256();
        let ch = pick_glyph_with_ink(&mut e, true);
        let ink = e.glyph_ink_ratio(ch);
        assert!(ink < 0.25, "sparse glyph ink {ink}");
        let g = e.glyph_at_scale(ch, 0.5).expect("glyph").clone();
        let slot = g.mip_at(1).expect("L1 应存在");
        assert!(slot.width < g.width, "L1 应比 L0 窄");
    }

    #[test]
    fn scale_0_5_returns_l2_for_dense_glyph() {
        let mut e = engine_256();
        let ch = pick_glyph_with_ink(&mut e, false);
        let ink = e.glyph_ink_ratio(ch);
        assert!(ink > 0.25, "dense glyph ink {ink}");
        let g = e.glyph_at_scale(ch, 0.5).expect("glyph").clone();
        // 密字形：level_for(0.5, >0.25) = 2 → 取 L2（若 L2 未生成则回退 L1，也不应为 None）
        let slot = g.mip_at(2).expect("L2（或回退的 L1）应存在");
        assert!(slot.width < g.width, "L2 应比 L0 窄");
    }

    #[test]
    fn scale_0_25_returns_l2() {
        let mut e = engine_256();
        let g = e.glyph_at_scale('A', 0.25).expect("A").clone();
        assert_eq!(crate::mip::level_for(0.25, 0.0, 2), 2, "0.25 的 LOD 应为 2");
        assert!(g.mip_at(2).is_some(), "L2 应存在");
    }

    #[test]
    fn scale_nan_returns_l0_without_panic() {
        let mut e = engine_256();
        let g = e.glyph_at_scale('A', f32::NAN).expect("A").clone();
        assert!(g.mips[0].is_none(), "NaN 应按不缩放处理，不生成 mip");
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
        assert!(after.mips[0].is_some(), "此时 mip 已生成");
    }

    #[test]
    fn l2_is_skipped_when_the_atlas_is_nearly_full() {
        let mut e = engine(256);
        let _ = e.glyph('A').expect("A");
        e.force_occupancy_for_test(0.85);
        assert!(e.ensure_mips('A'));
        let g = e.glyph('A').expect("A").clone();
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
