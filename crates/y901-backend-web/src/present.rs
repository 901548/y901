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

    /// 整幅上屏。实现在 S1。
    pub fn present(&mut self, _pixels: &[u8], _w: u32, _h: u32) {
        todo!("S1: ImageData + putImageData")
    }
}