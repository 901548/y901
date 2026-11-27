//! 呈现：软后端产出的 BGRA 像素 → `putImageData`（见 `docs/WASM.md` §3.2）。
//!
//! **2026-11 极简重写**：这里只保留"整帧上屏"一条路。
//!
//! 为何废弃局部上传：它为每帧省下的那 10ms，换来的是
//! 脏区截取、边界取整、阈值回退、ImageData 复用、坐标翻转一致性……
//! 一整套只会出 bug 的代码。release 下 800×600 全帧渲染+上屏约 15ms，
//! 足以稳定 60fps，**宁可每帧多花几毫秒，也不要那些偶现的局部绘制 bug**。

use std::time::Duration;

use y901_runtime::render_stats::{self, BackendKind, RenderStats, StageId};

use crate::bindings::{CanvasRenderingContext2d, HtmlCanvasElement};

/// 上屏四段的实测耗时（只在统计开启时填充）。
///
/// 为什么单独成结构体而不是直接在 `present()` 里写 `RenderStats`：
/// 1. `Present` 需要 `&mut self` 取 canvas 句柄，再借 `stats` 会产生借用冲突；
/// 2. **可单测**——`record_upload` 是纯函数，宿主机上就能断言"哪一段写进哪个
///    `StageId`"，而真正的 canvas 调用无法在没有浏览器时执行。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStages {
    /// BGRA → RGBA 通道交换。
    pub pixel_convert: Duration,
    /// `createImageData(w, h)`（浏览器侧分配）。
    pub image_alloc: Duration,
    /// `TypedArray.set(...)`（wasm 内存 → ImageData 的一次批量拷贝）。
    pub pixel_copy: Duration,
    /// `putImageData(0, 0)`（真正把像素交给 canvas）。
    pub canvas_blit: Duration,
}

/// 把上屏四段写进统计（`collector` 关闭时**什么都不做**）。
///
/// 为什么不合并成一段：这四段在 Web 上的耗时结构完全不同（通道交换是纯 CPU 循环、
/// `createImageData` 可能触发浏览器分配、`set` 是跨界拷贝、`putImageData` 是合成前
/// 的最后一步），合并后就无法判断优化该打哪里。
///
/// 为什么收 `&mut Option<RenderStats>`：本函数是"一次写四段"的批量写入，而
/// `Collector` 的规则（关闭时不写、取走语义）由调用方持有；`None` 时本函数是纯
/// no-op。这样单测可以直接构造 `Some(RenderStats::…)`，不必先打开进程级开关。
pub(crate) fn record_upload(stats: &mut Option<RenderStats>, stages: UploadStages) {
    let Some(s) = stats.as_mut() else {
        return;
    };
    s.stages.set(StageId::PixelConvert, stages.pixel_convert);
    s.stages.set(StageId::ImageAlloc, stages.image_alloc);
    s.stages.set(StageId::PixelCopy, stages.pixel_copy);
    s.stages.set(StageId::CanvasBlit, stages.canvas_blit);
}

/// 画布呈现器。
pub struct Present {
    /// canvas 句柄。
    pub canvas: HtmlCanvasElement,
    /// 2D 上下文（`getContext("2d")`）。
    pub ctx: Option<CanvasRenderingContext2d>,
    /// **分阶段统计收集器**（`Y901_RENDER_STATS` 开启时才有值）。
    ///
    /// 放在 `Present` 而不是 `WasmWindow` 的理由：`WasmWindow` 是**借用式薄包装**
    ///（`&mut SoftBackend` / `&mut Present`），给它加字段会改变公开结构体的字面量
    /// 构造方式；而 `Present` 是 `WasmState` 长期持有的所有者，天然适合放统计。
    collector: render_stats::Collector,
}

impl Present {
    /// 用 canvas 构造；上下文在这里取一次，后续每帧复用。
    pub fn new(canvas: HtmlCanvasElement) -> Self {
        let ctx = canvas.get_context("2d");

        Self {
            canvas,
            ctx,
            collector: render_stats::Collector::new(BackendKind::Web),
        }
    }

    // ---- 分阶段统计（A 阶段第 5 步：Web）----
    //
    // 规则集中在本 crate 依赖的 `render_stats::Collector`（三个软后端共用）。
    // 这里保留同名入口，是因为 `WasmWindow::render` 只借到 `&mut Present`，
    // 让 Present 直接转发比自己再抄一遍 6 个方法更不容易写错。

    /// 每帧开始（统计关闭时不分配、不读时钟）。
    pub fn stats_begin_frame(&mut self) {
        self.collector.begin_frame();
    }

    /// 统计是否开启（**本帧**是否要打点）。
    #[inline]
    pub fn stats_enabled(&self) -> bool {
        self.collector.enabled()
    }

    /// 写入某阶段的**已算好的耗时**（`None` = 统计关闭，什么都不做）。
    ///
    /// 为什么收 `Option<Duration>` 而不是 `Option<Instant>`：后者要求本 crate 的签名
    /// 暴露 `web_time::Instant`（本 crate 不依赖 `web-time`），而调用方拿到的正是
    /// `render_stats::since(start)` 的结果 —— 少一层类型穿透。
    #[inline]
    pub fn stats_record(&mut self, id: StageId, elapsed: Option<Duration>) {
        self.collector.record_elapsed(id, elapsed);
    }

    /// 记录脏区计数（块数 + 覆盖像素数）。
    pub fn stats_dirty(&mut self, rects: u32, pixels: u64) {
        self.collector.set_dirty(rects, pixels);
    }

    /// 跳过帧：后端没有渲染，但统计对象照常交付。
    pub fn stats_mark_skipped(&mut self) {
        self.collector.mark_skipped();
    }

    /// 帧末：把"CPU 后端确实没有的量"显式写 0（不是"测得 0"）。
    pub fn stats_finish(&mut self) {
        self.collector.finish();
    }

    /// 取走本帧统计（**取走语义**：第二次调用必为 `None`）。
    pub fn take_render_stats(&mut self) -> Option<RenderStats> {
        self.collector.take()
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
        // 统计关闭时 `mark(false)` **不读时钟**，四段开销为零。
        let on = self.collector.enabled();

        let t = render_stats::mark(on);
        let rgba = bgra_to_rgba(pixels, w, h);
        let convert = render_stats::since(t).unwrap_or_default();

        let t = render_stats::mark(on);
        let image_data = ctx.create_image_data(w, h);
        let alloc = render_stats::since(t).unwrap_or_default();

        // **批量拷贝**：`TypedArray.prototype.set` 一次把 wasm 内存搬进 ImageData。
        // 早期逐字节 `data[i] = v` 是 800×600 每帧 192 万次跨界调用（约 1 秒/帧）。
        let t = render_stats::mark(on);
        image_data.data().set_from(&rgba, 0);
        let copy = render_stats::since(t).unwrap_or_default();

        let t = render_stats::mark(on);
        ctx.put_image_data(&image_data, 0.0, 0.0);
        let blit = render_stats::since(t).unwrap_or_default();

        // 借用冲突（`ctx` 是 `self.ctx` 的借用、`stats` 又要 `&mut self`）的解法：
        // 先把四段收集成**值**，再写统计 —— 这也是 `UploadStages` 存在的第二个理由。
        record_upload(
            self.collector.stats_mut(),
            UploadStages {
                pixel_convert: convert,
                image_alloc: alloc,
                pixel_copy: copy,
                canvas_blit: blit,
            },
        );
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

    // ---- 上屏四段的统计映射（RenderStats A 阶段第 5 步）----

    /// 四段必须写进**各自**的 `StageId`。
    ///
    /// 这是 Web 埋点最容易错的地方（四个阶段名相近、顺序又相邻），
    /// 而真正的 canvas 调用无法在没有浏览器时执行 —— 所以映射逻辑单独成一个
    /// 纯函数，在这里逐项断言。
    #[test]
    fn upload_stages_map_to_their_own_stage_ids() {
        let mut stats = Some(RenderStats::new(3, BackendKind::Web));
        record_upload(
            &mut stats,
            UploadStages {
                pixel_convert: Duration::from_micros(11),
                image_alloc: Duration::from_micros(22),
                pixel_copy: Duration::from_micros(33),
                canvas_blit: Duration::from_micros(44),
            },
        );
        let s = stats.expect("统计对象");
        assert_eq!(s.backend, BackendKind::Web);
        assert_eq!(
            s.stages.get(StageId::PixelConvert),
            Some(Duration::from_micros(11))
        );
        assert_eq!(
            s.stages.get(StageId::ImageAlloc),
            Some(Duration::from_micros(22))
        );
        assert_eq!(
            s.stages.get(StageId::PixelCopy),
            Some(Duration::from_micros(33))
        );
        assert_eq!(
            s.stages.get(StageId::CanvasBlit),
            Some(Duration::from_micros(44))
        );
        // Web 的 upload 是"被拆成四段"的**不存在**阶段：不得顺带写入它。
        assert_eq!(s.stages.get(StageId::Upload), None);
        assert_eq!(
            s.validate(),
            Ok(()),
            "四段都是 Web 的可测阶段，不得自相矛盾"
        );
    }

    /// 统计关闭（`None`）时必须**什么都不做**，也不能 panic。
    #[test]
    fn record_upload_is_a_no_op_when_stats_are_off() {
        let mut stats: Option<RenderStats> = None;
        record_upload(
            &mut stats,
            UploadStages {
                pixel_convert: Duration::from_micros(1),
                image_alloc: Duration::from_micros(2),
                pixel_copy: Duration::from_micros(3),
                canvas_blit: Duration::from_micros(4),
            },
        );
        assert!(stats.is_none(), "关闭统计时不得凭空创建统计对象");
    }

    /// 四段之和应当等于"上屏总耗时"的量级，而不是互相覆盖 ——
    /// 覆盖的典型表现是"只有最后一段有值"。
    #[test]
    fn all_four_upload_stages_survive_a_round_trip() {
        let mut stats = Some(RenderStats::new(3, BackendKind::Web));
        record_upload(
            &mut stats,
            UploadStages {
                pixel_convert: Duration::from_millis(5),
                image_alloc: Duration::from_millis(6),
                pixel_copy: Duration::from_millis(7),
                canvas_blit: Duration::from_millis(8),
            },
        );
        let s = stats.expect("统计对象");
        let sum: Duration = [
            StageId::PixelConvert,
            StageId::ImageAlloc,
            StageId::PixelCopy,
            StageId::CanvasBlit,
        ]
        .iter()
        .filter_map(|id| s.stages.get(*id))
        .sum();
        assert_eq!(
            sum,
            Duration::from_millis(26),
            "四段必须并存（不是互相覆盖）"
        );
    }
}
