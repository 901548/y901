//! 字形引擎的**平台无关**部分：trait 与数据结构。
//!
//! 具体实现分居各处：
//! - `MyFontEngine`（自研 TTF）在本 crate 的 [`crate::font_engine`]；
//! - `TextEngine`（DirectWrite）在 `y901-backend-win32`（它依赖 windows-sys）。

/// 图集脏矩形：告诉后端这一块像素变了，需要重新上传。
pub struct DirtyRect {
    /// 矩形左上角 X（图集像素）。
    pub x: u32,
    /// 矩形左上角 Y（图集像素）。
    pub y: u32,
    /// 墨迹宽度（像素）。
    pub width: u32,
    /// 墨迹高度（像素）。
    pub height: u32,
}


/// 一级 mip 在**图集里的位置**。
///
/// 为什么不直接存 [`crate::mip::MipLevel`]：那会把覆盖率像素复制一份进 `GlyphInfo`，
/// 与"图集是唯一存储"的设计冲突（内存直接翻倍）。mip 的像素住在图集里，
/// `GlyphInfo` 只记坐标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MipSlot {
    /// 图集内 X（左上角）。
    pub atlas_x: u32,
    /// 图集内 Y（左上角）。
    pub atlas_y: u32,
    /// 墨迹宽度（像素）。
    pub width: u32,
    /// 墨迹高度（像素）。
    pub height: u32,
}

impl MipSlot {
    /// 是否为空槽（未生成）。
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

#[derive(Clone, Debug)]
/// 一个已光栅化字形的度量与图集位置。
/// 为什么带 `bearing`：字形墨迹不一定从原点开始，绘制时要按 bearing 偏移，
/// 否则字符间距会忽大忽小。
pub struct GlyphInfo {
    /// 墨迹宽度（像素）。
    pub width: u32,
    /// 墨迹高度（像素）。
    pub height: u32,
    /// 相对基线原点（pen position）的墨迹偏移。
    pub bearing_x: i32,
    /// 墨迹相对基线的垂直偏移（向上为正）。
    pub bearing_y: i32,
    /// 前进量（像素）；等宽网格里用于居中补偿。
    pub advance: f32,
    /// 图集内像素坐标。
    pub atlas_x: u32,
    /// 图集内 Y 坐标。
    pub atlas_y: u32,
    /// 最近一次使用帧（LRU）。
    pub last_used: u64,
    /// mip 链：`mips[0]` = 1/2 尺寸，`mips[1]` = 1/4，`mips[2]` = 1/8。
    ///
    /// 只对**缩小方向**生成；`None` 表示该级还没生成或生成失败，
    /// 选择器会回退到最近的可用级（最终回退到 L0）。
    pub mips: [Option<MipSlot>; 3],
}

impl GlyphInfo {
    /// 是否为空字形（空格等无墨迹字符）。空字形不该写图集，也不该画。
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// 取第 `level` 级的图集位置；**缺级时向上回退到最近的可用级**，
    /// 最终回退到 L0（即 `None` = 用 L0 字段）。
    ///
    /// 为什么要回退而不是返回 `None`：缺级是**正常状态**（懒生成没触发、
    /// 或图集过满被保护跳过 L2），此时用更细的级只是"多采样一点"，
    /// 而返回 `None` 会让调用方误以为"字形不存在"。
    pub fn mip_at(&self, level: usize) -> Option<MipSlot> {
        if level == 0 {
            return None;
        }
        for l in (1..=level.min(3)).rev() {
            if let Some(slot) = self.mips.get(l - 1).copied().flatten() {
                if !slot.is_empty() {
                    return Some(slot);
                }
            }
        }
        None
    }
}

/// 字形引擎的统一接口。
///
/// 为什么要这个 trait：两个后端（D3D11 / 纯 CPU）都只需这 9 个能力。
/// 把它们抽出来之后，`TextEngine`（DirectWrite）与 [`MyFontEngine`]（自研）
/// 可以**互相替换**，而 `y901-widgets` / `y901-ui` 一行不改。
///
/// 【硬约束】图集像素格式与脏区协议必须一致：
/// RGBA、`rgb` = 覆盖度、`a` = 255；`take_dirty` 的 `reset` 是消耗式。
pub trait FontEngine {
    /// 取字形（可能会现场光栅化并写图集）。
    fn glyph(&mut self, ch: char) -> Option<&GlyphInfo>;
    /// 字符占用的格数。
    fn cell_span(&self, ch: char) -> u32;
    /// 图集像素（RGBA）。
    fn atlas(&self) -> &[u8];
    /// 图集边长。
    fn atlas_size(&self) -> u32;
    /// 取走脏区（消耗式协议）。
    fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>);
    /// 开始一帧。
    fn begin_frame(&mut self);
    /// 基线高度。
    fn ascent(&self) -> f32;
    /// 格宽度。
    fn cell_width(&self) -> f32;
    /// 格高度。
    fn cell_height(&self) -> f32;
}


/// 由字号推出等宽单元格尺寸 `(宽, 高)`。
/// 为什么要独立函数：布局、字形引擎、后端必须用同一套格尺寸，否则文字与光标会错位。
pub fn cell_metrics_for(font_size: f32) -> (f32, f32) {
    let size = if font_size.is_finite() && font_size > 0.0 {
        font_size
    } else {
        1.0
    };
    ((size * 0.5).round().max(1.0), size.round().max(1.0))
}
