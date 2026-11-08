//! 图片墙示例：`Grid` 布局 + PNG 图片 + 三种适配模式。
//!
//! 运行：
//! ```text
//! cargo run --example image_gallery -p y901-app
//! ```
//!
//! 展示：
//! 1. `Grid` 的固定列 / 弹性列混合（`GridTrack`）；
//! 2. `Image::from_png` 解码 + `ImageStyle` 的 `Fill` / `Contain` / `Cover` 与 tint；
//! 3. 图片尺寸由控件自己测量，布局不需要手写坐标。

use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::grid::{Grid, GridTrack};
use y901_ui::image::{ImageFit, ImageStyle};
use y901_ui::{Color, Command, Edges, Flex, FlexItem, UiEvent, UiTree};
use y901_widgets::image::Image;
use y901_widgets::TextLabel;

/// 示例自带的 4 色方块图（64×64，顶点着色 / 透明无关）。
const SAMPLE_PNG: &[u8] = include_bytes!("assets/quadrants.png");

struct GalleryDemo;

impl Host for GalleryDemo {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false
    }
}

/// 一种适配模式的图块（图 + 说明文字）。
fn tile(id: &str, title: &str, style: ImageStyle) -> Flex {
    let img = Image::from_png(format!("{id}-img"), SAMPLE_PNG)
        .expect("内置 PNG 应能解码")
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
    // 3 列等宽：200 行也不需要手算坐标
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
        Box::new(tile("fill", "Fill（拉伸）", ImageStyle::default())),
        Box::new(tile(
            "contain",
            "Contain（完整可见）",
            ImageStyle {
                fit: ImageFit::Contain,
                ..Default::default()
            },
        )),
        Box::new(tile(
            "cover",
            "Cover（裁切铺满）",
            ImageStyle {
                fit: ImageFit::Cover,
                ..Default::default()
            },
        )),
        Box::new(tile(
            "tint",
            "Tint（着色为橙色）",
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
            TextLabel::new("hint", "Grid + Image：四种适配模式")
                .color(Color::rgba(0.8, 0.86, 0.95, 1.0))
                .flex_item(FlexItem::height(24.0)),
        ))
        .push(Box::new(grid.flex_item(FlexItem::fill())));

    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    tree
}

fn main() -> Result<(), String> {
    let options = Options {
        title: "Y901 · image gallery".to_string(),
        width: 780,
        height: 420,
        ..Default::default()
        };
    create_runtime().run(Box::new(GalleryDemo), Box::new(build), options)
}
