//! 呈现：软后端产出的 BGRA 像素 → `putImageData`（见 `docs/WASM.md` §3.2）。

use crate::bindings::{CanvasRenderingContext2d, HtmlCanvasElement};

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
    /// - `rects == None` 或 `rects == Some([])`：**整帧**上传（旧路径，逐像素等价）。
    /// - `rects == Some([r1, r2, …])`：**只上传这些脏矩形**，每块单独 `putImageData`。
    ///
    /// **为什么局部上传是性能关键**：整帧 800×600 的 `putImageData` 每帧要跨 wasm/JS 边界搬运
    /// ~1.9 MB，浏览器还要重建整幅 ImageData；局部上传的代价只与脏区面积成正比。
    ///
    /// 调用方保证 `rects` 已经裁剪到画布范围内（见 `WasmWindow::render`）。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32, rects: Option<&[(u32, u32, u32, u32)]>) {
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
                    // **每个脏区单独建一张 ImageData**：尺寸 = 脏区，位置由 `putImageData` 的
                    // (x, y) 决定。这样浏览器只处理脏区像素。
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
/// **不翻行**：软后端缓冲是**自上而下**行主序——桌面 GDI 上屏
/// （`window.rs` 的 `biHeight: -height`，负值即自上而下 DIB）用的就是同一份缓冲，
/// 如果这里再按行翻转，整帧会上下镜像（每列子元素逆序、文字倒置）。
/// `ImageData` 同样要求自上而下，所以只需换通道。
/// 为什么不就地改：调用方给的是软后端的复用缓冲；
/// 这里返回新 `Vec` 更安全，也便于单测。
pub fn bgra_to_rgba(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    // **按行翻转（仅 Y 轴）**：四角探针已证实——四色块位置全对（说明
    // `present` 的搬运与通道交换都没错），但 UI 180° 倒置 ⇒ 缓冲本身是**自下而上**的。
    // 只翻 Y：若用整条 `.rev()`，X 也跟着翻，又回到 180°。
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

/// **纯函数**：从整帧 BGRA 缓冲里抠出 `(x, y, w, h)` 这一块，转成 RGBA（行主序）。
///
/// 与 [`bgra_to_rgba`] 的关系：整帧函数是"全抠 + 换通道"，这里是"按脏矩形抠 + 换通道"。
/// **两者对同一像素的结果必须一致**（T7 会验证）。
///
/// 边界处理：`(x, y, w, h)` 会被裁剪到帧内；越界部分不读、不产生字节。
/// 返回空 `Vec` 表示该矩形与帧完全不相交。
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
    let stride = fw * 4;
    if fw == 0 || fh == 0 || w == 0 || h == 0 {
        return Vec::new();
    }
    // 裁剪到帧内（右/下边界），左/上边界由 u32 保证非负。
    let x0 = (x as usize).min(fw);
    let y0 = (y as usize).min(fh);
    let x1 = (x as usize + w as usize).min(fw);
    let y1 = (y as usize + h as usize).min(fh);
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let rw = x1 - x0;
    let mut out = Vec::with_capacity(rw * (y1 - y0) * 4);
    for row in y0..y1 {
        let start = row * stride + x0 * 4;
        let end = row * stride + x1 * 4;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **Y 轴按行翻转**：末行先出、行内顺序不变。
    #[test]
    fn flips_rows_not_columns() {
        // 2x2：第 0 行 = [红, 绿]，第 1 行 = [蓝, 白]（BGRA）。
        let src = [
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
        ];
        let out = bgra_to_rgba(&src, 2, 2);
        assert_eq!(
            out,
            vec![0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn preserves_length_and_alpha_under_row_flip() {
        let src: Vec<u8> = (0..24u8).collect();
        let out = bgra_to_rgba(&src, 2, 3);
        assert_eq!(out.len(), src.len());
        let stride = 2 * 4;
        for y in 0..3usize {
            for x in 0..2usize {
                assert_eq!(
                    out[y * stride + x * 4 + 3],
                    src[(2 - y) * stride + x * 4 + 3],
                    "alpha 必须随着行翻转"
                );
            }
        }
    }

    // ---- T6：4x4 整帧里抠 2x2，字节必须与原图对应区域一致 ----

    /// 构造 4x4 帧：像素值为 `[B=x*10, G=y*10, R=100+idx, A=255]`，便于定位。
    fn frame_4x4() -> Vec<u8> {
        let mut v = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4u32 {
            for x in 0..4u32 {
                v.push((x * 10) as u8); // B
                v.push((y * 10) as u8); // G
                v.push((100 + y * 4 + x) as u8); // R
                v.push(255); // A
            }
        }
        v
    }

    #[test]
    fn t6_rect_extraction_matches_source_region() {
        let frame = frame_4x4();
        // 抠 (1,1) 起的 2x2。
        let out = bgra_rect_to_rgba(&frame, 4, 4, 1, 1, 2, 2);
        assert_eq!(out.len(), 2 * 2 * 4, "2x2 RGBA 应为 16 字节");

        // 期望：按行主序，每像素 R←B, G, B←R, A。
        let mut want = Vec::new();
        for y in 1..3u32 {
            for x in 1..3u32 {
                let r = (100 + y * 4 + x) as u8;
                want.push(r); // R（源里的 R）
                want.push((y * 10) as u8); // G
                want.push((x * 10) as u8); // B（源里的 B）
                want.push(255); // A
            }
        }
        assert_eq!(out, want, "抠图内容必须与原图对应区域逐字节一致");
    }

    // ---- T7：边界外像素绝不参与（验证 stride / 裁剪） ----

    #[test]
    fn t7_rect_extraction_never_reads_outside_rect() {
        let frame = frame_4x4();
        // 抠 (2,2) 起 2x2 ⇒ 正好右下角，行内只有 x=2,3，行只有 y=2,3。
        let out = bgra_rect_to_rgba(&frame, 4, 4, 2, 2, 2, 2);
        assert_eq!(out.len(), 16);

        // 每行第一个像素的 R 应为 (2,2) 与 (3,2) 两个源像素，绝不能混入 x=0/1 的 B 值（0/10）。
        let row0 = &out[0..8];
        assert_eq!(row0[0], (100 + 2 * 4 + 2) as u8, "左上像素的 R 来自 (2,2)");
        assert_eq!(row0[2], (2 * 10) as u8, "左上像素的 B 来自 (2,2) 的 B");
        assert_eq!(row0[4], (100 + 2 * 4 + 3) as u8, "同排第二个像素来自 (3,2)");
        assert_ne!(row0[2], 0, "绝不能取到 x=0 的 B 值");
        assert_ne!(row0[2], 10, "绝不能取到 x=1 的 B 值");
    }

    /// 越界裁剪：请求超出帧的矩形，只返回帧内部分，不 panic。
    #[test]
    fn t7b_rect_is_clamped_to_frame() {
        let frame = frame_4x4();
        let out = bgra_rect_to_rgba(&frame, 4, 4, 3, 3, 10, 10);
        assert_eq!(out.len(), 4, "只剩右下 1x1 一个像素");
        // 完全在帧外 ⇒ 空
        assert!(bgra_rect_to_rgba(&frame, 4, 4, 10, 10, 2, 2).is_empty());
        // 零尺寸 ⇒ 空
        assert!(bgra_rect_to_rgba(&frame, 4, 4, 0, 0, 0, 2).is_empty());
    }
}
