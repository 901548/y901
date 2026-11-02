//! TreeView 示例：层级数据 + 虚拟化 + 键盘导航。
//!
//! 运行：
//! ```text
//! cargo run --example tree_view -p y901-app
//! ```
//!
//! 操作：鼠标点行选中；点行首三角展开/折叠；方向键 ↑↓ 移动、→ 展开或进入子项、← 折叠或回父项。

use y901_agent::Policy;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Color, Command, Edges, Flex, FlexItem, UiEvent, UiTree};
use y901_widgets::tree_view::{TreeItem, TreeView};

/// 造一棵 3 层树，其中 `src` 下有 200 个文件用来展示虚拟化。
fn sample_tree() -> Vec<TreeItem> {
    let many_files: Vec<TreeItem> = (0..200)
        .map(|i| TreeItem::leaf(format!("file_{i:03}.rs")))
        .collect();

    vec![
        TreeItem::branch(
            "src",
            vec![
                TreeItem::leaf("main.rs"),
                TreeItem::leaf("lib.rs"),
                TreeItem::branch("widgets", vec![TreeItem::leaf("button.rs"), TreeItem::leaf("tree.rs")]),
                TreeItem::branch("generated", many_files),
            ],
        ),
        TreeItem::branch("docs", vec![TreeItem::leaf("FRAMEWORK_GUIDE.md")]),
        TreeItem::leaf("README.md"),
    ]
}

struct TreeDemo {
    /// `Y901_SNAPSHOT=<path>` 时：首帧渲染完自动截图并退出（用于生成文档截图）。
    snapshot_to: Option<std::path::PathBuf>,
    pending: bool,
}

impl TreeDemo {
    fn new() -> Self {
        let to = std::env::var_os("Y901_SNAPSHOT").map(std::path::PathBuf::from);
        Self {
            pending: to.is_some(),
            snapshot_to: to,
        }
    }
}

impl Host for TreeDemo {
    fn idle(&mut self, tree: &mut UiTree) {
        for _ in 0..4 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() {
                break;
            }
        }
    }

    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool {
        false // 这个示例没有应用级命令：交互全在 TreeView 内部
    }

    fn policy(&self) -> Policy {
        Policy::default()
    }

    fn take_snapshot_request(&mut self) -> bool {
        // 消费式：只请求一次
        std::mem::take(&mut self.pending)
    }

    fn on_snapshot(&mut self, pixels: &[u8], width: u32, height: u32) {
        let Some(path) = self.snapshot_to.clone() else {
            return;
        };
        match y901_shell::snapshot::write_bgra_png(&path, pixels, width, height) {
            Ok(()) => {
                eprintln!("[snapshot] 已写入 {} ({width}x{height})", path.display());
                std::process::exit(0);
            }
            Err(err) => {
                eprintln!("[snapshot] 失败: {err}");
                std::process::exit(1);
            }
        }
    }
}

fn build() -> UiTree {
    let root = Flex::column("root")
        .background(Color::from_rgb8(14, 14, 20))
        .padding(Edges::all(16.0))
        .gap(10.0)
        .push(Box::new(
            TreeView::new("tree", sample_tree())
                .flex_item(FlexItem::fill()),
        ));
    let mut tree = UiTree::new(Box::new(root));
    tree.invalidate_all();
    tree
}

fn main() -> Result<(), String> {
    let options = Options {
        title: "Y901 · tree view".to_string(),
        width: 640,
        height: 520,
        ..Default::default()
    };
    create_runtime().run(Box::new(TreeDemo::new()), Box::new(build), options)
}
