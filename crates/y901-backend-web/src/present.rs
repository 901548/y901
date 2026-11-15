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
pub fn bgra_to_rgba(src: &[u8], _w: u32, _h: u32) -> Vec<u8> {
    // **[角标探针版本] 正序**：本次只做一件事——用四角颜色
    // 确认浏览器里跑的到底是哪个版本、以及 Y 轴到底反不反。
    // 证据出来之前不加任何翻转。
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.push(px[2]); // R ← B
        out.push(px[1]); // G
        out.push(px[0]); // B ← R
        out.push(px[3]); // A
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 当前构建是**正序**（角标探针版）：只换通道、不翻行。
    #[test]
    fn converts_channels_in_source_order() {
        let src = [10u8, 20, 30, 255, 1, 2, 3, 4];
        assert_eq!(bgra_to_rgba(&src, 2, 1), vec![30, 20, 10, 255, 3, 2, 1, 4]);
    }

    #[test]
    fn preserves_length_and_alpha() {
        let src: Vec<u8> = (0..24u8).collect();
        let out = bgra_to_rgba(&src, 2, 3);
        assert_eq!(out.len(), src.len());
        for (i, px) in out.chunks_exact(4).enumerate() {
            assert_eq!(px[3], src[i * 4 + 3], "alpha 必须随自己的像素");
        }
    }
}
