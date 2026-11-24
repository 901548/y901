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
use crate::font_engine::rasterize_gid_phased;
use crate::text::GlyphKey;

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
    /// **回退字体**（face 1..N 的字节）。
    ///
    /// 为什么要它们：主字体缺阿拉伯/印度语系字形时，`GlyphKey.face` 指向这里的某一项，
    /// 由该字体出图；图集键里已经带了 face 维度，所以不会与主字体互相覆盖。
    fallbacks: Vec<Vec<u8>>,
    /// 逻辑字号（像素）。
    size: f32,
    /// 格尺寸（与 legacy 一致）。
    cell: Size,
    /// 基线高度（从字体 `ascender` 算）。
    ascent: f32,
    /// **基线下方的下伸高度**（从字体 `descender` 算，正数）。
    ///
    /// 与 `ascent` 一起决定"一行墨迹到底多深"：`ascent + descent`。
    /// 它通常**大于** `cell.height`（1em 行框），所以下伸字/圆字母会越过行框 1~2px。
    descent: f32,
    /// 图集：RGBA。
    atlas: Vec<u8>,
    atlas_size: u32,
    packer: SkylinePacker,
    /// 字形缓存：`(GlyphKey, phase, px)` → 字形信息。
    ///
    /// **键用 [`GlyphKey`] 而不是 `char`**：shaping 之后同一字符在不同上下文会对应
    /// 不同字形（连字、阿拉伯连写形），用 `char` 作键会让它们共用同一张位图。
    /// cell 路径用 `GlyphKey::from_char` 生成键，行为与以前完全一致。
    ///
    /// **第三个维度是字号**（px，取整）：同一个字在不同字号下是**不同的位图**，
    /// 必须分开存（否则 18px 的字会拿到 16px 的字形）。
    /// 默认字号用 `px_key()`（= 引擎 `size` 取整），**与加该维度前的行为一致**。
    cache: HashMap<(GlyphKey, u8, u32), GlyphInfo>,
    /// `char → glyph id` 的记忆表。
    ///
    /// 为什么要它：把字符键统一成 **glyph 键**之后，cell 路径与 shaping 路径才能
    /// 命中同一条缓存（否则同一个字形会被光栅化两次、占两块图集区域）。
    /// 解析 cmap 要过一遍 `ttf_parser::Face`，所以按字符记忆，避免每次取字形都解析。
    gid_cache: HashMap<char, Option<u16>>,
    /// 当前帧号（LRU 用）。
    frame: u64,
    /// 待上传的脏区。
    dirty: Vec<DirtyRect>,
    /// 图集是否需要整张重建。
    reset_pending: bool,
    /// 上一帧驱逐了多少个字形（验收用）。
    evicted_last_frame: usize,
}

/// **字体集（TTC）相关的提示文本**（单一来源，便于测试断言）。
///
/// 为何只在「解析已经失败」时用它：`ttf_parser` **能**直接读 TTC 的第 0 个 face，
/// 所以不能预先拒绝 `ttcf`（实测：hash-gate 使用的字体就能崩开）。
/// 但一旦它真失败，原始报错像“文件损坏”，而实际原因往往是多 face 容器。
pub const TTC_HINT: &str =
    "（检测到 ttcf 文件头：这是 TTC 字体集容器）请导出单个 TTF/OTF 字重后重试。";

/// **是否为字体集（TTC）**：看前 4 字节是否为 `ttcf`。
pub fn is_ttc(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"ttcf"
}

impl MyFontEngine {
    /// 创建引擎。
    ///
    /// 输入：`data`（TTF/OTF 字节，必须常驻）、`size_px`、`cell`。
    /// 返回：`Err` = 字体解析失败。副作用：无。
    pub fn new(data: Vec<u8>, size_px: f32, cell: Size) -> Result<Self, String> {
        // **TTC 提示（不预拒）**：`ttf_parser` 本身支持 TTC（取 face 0），
        // 预先拒绝会把可用的集合字体一起打死（实测：层哈希会因此变掉）。
        // 只有解析**已经失败**时才补上可执行提示。
        let face = ttf_parser::Face::parse(&data, 0).map_err(|e| {
            if is_ttc(&data) {
                format!("字体解析失败（{e}）{TTC_HINT}")
            } else {
                format!("字体解析失败：{e}")
            }
        })?;
        let units = face.units_per_em() as f32;
        let ascent = face.ascender() as f32 * size_px / units.max(1.0);
        // 下伸高度：**优先 OS/2 的 sTypoDescender**，它更贴近真实墨迹；
        // `hhea.descender` 在 CJK 字体上常常包含大量下部预留（本机实测 0.47em），
        // 直接拿来扩张脏区会让每次重绘多扫 5px/行。
        // 两者在 OpenType 里都是负值（基线以下），取反得到正数。
        let typo_desc = face.typographic_descender().map(|d| d as f32);
        let desc_units = typo_desc.unwrap_or_else(|| face.descender() as f32);
        let descent = (-desc_units) * size_px / units.max(1.0);
        Ok(Self {
            data,
            fallbacks: Vec::new(),
            size: size_px,
            cell,
            ascent,
            descent,
            atlas: vec![0; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            atlas_size: ATLAS_SIZE,
            packer: SkylinePacker::new(ATLAS_SIZE),
            cache: HashMap::new(),
            gid_cache: HashMap::new(),
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
            if let Some(gk) = self.key_of(*ch) {
                if self.cache.contains_key(&(gk, 0, self.px_key())) {
                    continue;
                }
            }
            if self.glyph_phased(*ch, 0).is_some() {
                added += 1;
            }
        }
        added
    }

    /// 解析字符的字形 id 并记忆（仅主字体；缺字时走替身表，仍无则记 `None`）。
    ///
    /// 保留它是为了记住"主字体有没有这个字"，`face_for` 用它做快速判定。
    #[allow(dead_code)]
    fn gid_of(&mut self, ch: char) -> Option<u16> {
        if let Some(hit) = self.gid_cache.get(&ch) {
            return *hit;
        }
        let gid = self.gid_of_in(0, ch);
        self.gid_cache.insert(ch, gid);
        gid
    }

    /// **指定字体面**的字形 id（face 0 = 主字体，1.. = 回退链）。
    ///
    /// 与 [`Self::gid_of`] 的差别：不做记忆（回退链查询量小，记忆表的键还要加 face，
    /// 收益不如保持简单）。
    fn gid_of_in(&self, face: u16, ch: char) -> Option<u16> {
        let data = self.face_data(face)?;
        crate::font_engine::glyph_id_of(data, ch)
    }

    /// 取某个面的字体字节（越界返回 `None`）。
    fn face_data(&self, face: u16) -> Option<&[u8]> {
        if face == 0 {
            return Some(&self.data);
        }
        self.fallbacks.get(face as usize - 1).map(Vec::as_slice)
    }

    /// **覆盖 `ch` 的第一个字体面**（0 = 主字体，1.. = 回退链）。
    ///
    /// 返回 `None` = 整条链都没有这个字形（调用方应画 .notdef 或跳过）。
    pub fn face_for(&self, ch: char) -> Option<u16> {
        if self.gid_of_in(0, ch).is_some() {
            return Some(0);
        }
        (1..=self.fallbacks.len())
            .map(|i| i as u16)
            .find(|f| self.gid_of_in(*f, ch).is_some())
    }

    /// 回退链长度（不含主字体）。
    pub fn fallback_count(&self) -> usize {
        self.fallbacks.len()
    }

    /// 追加一个**回退字体**（face 编号 = 追加顺序 + 1）。
    ///
    /// 为什么按字符覆盖判断而不是"整段用一个面"：真实文档一句话里可能同时有
    /// 拉丁、阿拉伯、CJK，必须逐字符决定谁出图 —— 这正是 `GlyphKey.face` 的用途。
    ///
    /// 解析失败的字节会被忽略（返回 `false`），不影响已有链。
    pub fn push_fallback(&mut self, data: Vec<u8>) -> bool {
        if ttf_parser::Face::parse(&data, 0).is_err() {
            return false;
        }
        self.fallbacks.push(data);
        // 新面可能覆盖旧缺字：清掉记忆表，让查询重新判定。
        self.gid_cache.clear();
        // **图集缓存也要整体作废**：字形缓存的键里带 `face`，而"面号未知时退回主字体"
        // 那条路径会把**主字体的位图**存进 `face = N` 的条目里。一旦 N 号面真的注册进来，
        // 同一个键就会永久命中那张错位图（`cache` 没人清）。清空图集最省事也最安全：
        // 少画一帧的字形，换正确的字形来源。
        self.clear_atlas();
        true
    }

    /// 字符 → 缓存/图集键（**沿回退链**解析到第一个有该字形的面；都没有返回 `None`）。
    fn key_of(&mut self, ch: char) -> Option<GlyphKey> {
        let face = self.face_for(ch)?;
        let gid = self.gid_of_in(face, ch)?;
        Some(GlyphKey::new(face, u32::from(gid)))
    }

    /// 只读版：拿字符的缓存键（`&self`，用于 `glyph_at_scale` 等只读路径）。
    ///
    /// 与 [`Self::key_of`] 的区别：不写记忆表，因此缺字时**每次都重新解析**；
    /// 只读路径的调用频次低于取字形，换取 `&self` 可用性。
    fn cache_key_of(&self, ch: char) -> Option<GlyphKey> {
        let face = self.face_for(ch)?;
        let gid = self.gid_of_in(face, ch)?;
        return Some(GlyphKey::new(face, u32::from(gid)));
        #[allow(unreachable_code)]
        crate::font_engine::glyph_id_of(&self.data, ch).map(|gid| GlyphKey::new(0, u32::from(gid)))
    }

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
        let gk = self.key_of(ch)?;
        let key = (gk, 0u8, k);
        if !self.cache.contains_key(&key) {
            // 按目标字号光栅化；**不生成 mip**（mip 按基准字号算的级尺寸不适用）。
            let gid = u16::try_from(gk.glyph).ok()?;
            // **借用而不是 `.to_vec()`**：只为绕开"`&self.data` 与 `&mut self` 冲突"
            // 就克隆整份字体，等于每次冷缓存字形都 memcpy 1.4~10 MB
            //（三种字号 × 60 个字 ≈ 250 MB 拷贝，才画出第一屏）。作用域即可解决：
            // `rasterize_gid_phased` 返回**自有**位图，借用在此语句后即结束。
            let bitmap = {
                let face_data = self.face_data(gk.face).unwrap_or(&self.data);
                rasterize_gid_phased(face_data, gid, k as f32, 0)?
            };
            let info = self.insert_into_atlas(gk, &bitmap)?;
            self.cache.insert(key, info);
        }
        if let Some(g) = self.cache.get_mut(&key) {
            g.last_used = self.frame;
        }
        self.cache.get(&key)
    }

    /// 带亚像素相位取字形（`phase` ∈ 0..=3）。
    ///
    /// 缓存 key 是 `(GlyphKey, phase, px)`——同一字的 4 个相位是 4 张不同位图。
    /// **键里的 glyph 来自 cmap 解析**（带记忆表），因此与 [`Self::glyph_keyed`]
    /// 走 shaping 时命中同一条缓存：同一个字形不会被光栅化两次。
    pub fn glyph_phased(&mut self, ch: char, phase: u8) -> Option<&GlyphInfo> {
        let gk = self.key_of(ch)?;
        let key = (gk, phase & 3, self.px_key());
        if !self.cache.contains_key(&key) {
            let gid = u16::try_from(gk.glyph).ok()?;
            let bitmap = {
                let face_data = self.face_data(gk.face).unwrap_or(&self.data);
                rasterize_gid_phased(face_data, gid, self.size, phase)?
            };
            let info = self.insert_into_atlas(gk, &bitmap)?;
            self.cache.insert(key, info);
        }
        if let Some(g) = self.cache.get_mut(&key) {
            g.last_used = self.frame;
        }
        self.cache.get(&key)
    }

    /// **按字形键取字形**（shaping 路径）。
    ///
    /// 与 [`Self::glyph_phased`] 的区别：`key.glyph` 直接当作 glyph id 光栅化，
    /// **不经 cmap**。连字（`fi` → 单字形）和阿拉伯连写形只有这样才画得对——
    /// 拿字符再查一次 cmap 会退回"每字符一个字形"。
    ///
    /// `key.face` 目前只有 0（单字体）；将来的回退链会用到它，键里已经带了。
    pub fn glyph_keyed(&mut self, key: GlyphKey, phase: u8) -> Option<&GlyphInfo> {
        self.glyph_keyed_at(key, phase, self.size)
    }

    /// 同 [`Self::glyph_keyed`]，但**显式指定像素字号**。
    ///
    /// 为什么需要：shaping 路径按 `px_final = 字号 × DPI × 缩放` 算推进与偏移，
    /// 若位图仍按引擎基准字号取，字会"按 30px 排版、用 15px 的图"——
    /// 字符间距翻倍、字形只有一半大（而 cell 路径一直是对的）。
    pub fn glyph_keyed_at(&mut self, key: GlyphKey, phase: u8, px: f32) -> Option<&GlyphInfo> {
        let px = px.max(1.0);
        let cache_key = (key, phase & 3, px.round() as u32);
        if !self.cache.contains_key(&cache_key) {
            let bitmap = rasterize_gid_phased(
                // 面号未知时退回主字体（shaping 可能先于 `push_fallback` 给出面号）。
                self.face_data(key.face).unwrap_or(&self.data),
                // 越界 glyph id 返回 `None`（走调用方的"缺字形"处理），
                // **不能**退化成 0：那会把 `.notdef` 方框按错误键缓存一辈子。
                u16::try_from(key.glyph).ok()?,
                px,
                phase,
            )?;
            let info = self.insert_into_atlas(key, &bitmap)?;
            // 面号未知时这份位图来自主字体，**看起来**会污染缓存；由
            // `push_fallback` 里的 `clear_atlas()` 整体作废来兜住 ——
            // 这样既保持了"先 shaping 后注册"的可用性，也不会留下错位缓存。
            self.cache.insert(cache_key, info);
        }
        if let Some(g) = self.cache.get_mut(&cache_key) {
            g.last_used = self.frame;
        }
        self.cache.get(&cache_key)
    }

    /// 把一张覆盖度位图写进图集，返回字形信息。
    fn insert_into_atlas(&mut self, _key: GlyphKey, bmp: &GlyphBitmap) -> Option<GlyphInfo> {
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
                    break; // 驱不动了
                }
                attempts += 1;
                pos = self.packer.insert(w + PADDING, h + PADDING);
            }
        }
        if pos.is_none() {
            // **驱逐救不了就重打包**：`SkylinePacker` 的空洞不会因为"删掉字形"而消失，
            // 碎片化后无论驱逐多少次 `insert` 都会失败 —— 旧实现就在这一步
            // `pos?` 直接返回 `None`，于是文本静默消失，而且每失败一次还顺手删掉
            // 几十个刚缓存好的字形（抖动）。
            self.compact_atlas();
            pos = self.packer.insert(w + PADDING, h + PADDING);
            if pos.is_none() {
                // 重打包后仍装不下 ⇒ 图集确实太小：再驱逐一轮（此时驱逐有效，
                // 因为布局是紧凑的），最后一搏。
                let mut attempts = 0;
                while pos.is_none() && attempts < 64 {
                    if !self.evict_one() {
                        break;
                    }
                    attempts += 1;
                    pos = self.packer.insert(w + PADDING, h + PADDING);
                }
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
    /// 输出：`MipSelection`；`level` 是**实际用到的**级（不是理论上想用的）。
    /// 副作用：首次对某个字形变小会生成 mip（约 14µs）并产生脏区；后续调用幂等。
    ///
    /// 为什么要把 level **一起返回**而不让渲染器自己算：
    /// 占用率保护可能跳过 L2，此时实际用的是 L1；渲染器若自己重算，
    /// 就会以为"选了 L2"而去读一个不存在的图集坐标。
    pub fn glyph_at_scale(&mut self, ch: char, scale: f32) -> Option<MipSelection<'_>> {
        // 不缩放 / 非法值 / 放大：直接 L0，不生成 mip
        if !(scale.is_finite() && scale > 0.0 && scale < 1.0) {
            self.glyph(ch)?;
            let gk = self.cache_key_of(ch)?;
            let g = self.cache.get(&(gk, 0u8, self.px_key()))?;
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
            let gk = self.cache_key_of(ch)?;
            let g = self.cache.get(&(gk, 0u8, self.px_key()))?;
            crate::mip::level_for(scale, g.ink, 2)
        };
        // level 0 就是 L0：**不要生成 mip**。否则 0.75 这类"基础级为 0"的调用
        // 会白白付 14µs 并产生脏区，而画面与不生成时完全一致。
        if wanted == 0 {
            let gk = self.cache_key_of(ch)?;
            let g = self.cache.get(&(gk, 0u8, self.px_key()))?;
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

        let gk = self.cache_key_of(ch)?;
        let g = self.cache.get(&(gk, 0u8, self.px_key()))?;
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
        let Some(gk) = self.key_of(ch) else {
            return false;
        };
        let key = (gk, 0u8, self.px_key());
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

    /// **重新打包图集**：把仍缓存着的字形搬进一张紧凑的新图集。
    ///
    /// 为什么必须有：`SkylinePacker` 不回收空洞。驱逐只是把像素清零、
    /// 把条目从缓存里删掉，**打包器的 skyline 一点没动** ——
    /// 于是"删了字形却还是插不进新字形"，重试 64 次全部失败，
    /// 最后 `insert_into_atlas` 返回 `None`：文本静默消失。
    ///
    /// 做法：新建一张全 0 的图集与全新打包器，按面积从大到小重放存活字形
    /// （大块先放能把边角留给小块），逐个把像素块从旧图集抄过去并更新坐标；
    /// 放不下的条目不进新缓存（它们本来也装不下）。
    ///
    /// 代价：一次 `O(图集大小)` 的内存与拷贝 + 一次整张上传，但只在"驱逐无效"时才发生。
    /// mip 槽在移动后坐标失效，这里直接丢弃 —— 渲染侧会回落到 L0，正确性不受影响，
    /// 需要时 `ensure_mips` 会重建。
    fn compact_atlas(&mut self) {
        let asz = self.atlas_size as usize;
        let mut order: Vec<(GlyphKey, u8, u32)> = self.cache.keys().copied().collect();
        // 大块优先：Skyline 下这是最有效的降碎片顺序
        order.sort_by_key(|k| {
            let g = &self.cache[k];
            std::cmp::Reverse((g.width as u64) * (g.height as u64))
        });
        let mut packer = SkylinePacker::new(self.atlas_size);
        let mut atlas = vec![0u8; self.atlas.len()];
        let mut moved: Vec<((GlyphKey, u8, u32), u32, u32)> = Vec::new();
        let mut dropped: Vec<(GlyphKey, u8, u32)> = Vec::new();
        for key in order {
            let (w, h, ox, oy) = {
                let g = &self.cache[&key];
                (g.width, g.height, g.atlas_x, g.atlas_y)
            };
            match packer.insert(w + PADDING, h + PADDING) {
                Some((nx, ny)) => {
                    for y in 0..h as usize {
                        let src = ((oy as usize + y) * asz + ox as usize) * 4;
                        let dst = ((ny as usize + y) * asz + nx as usize) * 4;
                        let len = (w as usize) * 4;
                        // 越界保护：图集被换过尺寸/损坏时宁可丢字形也不写越界
                        if src + len <= self.atlas.len() && dst + len <= atlas.len() {
                            atlas[dst..dst + len].copy_from_slice(&self.atlas[src..src + len]);
                        }
                    }
                    moved.push((key, nx, ny));
                }
                None => dropped.push(key),
            }
        }
        for key in &dropped {
            self.cache.remove(key);
        }
        for (key, nx, ny) in moved {
            if let Some(g) = self.cache.get_mut(&key) {
                g.atlas_x = nx;
                g.atlas_y = ny;
                g.mips = [None, None, None];
            }
        }
        self.packer = packer;
        self.atlas = atlas;
        // 整张重建：一次全量上传，比拼几十个互相覆盖的脏区更省事也更安全
        self.dirty.clear();
        self.dirty.push(DirtyRect {
            x: 0,
            y: 0,
            width: self.atlas_size,
            height: self.atlas_size,
        });
    }

    /// 驱逐**一个**最久未用的字形（保留热集）。
    ///
    /// 返回：`false` = 没有可驱逐的候选（全在热集保护期内）。
    fn evict_one(&mut self) -> bool {
        let cutoff = self.frame.saturating_sub(PIN_LAST as u64);
        // **热集保护不能变成"永不驱逐"**：`frame` 从 0 开始时 `cutoff == 0`，
        // 而 `last_used >= 0` 恒成立，于是前 512 帧里一个都驱不掉 ——
        // 图集一满，字形就永久丢失（会话刚开始、字号动画把图集填满就是这种场景）。
        // 所以先按"冷集"挑，挑不到就退化为"全表最冷"，至少能腾出空间。
        let victim = self
            .cache
            .iter()
            .filter(|(_, g)| g.last_used < cutoff)
            .min_by_key(|(_, g)| g.last_used)
            .map(|(c, _)| *c)
            .or_else(|| {
                self.cache
                    .iter()
                    .min_by_key(|(_, g)| g.last_used)
                    .map(|(c, _)| *c)
            });
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
        // **必须用 `self.atlas_size`**：写死 `ATLAS_SIZE`(2048) 会让 packer 把后续字形
        // 分配到"像素缓冲区之外"（`with_atlas_size` 可以把缓冲调小），
        // 紧接着的写入就是越界 panic 或写进下一行造成花屏。
        self.packer = SkylinePacker::new(self.atlas_size);
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

    /// 指定面的字体原始字节。
    pub fn font_data_for(&self, face: u16) -> Option<&[u8]> {
        self.face_data(face)
    }

    /// **字体原始字节**（shaping 需要它重新解析 OpenType 表）。
    pub fn font_data(&self) -> Option<&[u8]> {
        Some(&self.data)
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
    /// **真实字宽**（hmtx）：比例字体下 `'i'` 与 `'m'` 宽度不同。
    ///
    /// 只有在字体有该字形时才用真实值，否则回退到格宽；
    /// 不同字号按 `px / units_per_em` 线性换算（与光栅化同一比例）。
    /// 本引擎构造时的字号（与光栅化基准一致）。
    fn size_px(&self) -> f32 {
        self.size
    }

    /// **真实字宽**（hmtx）：比例字体下 `'i'` 与 `'m'` 宽度不同。
    ///
    /// 只有在字体有该字形时才用真实值，否则回退到格宽。
    fn advance(&self, ch: char, px: f32) -> f32 {
        match crate::font_engine::glyph_advance_px(&self.data, ch, px.max(1.0)) {
            Some(w) if w > 0.0 => w,
            _ => {
                let cell_w = crate::font::cell_metrics_for(px.max(1.0)).0;
                self.cell_span(ch) as f32 * cell_w
            }
        }
    }

    fn cell_span(&self, ch: char) -> u32 {
        MyFontEngine::cell_span(self, ch)
    }

    /// **按字号取字形**（per-text 字号）：必须转发到本引擎的同名方法。
    ///
    /// 不转发会静默采用 trait 的默认实现 `self.glyph(ch)`——那**永远返回基准字号**的
    /// 位图。于是 `TextStyle::size = 12` 的标签按 12px 排版却画 16px 字形
    /// （字挤在一起），`size = 24` 的标题按 24px 排版却画 16px 字形
    /// （一行里字高低大小不一）。这正是“文字发紧、高低不齐”的直接原因之一。
    fn glyph_at_size(&mut self, ch: char, px: f32) -> Option<&GlyphInfo> {
        MyFontEngine::glyph_at_size(self, ch, px)
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

    fn descent(&self) -> f32 {
        self.descent
    }
    fn cell_width(&self) -> f32 {
        self.cell.w
    }
    fn cell_height(&self) -> f32 {
        self.cell.h
    }

    // ---- shaping / 回退链：**必须逐个转发** ----
    //
    // 这组方法以前只作为**固有方法**存在于 `MyFontEngine` 上，trait 侧没有覆盖 ——
    // 而 `SoftBackend` 持有的是 `Box<dyn FontEngine>`，所以调用落到 trait 的默认实现：
    //   * `font_data()` 默认返回 `None` → `font_data_for(0)` 也返回 `None`
    //     → `draw_text_shaped` 一检测到"没有字体字节"就整体退回 cell 路径。
    //   也就是说 **shaping 路径在多行文本之外从未真正跑过**（阿拉伯/希伯来/组合记号
    //   全部静默走 cell 路径），所有 shaping 相关的修复都成了死代码。
    fn font_data(&self) -> Option<&[u8]> {
        Some(&self.data)
    }

    fn font_data_for(&self, face: u16) -> Option<&[u8]> {
        self.face_data(face)
    }

    fn face_for(&self, ch: char) -> Option<u16> {
        MyFontEngine::face_for(self, ch)
    }

    /// **shaped 推进宽度**：与 `draw_text_shaped` 同源（同样切面、同样 shaping、
    /// 同样按 `run.width` 求和），保证"量出来的宽度 == 画出来的宽度"。
    #[cfg(feature = "shaping")]
    fn shaped_width(&self, text: &str, px: f32) -> Option<f32> {
        let px = px.max(1.0);
        let mut total = 0.0f32;
        // 与绘制路径一致：先按 Bidi 视觉 run 切、再在 run 内按字体面切。
        for (run_range, _rtl) in crate::text::visual_runs_of(text) {
            let slice = &text[run_range.clone()];
            for seg in crate::text::face_segments(slice, |ch| self.face_for(ch)) {
                let data = self.font_data_for(seg.face)?;
                let run =
                    crate::text::shape_run(data, &slice[seg.range.clone()], px, seg.face).ok()?;
                total += run.width;
            }
        }
        Some(total)
    }

    fn glyph_keyed(
        &mut self,
        key: crate::text::GlyphKey,
        phase: u8,
    ) -> Option<&crate::font::GlyphInfo> {
        MyFontEngine::glyph_keyed(self, key, phase)
    }

    fn glyph_keyed_at(
        &mut self,
        key: crate::text::GlyphKey,
        phase: u8,
        px: f32,
    ) -> Option<&crate::font::GlyphInfo> {
        MyFontEngine::glyph_keyed_at(self, key, phase, px)
    }
}

#[cfg(test)]
mod glyph_key_tests {
    //! 图集键换成 [`GlyphKey`] 之后的两条硬约束：
    //! 1. cell 路径（按字符）必须与改造前逐像素一致；
    //! 2. shaping 路径（按 glyph id）必须能画出「同一字符的另一个字形」，
    //!    且不与按字符的条目共用缓存槽位。

    use super::*;

    fn engine() -> MyFontEngine {
        let (cw, ch) = crate::cell_metrics_for(16.0);
        MyFontEngine::new(
            crate::load_font(&crate::default_test_font_path()).expect("font"),
            16.0,
            y901_ui::Size::new(cw, ch),
        )
        .expect("engine")
    }

    fn font() -> Vec<u8> {
        crate::load_font(&crate::default_test_font_path()).expect("font")
    }

    #[test]
    fn char_path_and_glyph_id_path_produce_identical_bitmaps() {
        let data = font();
        let gid = super::super::glyph_id_of(&data, 'A').expect("A 应有字形");
        let by_char = super::super::rasterize_glyph_phased(&data, 'A', 16.0, 0).expect("char");
        let by_gid = super::super::rasterize_gid_phased(&data, gid, 16.0, 0).expect("gid");
        assert_eq!(by_char.width, by_gid.width);
        assert_eq!(by_char.height, by_gid.height);
        assert_eq!(by_char.bearing_x, by_gid.bearing_x);
        assert_eq!(by_char.bearing_y, by_gid.bearing_y);
        assert_eq!(by_char.coverage, by_gid.coverage, "两条路径必须逐像素一致");
    }

    #[test]
    fn glyph_keyed_rasterizes_by_glyph_id_not_by_cmap() {
        let mut e = engine();
        let data = font();
        let gid_a = super::super::glyph_id_of(&data, 'A').expect("A");
        let expected = e.glyph_phased('A', 0).expect("glyph(A)").clone();
        let got = e
            .glyph_keyed(GlyphKey::new(0, gid_a as u32), 0)
            .expect("keyed");
        assert_eq!((got.width, got.height), (expected.width, expected.height));
        assert_eq!(
            (got.atlas_x, got.atlas_y),
            (expected.atlas_x, expected.atlas_y),
            "同一字形应命中同一槽位（键等价）"
        );
    }

    #[test]
    fn different_glyph_ids_do_not_share_a_slot() {
        let mut e = engine();
        let data = font();
        let a = super::super::glyph_id_of(&data, 'A').expect("A") as u32;
        let b = super::super::glyph_id_of(&data, 'B').expect("B") as u32;
        let pa = {
            let g = e.glyph_keyed(GlyphKey::new(0, a), 0).expect("A");
            (g.atlas_x, g.atlas_y)
        };
        let pb = {
            let g = e.glyph_keyed(GlyphKey::new(0, b), 0).expect("B");
            (g.atlas_x, g.atlas_y)
        };
        assert_ne!(pa, pb, "不同 glyph id 不能共用槽位");
    }

    #[test]
    fn platform_fallbacks_actually_prepare_faces() {
        // 平台候选表里至少有一个字体（Windows: Segoe UI 等；Linux: DejaVu）。
        let paths = crate::font_engine::platform_fallback_paths();
        assert!(!paths.is_empty(), "平台默认回退字体链不应为空");
        let mut e = engine();
        let mut added = 0;
        for path in &paths {
            if let Ok(bytes) = crate::load_font(path) {
                if e.push_fallback(bytes) {
                    added += 1;
                }
            }
        }
        assert_eq!(
            e.fallback_count(),
            added,
            "fallback_count 应与实际注册数一致"
        );
        // 任一平台至少能注册一个回退面（否则说明候选表与真实环境脱节）。
        assert!(added >= 1, "候选表里的字体一个都没注册上，实得 {added}");
    }
    #[test]
    fn face_dimension_separates_the_same_glyph_id() {
        let mut e = engine();
        let data = font();
        let gid = super::super::glyph_id_of(&data, 'A').expect("A") as u32;
        // 注册一个真实回退面：face 1 = 同一份字体字节（内容相同也算**不同面**）。
        assert!(e.push_fallback(data.clone()), "回退面应可注册");
        assert_eq!(e.fallback_count(), 1);
        let p0 = {
            let g = e.glyph_keyed(GlyphKey::new(0, gid), 0).expect("face 0");
            (g.atlas_x, g.atlas_y)
        };
        let p1 = {
            let g = e.glyph_keyed(GlyphKey::new(1, gid), 0).expect("face 1");
            (g.atlas_x, g.atlas_y)
        };
        assert_ne!(p0, p1, "不同 face 的同号字形必须分开缓存");
    }

    /// **回归**：同一个字形键在不同 `px` 下必须是**不同位图**。
    ///
    /// shaping 路径按 `px_final` 排布，旧实现取位图时不带字号，于是 30px 的阿拉伯文
    /// 会"按 30px 排、用 15px 的图"：字符间距翻倍、字形只有一半大。
    #[test]
    fn glyph_keyed_at_uses_the_requested_size() {
        let mut e = engine();
        let data = font();
        let gid = super::super::glyph_id_of(&data, 'A').expect("A") as u32;
        let small = e
            .glyph_keyed_at(GlyphKey::new(0, gid), 0, 15.0)
            .expect("15px")
            .clone();
        let big = e
            .glyph_keyed_at(GlyphKey::new(0, gid), 0, 30.0)
            .expect("30px")
            .clone();
        assert!(
            big.height > small.height,
            "30px 的字形应比 15px 高：{} vs {}",
            big.height,
            small.height
        );
        assert!(
            big.width > small.width,
            "30px 的字形应比 15px 宽：{} vs {}",
            big.width,
            small.width
        );
    }

    #[test]
    fn unknown_face_falls_back_to_the_primary_font() {
        // shaping 可能先给出面号、之后才注册回退字体：此时应退回主字体而不是失败。
        let mut e = engine();
        let data = font();
        let gid = super::super::glyph_id_of(&data, 'A').expect("A") as u32;
        let info = e
            .glyph_keyed(GlyphKey::new(7, gid), 0)
            .expect("面号未知时应退回主字体");
        assert!(info.width > 0);
    }

    #[test]
    fn fallback_chain_reports_coverage_per_char() {
        let mut e = engine();
        let data = font();
        // 主字体（Windows: Segoe UI / Linux: DejaVu）有拉丁、**没有 CJK**。
        assert_eq!(e.face_for('A'), Some(0), "拉丁应命中主字体");
        assert_eq!(
            e.face_for('中'),
            None,
            "主字体没有 CJK 字形（两个平台的默认测试字体都没有）"
        );
        // 注册同一个字体当回退面：它同样没有 CJK，链上仍找不到。
        assert!(e.push_fallback(data));
        assert_eq!(
            e.face_for('中'),
            None,
            "回退面也没有该字形时必须诚实返回 None"
        );
        // 拉丁仍然命中主字体（顺序优先，不会被回退面抢走）。
        assert_eq!(e.face_for('B'), Some(0));
    }

    #[test]
    fn invalid_fallback_bytes_are_rejected_without_touching_the_chain() {
        let mut e = engine();
        assert!(!e.push_fallback(vec![0u8; 16]), "坏字体应被拒绝");
        assert_eq!(e.fallback_count(), 0);
    }

    #[test]
    fn out_of_range_glyph_id_does_not_panic() {
        let mut e = engine();
        // u32 → u16 截断 + 必定不存在的 glyph id：允许失败，但不能 panic。
        let key = GlyphKey::new(0, u32::from(u16::MAX));
        let _ = e.glyph_keyed(key, 0);
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

    /// **回归（数据丢失）**：图集被填满后，后续字形不能永久消失。
    ///
    /// 旧实现的问题有两层：
    /// 1. `evict_one` 用"最近 512 帧保护期"做筛选，`frame` 从 0 起算时
    ///    `cutoff == 0`，前 512 帧一个都驱不掉；
    /// 2. 就算驱掉了，`SkylinePacker` 的空洞不会回收 —— 重试 64 次必然全失败，
    ///    `insert_into_atlas` 返回 `None`，文本静默消失，还顺手删掉刚缓存的字形。
    ///
    /// 现在：驱逐会退化到"全表最冷"，驱逐无效时重打包图集再重试。
    #[test]
    fn glyphs_never_vanish_when_the_atlas_fills() {
        // 64² 极小图集：一两百个字形就会填满
        let mut e = engine(64);
        let chars: Vec<char> = ('A'..='Z')
            .chain('a'..='z')
            .chain('0'..='9')
            .chain([
                '.', ',', '-', '_', '+', '=', '/', '\\', '|', '~', '^', '*', '#', '@', '$', '%',
            ])
            .collect();
        let mut missing = Vec::new();
        for round in 0..3 {
            for ch in &chars {
                // 每轮推进帧号：让"最近使用"变化，覆盖 LRU 与保护期两条路径
                e.begin_frame();
                if e.glyph(*ch).is_none() {
                    missing.push((*ch, round));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "有 {} 个字形取不到（图集 64²，三轮）：{:?}",
            missing.len(),
            &missing[..missing.len().min(8)]
        );
        // 图集仍然能用：占有率不应超过 100%
        assert!(e.occupancy() <= 1.0);
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

#[cfg(test)]
mod ttc_hint_tests {
    use super::*;

    /// ttcf 头能被认出；真 TTF/OTF 不会被误判；不足 4 字节不越界。
    #[test]
    fn ttcf_magic_is_recognised_without_false_positives() {
        assert!(is_ttc(b"ttcf"));
        assert!(is_ttc(b"ttcf\x00\x01\x00\x00"));
        assert!(!is_ttc(&[0x00, 0x01, 0x00, 0x00]));
        assert!(!is_ttc(b"OTTO"));
        assert!(!is_ttc(b"ttc"));
        assert!(!is_ttc(&[]));
    }

    /// **关键回归：真字体仍然能构造**。
    ///
    /// 曾经在 `new()` 里预先拒绝 `ttcf`，结果把能用的字体也挡在外面，
    /// `hash-gate` 的帧哈希因此变掉。这条用**真字体**钉住"不得预拒。
    #[test]
    fn real_font_still_constructs() {
        let data = crate::font_engine::load_font(&crate::font_engine::default_test_font_path())
            .expect("font");
        let cell = crate::cell_metrics_for(15.0);
        let engine = MyFontEngine::new(data, 15.0, Size::new(cell.0, cell.1));
        assert!(engine.is_ok(), "真字体必须能构造：{:?}", engine.err());
    }

    /// 伪造 ttcf + 垃圾体：解析失败时错误必须带 TTC 提示与可执行建议。
    #[test]
    fn ttc_parse_failure_carries_actionable_hint() {
        let mut data = b"ttcf".to_vec();
        data.extend_from_slice(&[0u8; 64]); // 不是合法字体 ⇒ 解析必失败
        let err = MyFontEngine::new(data, 16.0, Size::new(8.0, 16.0))
            .err()
            .expect("应当失败");
        assert!(err.contains("TTC"), "错误应提到 TTC，实际: {err}");
        assert!(
            err.contains("TTF") && err.contains("OTF"),
            "错误应告知导出单个 TTF/OTF，实际: {err}"
        );
    }
}
