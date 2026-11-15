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

    /// 整幅上屏。`pixels` 是**软后端产出的 BGRA**，浏览器要 RGBA。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32) {
        let Some(ctx) = self.ctx.as_ref() else {
            return; // 没取到 2D 上下文：静默跳过上屏（不崩）
        };
        let rgba = bgra_to_rgba(pixels, w, h);
        // **绕过 `Uint8ClampedArray::new` + `set`**：实测发现那条路径把数据读反了
        // （画面 180°）。现在用 `createImageData` 内部的 data 数组逐字节写入。
        let image_data = ctx.create_image_data(w, h);
        let data = image_data.data();
        for (i, &byte) in rgba.iter().enumerate() {
            data.set_index(i as u32, byte);
        }
        ctx.put_image_data(&image_data, 0.0, 0.0);
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
    // **按行翻转（仅 Y 轴）**：`SoftBackend::pixels()` 交出的是**自下而上**的缓冲，
    // 而 `ImageData` 要求自上而下。**不能整条 `.rev()`**（那会连 X 轴一起翻，得到 180°）。
    //
    // 这个结论经历了很多轮：四次探针（角标、字形、尺寸、像素顺序）+ 每次更换
    // `?v=` 版本参数后才确定。之前“按行翻转无效”是**浏览器跑的旧产物**。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_flip_reverses_rows_not_columns() {
        // 2x2：第 0 行 = [红, 绿]，第 1 行 = [蓝, 白]（BGRA）。
        let src = [
            0, 0, 255, 255, 0, 255, 0, 255,
            255, 0, 0, 255, 255, 255, 255, 255,
        ];
        let out = bgra_to_rgba(&src, 2, 2);
        assert_eq!(
            out,
            vec![0, 0, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn row_flip_preserves_length_and_alpha() {
        let src: Vec<u8> = (0..24u8).collect();
        let out = bgra_to_rgba(&src, 2, 3);
        assert_eq!(out.len(), src.len());
        let stride = 2 * 4;
        for y in 0..3usize {
            for x in 0..2usize {
                assert_eq!(out[y * stride + x * 4 + 3], src[(2 - y) * stride + x * 4 + 3]);
            }
        }
    }
}
