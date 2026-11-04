//! soft.rs 纹理路径的分配计数基准（审查 #18 的 soft.rs 候选）。
//!
//! **必须 `--test-threads=1`**：计数器是进程全局的，并行会让别的用例的分配记到本轮上
//! （上一轮 dispatch 基准就因此得到过 0.01 / 0.25 / 6.00 三种互相矛盾的读数）。
//!
//! 跑法：
//! ```text
//! cargo test -p y901-render --release --test perf_texture_alloc -- --ignored --nocapture --test-threads=1
//! ```

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// **线程局部**计数器。
//
// 为何不用全局 `AtomicUsize`：全局计数器在 `--include-ignored` 全量并行时
// 会把**别的测试线程**的分配记到本用例头上（实测：应 0 却测到 4），
// 于是那条 100 帧基准必然失败、整轮中止。
// `const { Cell::new(0) }` 保证初始化不分配（避免分配器重入）。
thread_local! {
    static T_ALLOCS: Cell<usize> = const { Cell::new(0) };
    static T_BYTES: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump(layout.size());
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump(new_size);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn bump(bytes: usize) {
    // `try_with`：线程退出时 TLS 已销毁会返回 Err，不能 panic 在分配器里。
    let _ = T_ALLOCS.try_with(|c| c.set(c.get() + 1));
    let _ = T_BYTES.try_with(|c| c.set(c.get() + bytes));
}

fn take() -> (usize, usize) {
    let a = T_ALLOCS.with(|c| c.replace(0));
    let b = T_BYTES.with(|c| c.replace(0));
    (a, b)
}

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{image::ImageStyle, Color, Pos, Rect, RenderCtx, Size, TextStyle};

const W: u32 = 512;
const H: u32 = 512;
/// 1024×1024 RGBA = 4 MiB，正好是"每帧 clone 一次就很贵"的典型尺寸。
const TEX: u32 = 1024;

fn backend() -> SoftBackend {
    let (cw, ch) = y901_render::cell_metrics_for(15.0);
    let engine = MyFontEngine::new(
        y901_render::load_font(&y901_render::default_test_font_path()).expect("font"),
        15.0,
        Size::new(cw, ch),
    )
    .expect("engine");
    SoftBackend::new_headless(W, H, Box::new(engine), Size::new(cw, ch)).expect("soft")
}

fn rgba(n: u32) -> Vec<u8> {
    let mut v = vec![0u8; (n * n * 4) as usize];
    for (i, p) in v.chunks_exact_mut(4).enumerate() {
        p[0] = (i % 251) as u8;
        p[1] = (i % 241) as u8;
        p[2] = (i % 239) as u8;
        p[3] = 255;
    }
    v
}

/// 首次上传：空纹理表 → `upload_texture` 一次。
#[test]
#[ignore = "基准：--ignored --nocapture --test-threads=1"]
fn first_upload_allocs() {
    let mut s = backend();
    let pixels = rgba(TEX);
    let _ = take();
    let id = s.upload_texture(TEX, TEX, &pixels);
    let (a, b) = take();
    eprintln!(
        "[tex] 首次上传 {TEX}×{TEX}：{a} 次分配 / {:.1} MiB",
        b as f64 / (1024.0 * 1024.0)
    );
    assert!(id.is_valid());
}

/// 后续渲染：纹理已上传，连续 100 帧 `draw_image`。
#[test]
#[ignore = "基准：--ignored --nocapture --test-threads=1"]
fn per_frame_draw_image_allocs() {
    let mut s = backend();
    let pixels = rgba(TEX);
    let id = s.upload_texture(TEX, TEX, &pixels);

    let rect = Rect::new(0.0, 0.0, W as f32, H as f32);
    let style = ImageStyle::default();
    // 预热
    s.draw_image(rect, id, style);

    const N: usize = 100;
    let _ = take();
    for _ in 0..N {
        s.draw_image(rect, id, style);
    }
    let (a, b) = take();
    eprintln!(
        "[tex] 后续 100 帧 draw_image（{TEX}×{TEX} 源）：{} 次分配 / {:.1} MiB，每帧 {:.1} MiB",
        a,
        b as f64 / (1024.0 * 1024.0),
        b as f64 / (1024.0 * 1024.0) / N as f64
    );
    // 零拷贝修复后，每帧不应再有任何分配（对照组清屏也是 0）。
    assert_eq!(a, 0, "每帧应 0 次分配，实测 {a}（{b} 字节）");
    assert_eq!(b, 0, "每帧应 0 字节分配，实测 {b}");
}

/// 对照：不做 `draw_image`，只清屏 100 帧 —— 用来扣除"渲染本身"的固定分配。
#[test]
#[ignore = "基准：--ignored --nocapture --test-threads=1"]
fn per_frame_clear_only_allocs() {
    let mut s = backend();
    s.clear();
    const N: usize = 100;
    let _ = take();
    for _ in 0..N {
        s.clear();
    }
    let (a, b) = take();
    eprintln!(
        "[tex] 对照：清屏 100 帧 = {a} 次分配 / {:.2} MiB（每帧 {:.3} MiB）",
        b as f64 / (1024.0 * 1024.0),
        b as f64 / (1024.0 * 1024.0) / N as f64
    );
    let _ = (Pos::new(0.0, 0.0), TextStyle::fg(Color::WHITE));
    assert!(a < 1000);
}

/// **快速回归**（非 ignored）：画一帧就要求零分配。
///
/// 为什么只画 1 帧：`per_frame_draw_image_allocs` 需要 100 帧才能统计分布，
/// 而“每帧复制整张纹理”这个回归**画一帧就能抓到**（一帧就是 4 MiB）。
#[test]
fn texture_draw_is_zero_alloc_quick() {
    let mut s = backend();
    let pixels = rgba(TEX);
    let id = s.upload_texture(TEX, TEX, &pixels);

    let rect = Rect::new(0.0, 0.0, W as f32, H as f32);
    let style = ImageStyle::default();
    s.draw_image(rect, id, style); // 预热

    let _ = take();
    s.draw_image(rect, id, style);
    let (a, b) = take();
    assert_eq!(a, 0, "texture draw must be zero-alloc, got {a} allocs");
    assert_eq!(b, 0, "texture draw must be zero-alloc, got {b} bytes");
}
