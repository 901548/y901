//! 呈现：软后端产出的 BGRA 像素 → `putImageData`（见 `docs/WASM.md` §3.2）。

use crate::bindings::{CanvasRenderingContext2d, HtmlCanvasElement};

/// 脏矩形（屏幕坐标，像素）：`(x, y, w, h)`。
pub type RectPx = (u32, u32, u32, u32);

/// 画布呈现器。
pub struct Present {
    /// canvas 句柄。
    pub canvas: HtmlCanvasElement,
    /// 2D 上下文（`getContext("2d")`）。
    pub ctx: Option<CanvasRenderingContext2d>,
}

impl Present {
    /// 用 canvas 构造；上下文在这里取一次，后续每帧复用。
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        let ctx = canvas.get_context("2d");

        Self { canvas, ctx }
    }

    /// 上屏。`pixels` 是**软后端产出的 BGRA**，浏览器要 RGBA。
    ///
    /// 两种模式：
    /// - `rects == None`：**整帧**上传（旧路径）。
    /// - `rects == Some(list)`：**只上传这些脏矩形**，每块单独 `putImageData`。
    ///
    /// **两条路径必须逐像素一致**：整帧路径按"屏幕行 ← 缓冲行 `h-1-y`"取行，
    /// 局部路径对每个矩形做**同样的行映射**（见 `bgra_rect_to_rgba`）。
    /// T8 单测把这条一致性钉死。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32, rects: Option<&[RectPx]>) {
        let Some(ctx) = self.ctx.as_ref() else {
            return; // 没取到 2D 上下文：静默跳过上屏（不崩）
        };
        match rects {
            Some(list) if !list.is_empty() => {
                for &(x, y, rw, rh) in list {
                    let rgba = bgra_rect_to_rgba(pixels, w, h, x, y, rw, rh);
                    if rgba.is_empty() {
                        continue;
                    }
                    let image_data = ctx.create_image_data(rw, rh);
                    let data = image_data.data();
                    for (i, &byte) in rgba.iter().enumerate() {
                        data.set_index(i as u32, byte);
                    }
                    ctx.put_image_data(&image_data, x as f64, y as f64);
                }
            }
            _ => {
                let rgba = bgra_to_rgba(pixels, w, h);
                let image_data = ctx.create_image_data(w, h);
                let data = image_data.data();
                for (i, &byte) in rgba.iter().enumerate() {
                    data.set_index(i as u32, byte);
                }
                ctx.put_image_data(&image_data, 0.0, 0.0);
            }
        }
    }
}

/// **纯函数**：BGRA → RGBA，逐像素换 R/B 通道，alpha 不变。
///
/// **行映射**：软后端缓冲的存储方向与屏幕相反，因此输出第 `Y` 行取缓冲第 `h-1-Y` 行。
/// 这条映射是**全局唯一约定**，`bgra_rect_to_rgba` 必须与之完全一致（T8 保证）。
/// 为什么不就地改：调用方给的是软后端的复用缓冲；这里返回新 `Vec` 更安全，也便于单测。
pub fn bgra_to_rgba(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let w = w as usize;
    let h = h as usize;
    let stride = w * 4;
    let mut out = Vec::with_capacity(src.len());
    for y in (0..h).rev() {
        let row_start = y * stride;
        let row_end = (row_start + stride).min(src.len());
        for px in src[row_start..row_end].chunks_exact(4) {
            out.push(px[2]); // R ← B
            out.push(px[1]); // G
            out.push(px[0]); // B ← R
            out.push(px[3]); // A
        }
    }
    out
}

/// **纯函数**：从整帧 BGRA 缓冲里抠出屏幕坐标下的 `(x, y, w, h)`，转成 RGBA（行主序）。
///
/// **与 [`bgra_to_rgba`] 完全同一套行映射**：输出第 `k` 行（屏幕行 `y+k`）取缓冲行
/// `frame_h - 1 - (y + k)`。X 方向不翻转。
///
/// 边界处理：矩形被裁剪到帧内；越界部分不读、不产生字节；完全不相交返回空 `Vec`。
pub fn bgra_rect_to_rgba(
    src: &[u8],
    frame_w: u32,
    frame_h: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let fw = frame_w as usize;
    let fh = frame_h as usize;
    if fw == 0 || fh == 0 || w == 0 || h == 0 {
        return Vec::new();
    }
    let stride = fw * 4;
    let x0 = (x as usize).min(fw);
    let x1 = (x as usize + w as usize).min(fw);
    let y0 = (y as usize).min(fh);
    let y1 = (y as usize + h as usize).min(fh);
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let rw = x1 - x0;
    let mut out = Vec::with_capacity(rw * (y1 - y0) * 4);
    // 屏幕行 y0..y1 ⇒ 缓冲行 (fh-1-y)，从下往上取，所以逐行递减。
    for screen_y in y0..y1 {
        let src_row = fh - 1 - screen_y;
        let start = src_row * stride + x0 * 4;
        let end = src_row * stride + x1 * 4;
        if end > src.len() {
            break;
        }
        for px in src[start..end].chunks_exact(4) {
            out.push(px[2]); // R ← B
            out.push(px[1]); // G
            out.push(px[0]); // B ← R
            out.push(px[3]); // A
        }
    }
    out
}

/// **纯函数**：把浮点脏矩形转成整数像素矩形（左上取 floor、右下取 ceil）。
///
/// 为什么要 ceil：只取 floor 会**漏掉右/下边界那一列像素**，留下半个像素宽的残影。
/// 为什么要 floor 左上：向内取整会漏掉紧贴左/上边界的像素。
///
/// 返回 `None` 表示裁剪后为空（完全在视口外或尺寸为零）。
pub fn clamp_dirty_rect(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    viewport_w: u32,
    viewport_h: u32,
) -> Option<RectPx> {
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return None;
    }
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let x0 = x.floor().max(0.0) as u32;
    let y0 = y.floor().max(0.0) as u32;
    let x1 = (x + w).ceil().clamp(0.0, viewport_w as f32) as u32;
    let y1 = (y + h).ceil().clamp(0.0, viewport_h as f32) as u32;
    if x1 <= x0 || y1 <= y0 || x0 >= viewport_w || y0 >= viewport_h {
        return None;
    }
    Some((x0, y0, x1 - x0, y1 - y0))
}

/// **纯函数**：判断这批脏矩形是否值得走"局部上传"。
///
/// 何时改用整帧：
/// - **块数 > 8**：每块一次 `putImageData` 有固定开销（跨界调用 + ImageData 构造），
///   碎块多时反而更慢；
/// - **总面积 > 视口 50%**：整帧上传只多搬一点字节，却省掉多次调用的固定成本。
///
/// 这两个阈值来自"局部上传的收益 ∝ 脏区面积"，也是悬停/打字（小面积）与
/// 滚动/尺寸变化（大面积）的分界。
pub fn should_upload_partial(rects: &[RectPx], viewport_w: u32, viewport_h: u32) -> bool {
    const MAX_RECTS: usize = 8;
    const MAX_AREA_RATIO: f32 = 0.5;
    if rects.is_empty() || rects.len() > MAX_RECTS {
        return false;
    }
    let total: u64 = rects
        .iter()
        .map(|&(_, _, w, h)| w as u64 * h as u64)
        .sum();
    let vp = viewport_w as u64 * viewport_h as u64;
    if vp == 0 {
        return false;
    }
    (total as f32) <= vp as f32 * MAX_AREA_RATIO
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x4 帧：`B = x*10`、`G = y*10`、`R = 100 + y*4 + x`、`A = 255`。
    fn frame_4x4() -> Vec<u8> {
        let mut v = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u32 {
            for x in 0..4u32 {
                v.push((x * 10) as u8);
                v.push((y * 10) as u8);
                v.push((100 + y * 4 + x) as u8);
                v.push(255);
            }
        }
        v
    }

    /// 期望的 RGBA 像素（来自源 (x, y)）。
    fn expect_px(x: u32, y: u32) -> [u8; 4] {
        [
            (100 + y * 4 + x) as u8, // R
            (y * 10) as u8,          // G
            (x * 10) as u8,          // B
            255,                     // A
        ]
    }

    /// 期望的屏幕像素：屏幕行 `sy` 对应缓冲行 `3-sy`（4x4 帧）。
    fn expect_screen_px(x: u32, screen_y: u32) -> [u8; 4] {
        expect_px(x, 3 - screen_y)
    }

    // ---- T6：抠图内容与源区域一致（含行映射）----

    #[test]
    fn t6_rect_extraction_matches_source_region() {
        let frame = frame_4x4();
        let out = bgra_rect_to_rgba(&frame, 4, 4, 1, 1, 2, 2);
        assert_eq!(out.len(), 16);

        // 输出第 0 行 = 屏幕行 1；第 1 行 = 屏幕行 2。X 不翻转。
        let mut want = Vec::new();
        for screen_y in 1..3u32 {
            for x in 1..3u32 {
                want.extend_from_slice(&expect_screen_px(x, screen_y));
            }
        }
        assert_eq!(out, want, "抠图内容必须与源区域逐字节一致");
    }

    // ---- T7：不越界读（stride / 左右边界）----

    #[test]
    fn t7_rect_extraction_never_reads_outside_rect() {
        let frame = frame_4x4();
        let out = bgra_rect_to_rgba(&frame, 4, 4, 2, 2, 2, 2);
        assert_eq!(out.len(), 16);
        // 第一行第一个像素 = 屏幕 (2,2)
        assert_eq!(&out[0..4], &expect_screen_px(2, 2));
        // 第二行第一个像素 = 屏幕 (2,3)
        assert_eq!(&out[8..12], &expect_screen_px(2, 3));
        // 绝不能混入 x=0/1 的 B 值
        assert_ne!(out[2], 0);
        assert_ne!(out[2], 10);
    }

    /// 越界裁剪：不 panic，只返回帧内部分。
    #[test]
    fn t7b_rect_is_clamped_to_frame() {
        let frame = frame_4x4();
        let out = bgra_rect_to_rgba(&frame, 4, 4, 3, 3, 10, 10);
        assert_eq!(out.len(), 4, "只剩右下 1x1");
        assert_eq!(&out[0..4], &expect_screen_px(3, 3));
        assert!(bgra_rect_to_rgba(&frame, 4, 4, 10, 10, 2, 2).is_empty());
        assert!(bgra_rect_to_rgba(&frame, 4, 4, 0, 0, 0, 2).is_empty());
    }

    // ---- T8：**两条路径一致性硬约束** ----

    #[test]
    fn t8_full_and_partial_paths_agree_byte_for_byte() {
        let frame = frame_4x4();
        let full = bgra_to_rgba(&frame, 4, 4);
        // 脏区覆盖整帧：局部路径必须与整帧路径完全相同
        let partial = bgra_rect_to_rgba(&frame, 4, 4, 0, 0, 4, 4);
        assert_eq!(
            full, partial,
            "整帧与局部路径必须逐字节一致；不一致说明两者的 Y 映射不同（历史 Bug）"
        );

        // 再验证一次非整帧：任何矩形的输出都等于"从整帧结果里裁出来"的对应区域
        let (x, y, w, h) = (1u32, 1u32, 2u32, 2u32);
        let sub = bgra_rect_to_rgba(&frame, 4, 4, x, y, w, h);
        let mut want = Vec::new();
        for row in y..y + h {
            let start = ((row * 4 + x) * 4) as usize;
            want.extend_from_slice(&full[start..start + (w * 4) as usize]);
        }
        assert_eq!(sub, want, "局部抠图必须等于整帧结果的对应区域");
    }

    // ---- T9：分数坐标的脏区必须向外取整，覆盖右/下边界 ----

    #[test]
    fn t9_fractional_dirty_rect_is_expanded_outwards() {
        // x=10.5, w=3.7 ⇒ 覆盖 [10.5, 14.2) ⇒ 整数像素 [10, 15)
        let r = clamp_dirty_rect(10.5, 4.2, 3.7, 2.1, 100, 100).expect("应有结果");
        assert_eq!(r.0, 10, "左边界向下取整");
        assert_eq!(r.1, 4, "上边界向下取整");
        assert_eq!(r.0 + r.2, 15, "右边界 = ceil(14.2) = 15");
        assert_eq!(r.1 + r.3, 7, "下边界 = ceil(6.3) = 7");

        // 完全在视口外 / 尺寸为零 ⇒ None
        assert!(clamp_dirty_rect(200.0, 0.0, 10.0, 10.0, 100, 100).is_none());
        assert!(clamp_dirty_rect(0.0, 0.0, 0.0, 10.0, 100, 100).is_none());
        assert!(clamp_dirty_rect(f32::NAN, 0.0, 1.0, 1.0, 100, 100).is_none());
    }

    // ---- T10：局部上传阈值决策 ----

    #[test]
    fn t10_partial_upload_only_for_small_dirty_regions() {
        // 小脏区：悬停/打字 → 走局部
        assert!(should_upload_partial(&[(0, 0, 20, 20)], 800, 600));
        // 面积超视口一半 → 整帧
        assert!(!should_upload_partial(&[(0, 0, 800, 400)], 800, 600));
        // 碎块过多 → 整帧（每块一次 putImageData 的固定开销）
        let many: Vec<RectPx> = (0..9).map(|i| (i * 5, 0, 2, 2)).collect();
        assert!(!should_upload_partial(&many, 800, 600));
        // 空列表 → 不该走局部
        assert!(!should_upload_partial(&[], 800, 600));
    }
}
