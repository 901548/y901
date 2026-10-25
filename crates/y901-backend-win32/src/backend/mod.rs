//! D3D11 后端：`y901_ui::RenderCtx` + `TextMetrics` 的唯一实现。
//!
//! 这一层是框架与 GPU 之间唯一的桥：
//! - `fill_rect` / `draw_text` → 追加到实例数组（同一帧只有一次 DrawInstanced）
//! - DirectWrite 字形 → 图集纹理，增量上传
//!
//! 框架和组件都看不到 D3D11 类型，只看到 `RenderCtx`。

#[allow(dead_code)] // Atlas/Placement 是给多页图集预留的 API，暂由 SkylinePacker 先行使用
pub mod atlas;
pub mod d3d11;
pub mod font;


use windows_sys::Win32::Foundation::HWND;

use y901_ui::{Color, Pos, Rect, RenderCtx, Size, TextStyle, TextMetrics};

use d3d11::{Gfx, Instance};
use font::TextEngine;

/// 逻辑像素下的单元格尺寸。终端组件会通过 `measure_text("M")` 拿到同样的值。
const CELL_W: f32 = 8.0;
const CELL_H: f32 = 16.0;
const FONT_SIZE: f32 = 16.0;
const FONT_FAMILY: &str = "NSimSun";

pub struct D3dBackend {
    pub gfx: Gfx,
    text: TextEngine,
    instances: Vec<Instance>,
    viewport: Size,
    clips: Vec<Rect>,
}

impl D3dBackend {
    /// 初始化 D3D11 设备、交换链与 DirectWrite 文本引擎。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `hwnd` 是有效且未销毁的窗口句柄，且调用期间不会被其它线程销毁；
    /// - 调用发生在拥有该窗口的线程上；
    /// - `width`、`height` 均大于 0。
    ///
    /// 违反会导致：交换链绑定到悬垂 HWND，`Present` 时访问已释放的窗口内存
    /// （未定义行为）；DirectWrite 一侧失败则返回 `Err`，可恢复。
    pub unsafe fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self, String> {
        // 逐字节核对 CPU 实例布局与 GPU InputLayout：错一个字节整屏都是乱码。
        d3d11::verify_instance_layout()?;

        let gfx = Gfx::new(hwnd, width, height)?;
        let text = TextEngine::new(FONT_FAMILY, FONT_SIZE, CELL_W, CELL_H)?;
        eprintln!(
            "[y901] backend ready: ascent={:.2} descent={:.2} atlas={}",
            text.ascent, text.descent, text.atlas_size
        );

        let mut backend = Self {
            gfx,
            text,
            instances: Vec::with_capacity(4096),
            viewport: Size::new(width as f32, height as f32),
            clips: Vec::new(),
        };
        backend.sync_atlas()?;
        Ok(backend)
    }

    /// 每帧开始：清空实例数组，并推进图集帧号（LRU 用）。
    pub fn begin(&mut self) {
        self.instances.clear();
        self.clips.clear();
        self.text.begin_frame();
    }

    /// 把 CPU 图集的脏区同步到 GPU。首次调用会创建纹理。
    unsafe fn sync_atlas(&mut self) -> Result<(), String> {
        let (reset, dirty) = self.text.take_dirty();
        if !reset && dirty.is_empty() {
            return Ok(());
        }
        let rects: Vec<(u32, u32, u32, u32)> = dirty
            .iter()
            .map(|d| (d.x, d.y, d.width, d.height))
            .collect();
        self.gfx
            .update_atlas(&self.text.atlas, self.text.atlas_size, reset, &rects)
    }

    /// 提交一帧。返回是否需要继续重绘。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `hwnd` 仍是创建本后端时使用的窗口，且尚未销毁；
    /// - `width`、`height` 是**当前**客户区尺寸（不一致会走重建路径）；
    /// - 同线程调用。
    ///
    /// 违反会导致：重建路径用悬垂 HWND 创建交换链（未定义行为），
    /// 或尺寸反复不一致造成资源重建风暴。设备丢失属于预期，返回 `false` 而非崩溃。
    pub unsafe fn present(&mut self, hwnd: HWND, width: u32, height: u32) -> bool {
        if self.gfx.is_empty() {
            if let Err(err) = self.gfx.rebuild(hwnd, width, height) {
                eprintln!("[y901] rebuild failed: {err}");
                return false;
            }
        }

        if let Err(err) = self.sync_atlas() {
            eprintln!("[y901] atlas sync failed: {err}");
        }

        // sync_interval = 0：不阻塞在 vsync 上。
        // 我们是按需重绘（invalidate 驱动），不需要垂直同步；
        // 用 1 时窗口被遮挡会让 Present 永久阻塞，把消息循环卡死。
        let mut hr = self.gfx.render(&self.instances, 0);

        if Gfx::device_lost(hr) {
            eprintln!("[y901] device lost: 0x{:08X}, rebuilding", hr as u32);
            if let Err(err) = self.gfx.rebuild(hwnd, width, height) {
                eprintln!("[y901] rebuild failed: {err}");
                return false;
            }
            // 设备重建后纹理是空的：全量重传
            if let Err(err) = self
                .gfx
                .update_atlas(&self.text.atlas, self.text.atlas_size, true, &[])
            {
                eprintln!("[y901] atlas re-upload failed: {err}");
            }
            hr = self.gfx.render(&self.instances, 0);
        }

        if hr < 0 {
            eprintln!("[y901] Present failed: 0x{:08X}", hr as u32);
            return false;
        }
        !Gfx::is_occluded(hr)
    }

    /// 记录新的客户区尺寸并尝试同步交换链。
    ///
    /// # Safety
    ///
    /// 调用者必须保证：
    /// - `width`、`height` 大于 0（0 尺寸会让 `ResizeBuffers` 失败）；
    /// - 同线程调用，且此刻没有外部代码持有本后端的 back buffer 引用。
    ///
    /// 违反会导致：`ResizeBuffers` 返回 `DXGI_ERROR_INVALID_CALL`；
    /// 本方法只打印错误并保留旧尺寸，不会 panic。
    pub unsafe fn resize(&mut self, width: u32, height: u32) {
        self.viewport = Size::new(width as f32, height as f32);
        // 交换链尺寸更新交给 present 前的 rebuild 路径；这里只记尺寸。
        if let Err(err) = self.gfx.resize(width, height) {
            eprintln!("[y901] swapchain resize failed: {err}");
        }
    }

    /// 当前帧的实例数（诊断用）。
    #[allow(dead_code)] // 诊断接口，主循环不调用
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    #[allow(dead_code)] // 诊断接口，主循环不调用
    pub fn live_objects() -> isize {
        Gfx::live_objects()
    }

    fn clip_of(&self, rect: Rect) -> Option<Rect> {
        match self.clips.last() {
            Some(clip) => {
                let r = rect.intersect(*clip);
                if r.is_empty() {
                    None
                } else {
                    Some(r)
                }
            }
            None => Some(rect),
        }
    }

    /// 把字形推进实例列表。
    ///
    /// `span` 来自格契约（`y901_core::width_of_char`）：ASCII=1，CJK=2。
    /// **不是**字形 advance，而是"这个字形应占几格"。
    /// 居中补偿 `offset_x = (span*CELL_W - advance)/2`：字体 advance 与格宽
    /// 不精确相等时（跨平台常见），字形仍落在格子中央而不是贴左。
    fn push_glyph(&mut self, x: f32, y: f32, span: f32, ch: char, color: Color, bold: bool) {
        let atlas_size = self.text.atlas_size as f32;
        let ascent = self.text.ascent;

        let Some(glyph) = self.text.glyph(ch).cloned() else {
            return;
        };
        if glyph.is_empty() {
            return;
        }

        let offset_x = glyph_offset(span, CELL_W, glyph.advance);
        let pos = Pos::new(
            x + offset_x + glyph.bearing_x as f32,
            y + ascent + glyph.bearing_y as f32,
        );
        let size = [glyph.width as f32, glyph.height as f32];
        let uv = [
            glyph.atlas_x as f32 / atlas_size,
            glyph.atlas_y as f32 / atlas_size,
            (glyph.atlas_x + glyph.width) as f32 / atlas_size,
            (glyph.atlas_y + glyph.height) as f32 / atlas_size,
        ];
        let rgba = [color.r, color.g, color.b, color.a];

        self.instances.push(Instance {
            pos: [pos.x, pos.y],
            size,
            uv_rect: uv,
            color: rgba,
        });

        // 假粗体：右移 1px 再画一遍（比加载粗体字形便宜，且等宽不变）
        if bold {
            self.instances.push(Instance {
                pos: [pos.x + 1.0, pos.y],
                size,
                uv_rect: uv,
                color: rgba,
            });
        }
    }
}

impl TextMetrics for D3dBackend {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        let cols: u32 = text.chars().map(|c| self.text.cell_span(c)).sum();
        Size::new(cols as f32 * CELL_W, CELL_H)
    }

    fn line_height(&self, _style: &TextStyle) -> f32 {
        CELL_H
    }

    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for D3dBackend {
    fn size(&self) -> Size {
        self.viewport
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let Some(r) = self.clip_of(rect) else {
            return;
        };
        self.instances.push(Instance {
            pos: [r.x, r.y],
            size: [r.w, r.h],
            // 纯色哨兵：uv 两端相等，PS 直接输出 color
            uv_rect: [0.0, 0.0, 0.0, 0.0],
            color: [color.r, color.g, color.b, color.a],
        });
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        self.fill_rect(Rect::new(rect.x, rect.y, rect.w, width), color);
        self.fill_rect(
            Rect::new(rect.x, rect.bottom() - width, rect.w, width),
            color,
        );
        self.fill_rect(Rect::new(rect.x, rect.y, width, rect.h), color);
        self.fill_rect(
            Rect::new(rect.right() - width, rect.y, width, rect.h),
            color,
        );
    }

    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
        let mut pen_x = origin.x;
        for ch in text.chars() {
            let span = self.text.cell_span(ch).max(1) as f32;
            // 背景（如果有）
            if style.bg.a > 0.0 {
                self.fill_rect(
                    Rect::new(pen_x, origin.y, span * CELL_W, CELL_H),
                    style.bg,
                );
            }
            if ch != ' ' {
                self.push_glyph(pen_x, origin.y, span, ch, style.fg, style.bold);
            }
            if style.underline {
                self.fill_rect(
                    Rect::new(pen_x, origin.y + CELL_H - 2.0, span * CELL_W, 1.0),
                    style.fg,
                );
            }
            pen_x += span * CELL_W;
        }
        pen_x - origin.x
    }

    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.clips.last() {
            Some(outer) => rect.intersect(*outer),
            None => rect,
        };
        self.clips.push(clipped);
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
    }
}

/// 字形在格子里的水平居中偏移。
///
/// `span * cell_w` 是"该字形应占的宽度"（格契约），`advance` 是字体实际推进宽度。
/// 二者不精确相等时（跨平台常见：后备字体、合成字形），把差值的一半作为左偏移，
/// 让字形居中而不是贴左。
///
/// 精确 2:1 的字体（如 NSimSun 16px）会得到 0.0 —— Windows 外观不变。
pub(crate) fn glyph_offset(span: f32, cell_w: f32, advance: f32) -> f32 {
    (span * cell_w - advance) * 0.5
}

#[cfg(test)]
mod glyph_offset_tests {
    use super::glyph_offset;

    /// 精确 2:1：offset 必须为 0（Windows 现有外观的回归保证）。
    #[test]
    fn exact_2to1_has_zero_offset() {
        assert_eq!(glyph_offset(2.0, 8.0, 16.0), 0.0);
        assert_eq!(glyph_offset(1.0, 8.0, 8.0), 0.0);
    }

    /// advance 偏小 10%（14.4 vs 16）：offset 为正，且字形居中在 2 格内。
    #[test]
    fn cjk_offset_centers_glyph_in_two_cells() {
        let cell_w = 8.0;
        let advance = 1.8 * cell_w; // 14.4
        let off = glyph_offset(2.0, cell_w, advance);
        assert!(off > 0.0, "advance 偏小时 offset 应为正，实际 {off}");
        assert!(
            (off - (2.0 * cell_w - advance) / 2.0).abs() < 1e-5,
            "offset 必须是差值的一半"
        );
        // 字形左边界 = 0 + off，墨迹宽 = advance，中心应落在 2 格中点 (8.0)
        let center = off + advance / 2.0;
        assert!(
            (center - cell_w).abs() < 1e-4,
            "字形中心应落在 2 格中点 {cell_w}，实际 {center}"
        );
    }

    /// advance 偏大（17 vs 16）：offset 为负，同样居中。
    #[test]
    fn oversized_advance_gets_negative_offset() {
        let off = glyph_offset(2.0, 8.0, 17.0);
        assert!(off < 0.0);
        let center = off + 17.0 / 2.0;
        assert!((center - 8.0).abs() < 1e-4, "仍应居中，实际 {center}");
    }
}