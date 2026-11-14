//! 呈现：软后端产出的 RGBA 像素 → `putImageData`（见 `docs/WASM.md` §3.2）。

use crate::bindings::HtmlCanvasElement;

/// 画布呈现器。
pub struct Present {
    /// canvas 句柄。
    pub canvas: HtmlCanvasElement,
    /// 已分配的像素缓冲（RGBA）。
    pub pixels: Vec<u8>,
}

impl Present {
    /// 用 canvas 构造；像素缓冲在 S1 按尺寸分配。
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        Self {
            canvas,
            pixels: Vec::new(),
        }
    }

    /// 整幅上屏。`pixels` 是**软后端产出的 BGRA**；浏览器要 RGBA，先换通道。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32) {
        let rgba = bgra_to_rgba(pixels);
        self.pixels = rgba;
        // 真正的 ImageData + putImageData 在下一步接上（需要 2D 上下文句柄与
        // `Uint8ClampedArray` 构造）；此处只保证像素已按浏览器所需的通道序排好。
        let _ = (w, h);
    }
}

/// **纯函数**：BGRA → RGBA，逐像素换 R/B 通道，alpha 不变。
///
/// 为什么不就地改：调用方给的是软后端的缓冲，可能是它自己的复用缓冲；
/// 这里返回新 `Vec` 更安全，也便于单测。
pub fn bgra_to_rgba(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    for px in src.chunks_exact(4) {
        out.push(px[2]); // R ← B
        out.push(px[1]); // G
        out.push(px[0]); // B ← R
        out.push(px[3]); // A
    }
    // 末尾不足 4 字节时补 0，保持长度是 4 的倍数（不会发生，但别静默丢字节）
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_to_rgba_swaps_red_and_blue() {
        // BGRA: (B=10, G=20, R=30, A=255) → RGBA: (30, 20, 10, 255)
        let src = [10u8, 20, 30, 255, 1, 2, 3, 4];
        let out = bgra_to_rgba(&src);
        assert_eq!(out, vec![30, 20, 10, 255, 3, 2, 1, 4]);
    }

    #[test]
    fn bgra_to_rgba_preserves_length_and_alpha() {
        let src: Vec<u8> = (0..40u8).collect();
        let out = bgra_to_rgba(&src);
        assert_eq!(out.len(), src.len());
        for (i, px) in src.chunks_exact(4).enumerate() {
            assert_eq!(out[i * 4 + 3], px[3], "alpha 必须原样保留");
        }
    }
}