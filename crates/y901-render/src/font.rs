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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MipSlot {
    /// 图集内 X（左上角）。
    pub atlas_x: u32,
    /// 图集内 Y（左上角）。
    pub atlas_y: u32,
    /// 墨迹宽度（像素）。
    pub width: u32,
    /// 墨迹高度（像素）。
    pub height: u32,
    /// **该级自己**的 bearing_x（在该级字号下光栅化得到）。
    ///
    /// 不能用 L0 的 bearing 乘缩放：每级都是在自己字号下重新光栅化的，
    /// 边界取整与载距略微不同，算出的 bearing 相差 0.5~1px。
    pub bearing_x: f32,
    /// 该级自己的 bearing_y。
    pub bearing_y: f32,
}

impl MipSlot {
    /// 是否为空槽（未生成）。
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// 按缩放比选好的一级字形（**纯值**，可跨借用边界返回）。
///
/// 为什么不是 `MipSelection<'_>`：`FontEngine` 是 trait object，
/// 返回带生命周期的引用会把借用关系带进 trait，`Box<dyn FontEngine>` 侧很难用。
///
/// **`w`/`h` 是 L0 的尺寸，不是所选级的位图尺寸。** 这一点必须写清：
/// 布局（摆放、换行、光标）永远按原字号算；绘制时把**整个 slot** 拉伸到 `(w*s, h*s)`
/// 即可，采样倍率由 `w / slot.width` 隐式决定。
/// 若把 `w` 定义成所选级的宽度，调用方再乘 `scale` 就会把字画成 `1/2^level` 大小。
#[derive(Clone, Copy, Debug)]
pub struct ScaledGlyph {
    /// 实际用到的级：0 = L0，1 = 1/2，2 = 1/4。
    pub level: usize,
    /// 该级在图集里的区域（可能比 L0 小）。
    pub slot: MipSlot,
    /// **L0** 墨迹宽度（布局与绘制目标尺寸都用它）。
    pub w: u32,
    /// **L0** 墨迹高度。
    pub h: u32,
    /// 原字号的 advance。**调用方布局时必须自己乘 `scale`**（`pen_x += advance * scale`）。
    ///
    /// 为什么不预乘：advance 是字形的**固有属性**，`scale` 是调用方的**变换**，
    /// 两者混在一起会让"这个值代表什么"变得不可推断。但布局时若不乘，缩放后字距会是原尺寸。
    pub advance: f32,
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
    /// 位图底边相对基线的 y 偏移，统一使用 y-up 语义（基线处为 0，上方为正，下伸为负）。
    /// 位图顶部的屏幕坐标为 `baseline - bearing_y - height`。
    pub bearing_y: i32,
    /// 前进量（像素）；等宽网格里用于居中补偿。
    pub advance: f32,
    /// 图集内像素坐标。
    pub atlas_x: u32,
    /// 图集内 Y 坐标。
    pub atlas_y: u32,
    /// 最近一次使用帧（LRU）。
    pub last_used: u64,
    /// 墨迹密度：墨迹像素数 / 字身框面积（`size²`）。
    ///
    /// 为什么分母是字身框而不是包围盒：包围盒紧贴墨迹，
    /// 连一个句点的覆盖率都有 ~0.5，会让"密字多降一级"几乎恒为真。
    /// 为什么缓存而不每次算：算一次要遍历整块位图，而渲染每帧都要用它。
    pub ink: f32,
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
    /// **字符推进宽度**（像素）。
    ///
    /// 默认实现 = **格宽 × 格数**，即框架一贯的字库布局行为；
    /// 桌面与 GPU 路径因此逐像素不变。
    /// 比例字体需要真实 advance 时，由实现者覆盖（参见 `MyFontEngine`）。
    /// **引擎创建时的基准字号**（像素）。
    ///
    /// 用于"文本带有显式字号时用那个，否则用基准字号"
    /// 的场合拿到与光栅化一致的 px。
    fn size_px(&self) -> f32 {
        16.0
    }

    /// **字符推进宽度**（像素）。
    ///
    /// 默认实现 = **格宽 × 格数**，即框架一贯的格子布局行为；
    /// 桌面与 GPU 路径因此逐像素不变。比例字体需要真实
    /// advance 时，由实现者覆盖（参见 `MyFontEngine`）。
    fn advance(&self, ch: char, px: f32) -> f32 {
        let cell_w = crate::font::cell_metrics_for(px.max(1.0)).0;
        self.cell_span(ch) as f32 * cell_w
    }

    /// 字符占用的**格数**（CJK = 2，ASCII = 1）。
    ///
    /// 格宽布局的根源；比例字体的真实字宽请用 `advance()`。
    fn cell_span(&self, ch: char) -> u32;
    /// 图集像素（RGBA）。
    fn atlas(&self) -> &[u8];
    /// 图集边长。
    fn atlas_size(&self) -> u32;
    /// **按字形键取字形**（shaping 路径）。
    ///
    /// 默认 `None` = 该实现不支持按字形 id 取（调用方会自动退回逐字符路径）。
    fn glyph_keyed(
        &mut self,
        _key: crate::text::GlyphKey,
        _phase: u8,
    ) -> Option<&crate::font::GlyphInfo> {
        None
    }
    /// **字体原始字节**（shaping 用）。
    ///
    /// 默认 `None`：不提供字体字节的实现会自动跳过 shaped 路径，退回逐字符 cell 排版。
    fn font_data(&self) -> Option<&[u8]> {
        None
    }
    /// 取走脏区（消耗式协议）。
    fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>);
    /// 开始一帧。
    fn begin_frame(&mut self);
    /// 按缩放比选一级字形；默认实现**永远返回 L0**。
    ///
    /// 默认实现的语义是"这个引擎不支持缩放"——它不是错误，
    /// 只是能力差异：DirectWrite 路径没有 mip 链，也就没有更粗的级可选。
    /// **按字号取字形**（per-text 字号）。
    ///
    /// 默认实现：**忽略 `px`，委托 `glyph(ch)`** ——
    /// 这样其他 `FontEngine` 实现者（如 D3D 的 `TextEngine`）**不需要改动**就能编译，
    /// 行为与现状一致（字号参数被忽略）。
    ///
    /// 真正实现在 [`crate::font_engine::MyFontEngine`]（按 px 分开缓存并现场光栅化）。
    fn glyph_at_size(&mut self, ch: char, _px: f32) -> Option<&GlyphInfo> {
        self.glyph(ch)
    }

    /// 按**变换栈缩放比**取字形（mip 降采样链）。
    fn glyph_at_scale(&mut self, ch: char, _scale: f32) -> Option<ScaledGlyph> {
        let g = self.glyph(ch)?.clone();
        Some(ScaledGlyph {
            level: 0,
            slot: MipSlot {
                atlas_x: g.atlas_x,
                atlas_y: g.atlas_y,
                width: g.width,
                height: g.height,
                bearing_x: g.bearing_x as f32,
                bearing_y: g.bearing_y as f32,
            },
            w: g.width,
            h: g.height,
            advance: g.advance,
        })
    }

    /// 基线高度。
    fn ascent(&self) -> f32;
    /// **基线下方的下伸高度**（基准字号下的像素值，正数）。
    ///
    /// 用途：判断"一行文本的墨迹到底有多深"。`line_height` 是**行距**（1em 字身框），
    /// 而真实墨迹深度是 `ascent + descent`，二者不相等 —— 例如 16px 的 `SimSun`
    /// 行高 16 但墨迹深度约 18，下伸字/圆字母 overshoot 会越过行框 1~2px。
    /// 需要"框必须装得下墨迹"的地方（脏区、定高控件）用这个，而不是 `line_height`。
    /// 默认实现：由 `cell_height - ascent` 反推（对测试替身与老实现保持兼容）。
    /// 真实引擎应覆盖成字体自己的 `descender`。
    fn descent(&self) -> f32 {
        (self.cell_height() - self.ascent()).max(0.0)
    }
    /// 格宽度。
    fn cell_width(&self) -> f32;
    /// 格高度。
    fn cell_height(&self) -> f32;
}

/// **字形位图顶部的屏幕 y**（唯一实现，软/硬后端都必须调用它）。
///
/// `bearing_y` 是**位图底边**相对基线的 y-up 偏移（基线处 0、上方为正、下伸为负），
/// 因此 `top = 基线 − bearing_y − 高度`，其中基线 = `origin_y + ascent`。
///
/// **行级基线只取整一次**：位图是 1:1 贴图、采样器是线性过滤，四边形落在分数像素上
/// 就会把相邻纹素混一次（"字体发糊"，见 `docs/design/text-sharpness.md`）。
/// 取整后同一行的所有字形共用一条整数基线；不同后端也必须算出同一个值，
/// 否则软渲染与 GPU 渲染会错行——所以这必须是**共享函数**，不能各写一份。
#[inline]
pub fn glyph_top(origin_y: f32, ascent: f32, bearing_y: f32, height: f32) -> f32 {
    (origin_y + ascent).round() - bearing_y - height
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

#[cfg(test)]
mod dpi_scale_tests {
    use super::cell_metrics_for;

    /// **DPI 缩放后，字号层级顺序必须保持**，且 cell 必须变大。
    ///
    /// 只断言"顺序 + 变大"，**不锁死具体数值**（字体/字号可调）。
    #[test]
    fn dpi_scale_grows_cell_and_preserves_order() {
        let base = cell_metrics_for(16.0 * 1.25);
        let title = cell_metrics_for(18.0 * 1.25);
        let secondary = cell_metrics_for(13.0 * 1.25);
        assert!(
            title.0 > base.0 && base.0 > secondary.0,
            "缩放后层级顺序必须保持：标题 > 正文 > 表头"
        );
        assert!(
            base.0 > cell_metrics_for(16.0).0,
            "1.25 缩放后 cell 必须更大"
        );
    }
}
