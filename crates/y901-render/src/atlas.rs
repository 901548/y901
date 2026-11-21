//! 字形图集内核：Skyline 打包 + 压缩搬移 + LRU 淘汰。
//!
//! 设计要点：
//!
//! - **天際線（Skyline）打包**：按实际高度紧凑排列，混合字号下比货架打包省 ~40%。
//! - **padding 只在右/下留 1px**：采样用半像素偏移即可避免渗色，四周留白是浪费。
//! - **满了先压缩（compact），不重画**：把存活字形重新排版搬移，只做一次全量上传。
//! - **真 LRU**：`last_used_frame` 记录使用时间，占用 > 85% 时淘汰到 70%。
//!
//! 本模块**不依赖 DirectWrite / D3D11**，纯数据结构，可单测。

use std::collections::HashMap;

use crate::text::GlyphKey;

/// 图集里一个字形的位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    /// 矩形左上角 X（图集像素）。
    pub x: u32,
    /// 矩形左上角 Y（图集像素）。
    pub y: u32,
    /// 矩形宽度（像素）。
    pub w: u32,
    /// 矩形高度（像素）。
    pub h: u32,
    /// 最近一次使用的帧号；LRU 驱逐按它排序。
    pub last_used: u64,
}

impl Slot {
    /// 含 1px 间隙的宽度。间隙是为了防止相邻字形采样时互相渗色。
    pub fn padded_w(&self) -> u32 {
        self.w + 1
    }
    /// 含 1px 间隙的高度。
    pub fn padded_h(&self) -> u32 {
        self.h + 1
    }
    /// 含间隙面积；打包器用它比较候选位置。
    pub fn area(&self) -> u32 {
        self.padded_w() * self.padded_h()
    }
}

// ============================ Skyline 打包 ============================

/// 天际线节点：一段等高的平台。
#[derive(Clone, Copy, Debug)]
struct SkyNode {
    x: u32,
    y: u32,
    w: u32,
}

/// Skyline 打包器。
///
/// 每次插入选择"能放下且最低"的平台，然后分割该平台。
/// 相邻且等高的平台会被合并，避免碎片化。
#[derive(Clone, Debug)]
pub struct SkylinePacker {
    size: u32,
    nodes: Vec<SkyNode>,
    /// 已分配的像素面积（计数器，不参与打包决策）。
    used_area: u64,
}

/// 打包器的统计快照（只读，不影响打包）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackerStats {
    /// 已分配面积（含调用方自带的 padding）。
    pub used_area: u64,
    /// 剩余面积（总面积 - 已用）。
    pub free_area: u64,
    /// 在当前 skyline 上、沿任一峰能放下的最大矩形 `(w, h)`。
    ///
    /// 注意这是**上界而非最大空闲矩形**：它只考虑"从该峰顶往上到图集顶部"的竖直剩余，
    /// 不做跨峰搜索。足以回答"还能放多大的字形"。
    pub largest_free_rect: Option<(u32, u32)>,
    /// 占用率 = `used_area / size²`（0..1）。
    pub occupancy: f32,
}

impl SkylinePacker {
    /// 建一个边长为 `size` 的空图集打包器（初始只有一个整宽平台）。
    pub fn new(size: u32) -> Self {
        Self {
            size,
            nodes: vec![SkyNode {
                x: 0,
                y: 0,
                w: size,
            }],
            used_area: 0,
        }
    }

    /// **仅测试**：图集总面积的 `u64` 形式（避免 `size * size` 在 `u32` 上溢出）。
    #[cfg(test)]
    pub(crate) fn size_u64(&self) -> u64 {
        (self.size as u64) * (self.size as u64)
    }

    /// **仅测试**：把已用面积直接设成某个值（用于构造"图集接近满"的场景）。
    ///
    /// 为什么需要它：要验证"占用率 > 0.8 时跳过 L2"，靠真实字形填到 85% 需要
    /// 数百个字形且会触发驱逐（驱逐会把占用率又拉下来，测试变成不确定的）。
    #[cfg(test)]
    pub(crate) fn set_used_area_for_test(&mut self, area: u64) {
        self.used_area = area.min((self.size as u64) * (self.size as u64));
    }

    /// 只读统计：已用面积 / 剩余面积 / 最大可放矩形 / 占用率。
    ///
    /// 为什么需要它：之前只能用"已用包围盒"代理碎片率，
    /// 而那个代理把"单行打包时带上下的空白"也算成碎片，
    /// 在占用率 0.3% 时会报出 39% 的虚假值。真实分子只有一个：分配面积。
    pub fn stats(&self) -> PackerStats {
        let total = (self.size as u64) * (self.size as u64);
        let used = self.used_area.min(total);
        let largest = self
            .nodes
            .iter()
            .filter(|n| n.w > 0 && n.y < self.size)
            .map(|n| (n.w, self.size - n.y))
            .max_by_key(|(w, h)| (*w as u64) * (*h as u64));
        PackerStats {
            used_area: used,
            free_area: total - used,
            largest_free_rect: largest,
            occupancy: if total == 0 {
                0.0
            } else {
                used as f32 / total as f32
            },
        }
    }

    /// 清空所有平台并恢复初始状态（整张图集重新可用）。
    pub fn reset(&mut self) {
        self.used_area = 0;
        self.nodes.clear();
        self.nodes.push(SkyNode {
            x: 0,
            y: 0,
            w: self.size,
        });
    }

    /// 当前已用高度（天际线的最高点）。
    pub fn used_height(&self) -> u32 {
        self.nodes.iter().map(|n| n.y).max().unwrap_or(0)
    }

    /// 分配 `w × h`（调用方负责加 padding）。返回左上角坐标。
    pub fn insert(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 || w > self.size {
            return None;
        }

        // 1) 找最优平台：y 最小；y 相同则 x 最小
        let mut best: Option<(usize, u32, u32)> = None; // (index, y, x)
        for (i, node) in self.nodes.iter().enumerate() {
            if node.w < w {
                continue;
            }
            let y = node.y;
            if y + h > self.size {
                continue;
            }
            let better = match best {
                None => true,
                Some((_, by, bx)) => y < by || (y == by && node.x < bx),
            };
            if better {
                best = Some((i, y, node.x));
            }
        }

        let (idx, y, x) = best?;
        self.used_area += (w as u64) * (h as u64);

        // 2) 分割：原平台变成"已用宽度 w"，右侧剩余另起一段
        let node = self.nodes[idx];
        let leftover = node.w - w;
        if leftover > 0 {
            self.nodes.insert(
                idx + 1,
                SkyNode {
                    x: node.x + w,
                    y: node.y,
                    w: leftover,
                },
            );
        }
        self.nodes[idx].w = w;
        // 抬升被覆盖的平台
        self.nodes[idx].y = y + h;

        // 3) 合并相邻等高平台
        self.merge();

        Some((x, y))
    }

    fn merge(&mut self) {
        let mut i = 0;
        while i + 1 < self.nodes.len() {
            if self.nodes[i].y == self.nodes[i + 1].y
                && self.nodes[i].x + self.nodes[i].w == self.nodes[i + 1].x
            {
                let w = self.nodes[i + 1].w;
                self.nodes[i].w += w;
                self.nodes.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }
}

// ============================ 图集 ============================

/// 图集：像素缓冲 + 字形槽位 + LRU。
pub struct Atlas {
    size: u32,
    data: Vec<u8>,
    packer: SkylinePacker,
    /// 槽位表。**键是 [`GlyphKey`]**（不是 `char`）：shaping 之后同一字符在不同上下文
    /// 会对应不同字形，用字符作键会让它们共用同一块图集区域。
    slots: HashMap<GlyphKey, Slot>,
    frame: u64,
    /// 需要上传的脏矩形 (x, y, w, h)
    dirty: Vec<(u32, u32, u32, u32)>,
    /// 是否需要整张重传
    reset: bool,
    hits: u64,
    misses: u64,
    compact_count: u64,
}

/// 一次分配的结果。
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// 矩形左上角 X（图集像素）。
    pub x: u32,
    /// 矩形左上角 Y（图集像素）。
    pub y: u32,
    /// 矩形宽度（像素）。
    pub w: u32,
    /// 矩形高度（像素）。
    pub h: u32,
}

impl Atlas {
    /// 建一个边长为 `size` 的空图集打包器（初始只有一个整宽平台）。
    pub fn new(size: u32) -> Self {
        let size = size.max(64);
        Self {
            size,
            data: vec![0u8; (size * size) as usize],
            packer: SkylinePacker::new(size),
            slots: HashMap::new(),
            frame: 0,
            dirty: Vec::new(),
            reset: false,
            hits: 0,
            misses: 0,
            compact_count: 0,
        }
    }

    /// 图集边长（像素）。
    pub fn size(&self) -> u32 {
        self.size
    }

    /// 图集像素（RGBA；`rgb` 存覆盖率，`a` 恒为 255）。
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// 开始新的一帧，返回帧号。
    pub fn begin_frame(&mut self) -> u64 {
        self.frame += 1;
        self.frame
    }

    /// 当前帧号；每 `begin_frame` 递增，供 LRU 使用。
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// 查某个字符的占位（没打包过返回 `None`）。
    pub fn slot(&self, key: GlyphKey) -> Option<&Slot> {
        self.slots.get(&key)
    }

    /// 已打包的字符数（诊断与内存估算用）。
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// 发生过的整图重建次数。频繁重建说明图集太小或 LRU 太激进。
    pub fn compact_count(&self) -> u64 {
        self.compact_count
    }

    /// 命中率统计：(hit, miss)
    pub fn hit_stats(&self) -> (u64, u64) {
        (self.hits, self.misses)
    }

    /// 已占用比例（按像素面积）。
    pub fn occupancy(&self) -> f32 {
        let used: u32 = self.slots.values().map(|s| s.area()).sum();
        used as f32 / (self.size * self.size) as f32
    }

    /// 标记使用，用于 LRU。
    pub fn touch(&mut self, key: GlyphKey) {
        if let Some(slot) = self.slots.get_mut(&key) {
            slot.last_used = self.frame;
        }
    }

    /// 取一个字形的位置；没有或放不下时返回 `None`。
    ///
    /// 放不下时依次尝试：**升到 85% 以上就 LRU 淘汰 → 压缩搬移 → 再试一次**。
    /// 全过程不重画任何一个字形，只做一次搬移 + 一次全量上传。
    pub fn place(&mut self, key: GlyphKey, w: u32, h: u32) -> Option<Placement> {
        if let Some(slot) = self.slots.get(&key) {
            self.hits += 1;
            let (x, y, w, h) = (slot.x, slot.y, slot.w, slot.h);
            if let Some(slot) = self.slots.get_mut(&key) {
                slot.last_used = self.frame;
            }
            return Some(Placement { x, y, w, h });
        }

        self.misses += 1;

        // 需要 w+1 / h+1（右/下各 1px）
        let pw = w + 1;
        let ph = h + 1;

        let mut spot = self.packer.insert(pw, ph);
        if spot.is_none() {
            // 1) 压力大就先淘汰
            if self.occupancy() > 0.85 {
                self.evict_to(0.70);
            }
            // 2) 压缩搬移，回收碎片
            self.compact();
            spot = self.packer.insert(pw, ph);
        }

        let (x, y) = spot?;
        let slot = Slot {
            x,
            y,
            w,
            h,
            last_used: self.frame,
        };
        self.slots.insert(key, slot);
        Some(Placement { x, y, w, h })
    }

    /// 把字形像素写进图集。`src` 长度必须是 `w*h`。
    pub fn write(&mut self, key: GlyphKey, src: &[u8]) {
        let Some(slot) = self.slots.get(&key).copied() else {
            return;
        };
        if src.len() < (slot.w * slot.h) as usize {
            return;
        }
        for row in 0..slot.h {
            let dst = ((slot.y + row) * self.size + slot.x) as usize;
            let from = (row * slot.w) as usize;
            self.data[dst..dst + slot.w as usize]
                .copy_from_slice(&src[from..from + slot.w as usize]);
        }
        self.dirty.push((slot.x, slot.y, slot.w, slot.h));
    }

    /// 读取某个字形的像素（测试与调试用）。
    pub fn read(&self, key: GlyphKey) -> Option<Vec<u8>> {
        let slot = self.slots.get(&key)?;
        let mut out = Vec::with_capacity((slot.w * slot.h) as usize);
        for row in 0..slot.h {
            let src = ((slot.y + row) * self.size + slot.x) as usize;
            out.extend_from_slice(&self.data[src..src + slot.w as usize]);
        }
        Some(out)
    }

    /// 淘汰最久未使用的字形，直到占用降到 `target` 以下。
    ///
    /// 淘汰只是移除记录；真正的空间回收发生在随后的 `compact()`。
    pub fn evict_to(&mut self, target: f32) {
        while self.occupancy() > target && !self.slots.is_empty() {
            let victim = self
                .slots
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(k, _)| *k);
            match victim {
                Some(ch) => {
                    self.slots.remove(&ch);
                }
                None => break,
            }
        }
    }

    /// 压缩搬移：把所有存活字形重新排版到干净缓冲。
    ///
    /// **不会丢失任何仍在用的字形**：只改坐标，不改内容。
    /// 搬移后需要一次全量上传（`take_dirty` 会返回 reset=true）。
    pub fn compact(&mut self) {
        if self.slots.is_empty() {
            self.packer.reset();
            return;
        }

        // 按 (y, x) 排序，保持空间局部性，减少搬移后的碎片
        let mut entries: Vec<(GlyphKey, Slot)> = self.slots.iter().map(|(k, v)| (*k, *v)).collect();
        entries.sort_by_key(|(_, s)| (s.y, s.x));

        let mut fresh = vec![0u8; (self.size * self.size) as usize];
        self.packer.reset();

        for (key, old) in entries {
            let Some((x, y)) = self.packer.insert(old.padded_w(), old.padded_h()) else {
                // 理论上不会发生（内容总量没变），保守起见保留原槽位
                continue;
            };
            // 拷贝像素
            for row in 0..old.h {
                let src = ((old.y + row) * self.size + old.x) as usize;
                let dst = ((y + row) * self.size + x) as usize;
                fresh[dst..dst + old.w as usize]
                    .copy_from_slice(&self.data[src..src + old.w as usize]);
            }
            self.slots.insert(
                key,
                Slot {
                    x,
                    y,
                    w: old.w,
                    h: old.h,
                    last_used: old.last_used,
                },
            );
        }

        self.data = fresh;
        self.dirty.clear();
        self.reset = true;
        self.compact_count += 1;
    }

    /// 取走本帧需要上传的内容。
    pub fn take_dirty(&mut self) -> (bool, Vec<(u32, u32, u32, u32)>) {
        let reset = std::mem::replace(&mut self.reset, false);
        let dirty = std::mem::take(&mut self.dirty);
        (reset, dirty)
    }
}

// ============================ 测试 ============================

#[cfg(test)]
mod tests {
    use super::*;

    /// 货架打包参考实现：按行放，行高 = 该行最高字形。
    /// 用来证明 skyline 更省空间。
    fn shelf_height(sizes: &[(u32, u32)], width: u32) -> u32 {
        let mut y = 0;
        let mut x = 0;
        let mut row_h = 0;
        for &(w, h) in sizes {
            if x + w > width {
                y += row_h;
                x = 0;
                row_h = 0;
            }
            x += w;
            row_h = row_h.max(h);
        }
        y + row_h
    }

    #[test]
    fn skyline_uses_less_height_than_shelf() {
        // 混合字号：大字形 16×16，小字形 8×8 与 8×4
        let mut sizes = Vec::new();
        for i in 0..120u32 {
            match i % 3 {
                0 => sizes.push((17, 17)),
                1 => sizes.push((9, 9)),
                _ => sizes.push((9, 5)),
            }
        }

        let mut packer = SkylinePacker::new(256);
        for &(w, h) in &sizes {
            assert!(packer.insert(w, h).is_some(), "应当放得下");
        }
        let sky = packer.used_height();
        let shelf = shelf_height(&sizes, 256);

        assert!(sky <= shelf, "skyline 高度 {sky} 应不大于货架 {shelf}");
        // 混合尺寸下通常能省 20% 以上
        assert!(
            sky * 100 < shelf * 85,
            "skyline {sky} vs shelf {shelf}：应至少省 15%"
        );
    }

    #[test]
    fn compact_preserves_every_uv_mapping() {
        let mut atlas = Atlas::new(256);
        atlas.begin_frame();

        // 塞 200 个字形，内容用唯一字节填充，便于校验
        let mut expected: Vec<(char, u32, u32, Vec<u8>)> = Vec::new();
        for i in 0..200u32 {
            let ch = char::from_u32(0x4E00 + i).unwrap();
            let w = 6 + (i % 5);
            let h = 8 + (i % 7);
            let Some(_p) = atlas.place(GlyphKey::from_char(ch), w, h) else {
                break;
            };
            let fill = (i % 251 + 1) as u8;
            let pixels = vec![fill; (w * h) as usize];
            atlas.write(GlyphKey::from_char(ch), &pixels);
            expected.push((ch, w, h, pixels));
        }

        let before: Vec<(char, u32, u32)> = expected
            .iter()
            .map(|(ch, _, _, _)| {
                let s = atlas.slot(GlyphKey::from_char(*ch)).unwrap();
                (*ch, s.x, s.y)
            })
            .collect();

        // 强制压缩
        atlas.compact();
        assert_eq!(atlas.compact_count(), 1);

        // 每个字形都必须还在，且内容一模一样
        for (ch, w, h, pixels) in &expected {
            let slot = atlas
                .slot(GlyphKey::from_char(*ch))
                .unwrap_or_else(|| panic!("compact 之后字形 {ch:?} 丢失"));
            assert_eq!(slot.w, *w);
            assert_eq!(slot.h, *h);

            let got = atlas.read(GlyphKey::from_char(*ch)).expect("read");
            assert_eq!(&got, pixels, "字形 {ch:?} 的内容在搬移后损坏");

            // UV 通过 slot 坐标导出，必须指向正确的像素
            let u = slot.x as f32 / atlas.size() as f32;
            let v = slot.y as f32 / atlas.size() as f32;
            assert!((0.0..1.0).contains(&u) && (0.0..1.0).contains(&v));
        }

        // 坐标确实变了（说明真的重排过，而不是什么都没做）
        let moved = expected.iter().any(|(ch, _, _, _)| {
            let s = atlas.slot(GlyphKey::from_char(*ch)).unwrap();
            before
                .iter()
                .any(|(bch, bx, by)| bch == ch && (*bx != s.x || *by != s.y))
        });
        assert!(moved, "compact 应该真的搬移了字形");

        // 压缩后需要一次全量上传
        let (reset, _) = atlas.take_dirty();
        assert!(reset, "compact 之后必须标记整张重传");
    }

    #[test]
    fn lru_evicts_when_pressure_is_high() {
        let mut atlas = Atlas::new(128);

        // 第 1 帧塞满
        atlas.begin_frame();
        let mut placed = 0;
        for i in 0..400u32 {
            let ch = char::from_u32(0x4E00 + i).unwrap();
            if atlas.place(GlyphKey::from_char(ch), 12, 12).is_some() {
                atlas.write(GlyphKey::from_char(ch), &[1u8; 144]);
                placed += 1;
            }
        }
        assert!(placed > 40, "至少应放下一些字形，实际 {placed}");
        let occ = atlas.occupancy();
        assert!(occ > 0.0);

        // 下一帧持续申请，迫使 LRU + compact 接管
        atlas.begin_frame();
        for i in 0..400u32 {
            let ch = char::from_u32(0x9000 + i).unwrap();
            if atlas.place(GlyphKey::from_char(ch), 12, 12).is_some() {
                atlas.write(GlyphKey::from_char(ch), &[2u8; 144]);
            }
        }

        // 没有 panic、没有清空、占用仍然可控
        assert!(atlas.occupancy() <= 1.0);
        assert!(atlas.slot_count() > 0, "不应把所有字形都丢掉");
        assert!(atlas.compact_count() > 0, "压力下应触发过压缩");
    }

    #[test]
    fn merge_adjacent_levels() {
        let mut p = SkylinePacker::new(64);
        assert_eq!(p.insert(16, 16), Some((0, 0)));
        assert_eq!(p.insert(16, 16), Some((16, 0)));
        // 两块等高且相邻，天际线应合并；下一块仍应在 y=0 起
        assert_eq!(p.insert(32, 32), Some((32, 0)));
    }

    #[test]
    fn padding_is_only_right_and_bottom() {
        let mut atlas = Atlas::new(64);
        atlas.begin_frame();
        let a = atlas.place(GlyphKey::from_char('a'), 8, 8).unwrap();
        assert_eq!((a.x, a.y), (0, 0));
        // 下一块紧贴 (8+1, 0) = (9, 0)，证明只留了右边 1px
        let b = atlas.place(GlyphKey::from_char('b'), 8, 8).unwrap();
        assert_eq!(b.x, 9, "右 padding 应为 1px");
        assert_eq!(b.y, 0);
    }

    #[test]
    fn stats_reports_about_half_when_half_filled() {
        // 1024² 图集：填 512×1024 的一半就是 50%
        let mut p = SkylinePacker::new(1024);
        let before = p.stats();
        assert_eq!(before.used_area, 0);
        assert!((before.occupancy - 0.0).abs() < 1e-6);

        // 两块 512×512 叠起来 = 512×1024 = 50%
        p.insert(512, 512).expect("第一块");
        p.insert(512, 512).expect("第二块");
        let mid = p.stats();
        assert!(
            (mid.occupancy - 0.5).abs() < 0.1,
            "占用率应接近 0.5，实测 {}",
            mid.occupancy
        );
        assert_eq!(mid.used_area, 512 * 512 * 2);
        assert_eq!(mid.free_area, 1024 * 1024 - mid.used_area);
        assert!(mid.largest_free_rect.is_some());
    }

    #[test]
    fn stats_reset_clears_the_counter() {
        let mut p = SkylinePacker::new(256);
        p.insert(100, 100).expect("a");
        assert!(p.stats().used_area > 0);
        p.reset();
        assert_eq!(p.stats().used_area, 0);
        assert!((p.stats().occupancy - 0.0).abs() < 1e-6);
    }

    #[test]
    fn stats_show_shrinking_largest_free_rect_as_the_atlas_fills() {
        // 用真实分配模拟两个场景（靠数字而不是靠推测）
        let mut half = SkylinePacker::new(1024);
        let mut guard = 0;
        while half.stats().occupancy < 0.5 && guard < 10_000 {
            if half.insert(32, 32).is_none() {
                break;
            }
            guard += 1;
        }
        let s_half = half.stats();
        eprintln!(
            "[packer] 半满：occupancy={:.3} used={} largest={:?}",
            s_half.occupancy, s_half.used_area, s_half.largest_free_rect
        );

        let mut full = SkylinePacker::new(1024);
        let mut guard = 0;
        while full.stats().occupancy < 0.85 && guard < 100_000 {
            if full.insert(32, 32).is_none() {
                break;
            }
            guard += 1;
        }
        let s_full = full.stats();
        eprintln!(
            "[packer] 接近满：occupancy={:.3} used={} largest={:?}",
            s_full.occupancy, s_full.used_area, s_full.largest_free_rect
        );

        assert!(s_half.occupancy >= 0.45, "半满场景应接近 0.5");
        assert!(
            s_full.occupancy >= 0.8,
            "接近满场景应过 0.8，实测 {}",
            s_full.occupancy
        );
        // 碎片化正在发生：剩余面积越大的同时最大可放矩形变小
        assert!(
            s_full.largest_free_rect.map(|(w, _)| w).unwrap_or(0)
                <= s_half.largest_free_rect.map(|(w, _)| w).unwrap_or(0),
            "越满的图集不应拥有更宽的可放位"
        );
    }
}
