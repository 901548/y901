//! 呈现：软后端产出的 BGRA 像素 → `putImageData`（见 `docs/WASM.md` §3.2）。
//!
//! **2026-11 极简重写**：这里只保留"整帧上屏"一条路。
//!
//! 为何废弃局部上传：它为每帧省下的那 10ms，换来的是
//! 脏区截取、边界取整、阈值回退、ImageData 复用、坐标翻转一致性……
//! 一整套只会出 bug 的代码。release 下 800×600 全帧渲染+上屏约 15ms，
//! 足以稳定 60fps，**宁可每帧多花几毫秒，也不要那些偶现的局部绘制 bug**。

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

    /// **整帧上屏**。`pixels` 是软后端产出的 BGRA，浏览器要 RGBA。
    ///
    /// 两步：换通道 → 写进 `ImageData`（**一次** `TypedArray.set`）→ `putImageData(0,0)`。
    /// **没有行翻转**：软后端缓冲与 `ImageData` 一样是**自上而下**的行序
    /// （`SoftBackend` 直接按 `y * width + x` 写入，与 Win32 `biHeight < 0`
    /// 的 DIB 和 X11 ZPixmap 完全一致）。曾经在这里按行翻转，
    /// 结果把**整个页面上下镜像**（"标题跑到最底下"），却因为字形位图当时是
    /// 自下而上的行序，文字看起来还是正的——两个错误互相掩盖。
    /// 现在光栅器的行序已在源头统一为自上而下，这里只做通道交换。
    pub fn present(&mut self, pixels: &[u8], w: u32, h: u32) {
        let Some(ctx) = self.ctx.as_ref() else {
            return; // 没取到 2D 上下文：静默跳过上屏（不崩）
        };
        let rgba = bgra_to_rgba(pixels, w, h);
        let image_data = ctx.create_image_data(w, h);
        // **批量拷贝**：`TypedArray.prototype.set` 一次把 wasm 内存搬进 ImageData。
        // 早期逐字节 `data[i] = v` 是 800×600 每帧 192 万次跨界调用（约 1 秒/帧）。
        image_data.data().set_from(&rgba, 0);
        ctx.put_image_data(&image_data, 0.0, 0.0);
    }
}

/// **纯函数**：BGRA → RGBA，逐像素换 R/B 通道，alpha 与**行序**都不变。
///
/// 为什么不就地改：调用方给的是软后端的复用缓冲；这里返回新 `Vec` 更安全，也便于单测。
pub fn bgra_to_rgba(src: &[u8], _w: u32, _h: u32) -> Vec<u8> {
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

    /// **只换通道，不重排行**：第 0 行仍是第 0 行。
    #[test]
    fn swaps_channels_without_reordering_rows() {
        // 2x2：第 0 行 = [红, 绿]，第 1 行 = [蓝, 白]（BGRA）。
        let src = [
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
        ];
        let out = bgra_to_rgba(&src, 2, 2);
        assert_eq!(
            out,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, // 红, 绿
                0, 0, 255, 255, 255, 255, 255, 255, // 蓝, 白
            ],
            "行序必须原样保留：整帧镜像会让页面上下颠倒"
        );
    }

    #[test]
    fn preserves_length_alpha_and_row_order() {
        let src: Vec<u8> = (0..24u8).collect();
        let out = bgra_to_rgba(&src, 2, 3);
        assert_eq!(out.len(), src.len());
        let stride = 2 * 4;
        for y in 0..3usize {
            for x in 0..2usize {
                assert_eq!(
                    out[y * stride + x * 4 + 3],
                    src[y * stride + x * 4 + 3],
                    "alpha 必须留在同一行同一列"
                );
            }
        }
    }
}
