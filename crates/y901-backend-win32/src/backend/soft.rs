//! 纯 CPU 软渲染后端（Step 1）。
//!
//! **目标**：把"画到窗口"这一步从 GPU 上摆脱，字体仍用 DirectWrite（图集在 `TextEngine` 里）。
//!
//! 设计要点：
//! 1. 一块 `Vec<u32>` 帧缓冲（每像素 `0xAARRGGBB`，内存里就是 B,G,R,A），尺寸跟随窗口；
//! 2. 手写扫描线光栅化：实心、渐变、SDF 圆角（含 1px 抗锯）、字形图集拼贴；
//! 3. 混合用标准源超并发（straight alpha），与 GPU 侧 `lerp` 一致；
//! 4. 呈现用 GDI `StretchDIBits`（不要求 D3D）。
//!
//! 为什么先不动字体：字形光栅化需要 TTF 解析 + 贝塞尔离散化，
//! 那是 Step 2；本轮先把"图元 + 呈现"这两段彻底搬到 CPU。

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    GetDC, ReleaseDC, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use y901_ui::theme::Theme;
use y901_ui::{
    Color, Edges, Pos, Rect, RenderCtx, Size, TextStyle, TextMetrics,
};

use super::font::TextEngine;

/// 一个 32 位像素（存储为 `0xAARRGGBB`，适后直接交给 GDI）。
type Px = u32;

#[inline]
fn pack(c: Color, cov: f32) -> Px {
    let a = (c.a.clamp(0.0, 1.0) * cov.clamp(0.0, 1.0) * 255.0).round() as u32;
    let r = (c.r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let g = (c.g.clamp(0.0, 1.0) * 255.0).round() as u32;
    let b = (c.b.clamp(0.0, 1.0) * 255.0).round() as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

#[inline]
fn blend(dst: Px, src: Px) -> Px {
    let sa = (src >> 24) & 0xff;
    if sa == 0 {
        return dst;
    }
    if sa == 255 {
        return src;
    }
    let inv = 255 - sa;
    let rb = |shift: u32| -> u32 {
        let s = (src >> shift) & 0xff;
        let d = (dst >> shift) & 0xff;
        (s * sa + d * inv + 127) / 255
    };
    let a = sa + ((dst >> 24) & 0xff) * inv / 255;
    (a << 24) | (rb(16) << 16) | (rb(8) << 8) | rb(0)
}

/// 圆角矩形的 SDF：`q = |p - center| - half + r`，`d = min(max(qx,qy),0)+|max(q,0)|-r`。
#[inline]
fn round_sdf(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - hw + r;
    let qy = (py - cy).abs() - hh + r;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    qx.max(qy).min(0.0) + outside - r
}

/// 纯 CPU 后端。
pub struct SoftBackend {
    hwnd: HWND,
    width: u32,
    height: u32,
    /// 帧缓冲：`0xAARRGGBB`。
    buf: Vec<Px>,
    clips: Vec<Rect>,
    text: TextEngine,
    cell: Size,
    theme: Theme,
    /// 本帧清屏用的背景色（与 GPU 清屏色一致：0.02,0.02,0.03）。
    clear_color: Color,
}

impl SoftBackend {
    /// 创建软后端（同时创建 DirectWrite 字形引擎）。
    ///
    /// # Safety
    ///
    /// `hwnd` 必须是调用线程拥有的有效窗口。
    pub unsafe fn new(
        hwnd: HWND,
        width: u32,
        height: u32,
        theme: Option<Theme>,
        font_family: &str,
    ) -> Result<Self, String> {
        // cell 尺寸与 D3D 后端保持一致（否则同一布局在两个后端下排版不同）
        const FONT_SIZE: f32 = 15.0;
        let (cw, ch) = super::font::cell_metrics_for(FONT_SIZE);
        let text = TextEngine::new(font_family, FONT_SIZE, cw, ch)?;
        let cell = Size::new(cw, ch);
        Ok(Self {
            hwnd,
            width: width.max(1),
            height: height.max(1),
            buf: vec![0; (width.max(1) * height.max(1)) as usize],
            clips: Vec::new(),
            text,
            cell,
            theme: theme.unwrap_or(Theme::DARK),
            clear_color: Color::rgba(0.02, 0.02, 0.03, 1.0),
        })
    }

    /// 无窗口构造（仅测试用）：不能 `present`，但光栅化行为完全一致。
    #[doc(hidden)]
    pub fn new_headless(width: u32, height: u32) -> Result<Self, String> {
        let (cw, ch) = super::font::cell_metrics_for(15.0);
        let text = TextEngine::new("Segoe UI", 15.0, cw, ch)?;
        let (w, h) = (width.max(1), height.max(1));
        Ok(Self {
            hwnd: std::ptr::null_mut(),
            width: w,
            height: h,
            buf: vec![0; (w * h) as usize],
            clips: Vec::new(),
            text,
            cell: Size::new(cw, ch),
            theme: Theme::DARK,
            clear_color: Color::rgba(0.02, 0.02, 0.03, 1.0),
        })
    }

    /// 取某个像素（`0xAARRGGBB`）。越界返回 `None`。
    pub fn pixel(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.buf.get((y * self.width + x) as usize).copied()
    }

    /// 调整尺寸。返回：`true` = 缓冲被重建（内容丢失，调用方需整窗重绘）。
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let (w, h) = (width.max(1), height.max(1));
        if w == self.width && h == self.height {
            return false;
        }
        self.width = w;
        self.height = h;
        self.buf = vec![0; (w * h) as usize];
        true
    }

    /// 切换主题（只影响后续绘制的槽位色）。
    pub fn set_theme(&mut self, theme: Theme) -> Result<(), String> {
        self.theme = theme;
        Ok(())
    }

    /// 切换字体：建新的字形引擎（尺寸变化 → 重排）。
    pub fn set_font(&mut self, family: &str, size: f32) -> Result<(), String> {
        let (cw, ch) = super::font::cell_metrics_for(size);
        self.text = TextEngine::new(family, size, cw, ch)?;
        self.cell = Size::new(cw, ch);
        Ok(())
    }

    /// 清屏（整张缓冲）。
    pub fn clear(&mut self) {
        let px = pack(self.clear_color, 1.0);
        self.buf.fill(px);
    }

    /// 当前帧缓冲尺寸。
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 当前帧缓冲高度。
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 像素内存（BGRA 字节序，GDI 直接可用）。
    pub fn pixels(&self) -> &[u8] {
        // SAFETY: `Px` 是 `u32`，`u32` 的内存布局就是 4 个字节；
        // 这里只是重解释成字节切片，不修改容器。
        unsafe {
            std::slice::from_raw_parts(
                self.buf.as_ptr() as *const u8,
                self.buf.len() * std::mem::size_of::<Px>(),
            )
        }
    }

    /// 整帧像素的 FNV-1a 哈希（供确定性对比）。
    pub fn frame_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.pixels() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// 把帧缓冲贴到窗口客户区（GDI `StretchDIBits`）。
    ///
    /// 返回：`true` = 成功提交。
    /// # Safety
    ///
    /// 必须在拥有窗口的线程上调用，且 `hwnd` 仍有效。
    pub unsafe fn present(&mut self) -> bool {
        let hdc = GetDC(self.hwnd);
        if hdc.is_null() {
            return false;
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: self.width as i32,
            // 负高度 = 从上到下的 DIB（与我们的行布局一致）
            biHeight: -(self.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let ok = StretchDIBits(
            hdc,
            0,
            0,
            self.width as i32,
            self.height as i32,
            0,
            0,
            self.width as i32,
            self.height as i32,
            self.buf.as_ptr() as *const core::ffi::c_void,
            &bmi,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        ReleaseDC(self.hwnd, hdc);
        ok > 0
    }

    #[inline]
    fn clip_rect(&self, r: Rect) -> Rect {
        let mut out = Rect::new(
            r.x.max(0.0),
            r.y.max(0.0),
            r.w.max(0.0),
            r.h.max(0.0),
        );
        out.w = out.w.min(self.width as f32 - out.x);
        out.h = out.h.min(self.height as f32 - out.y);
        if let Some(c) = self.clips.last() {
            out = out.intersect(*c);
        }
        out
    }
}


impl TextMetrics for SoftBackend {
    fn measure_text(&self, text: &str, _style: &TextStyle) -> Size {
        let cells: f32 = text.chars().map(|c| self.text.cell_span(c).max(1) as f32).sum();
        Size::new(cells * self.cell.w, self.cell.h)
    }
    fn line_height(&self, _style: &TextStyle) -> f32 {
        self.cell.h
    }
    fn scale(&self) -> f32 {
        1.0
    }
}

impl RenderCtx for SoftBackend {
    fn size(&self) -> Size {
        Size::new(self.width as f32, self.height as f32)
    }

    fn theme(&self) -> &Theme {
        &self.theme
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let r = self.clip_rect(rect);
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let px = pack(color, 1.0);
        let x0 = r.x.floor() as i32;
        let x1 = r.right().ceil() as i32;
        let y0 = r.y.floor() as i32;
        let y1 = r.bottom().ceil() as i32;
        for y in y0..y1 {
            let row = (y as u32 * self.width) as usize;
            for x in x0..x1 {
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    *dst = blend(*dst, px);
                }
            }
        }
    }

    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32) {
        let w = width.max(1.0);
        self.fill_rect(Rect::new(rect.x, rect.y, rect.w, w), color);
        self.fill_rect(Rect::new(rect.x, rect.bottom() - w, rect.w, w), color);
        self.fill_rect(Rect::new(rect.x, rect.y, w, rect.h), color);
        self.fill_rect(Rect::new(rect.right() - w, rect.y, w, rect.h), color);
    }

    fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
        let mut pen_x = origin.x;
        let start = pen_x;
        let cell_w = self.cell.w;
        let ascent = self.text.ascent;
        for ch in text.chars() {
            let span = self.text.cell_span(ch).max(1) as f32;
            if style.bg.a > 0.0 {
                self.fill_rect(
                    Rect::new(pen_x, origin.y, span * cell_w, self.cell.h),
                    style.bg,
                );
            }
            if ch != ' ' {
                // 字形贴图：从 TextEngine 的 RGBA 图集抽 coverage → 按颜色混合
                if let Some(g) = self.text.glyph(ch).cloned() {
                    if !g.is_empty() {
                        let atlas_size = self.text.atlas_size as usize;
                        let bx = (pen_x + g.bearing_x as f32).floor() as i32;
                        let by = (origin.y + ascent + g.bearing_y as f32).floor() as i32;
                        for gy in 0..g.height as i32 {
                            let dy = by + gy;
                            if dy < 0 || dy >= self.height as i32 {
                                continue;
                            }
                            for gx in 0..g.width as i32 {
                                let dx = bx + gx;
                                if dx < 0 || dx >= self.width as i32 {
                                    continue;
                                }
                                if let Some(clip) = self.clips.last() {
                                    if !clip.contains(Pos::new(dx as f32 + 0.5, dy as f32 + 0.5)) {
                                        continue;
                                    }
                                }
                                let ax = g.atlas_x as usize + gx as usize;
                                let ay = g.atlas_y as usize + gy as usize;
                                let si = (ay * atlas_size + ax) * 4;
                                let cov = self.text.atlas.get(si).copied().unwrap_or(0) as f32 / 255.0;
                                if cov <= 0.0 {
                                    continue;
                                }
                                let px = pack(style.fg, cov);
                                let di = dy as u32 * self.width + dx as u32;
                                if let Some(dst) = self.buf.get_mut(di as usize) {
                                    *dst = blend(*dst, px);
                                }
                            }
                        }
                    }
                }
            }
            if style.underline {
                self.fill_rect(
                    Rect::new(pen_x, origin.y + self.cell.h - 2.0, span * cell_w, 1.0),
                    style.fg,
                );
            }
            pen_x += span * cell_w;
        }
        pen_x - start
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

    fn fill_round_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
        if r <= 0.5 {
            self.fill_rect(rect, color);
            return;
        }
        let area = self.clip_rect(rect);
        if area.w <= 0.0 || area.h <= 0.0 {
            return;
        }
        let cx = rect.x + rect.w * 0.5;
        let cy = rect.y + rect.h * 0.5;
        let hw = rect.w * 0.5;
        let hh = rect.h * 0.5;
        // 抗锯：1px 过渡带，与 GPU 侧 `soft = 1.0` 一致
        let soft = 1.0;
        let y0 = area.y.floor() as i32;
        let y1 = area.bottom().ceil() as i32;
        let x0 = area.x.floor() as i32;
        let x1 = area.right().ceil() as i32;
        for y in y0..y1 {
            let row = (y as u32 * self.width) as usize;
            let py = y as f32 + 0.5;
            for x in x0..x1 {
                let px_pos = x as f32 + 0.5;
                let d = round_sdf(px_pos, py, cx, cy, hw, hh, r);
                let cov = (0.5 - d / soft).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let px = pack(color, cov);
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    *dst = blend(*dst, px);
                }
            }
        }
    }

    fn fill_gradient_rect_dir(&mut self, rect: Rect, from: Color, to: Color, dir: (f32, f32)) {
        let area = self.clip_rect(rect);
        if area.w <= 0.0 || area.h <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let (dx, dy) = dir;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            self.fill_rect(rect, from);
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        // 递增式 t：每行一次乘法，每像素一次加法。
        // 之前每像素两次除法 + 一次构造 Color，1280x800 要 14.5ms；
        // 改成递增后降到毫秒级。
        let step_x = ux / rect.w;
        let inv_w = 1.0 / rect.w;
        let inv_h = 1.0 / rect.h;
        let (r0, g0, b0, a0) = (from.r, from.g, from.b, from.a);
        let (dr, dg, db, da) = (to.r - r0, to.g - g0, to.b - b0, to.a - a0);
        let y0 = area.y.floor() as i32;
        let y1 = area.bottom().ceil() as i32;
        let x0 = area.x.floor() as i32;
        let x1 = area.right().ceil() as i32;
        for y in y0..y1 {
            let row = (y as u32 * self.width) as usize;
            let ly = (y as f32 + 0.5 - rect.y) * inv_h;
            let mut t = ((x0 as f32 + 0.5 - rect.x) * inv_w) * ux + ly * uy;
            let opaque = a0 >= 1.0 && da == 0.0;
            for x in x0..x1 {
                let tt = t.clamp(0.0, 1.0);
                let r = (r0 + dr * tt).clamp(0.0, 1.0);
                let g = (g0 + dg * tt).clamp(0.0, 1.0);
                let b = (b0 + db * tt).clamp(0.0, 1.0);
                let px = if opaque {
                    // 不透明渐变是常见情况：直接写，省掉混合的三次除法
                    (255u32 << 24)
                        | ((r * 255.0 + 0.5) as u32) << 16
                        | ((g * 255.0 + 0.5) as u32) << 8
                        | (b * 255.0 + 0.5) as u32
                } else {
                    pack(
                        Color::rgba(r, g, b, a0 + da * tt),
                        1.0,
                    )
                };
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    if opaque {
                        *dst = px;
                    } else {
                        *dst = blend(*dst, px);
                    }
                }
                t += step_x;
            }
        }
    }

    fn fill_gradient_round_rect(
        &mut self,
        rect: Rect,
        from: Color,
        to: Color,
        dir: (f32, f32),
        radius: f32,
    ) {
        // 先画圆角不透明遮罩再叠渐变：保证角上的抗锯与纯圆角一致
        let r = radius.clamp(0.0, rect.w.min(rect.h) * 0.5);
        if r <= 0.5 {
            self.fill_gradient_rect_dir(rect, from, to, dir);
            return;
        }
        let area = self.clip_rect(rect);
        if area.w <= 0.0 || area.h <= 0.0 {
            return;
        }
        let cx = rect.x + rect.w * 0.5;
        let cy = rect.y + rect.h * 0.5;
        let (hw, hh) = (rect.w * 0.5, rect.h * 0.5);
        let (dx, dy) = dir;
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = if len < 1e-6 { (0.0, 1.0) } else { (dx / len, dy / len) };
        let soft = 1.0;
        for y in area.y.floor() as i32..area.bottom().ceil() as i32 {
            let row = (y as u32 * self.width) as usize;
            let py = y as f32 + 0.5;
            let ly = (py - rect.y) / rect.h;
            for x in area.x.floor() as i32..area.right().ceil() as i32 {
                let px_pos = x as f32 + 0.5;
                let d = round_sdf(px_pos, py, cx, cy, hw, hh, r);
                let cov = (0.5 - d / soft).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let lx = (px_pos - rect.x) / rect.w;
                let t = (lx * ux + ly * uy).clamp(0.0, 1.0);
                let c = Color::rgba(
                    from.r + (to.r - from.r) * t,
                    from.g + (to.g - from.g) * t,
                    from.b + (to.b - from.b) * t,
                    from.a + (to.a - from.a) * t,
                );
                let px = pack(c, cov);
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    *dst = blend(*dst, px);
                }
            }
        }
    }
}

/// 未使用的导入占位（`Edges` 在未来的文本布局里会用到）。
#[allow(dead_code)]
fn _unused(_e: Edges) {}
