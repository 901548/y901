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

use y901_ui::theme::Theme;
use y901_ui::{Color, Edges, Pos, Rect, RenderCtx, Size, TextMetrics, TextStyle};

use crate::font::FontEngine;

/// 一个 32 位像素（存储为 `0xAARRGGBB`，适后直接交给 GDI）。
type Px = u32;

#[inline]
fn pack(c: Color, cov: f32, opacity: f32) -> Px {
    let a = (c.a.clamp(0.0, 1.0) * cov.clamp(0.0, 1.0) * opacity.clamp(0.0, 1.0) * 255.0).round()
        as u32;
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
    width: u32,
    height: u32,
    /// 帧缓冲：`0xAARRGGBB`。
    buf: Vec<Px>,
    /// 剪辀栈 / 变换栈 / 悬停 / 主题 / 动画：与 D3D 后端共用。
    state: crate::render_state::RenderState,
    textures: std::collections::HashMap<y901_ui::image::TextureId, (u32, u32, Vec<u8>, u32)>,
    /// 静态图片的 mip 级（**只存 L1 及以上**：L0 就是 `textures` 里的原始字节）。
    ///
    /// 为何不存 L0：存一份就多占 `w*h*4` 字节，而 `level == 0` 的路径**必须**和改造前
    /// 逐字节相同（否则的哈希门禁会报“渲染回归”但实际上什么都没变）。
    mips: std::collections::HashMap<y901_ui::image::TextureId, Vec<(u32, u32, Vec<u8>)>>,
    next_texture: u64,
    text: Box<dyn super::font::FontEngine>,
    cell: Size,
    /// **DPI 缩放（物理 / 逻辑）**。默认 1.0（X11 / 测试），
    /// 由平台层在窗口创建后注入（Windows 软渲染 fallback 也注入）。
    dpi_scale: f32,
    /// 本帧清屏用的背景色（与 GPU 清屏色一致：0.02,0.02,0.03）。
    clear_color: Color,

    /// **是否按字体的真实字宽推进**（比例字体用）。
    ///
    /// 默认 `false` = **格宽布局**（所有 ASCII 占 1 格）——桌面与 GPU 路径因此逐像素不变。
    /// web 端启用后字符按字体自身的 `hmtx` 推进，比例字体才不会挤在一起。
    proportional_text: bool,
}

/// **字形顶部的 y 坐标**（buffer 坐标，y 向下）。
///
/// 引擎的 `GlyphInfo::bearing_y` 是有符号的**位图底边**偏移（y 向上）：
/// `'8'` = 0（底边贴基线）、`'设'` = -2（有 2px 下伸）。它不是"顶部偏移"。
///
/// 顶部 = 基线 − bearing_y − 字形高度，基线 = origin.y + ascent。
/// 旧代码用 `origin.y + ascent + bearing_y`，把字形整体画低了一个字高；
/// 底部溢出裁剪区后，因为画面在 present 层转置，表现为**顶部被切**。
fn glyph_top(origin_y: f32, ascent: f32, bearing_y: f32, height: f32) -> f32 {
    // **行级 baseline 只取整一次**：之后全是整数运算。
    //
    // `bearing_y` **原样使用**：它是字形底边相对基线的位置
    // （向下为负），包含圆形字母的 1px overshoot、下伸字的 -4~-5、
    // 标点的自身偏移。**归零它会让冒号浮空、中英错位**（上一轮的错误）。
    (origin_y + ascent).round() - bearing_y - height
}

impl SoftBackend {
    /// 设置 **DPI 缩放比**（物理 / 逻辑）。
    ///
    /// 由平台层在窗口创建后调用；`y901-render` 本身不知道 DPI。
    /// **开关：是否按字体真实字宽推进**。
    ///
    /// 开：比例字体（微软雅黑等）按 `hmtx` 推进，字距自然；
    /// 关（默认）：格宽布局，桌面/帧哈希不受影响。
    pub fn set_proportional_text(&mut self, on: bool) {
        self.proportional_text = on;
    }

    /// 当前一个字符的推进宽度（像素）。
    ///
    /// `proportional_text=false` 时一律返回 `格数 × 格宽`（与旧行为一致）。
    fn advance_of(&self, ch: char, px: f32, cell_w: f32) -> f32 {
        let grid = self.text.cell_span(ch) as f32 * cell_w;
        if !self.proportional_text {
            return grid;
        }
        let adv = self.text.advance(ch, px);
        // **保底**：算出 0 / 负数 / 非法值时回退到格宽。
        // 不然一旦 advance 出错，所有字就叠在同一个 X 上（看起来像乱码）。
        if !(adv.is_finite() && adv > 0.5) {
            return grid;
        }
        adv
    }

    /// 设置 **DPI 缩放比**（物理 / 逻辑），由平台层在窗口创建后调用。
    pub fn set_dpi_scale(&mut self, scale: f32) {
        self.dpi_scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
    }

    /// 当前纹理表大小（测试与诊断用）。
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// 某张纹理的引用计数（测试与诊断用）。
    pub fn texture_refcount(&self, id: y901_ui::image::TextureId) -> Option<u32> {
        self.textures.get(&id).map(|e| e.3)
    }

    /// 创建软后端（同时创建 DirectWrite 字形引擎）。
    ///
    /// # Safety
    ///
    /// `hwnd` 必须是调用线程拥有的有效窗口。
    pub fn new(
        width: u32,
        height: u32,
        font: Box<dyn FontEngine>,
        cell: Size,
        theme: Option<Theme>,
    ) -> Result<Self, String> {
        let _ = Size::new(0.0, 0.0);
        Ok(Self {
            width: width.max(1),
            height: height.max(1),
            buf: vec![0; (width.max(1) * height.max(1)) as usize],
            textures: std::collections::HashMap::new(),
            mips: std::collections::HashMap::new(),
            next_texture: 0,
            text: font,
            cell,
            state: crate::render_state::RenderState::new(theme.unwrap_or(Theme::DARK)),
            clear_color: Color::rgba(0.02, 0.02, 0.03, 1.0),
            dpi_scale: 1.0,
            proportional_text: false,
        })
    }

    /// 无窗口构造（仅测试用）：不能 `present`，但光栅化行为完全一致。
    #[doc(hidden)]
    pub fn new_headless(
        width: u32,
        height: u32,
        font: Box<dyn FontEngine>,
        cell: Size,
    ) -> Result<Self, String> {
        let (w, h) = (width.max(1), height.max(1));
        Ok(Self {
            width: w,
            height: h,
            buf: vec![0; (w * h) as usize],
            state: crate::render_state::RenderState::new(Theme::DARK),
            textures: std::collections::HashMap::new(),
            mips: std::collections::HashMap::new(),
            next_texture: 0,
            text: font,
            cell,
            clear_color: Color::rgba(0.02, 0.02, 0.03, 1.0),
            dpi_scale: 1.0,
            proportional_text: false,
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
        self.state.set_theme(theme);
        Ok(())
    }

    /// 切换字体：建新的字形引擎（尺寸变化 → 重排）。
    pub fn set_font(&mut self, engine: Box<dyn FontEngine>, cell: Size) {
        self.text = engine;
        self.cell = cell;
    }

    /// 清屏（整张缓冲）。
    ///
    /// **同时清空裁剪栈与变换栈**：清屏是"新一帧的起点"，
    /// 上一帧遗留的裁剪区不应跨帧生效（否则一次泄漏会让
    /// **之后每一帧**都只剩一个小矩形可见，就是那个全黑现象）。
    pub fn clear(&mut self) {
        let px = pack(self.clear_color, 1.0, 1.0);
        self.buf.fill(px);
        // 防御式：任何组件的 push_clip 不配对都不会污染到下一帧。
        self.state.clear_clips();
    }

    /// 当前帧缓冲尺寸。
    pub fn width(&self) -> u32 {
        self.width
    }

    /// 当前帧缓冲高度。
    pub fn height(&self) -> u32 {
        self.height
    }

    /// 把一帧 **BGRA** 字节拷贝进帧缓冲（供 `Presenter::present` 使用）。
    ///
    /// 为什么不直接换成 `Vec<u32>`：底层已经是 `u32`，而 `present` 只拿到字节切片；
    /// 这里做一次无分配的按字节拷贝（不转换颜色通道顺序，因为它们本来就一致）。
    pub fn blit(&mut self, pixels: &[u8], width: u32, height: u32) {
        if self.resize(width, height) {
            // 尺寸变了 → 缓冲区已重建；接下来全量拷贝即可
        }
        let need = (width as usize) * (height as usize) * 4;
        if pixels.len() < need {
            return;
        }
        // SAFETY: `buf` 是 `u32` 数组，`pixels` 至少 `need` 字节，
        // 且两边长度一致；这是按字节拷贝到已分配的同尺寸内存。
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.buf.as_mut_ptr() as *mut u8, need);
        }
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
    #[inline]
    /// 叠代复合变换：从最内层开始得到屏幕坐标。
    /// 变换栈的**缩放乘积**（不含平移）。
    ///
    /// 用于选 mip 级：嵌套的变换会逐层相乘，所以这里也要相乘。
    /// 没有变换时返回 1.0（调用方无需特别判断）。
    fn map_scale(&self) -> f32 {
        let mut s = 1.0f32;
        for (_, scale) in self.state.transforms() {
            s *= *scale;
        }
        if s.is_finite() && s > 0.0 {
            s
        } else {
            1.0
        }
    }

    fn map_point(&self, p: y901_ui::Pos) -> y901_ui::Pos {
        let mut x = p.x;
        let mut y = p.y;
        for (o, s) in self.state.transforms().iter().rev() {
            x = o.x + (x - o.x) * s;
            y = o.y + (y - o.y) * s;
        }
        y901_ui::Pos::new(x, y)
    }

    /// 变换一个矩形（位置与尺寸同时缩放）。
    fn map_rect(&self, r: Rect) -> Rect {
        if self.state.transforms_empty() {
            return r;
        }
        let a = self.map_point(y901_ui::Pos::new(r.x, r.y));
        let b = self.map_point(y901_ui::Pos::new(r.right(), r.bottom()));
        Rect::new(a.x, a.y, (b.x - a.x).max(0.0), (b.y - a.y).max(0.0))
    }

    /// 双线性采样（像素中心对齐）。
    ///
    /// 为什么要 `-0.5`：直接用 `sx` 当索引会把边缘采到"半个像素外"，
    /// 2×2 纹理放大到 20×20 时四角颜色会明显偏 —— 这是最容易踩的 off-by-half。
    fn sample_bilinear_tex(data: (&[u8], u32, u32), sx: f32, sy: f32) -> [f32; 4] {
        let (data, w, h) = data;
        if w == 0 || h == 0 || data.is_empty() {
            return [0.0; 4];
        }
        let (wf, hf) = (w as f32, h as f32);
        let x = (sx - 0.5).clamp(0.0, wf - 1.0);
        let y = (sy - 0.5).clamp(0.0, hf - 1.0);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let x1 = (x0 + 1.0).min(wf - 1.0);
        let y1 = (y0 + 1.0).min(hf - 1.0);
        let at = |px: f32, py: f32| -> [f32; 4] {
            let i = ((py as u32 * w + px as u32) * 4) as usize;
            let d = |k: usize| data.get(i + k).copied().unwrap_or(0) as f32 / 255.0;
            [d(0), d(1), d(2), d(3)]
        };
        let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
        let mut out = [0.0f32; 4];
        for k in 0..4 {
            let top = p00[k] * (1.0 - fx) + p10[k] * fx;
            let bottom = p01[k] * (1.0 - fx) + p11[k] * fx;
            out[k] = top * (1.0 - fy) + bottom * fy;
        }
        out
    }

    fn clip_rect(&self, r: Rect) -> Rect {
        let mut out = Rect::new(r.x.max(0.0), r.y.max(0.0), r.w.max(0.0), r.h.max(0.0));
        out.w = out.w.min(self.width as f32 - out.x);
        out.h = out.h.min(self.height as f32 - out.y);
        if let Some(c) = self.state.current_clip() {
            out = out.intersect(c);
        }
        out
    }
}

impl TextMetrics for SoftBackend {
    fn measure_text(&self, text: &str, style: &TextStyle) -> Size {
        // **per-text 字号**：`Some(px)` 时按该字号**现算 cell**（不按比例缩放，
        // 避免 CJK/等宽混排的亚像素误差）；`None` 走引擎默认 cell —— 与加字段前逐像素一致。
        let cell = match style.size {
            Some(px) if px > 0.0 => {
                // **per-text 字号乘 DPI**：`px` 是逻辑像素，引擎需物理像素。
                let (w, h) = crate::font::cell_metrics_for(px * self.dpi_scale);
                Size::new(w, h)
            }
            _ => self.cell,
        };
        // 格数的**唯一来源**是 `y901_core::Grid::width_of_char`（经 `cell_span`），
        // 此处**不做二次加工**：零宽/组合字符就是 0 格（控制字符由 core 的 `unwrap_or(1)` 兵底为 1）。
        if self.proportional_text {
            // **无显式字号时用引擎基准字号**：
            // 光栅化用的就是它，用 `cell.w`(8) 会算出半宽的 advance，
            // 而字形仍按 16px 画 ⇒ **所有字重叠在一起**。
            let px = match style.size {
                Some(px) if px > 0.0 => px * self.dpi_scale,
                _ => self.text.size_px() * self.dpi_scale,
            };
            let w: f32 = text.chars().map(|c| self.text.advance(c, px)).sum();
            return Size::new(w, cell.h);
        }
        let cells: f32 = text.chars().map(|c| self.text.cell_span(c) as f32).sum();
        Size::new(cells * cell.w, cell.h)
    }
    fn line_height(&self, style: &TextStyle) -> f32 {
        match style.size {
            Some(px) if px > 0.0 => crate::font::cell_metrics_for(px * self.dpi_scale).1,
            _ => self.cell.h,
        }
    }
    fn scale(&self) -> f32 {
        self.dpi_scale
    }
}

impl SoftBackend {
    /// **按字号绘制文本**（per-text 字号）。
    ///
    /// 组合方式（**字号与变换缩放的关系**）：
    /// `px_final = px * map_scale()`，然后**按 `px_final` 光栅化**字形。
    ///
    /// 为何不用"固定字号 + mip 降采样"（旧路径的做法）：
    /// mip 链只能**缩小**（L1=1/2、L2=1/4），而字号可大可小；
    /// 按目标字号重新光栅化能得到**真实轮廓**，这正是引擎已具备的能力。
    ///
    /// 基线：`ascent` 按**基准字号的 ascent/cell_h 比例**换算到新字号
    /// （该比例是字体属性，不随字号变化）——**不是拍脑袋的 0.8**。
    fn draw_text_sized(&mut self, origin: Pos, text: &str, style: &TextStyle, px: f32) -> f32 {
        let mapped = self.map_point(origin);
        let origin = Pos::new(mapped.x, mapped.y);
        let scale = self.map_scale();
        // **DPI × 变换缩放**（两个不同来源，不重复）。
        let px_final = (px * self.dpi_scale * scale).max(1.0);
        let (cell_w, cell_h) = crate::font::cell_metrics_for(px_final);
        // 与旧路径同式：基线 = 顶部 + ascent
        // **引擎返回的就是像素级 ascent**：不能再乘 `cell_h / self.cell.h`。
        // 曾经按比例缩放（cell_h=12, cell.h=16 时 ascent 12.8 → 9.6），
        // 基线上移 → 字形顶部超出 clip 界 → **削顶**。
        let ascent = self.text.ascent();
        let atlas_size = self.text.atlas_size() as usize;
        let mut pen_x = origin.x;
        let start = pen_x;
        for ch in text.chars() {
            // 同上：格数只有一个来源，不做二次加工。
            let span = self.text.cell_span(ch) as f32;
            // **推进宽度**：比例字体用真实字宽，否则按格宽。
            let adv = self.advance_of(ch, px_final, cell_w);
            if style.bg.a > 0.0 {
                self.fill_rect(Rect::new(pen_x, origin.y, adv, cell_h), style.bg);
            }
            if ch != ' ' {
                // 先 clone 再画：避开 `&mut self.text` 与 `self.text.atlas()` 的借用冲突
                let glyph = self.text.glyph_at_size(ch, px_final).cloned();
                if let Some(g) = glyph {
                    if g.width > 0 && g.height > 0 {
                        let bx = (pen_x + g.bearing_x as f32).floor() as i32;
                        // **保证字形顶部不高于 `origin.y`**。
                        // 字形顶 = origin.y + ascent + bearing_y；`bearing_y` 为负且幅度大时
                        // （SimHei 的数字）顶部会跑到 origin.y 之上 → 被裁剪成上半截断
                        // （"8080" → "ouou"）。这里只补"溢出那部分"，不多补。
                        // **不限制裁剪区**：只要碰到 '8' 就记一次（每帧首个）。
                        // `clip` 无时仍记 -1，这样能分辨"压根没压裁剪"这种情况。
                        // **字形顶部**：引擎的 `bearing_y` 是**位图底边**相对基线的位置
                        // （y 向上：'8' = 0 贴基线，'设' = -2 下伸），不是顶部偏移。
                        // 顶部 = 基线 − bearing_y − 字形高度。
                        // **行级 baseline 取整 + 字形原始偏移**（见 `glyph_top`）
                        let by = glyph_top(
                            origin.y,
                            ascent,
                            g.bearing_y as f32,
                            g.height as f32,
                        ) as i32;
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
                                if let Some(clip) = self.state.current_clip() {
                                    if !clip.contains(Pos::new(dx as f32 + 0.5, dy as f32 + 0.5)) {
                                        continue;
                                    }
                                }
                                let ax = g.atlas_x as usize + gx as usize;
                                let ay = g.atlas_y as usize + gy as usize;
                                let si = (ay * atlas_size + ax) * 4;
                                let mut cov =
                                    self.text.atlas().get(si).copied().unwrap_or(0) as f32 / 255.0;
                                if style.bold {
                                    // 与旧路径同：取右侧邻居覆盖度取 max → 向右拓宽 1px
                                    let right = if (gx as u32) + 1 < g.width {
                                        self.text.atlas().get(si + 4).copied().unwrap_or(0) as f32
                                            / 255.0
                                    } else {
                                        0.0
                                    };
                                    cov = cov.max(right);
                                }
                                if cov <= 0.0 {
                                    continue;
                                }
                                let px_src = pack(style.fg, cov, self.state.current_opacity());
                                let di = dy as u32 * self.width + dx as u32;
                                if let Some(dst) = self.buf.get_mut(di as usize) {
                                    *dst = blend(*dst, px_src);
                                }
                            }
                        }
                    }
                }
            }
            if style.underline {
                self.fill_rect(
                    Rect::new(pen_x, origin.y + cell_h - 2.0, adv, 1.0),
                    style.fg,
                );
            }
            let _ = span;
            pen_x += adv;
        }
        pen_x - start
    }
}

impl RenderCtx for SoftBackend {
    fn size(&self) -> Size {
        Size::new(self.width as f32, self.height as f32)
    }

    fn theme(&self) -> &Theme {
        self.state.theme()
    }

    /// 压入一层缩放变换（以 `origin` 为中心）。
    ///
    /// 语义与 `Map` 一致：`p' = origin + (p - origin) * scale`，嵌套时逐层复合。
    fn push_transform(&mut self, origin: Pos, scale: f32) {
        // 缩放为 0 或负数会让矩形退化成 0/反向，直接忽略更安全
        if !(scale.is_finite() && scale > 0.0) {
            return;
        }
        self.state.push_transform(origin, scale);
    }

    fn pop_transform(&mut self) {
        self.state.pop_transform();
    }

    fn is_hovered(&self, id: &str) -> bool {
        self.state.is_hovered(id)
    }

    fn set_hovered(&mut self, id: Option<&str>) {
        self.state.set_hovered(id);
    }

    fn animation(&self, key: u64) -> f32 {
        self.state.animation(key)
    }

    fn set_animations(&mut self, snapshot: &y901_ui::animation::AnimSnapshot) {
        self.state.set_animations(snapshot);
    }

    fn upload_texture(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> y901_ui::image::TextureId {
        if width == 0 || height == 0 || rgba.len() < (width * height * 4) as usize {
            return y901_ui::image::TextureId::INVALID;
        }
        self.next_texture += 1;
        let id = y901_ui::image::TextureId(self.next_texture);
        // 只保留需要的字节：调用方可能持有一个很大的缓冲
        let bytes = rgba[..(width * height * 4) as usize].to_vec();
        // 上传即拥有一次引用（创建者）
        self.textures.insert(id, (width, height, bytes, 1));
        // 静态图片：就地建 mip 链（L1 起）。
        //
        // 存储成本：L0 + L1 + L2 = 1 + 1/4 + 1/16 = **1.3125×**（三级有限链）。
        // 注意 1.333 是无穷级数列 1+1/4+1/16+… 的极限，三级链拿不到那个值。
        let levels = box_levels_rgba(&self.textures[&id].2, width, height, 2);
        if !levels.is_empty() {
            self.mips.insert(id, levels);
        }
        id
    }

    fn retain_texture(&mut self, id: y901_ui::image::TextureId) {
        // 共享纹理（如图标复用）必须先加一次计数，
        // 否则第一个卸载的使用者会把别人还在用的纹理释放掉。
        if let Some(entry) = self.textures.get_mut(&id) {
            entry.3 = entry.3.saturating_add(1);
        }
    }

    fn release_texture(&mut self, id: y901_ui::image::TextureId) {
        // 引用计数归零才真正释放：同一张纹理可能被多个控件共享。
        // 为什么计数放在后端：`TextureId` 对控件是不透明的，控件不应该能知道"还有几个人在用"。
        let free = match self.textures.get_mut(&id) {
            Some(entry) => {
                entry.3 = entry.3.saturating_sub(1);
                entry.3 == 0
            }
            None => false,
        };
        if free {
            self.textures.remove(&id);
            self.mips.remove(&id);
        }
    }

    fn draw_image(
        &mut self,
        rect: Rect,
        texture: y901_ui::image::TextureId,
        style: y901_ui::image::ImageStyle,
    ) {
        // 变换在入口应用一次；采样本身用绝对像素坐标
        let rect = self.map_rect(rect);
        // 只取尺寸（不持有对 `self.textures` 的长作用域借用）。
        let Some((tw, th)) = self.textures.get(&texture).map(|t| (t.0, t.1)) else {
            return;
        };
        if tw == 0 || th == 0 {
            return;
        }

        // 适配模式在 CPU 侧就先化简成"目标子矩形 + 源子矩形"，
        // 采样循环里只做比例映射 —— 避免每条扫描线重算 fit 语义。
        let (dst, src) = style.layout(rect, tw, th);
        if dst.w <= 0.0 || dst.h <= 0.0 || src.w <= 0.0 || src.h <= 0.0 {
            return;
        }

        // 额外裁剪：与目标求交，并按同一比例把裁剪回推到源区域
        let area = match style.clip {
            Some(c) => dst.intersect(c),
            None => dst,
        };
        let area = self.clip_rect(area);
        if area.w <= 0.0 || area.h <= 0.0 {
            return;
        }

        // 预算 uv：`u0/v0` 是目标区域左上角在源图里的归一化坐标，`du/dv` 是每像素增量
        let sx0 = src.x + (area.x - dst.x) / dst.w * src.w;
        let sy0 = src.y + (area.y - dst.y) / dst.h * src.h;
        let du = src.w / dst.w;
        let dv = src.h / dst.h;

        // **零拷贝**：只借用 `self.textures` 这一个字段。
        //
        // 为什么这样能编译：`self.textures` 与下面要写的 `self.buf` 是**不相交字段**，
        // Rust 允许同时存在它们的不可变/可变借用；旧实现先把整个 `self` 借了一轮，
        // 才不得不拷整张纹理（`1024²` → 4 MiB/帧，实测见 `perf_texture_alloc`）。
        // ---- mip 选级（本步骤**强制 level = 0**，只验证映射逻辑不改变渲染）----
        //
        // 实际绘制倍率 = 目标子矩形 / 源子矩形（每 1 源像素占多少屏幕像素）。
        // 两个方向取 **min**：取更细的那个会让缩小较多的轴采样不足而走样，
        // 而走样是**错误**；取更粗的一个只是让另一轴略模糊，是**取舍**。
        let scale_x = dst.w / src.w;
        let scale_y = dst.h / src.h;
        let draw_scale = scale_x.min(scale_y);
        let max_level = self.mips.get(&texture).map(|m| m.len()).unwrap_or(0);
        let computed_level = crate::mip::level_for_image(draw_scale, max_level);
        // 第一步先不开启选级：只证明"选级与坐标映射的引入本身不改变渲染"。
        // 第 2 步：打开选级。`s >= 1` 时 `level_for_image` 返回 0，
        // 因此 s=1.0 的像素必须与第 1 步（强制 level=0）逐字节相同。
        let level = computed_level;
        let shift = 1u32 << level;
        // src 是 L0 像素坐标；选中的级坐标 = src / 2^level。dst 始终是屏幕坐标。

        // L0 字节：仅当 level == 0 时使用（保证与改造前逐字节相同）。
        // **坐标映射**：`sx0/sy0/du/dv` 之前是用 **L0 坐标** 算的，
        // 但下面要从选中的级采样，那张图只有 ·/2^level 的尺寸。
        // 不除的话，坐标会超出级图宽度 → 读到错词的像素（实测 MAD 高达 0.37）。
        let sx0 = sx0 / shift as f32;
        let sy0 = sy0 / shift as f32;
        let du = du / shift as f32;
        let dv = dv / shift as f32;

        let tex_bytes = self
            .textures
            .get(&texture)
            .map(|t| t.2.as_slice())
            .unwrap_or(&[]);
        let (tw, th, tex_bytes) = match self
            .mips
            .get(&texture)
            .and_then(|m| m.get(level.wrapping_sub(1)))
        {
            Some(lv) if level > 0 => (lv.0, lv.1, lv.2.as_slice()),
            _ => (tw / shift, th / shift, tex_bytes),
        };

        let x0 = area.x.floor() as i32;
        let x1 = area.right().ceil() as i32;
        let y0 = area.y.floor() as i32;
        let y1 = area.bottom().ceil() as i32;
        for y in y0..y1 {
            if y < 0 || y >= self.height as i32 {
                continue;
            }
            let py = y as f32 + 0.5;
            if py < area.y || py >= area.bottom() {
                continue;
            }
            let sy = sy0 + (py - area.y) * dv;
            for x in x0..x1 {
                if x < 0 || x >= self.width as i32 {
                    continue;
                }
                let px = x as f32 + 0.5;
                if px < area.x || px >= area.right() {
                    continue;
                }
                let sx = sx0 + (px - area.x) * du;
                let mut c = Self::sample_bilinear_tex((tex_bytes, tw, th), sx, sy);
                if let Some(t) = style.tint {
                    c[0] *= t.r;
                    c[1] *= t.g;
                    c[2] *= t.b;
                    c[3] *= t.a;
                }
                if c[3] <= 0.0 {
                    continue;
                }
                let src_px = pack(Color::rgba(c[0], c[1], c[2], c[3]), 1.0, 1.0);
                let di = (y as u32 * self.width + x as u32) as usize;
                if let Some(dst_px) = self.buf.get_mut(di) {
                    *dst_px = blend(*dst_px, src_px);
                }
            }
        }
    }

    /// 背景色：取主题 panel 槽位的底色（用于弹层淡入適罩）。
    fn background_color(&self) -> Color {
        self.state.background_color()
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let rect = self.map_rect(rect);
        let r = self.clip_rect(rect);
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let px = pack(color, 1.0, self.state.current_opacity());
        let x0 = r.x.floor() as i32;
        let x1 = r.right().ceil() as i32;
        let y0 = r.y.floor() as i32;
        let y1 = r.bottom().ceil() as i32;
        for y in y0..y1 {
            let row = (y as u32 * self.width) as usize;
            let start = row + x0 as usize;
            let end = row + x1 as usize;
            if px >> 24 == 0xff {
                if let Some(dst) = self.buf.get_mut(start..end) {
                    dst.fill(px);
                }
            } else if let Some(dst) = self.buf.get_mut(start..end) {
                for pixel in dst {
                    *pixel = blend(*pixel, px);
                }
            }
        }
    }

    /// 圆角矩形描边：与 D3D shader 同公式的 SDF 覆盖率（包含 1px 过渡）。
    ///
    /// 与 `fill_rect` 的区别：那里 alpha 恒为 1（硬边），这里每个像素都带一个
    /// 覆盖率，所以弧线不再锯齿。
    fn stroke_round_rect(&mut self, rect: Rect, color: Color, width: f32, radius: f32) {
        if width <= 0.0 || rect.is_empty() {
            return;
        }
        let rect = self.map_rect(rect);
        // 与 D3D 一致：有 transform 时 radius 与线宽都要跟随缩放。
        let (radius, width) = if self.state.transforms_empty() {
            (radius, width)
        } else {
            let one = self.map_rect(Rect::new(0.0, 0.0, 1.0, 1.0));
            let s = one.w.clamp(0.0001, 1000.0);
            (radius * s, width * s)
        };
        let r = self.clip_rect(rect);
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let half = (rect.w * 0.5, rect.h * 0.5);
        let cx = rect.x + half.0;
        let cy = rect.y + half.1;
        // 遍历：只扫环带外扩 width/2 + 1 的包围盒（边界像素需要部分覆盖率）。
        let pad = width * 0.5 + 1.0;
        let x0 = ((rect.x - pad).max(r.x)).floor() as i32;
        let x1 = ((rect.right() + pad).min(r.right())).ceil() as i32;
        let y0 = ((rect.y - pad).max(r.y)).floor() as i32;
        let y1 = ((rect.bottom() + pad).min(r.bottom())).ceil() as i32;
        let px_color = color;
        for y in y0..y1 {
            let py = y as f32 + 0.5 - cy; // 像素中心采样，与 shader 的 local 一致
            let row = (y as u32 * self.width) as usize;
            for x in x0..x1 {
                let px = x as f32 + 0.5 - cx;
                // 便宜剪枝：远离环带的像素直接跳过
                if px.abs() > half.0 + pad || py.abs() > half.1 + pad {
                    continue;
                }
                let d = crate::sdf::rounded_rect_distance((px, py), half, radius);
                let cov = crate::sdf::stroke_coverage(d, width, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    *dst = blend(*dst, pack(px_color, cov, self.state.current_opacity()));
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
        // **per-text 字号路径**：字号 × 变换缩放 = 最终像素字号，按该字号**光栅化**（不拉伸位图）。
        // 这是新分支；**下面的旧路径逐字未动**，`size` 为 `None` 时行为与改前逐像素相同。
        if let Some(px) = style.size {
            if px.is_finite() && px > 0.0 {
                return self.draw_text_sized(origin, text, style, px);
            }
        }
        // 文本第一版**只跟随平移，不缩放**（缩放需要重新光栅化或重采样）。
        // 这是已知限制，已写在报告与文档里。
        let mapped = self.map_point(origin);
        let origin = Pos::new(mapped.x, mapped.y);
        // 变换栈的缩放乘积：决定选哪一级 mip，也决定字形末尺寸
        let scale = self.map_scale();
        let mut pen_x = origin.x;
        let start = pen_x;
        let cell_w = self.cell.w;
        let ascent = self.text.ascent();
        for ch in text.chars() {
            // 格数只有一个来源（`width_of_char`）。**span = 0 时仍绘制字形、pen 不推进** ——
            // 组合字符（如 U+0301）必须画在前一个字上面才能显示重音。
            let span = self.text.cell_span(ch) as f32;
            // **推进宽度**：比例字体用真实字宽（否则按格宽）。
            // 旧路径的字形按**基准字号 × 变换缩放**绘制，advance 必须同比例。
            let base_px = self.text.size_px();
            let adv = self.advance_of(ch, base_px, cell_w) * scale;
            if style.bg.a > 0.0 {
                self.fill_rect(Rect::new(pen_x, origin.y, adv, self.cell.h), style.bg);
            }
            if ch != ' ' {
                // 按当前缩放选一级字形（level 0 = L0，与不缩放时逐像素相同）
                if let Some(sg) = self.text.glyph_at_scale(ch, scale) {
                    if sg.w > 0 && sg.h > 0 {
                        let atlas_size = self.text.atlas_size() as usize;
                        // **目标尺寸与 bearing 的来源必须和实际用的级一致**：
                        //
                        // - `level > 0`：slot **就是**按目标尺寸重新光栅化的位图
                        //   （L1=半尺寸、L2=四分之一），所以 1:1 贴过去最准，
                        //   它的 `bearing` 也已经在目标像素坐标里。
                        //   曾经无论哪一级都按 `L0 尺寸 × scale` 重采样：当目标 6 行、
                        //   源 7 行时 `gy * sh / height` 取不到最后一行，'A' 的
                        //   底边整行丢失（字形被削平），且 `bearing_y` 被二次缩放。
                        // - `level == 0`：slot 是 L0，必须按 `scale` 缩放采样。
                        let (dw, dh, bear_x, bear_y) = if sg.level > 0 {
                            (
                                sg.slot.width as i32,
                                sg.slot.height as i32,
                                sg.slot.bearing_x,
                                sg.slot.bearing_y,
                            )
                        } else {
                            (
                                (sg.w as f32 * scale).round().max(1.0) as i32,
                                (sg.h as f32 * scale).round().max(1.0) as i32,
                                sg.slot.bearing_x * scale,
                                sg.slot.bearing_y * scale,
                            )
                        };
                        // 基线：布局始终用原字号的 bearing，屏幕位置再乘缩放因子
                        let bx = (pen_x + bear_x).floor() as i32;
                        // **字形顶部**（见 `glyph_top`）：布局始终用 L0 字号的 ascent，
                        // 基线 = origin.y + ascent × scale；字形高度按 scale 缩放。
                        // `bearing_y` 是底边偏移，不能当顶部用。
                        let by = glyph_top(origin.y, ascent * scale, bear_y, dh as f32) as i32;
                        let g = crate::GlyphInfo {
                            width: dw as u32,
                            height: dh as u32,
                            bearing_x: 0,
                            bearing_y: 0,
                            advance: sg.advance,
                            atlas_x: sg.slot.atlas_x,
                            atlas_y: sg.slot.atlas_y,
                            last_used: 0,
                            ink: 0.0,
                            mips: [None, None, None],
                        };
                        // slot 实际像素尺寸（用于采样归一化）
                        let (sw, sh) = (sg.slot.width.max(1), sg.slot.height.max(1));
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
                                if let Some(clip) = self.state.current_clip() {
                                    if !clip.contains(Pos::new(dx as f32 + 0.5, dy as f32 + 0.5)) {
                                        continue;
                                    }
                                }
                                // 从 slot 取像素：目标像素映射到级内坐标
                                let sx = (gx as u32 * sw / g.width).min(sw - 1);
                                let sy = (gy as u32 * sh / g.height).min(sh - 1);
                                let ax = sg.slot.atlas_x as usize + sx as usize;
                                let ay = sg.slot.atlas_y as usize + sy as usize;
                                let si = (ay * atlas_size + ax) * 4;
                                let mut cov =
                                    self.text.atlas().get(si).copied().unwrap_or(0) as f32 / 255.0;
                                if style.bold {
                                    // 合成粗体（synthetic bold）：把当前像素与"左侧一列"的覆盖度取最大值，
                                    // 等价于把字形向右拓宽 1px。
                                    //
                                    // 为什么用 max 而不是"画两遍 blend"：两次混合会把半透明边缘叠加成更黑，
                                    // 字形边缘会发模糊；max 只拓宽、不加深，边缘仍保持单次抗锯。
                                    // 取**右侧邻居**的覆盖度取最大值 → 字形向右拓宽 1px。
                                    // （取左邻居是错的：它只会把已有的墨再加深，字形几乎不变宽）。
                                    let right = if (gx as u32) + 1 < g.width && sx + 1 < sw {
                                        self.text.atlas().get(si + 4).copied().unwrap_or(0) as f32
                                            / 255.0
                                    } else {
                                        0.0
                                    };
                                    cov = cov.max(right);
                                }
                                if cov <= 0.0 {
                                    continue;
                                }
                                let px = pack(style.fg, cov, self.state.current_opacity());
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
                    Rect::new(pen_x, origin.y + self.cell.h - 2.0, adv, 1.0),
                    style.fg,
                );
            }
            let _ = span;
            pen_x += adv;
        }
        pen_x - start
    }

    /// 压入一层不透明度（委托给 RenderState，嵌套乘积）。
    fn push_opacity(&mut self, opacity: f32) {
        self.state.push_opacity(opacity);
    }

    /// 弹出栈顶不透明度。
    fn pop_opacity(&mut self) {
        self.state.pop_opacity();
    }

    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.state.current_clip() {
            Some(outer) => rect.intersect(outer),
            None => rect,
        };
        self.state.push_clip(clipped);
    }

    fn pop_clip(&mut self) {
        self.state.pop_clip();
    }

    fn fill_round_rect(&mut self, rect: Rect, color: Color, radius: f32) {
        let rect = self.map_rect(rect);
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
                let px = pack(color, cov, self.state.current_opacity());
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    *dst = blend(*dst, px);
                }
            }
        }
    }

    fn fill_gradient_rect_dir(&mut self, rect: Rect, from: Color, to: Color, dir: (f32, f32)) {
        let rect = self.map_rect(rect);
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
            let opaque = a0 >= 1.0 && da == 0.0 && self.state.current_opacity() >= 1.0;
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
                        self.state.current_opacity(),
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
        let (ux, uy) = if len < 1e-6 {
            (0.0, 1.0)
        } else {
            (dx / len, dy / len)
        };
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
                let px = pack(c, cov, self.state.current_opacity());
                let i = row + x as usize;
                if let Some(dst) = self.buf.get_mut(i) {
                    *dst = blend(*dst, px);
                }
            }
        }
    }
}

/// 未使用的导入占位（`Edges` 在未来的文本布局里会用到）。
// 测试 API：`_unused`：类型见证，保证 `Edges` 仍被引用
// 归类：编译见证 —— `_unused` 证明 Edges 被使用
#[allow(dead_code)]
fn _unused(_e: Edges) {}

/// 对 RGBA 位图做 box 降采样，生成 L1..L`levels`（**不含 L0**）。
///
/// 为何不复用 `mip::MipChain`：它的 `MipLevel` 是**单通道 coverage**（字形用），
/// 而图片是交错的 RGBA；对交错字节做平均会**把颜色通道混在一起**。
/// 所以这里按 4 个通道分别求平均。
fn box_levels_rgba(base: &[u8], w: u32, h: u32, levels: usize) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let (mut cw, mut ch) = (w, h);
    let mut cur = base.to_vec();
    for _ in 0..levels {
        if cw <= 1 && ch <= 1 {
            break;
        }
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0u32; 4];
                let mut n = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = (x * 2 + dx).min(cw - 1);
                        let sy = (y * 2 + dy).min(ch - 1);
                        let si = ((sy * cw + sx) * 4) as usize;
                        if si + 4 <= cur.len() {
                            for c in 0..4 {
                                acc[c] += cur[si + c] as u32;
                            }
                            n += 1;
                        }
                    }
                }
                let di = ((y * nw + x) * 4) as usize;
                for c in 0..4 {
                    next[di + c] = (acc[c] / n.max(1)) as u8;
                }
            }
        }
        out.push((nw, nh, next.clone()));
        cur = next;
        cw = nw;
        ch = nh;
    }
    out
}

#[cfg(test)]
mod glyph_top_tests {
    use super::glyph_top;

    /// 数字底边贴基线（`'8'`: bearing_y = 0, height = 13, ascent = 13.75）。
    /// 旧公式给出 113.75（整整低了一个字高）；正确值是 100.75。
    #[test]
    fn digits_sitting_on_the_baseline_are_lifted_by_their_height() {
        // 新语义：基线 = round(100 + 13.75) = 114；顶部 = 114 - 0 - 13 = 101
        let top = glyph_top(100.0, 13.75, 0.0, 13.0);
        assert!((top - 101.0).abs() < 0.001, "top = {top}");
    }

    /// 有下伸的字形（`'设'`: bearing_y = -2, height = 16）：底边在基线下 2px，
    /// 所以顶部比贴基线的字形要低一点点。
    #[test]
    fn descenders_are_pushed_down_by_a_negative_bearing() {
        // 基线 114；top = 114 + 2 - 16 = 100
        let top = glyph_top(100.0, 13.75, -2.0, 16.0);
        assert!((top - 100.0).abs() < 0.001, "top = {top}");
    }

    /// 推导的不变量：底边 = 顶部 + 字高 = 基线 − bearing_y。
    #[test]
    fn bottom_edge_equals_baseline_minus_bearing() {
        let (origin_y, ascent, bearing, height) = (40.0, 12.0, -3.0, 15.0);
        let top = glyph_top(origin_y, ascent, bearing, height);
        let bottom = top + height;
        // 底边 = round(基线) − bearing（基线被取整，所有字都落在同一整数线）
        assert!((bottom - ((origin_y + ascent).round() - bearing)).abs() < 0.001);
    }

    // ---- clear() 必须清空裁剪栈（防"一次泄漏 ⇒ 永久全黑"）----

    #[test]
    fn clear_resets_clip_stack() {
        use y901_ui::{Color, Rect};
        let (cw, ch) = crate::cell_metrics_for(15.0);
        let engine = crate::MyFontEngine::new(
            crate::load_font(&crate::default_test_font_path()).expect("font"),
            15.0,
            y901_ui::Size::new(cw, ch),
        )
        .expect("engine");
        let mut soft = crate::soft::SoftBackend::new_headless(
            64,
            64,
            Box::new(engine),
            y901_ui::Size::new(cw, ch),
        )
        .expect("soft");
        // 故意压一层小裁剪区（模拟某个组件忘记 pop）
        <crate::soft::SoftBackend as y901_ui::RenderCtx>::push_clip(
            &mut soft,
            Rect::new(4.0, 4.0, 8.0, 8.0),
        );
        // 清屏后裁剪栈必须回到空
        soft.clear();
        <crate::soft::SoftBackend as y901_ui::RenderCtx>::fill_rect(
            &mut soft,
            Rect::new(0.0, 0.0, 64.0, 64.0),
            Color::rgba(1.0, 0.0, 1.0, 1.0),
        );
        let px = soft.pixel(0, 0).expect("px");
        assert_eq!(
            px & 0x00FF_FFFF,
            0x00FF_00FF,
            "清屏后仍受旧裁剪区限制：(0,0) 没被全屏矩形覆盖"
        );
    }

    // ---- 基线对齐（"字在跳舞"的回归）----

    #[test]
    fn all_glyph_bottoms_land_on_one_integer_baseline() {
        // Baseline formula: one integer baseline per line, bearing_y used as-is.
        let (origin_y, ascent) = (40.0f32, 16.93f32);
        let baseline = (origin_y + ascent).round();
        // (char, bearing_y, height): round latin, ascender, descender, colon, CJK
        let glyphs = [
            ("a", -1i32, 11i32),
            ("b", -1, 15),
            ("h", 0, 14),
            ("g", -5, 15),
            (":", -2, 8),
            ("\u{4f60}", -3, 18),
        ];
        for (name, bearing_y, height) in glyphs {
            let top = glyph_top(origin_y, ascent, bearing_y as f32, height as f32);
            let bottom = top + height as f32;
            // bottom = round(baseline) - bearing_y  (authentic bearing, no clamping)
            assert_eq!(bottom, baseline - bearing_y as f32, "{name}: bottom must be baseline - bearing");
            assert_eq!(bottom.fract(), 0.0, "{name}: bottom must be integer pixel");
        }
        // Same bearing_y => identical bottom (no per-glyph rounding jitter)
        let a = glyph_top(origin_y, ascent, -1.0, 11.0) + 11.0;
        let b = glyph_top(origin_y, ascent, -1.0, 15.0) + 15.0;
        assert_eq!(a, b, "a and b share bearing -1, so their bottoms must match");
        // Descenders keep their own lower bottom
        let g = glyph_top(origin_y, ascent, -5.0, 15.0) + 15.0;
        assert!(g > a, "descender must sit lower than baseline-sitting glyphs");
    }
        // 同一行内，所有“|bearing|≤1”的字底边必须**完全相等**
}
