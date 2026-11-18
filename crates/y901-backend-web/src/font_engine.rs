//! wasm 端的**等宽回退字体引擎**。
//!
//! 为什么需要：`SoftBackend::new` 要求一个 `Box<dyn FontEngine>`，而浏览器端没有 DirectWrite。
//! S1-b 只需要把管线跑通（布局、命中、出帧），文字可以先不可见；
//! 真 TTF 加载放在 S1-c（`fetch` + `FontFace`，见 `docs/WASM.md` §3.5）。
//!
//! 语义：**每个字符占 1 格，字形 0 像素**。于是
//! - `cell_span` 恒 1 → 文本宽度 = 字符数 × 格宽（与桌面等宽模型一致）
//! - `glyph` 恒返回空字形（`width = height = 0`）→ 不画任何像素
//! - `atlas` 为空 → 不占图集内存
//!
//! ⇒ 布局与命中测试**完全正确**，只是"看不见字"。这足以让 S1-b 的帧管线跑起来。

use y901_render::font::{DirtyRect, FontEngine, GlyphInfo};

/// 等宽回退字体引擎。
pub struct WasmFontEngine {
    /// 空字形：尺寸 0×0，advance 为 0（不参与排版推进，排版由 `cell_span` 决定）。
    blank: GlyphInfo,
}

impl WasmFontEngine {
    /// 新建；无参（等宽尺寸由 `SoftBackend` 的 `cell` 参数决定）。
    pub fn new() -> Self {
        Self {
            blank: GlyphInfo {
                width: 0,
                height: 0,
                bearing_x: 0,
                bearing_y: 0,
                advance: 0.0,
                atlas_x: 0,
                atlas_y: 0,
                last_used: 0,
                ink: 0.0,
                mips: [None, None, None],
            },
        }
    }
}

impl Default for WasmFontEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FontEngine for WasmFontEngine {
    fn glyph(&mut self, _ch: char) -> Option<&GlyphInfo> {
        Some(&self.blank)
    }

    /// 等宽：任何字符都只占 1 格。
    fn cell_span(&self, _ch: char) -> u32 {
        1
    }

    fn atlas(&self) -> &[u8] {
        &[]
    }

    fn atlas_size(&self) -> u32 {
        0
    }

    /// 从不产生脏区（没有图集内容）。
    fn take_dirty(&mut self) -> (bool, Vec<DirtyRect>) {
        (false, Vec::new())
    }

    fn begin_frame(&mut self) {}

    /// 基线到格顶的距离：等宽回退取格高的 0.8（与桌面等宽模型一致的常用比例）。
    fn ascent(&self) -> f32 {
        16.0 * 0.8
    }

    fn descent(&self) -> f32 {
        // 浏览器端字体度量是占位值（16px × 0.2 = 3.2），与 `ascent` 同一假设。
        16.0 * 0.2
    }

    /// 格宽（像素）：等宽回退固定 8。
    fn cell_width(&self) -> f32 {
        8.0
    }

    /// 格高（像素）：固定 16。
    fn cell_height(&self) -> f32 {
        16.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use y901_render::soft::SoftBackend;
    use y901_ui::Size;

    #[test]
    fn blank_glyph_is_empty() {
        let mut e = WasmFontEngine::new();
        let g = e.glyph('中').expect("blank glyph");
        assert!(g.is_empty(), "空字形应为 is_empty");
        assert_eq!(e.cell_span('中'), 1);
        assert_eq!(e.cell_span('A'), 1);
        assert_eq!(e.atlas_size(), 0);
    }

    #[test]
    fn constructs_soft_backend() {
        // 关键验证：`SoftBackend::new` 能接受 `Box<dyn FontEngine>`，并且像素缓冲已分配。
        let soft = SoftBackend::new(
            80,
            60,
            Box::new(WasmFontEngine::new()),
            Size::new(8.0, 16.0),
            None,
        )
        .expect("soft backend");
        assert!(!soft.pixels().is_empty(), "像素缓冲应已分配");
        assert_eq!(soft.pixels().len(), 80 * 60 * 4, "应为 80x60 的 RGBA");
    }
}
