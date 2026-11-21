#![cfg(windows)]

//! **Grid + Image 在 D3D 后端整帧空白**的复现与回归。
//!
//! 背景（README「已知限制」/ FRAMEWORK §16）：`examples/image_gallery.rs`（Grid 里放 Image）
//! 在 D3D11 cached-RT 路径下整帧空白，只剩清屏色；Flex + Image 正常、Grid + 文字正常。
//! 之前的结论是"差异在 `for b in batches` 循环"，需要用逐批次证据定位。
//!
//! 本用例把示例树原样缩小重建（Grid + 4 个 Image + 文字），渲染一帧后：
//!   1. 打印快照的颜色直方图与图片批次信息（复现时用）；
//!   2. 断言画面**不是**单色（整帧空白会只剩清屏色）；
//!   3. 断言四象限图的主色真的出现（图片确实被画出来）。

use std::collections::HashMap;

use y901_backend_win32::window::Window;
use y901_runtime::Options;
use y901_ui::grid::{Grid, GridTrack};
use y901_ui::image::{ImageFit, ImageStyle};
use y901_ui::{Color, Edges, Flex, FlexItem, UiTree};
use y901_widgets::image::Image;
use y901_widgets::TextLabel;

/// 2×2 四象限纹理（红/绿/蓝/白）——不依赖外部 PNG 资源。
fn quad_rgba() -> Vec<u8> {
    vec![
        255, 0, 0, 255, // (0,0) 红
        0, 255, 0, 255, // (1,0) 绿
        0, 0, 255, 255, // (0,1) 蓝
        255, 255, 255, 255, // (1,1) 白
    ]
}

fn tile(id: &str, title: &str, style: ImageStyle) -> Flex {
    let img = Image::from_rgba(format!("{id}-img"), 2, 2, &quad_rgba())
        .with_style(style)
        .flex_item(FlexItem::fill());

    Flex::column(format!("{id}-cell"))
        .gap(6.0)
        .background(Color::from_rgb8(20, 20, 28))
        .padding(Edges::all(8.0))
        .push(Box::new(
            TextLabel::new(format!("{id}-title"), title)
                .color(Color::rgba(0.85, 0.9, 1.0, 1.0))
                .flex_item(FlexItem::height(20.0)),
        ))
        .push(Box::new(img))
}

fn build() -> UiTree {
    let grid = Grid::new(
        "grid",
        vec![
            GridTrack::Fraction(1.0),
            GridTrack::Fraction(1.0),
            GridTrack::Fraction(1.0),
        ],
        vec![GridTrack::Fraction(1.0)],
    )
    .gap(12.0, 12.0)
    .with_children(vec![
        Box::new(tile("fill", "Fill", ImageStyle::default())),
        Box::new(tile(
            "contain",
            "Contain",
            ImageStyle {
                fit: ImageFit::Contain,
                ..Default::default()
            },
        )),
        Box::new(tile(
            "cover",
            "Cover",
            ImageStyle {
                fit: ImageFit::Cover,
                ..Default::default()
            },
        )),
        Box::new(tile(
            "tint",
            "Tint",
            ImageStyle {
                fit: ImageFit::Contain,
                tint: Some(Color::rgba(1.0, 0.62, 0.2, 1.0)),
                clip: None,
            },
        )),
    ]);

    let root = Flex::column("root")
        .background(Color::from_rgb8(12, 12, 16))
        .padding(Edges::all(16.0))
        .gap(12.0)
        .push(Box::new(
            TextLabel::new("hint", "Grid + Image")
                .color(Color::rgba(0.8, 0.86, 0.95, 1.0))
                .flex_item(FlexItem::height(24.0)),
        ))
        .push(Box::new(grid.flex_item(FlexItem::fill())));

    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    tree
}

static D3D_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn window(title: &str, w: u32, h: u32) -> Option<Window> {
    let options = Options {
        title: title.to_string(),
        width: w,
        height: h,
        theme: None,
        font: None,

        fullscreen: false,
    };
    Window::create(&options).ok()
}

fn rgb(px: &[u8], w: u32, x: u32, y: u32) -> (u8, u8, u8) {
    let i = ((y * w + x) * 4) as usize;
    (px[i + 2], px[i + 1], px[i])
}

#[test]
fn grid_with_images_is_not_blank_on_gpu() {
    let _guard = D3D_LOCK
        .lock()
        .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
    let Some(mut win) = window("y901-grid-image", 780, 420) else {
        eprintln!("[grid-image] 无 D3D11 设备，跳过（不能算通过）");
        return;
    };
    let mut tree = build();
    win.resize(&mut tree);
    tree.invalidate_all();
    win.request_snapshot();
    win.render(&mut tree);
    let (px, w, h) = match win.take_snapshot() {
        Some(v) => v,
        None => {
            let e = "take_snapshot 返回 None";
            // CI 的 windows runner 没有 GPU：D3D11 出不了图时**留痕跳过**（不是 panic，
            // 也不是静默 return —— gate 只认写进 y901-test-skips.txt 的 SKIP）。
            let path = std::env::temp_dir().join("y901-test-skips.txt");
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "SKIP: grid_image_gpu 快照失败（{e}）");
            }
            eprintln!("SKIP: grid_image_gpu 快照失败（{e}）");
            return;
        }
    };

    // 颜色直方图：整帧空白时只会有 1~2 种颜色（清屏色 + 可能残留）。
    let mut hist: HashMap<(u8, u8, u8), usize> = HashMap::new();
    for y in 0..h {
        for x in 0..w {
            *hist.entry(rgb(&px, w, x, y)).or_insert(0) += 1;
        }
    }
    let mut top: Vec<_> = hist.iter().map(|(&c, &n)| (n, c)).collect();
    top.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    eprintln!("[grid-image] 快照 {w}x{h}，不同颜色 {} 种", hist.len());
    for (n, c) in top.iter().take(8) {
        eprintln!("[grid-image]   {c:?} × {n}");
    }
    // 图片批次信息：受保护字段无法直接读，改为通过渲染日志（--nocapture 观察）。
    eprintln!(
        "[grid-image] 提示：设 Y901_DEBUG_BATCHES=1 可打印逐批次 start/count/image 与 draw 参数"
    );

    assert!(
        hist.len() > 8,
        "画面只有 {} 种颜色：整帧空白（复现成功）。直方图前三：{:?}",
        hist.len(),
        &top[..3.min(top.len())]
    );

    // 四象限主色至少出现两种（红/绿/蓝/白），说明图片批次真的被绑定并采样。
    let has_near = |want: (u8, u8, u8)| {
        hist.keys().any(|&(r, g, b)| {
            let d = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 40;
            d(r, want.0) && d(g, want.1) && d(b, want.2)
        })
    };
    let found = [
        ((255, 0, 0), "红"),
        ((0, 255, 0), "绿"),
        ((0, 0, 255), "蓝"),
        ((255, 255, 255), "白"),
    ]
    .iter()
    .filter(|(c, _)| has_near(*c))
    .count();
    assert!(
        found >= 2,
        "四象限主色只找到 {found} 种：图片没有被正确采样（整帧空白或纹理未绑定）"
    );
}
