//! 呈现：软后端产出的 BGRA 像素 → `putImageData`（见 `docs/WASM.md` §3.2）。

use crate::bindings::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, Uint8ClampedArray};

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
        // **构建标识**：用于判断浏览器跑的是不是最新 wasm（缓存会让旧产物存活）。
        crate::bindings::console_log("Y901-BUILD pixel-order=row-flip-6");

        Self { canvas, ctx }
    }

    /// 整幅上屏。`pixels` 是**软后端产出的 BGRA**，浏览器要 RGBA。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32) {
        let Some(ctx) = self.ctx.as_ref() else {
            return; // 没取到 2D 上下文：静默跳过上屏（不崩）
        };
        let rgba = bgra_to_rgba(pixels, w, h);
        let arr = Uint8ClampedArray::new(rgba.len() as u32);
        arr.set_from(&rgba, 0);
        let data = ImageData::new(&arr.into(), w, h);
        ctx.put_image_data(&data, 0.0, 0.0);
    }
}

/// **纯函数**：BGRA → RGBA，逐像素换 R/B 通道，alpha 不变。
///
/// 为什么不就地改：调用方给的是软后端的复用缓冲；
/// 这里返回新 `Vec` 更安全，也便于单测。
pub fn bgra_to_rgba(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    // **按行翻转（仅 Y 轴）**：底层缓冲行序是自下而上，而 `ImageData` 要求自上而下。
    // 不能用整条 `.rev()`：那会把 X 轴也翻过去，得到 180°（上下 + 左右同时翻）。
    let w = w as usize;
    let h = h as usize;
    let stride = w * 4;
    let mut out = Vec::with_capacity(src.len());
    for y in (0..h).rev() {
        let row_start = y * stride;
        let row_end = (row_start + stride).min(src.len());
        let row = &src[row_start..row_end];
        for px in row.chunks_exact(4) {
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

    #[test]
    fn row_flip_reverses_rows_not_columns() {
        // 2x2：第 0 行 = [红, 绿]，第 1 行 = [蓝, 白]（BGRA）。
        // 按行翻转后：先输出原第 1 行（蓝、白），再原第 0 行（红、绿），行内保持顺序。
        let src = [
            0, 0, 255, 255, /* 红 */ 0, 255, 0, 255, /* 绿 */
            255, 0, 0, 255, /* 蓝 */ 255, 255, 255, 255, /* 白 */
        ];
        let out = bgra_to_rgba(&src, 2, 2);
        assert_eq!(
            out,
            vec![
                0, 0, 255, 255, /* 蓝 */ 255, 255, 255, 255, /* 白 */
                255, 0, 0, 255, /* 红 */ 0, 255, 0, 255, /* 绿 */
            ]
        );
    }

    #[test]
    fn row_flip_preserves_length_and_alpha() {
        let src: Vec<u8> = (0..24u8).collect();
        let out = bgra_to_rgba(&src, 2, 3);
        assert_eq!(out.len(), src.len());
        // 每行内 alpha 位置不变；行顺序翻转后，输出第 0 行来自源的最后一行。
        let stride = 2 * 4;
        for y in 0..3usize {
            for x in 0..2usize {
                let o = (y * stride) + x * 4 + 3;
                let s_idx = ((2 - y) * stride) + x * 4 + 3;
                assert_eq!(out[o], src[s_idx], "alpha 应随行翻转");
            }
        }
    }
}
