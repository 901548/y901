//! **shaped 文本不得每帧克隆字体字节**（feature `shaping`）。
//!
//! 旧实现里 `draw_text_shaped` 收集"字体段"时写的是
//! `self.text.font_data_for(face).map(<[u8]>::to_vec)` —— 每画一次 shaped 文本
//! 就把**整份字体**复制一遍（测试字体 0.5~5 MB）。阿拉伯/希伯来/组合记号的
//! 每一帧、每一段都付这个代价，而且看起来只是"渲染慢"。
//!
//! 这条用例用计数分配器证明：一次绘制里**没有任何一次分配**接近字体大小。

#![cfg(feature = "shaping")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static MAX_SINGLE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        MAX_SINGLE.fetch_max(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size, Ordering::Relaxed);
        MAX_SINGLE.fetch_max(new_size, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn reset() {
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    MAX_SINGLE.store(0, Ordering::Relaxed);
}

use y901_render::soft::SoftBackend;
use y901_render::MyFontEngine;
use y901_ui::{Color, Pos, RenderCtx, Size, TextStyle};

fn backend() -> (SoftBackend, usize) {
    let (cw, ch) = y901_render::cell_metrics_for(16.0);
    let cell = Size::new(cw, ch);
    let data = y901_render::load_font(&y901_render::default_test_font_path()).expect("font");
    let font_len = data.len();
    let engine = MyFontEngine::new(data, 16.0, cell).expect("engine");
    (
        SoftBackend::new_headless(400, 120, Box::new(engine), cell).expect("soft"),
        font_len,
    )
}

#[test]
fn shaped_draw_does_not_clone_the_font() {
    let (mut s, font_len) = backend();
    s.clear();
    let style = TextStyle::fg(Color::WHITE).size(16.0);

    // 预热：第一次会把字形光栅化进图集（会分配，但不该是"整份字体"级别的大分配）
    s.draw_text(Pos::new(8.0, 8.0), "שלום hello", &style);

    // 正式计时：此时字形已在缓存里，这次绘制**不应**再复制字体
    reset();
    s.draw_text(Pos::new(8.0, 8.0), "שלום hello", &style);
    let max_single = MAX_SINGLE.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);

    assert!(
        max_single < font_len / 2,
        "一次绘制里出现了 {} 字节的单次分配，接近字体大小（{font_len}）—— \
         很可能又在克隆整份字体字节",
        max_single
    );
    // 总量上限取"字体大小"：shaping 本身（unicode-bidi + rustybuzz）本来就要分配
    // 几百 KB 的工作区，但**再加上一次字体克隆**就会越过这条线（实测：克隆在时
    // 基本等于 `字体 + 616 KB`）。所以这条断言专门盯"有没有把字体也拷了一份"。
    assert!(
        bytes < font_len,
        "一次绘制共分配 {bytes} 字节（字体 {font_len} 字节）：把整份字体也算进去的嫌疑很大"
    );
}
