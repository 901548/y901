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
        Self { canvas, ctx }
    }

    /// 整幅上屏。`pixels` 是**软后端产出的 BGRA**，浏览器要 RGBA。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32) {
        let Some(ctx) = self.ctx.as_ref() else {
            return; // 没取到 2D 上下文：静默跳过上屏（不崩）
        };
        let rgba = bgra_to_rgba(pixels);
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
pub fn bgra_to_rgba(src: &[u8]) -> Vec<u8> {
    // **像素顺序必须翻转**：软后端的缓冲以另一种顺序排列（实测直接照搬会得到 180° 旋转的画面）。
    // 此处按**整体逆序**读源像素，再按 BGRA→RGBA 换通道；两者叠加后切反了那个 180°。
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4).rev() {
        out.push(px[2]); // R ← B
        out.push(px[1]); // G
        out.push(px[0]); // B ← R
        out.push(px[3]); // A
    }
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_rgba_swaps_channels_in_reverse_pixel_order() {
        // 源像素 A=(B10,G20,R30,A255)、B=(B1,G2,R3,A4)。
        // 输出顺序为逆序：先 B 的 RGBA（ 3,2,1,4），再 A 的（ 30,20,10,255）。
        let src = [10u8, 20, 30, 255, 1, 2, 3, 4];
        assert_eq!(bgra_to_rgba(&src), vec![3, 2, 1, 4, 30, 20, 10, 255]);
    }

    #[test]
    fn bgra_to_rgba_preserves_length_and_alpha() {
        let src: Vec<u8> = (0..40u8).collect();
        let out = bgra_to_rgba(&src);
        assert_eq!(out.len(), src.len());
        // 像素顺序逆序，因此第 i 个输出像素对应源的第 (n-1-i) 个。
        let n = src.len() / 4;
        for (i, out_px) in out.chunks_exact(4).enumerate() {
            let src_px = &src[(n - 1 - i) * 4..];
            assert_eq!(out_px[3], src_px[3], "alpha 必须原样保留");
        }
    }
}