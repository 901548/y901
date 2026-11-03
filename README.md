# Y901

**纯 Rust 的跨平台桌面 GUI 框架**——双后端（Windows D3D11 / Linux X11 CPU 合成）、自研 TTF 字体引擎、
**零 C/C++ 编译依赖**（只有 `windows-sys` 一个原生绑定 crate），面向"人能读、Agent 也能读"的界面。

```
crates/
  y901-core     数据结构（Grid / Cell / Color / ColorSpec）
  y901-ui       机制层：Widget / UiTree / RenderCtx / 布局 / 主题 / 动画 / 图片
  y901-widgets  通用组件：Button / TextInput / Tabs / TreeView / Tooltip / Slider …
  y901-extras   子系统组件：终端（VT 解析 + PTY）
  y901-render   平台无关渲染器：软光栅 + 自研 TTF 字形引擎
  y901-runtime  平台契约：Host / EventSource / Presenter / PlatformRuntime
  y901-agent    语义树 + RPC + 策略（给 AI 用）
  y901-mcp      MCP 桥
  y901-backend-win32 / y901-backend-x11   两个平台后端
  y901-shell    平台选择 + 密钥 + 剪贴板
```

## 5 分钟快速开始

```toml
# Cargo.toml（当前为工作区内部使用；尚未发布到 crates.io）
[dependencies]
y901-ui = { path = "crates/y901-ui" }
y901-widgets = { path = "crates/y901-widgets" }
y901-runtime = { path = "crates/y901-runtime" }
y901-shell = { path = "crates/y901-shell" }
y901-agent = { path = "crates/y901-agent" }
```

```rust
use y901_agent::Policy;
use y901_runtime::{Host, Options};
use y901_shell::create_runtime;
use y901_ui::{Command, Flex, UiEvent, UiTree};
use y901_widgets::Button;

struct App;

impl Host for App {
    fn idle(&mut self, tree: &mut UiTree) {
        // 组件心跳：没有组件需要时立即返回，不占 CPU
        for _ in 0..4 {
            if !tree.dispatch(&UiEvent::Tick).is_handled() { break; }
        }
    }
    fn command(&mut self, _tree: &mut UiTree, _cmd: Command) -> bool { false }
    fn policy(&self) -> Policy { Policy::default() }
}

fn build() -> UiTree {
    UiTree::new(Box::new(
        Flex::column("root").push(Box::new(Button::new("btn", "点我"))),
    ))
}

fn main() -> Result<(), String> {
    create_runtime().run(
        Box::new(App),
        Box::new(build),
        Options { title: "Hello".into(), width: 480, height: 240, ..Default::default() },
    )
}
```

> 上例已在**干净目录**（仅 5 个路径依赖、无工作区继承）实际编译并运行验证：
> 首帧 9.1ms，窗口常驻正常。

## 界面

![TreeView 示例截图](docs/screenshot.png)

> 上图为 `tree_view` 示例的实际渲染结果（622×473，D3D11 后端）。
> 重新生成：`Y901_SNAPSHOT=docs/screenshot.png cargo run --example tree_view -p y901-app`
> （设置该环境变量后，示例在首帧渲染完成后自动截图并退出。）

## 可运行的示例

```bash
cargo run --example hello        -p y901-app   # 按钮 + 文字（最小用法）
cargo run --example counter      -p y901-app   # Flex + 状态 + 进度条
cargo run --example image_gallery -p y901-app  # Grid + PNG 图片 + 四种适配
cargo run --example tree_view    -p y901-app   # TreeView + 虚拟化 + 键盘导航
```

> 示例都在 `crates/y901-app/examples/`。写它们的过程暴露了 3 个 API 缺口（组件没导出到 crate 根、
> 部分组件缺 `flex_item` 构造器），已在本轮补齐——示例是**按用户会写的方式**写的，没有绕开 API。

## 能力清单

| 领域 | 已有能力 |
|---|---|
| **渲染** | 软光栅（CPU）与 D3D11 双后端；矩形/圆角/渐变/SDF 抗锯齿；字形图集 + LRU；纹理上传与双线性采样；变换栈（缩放）；像素回读 |
| **字体** | 自研 TTF 引擎（26.6 定点、自适应曲线扁平化、非零环绕填充、4 个亚像素相位）；CJK 支持 |
| **布局** | Flex（百分比/padding/gap）、Stack、Grid（`Fixed`/`Fraction`/`Auto` 轨道）、滚动容器、容器裁剪子控件 |
| **组件** | Button / TextLabel / TextInput / ListView / ScrollView / SplitView / Menu / Dialog / Tabs / ProgressBar / Checkbox / RadioGroup / Slider / Tooltip / ContextMenu / TreeView / Image / 终端 |
| **动画** | 7 条缓动曲线、时间轴、中途改目标（`retarget` 从当前值续）、完成回调、空闲零 CPU |
| **交互** | hover（含冒泡）、指针捕获、焦点与 Tab 遍历、IME、剪贴板、右键菜单、拖拽选区 |
| **Agent** | 语义树 + 差异订阅 + RPC + MCP 桥 + 策略过滤（默认不暴露） |

## 文档

- **[docs/FRAMEWORK_GUIDE.md](docs/FRAMEWORK_GUIDE.md)** —— 组件写法、能力清单（§40）、门禁与纪律（§41-42）
- **[docs/PERFORMANCE.md](docs/PERFORMANCE.md)** —— 布局 / 渲染 / 字体基准数据
- **[docs/CROSS_PLATFORM_PLAN.md](docs/CROSS_PLATFORM_PLAN.md)** —— 平台后端与已知限制
- **[docs/FONT_ENGINE_PLAN.md](docs/FONT_ENGINE_PLAN.md)** —— 自研字体引擎的设计与验收证明
- **`cargo doc --no-deps --open`** —— API 参考（`cargo doc` 无 `missing_docs` 警告）

## 工程纪律（摘要）

1. **四条件门禁**：`0 failed` + clippy 0 警告 / 0 错误 + 像素哈希（同尺寸下）一致 + `missing_docs` 为 0。
   全部由 `scripts/gate.ps1` 一次验证（不再手打）。
   只要漏一条，"验证后提交"就会变成看起来在执行的空动作（真实事故：`bbdbd89` 带 5 个 warning 提交）。
2. **像素断言先怀疑基准**：本项目出现过 6 次"度量方法本身是 bug"（背景取在被测物内部、扫全画布、
   用遮罩前的基准…）。断言红了先问"基准对不对"，再问实现。
3. **"画了但看不见" ≡ "没画"**：`TextStyle::default()` 曾是透明前景，导致文字完全不可见，
   看起来像"后端不支持文字"。默认值也是 API。

完整纪律见 [docs/FRAMEWORK_GUIDE.md §0.1 / §42](docs/FRAMEWORK_GUIDE.md)。

## 当前状态

| 项 | 状态 |
|---|---|
| 测试 | **728**（默认）/ **766**（`--include-ignored`），0 失败 |
| clippy | **0 warning / 0 error** |
| 文档完整性 | **12 个 crate 全部 `#![deny(missing_docs)]`**，`RUSTFLAGS="-W missing_docs"` = 0 |
| 像素哈希门禁 | 通过（`scripts/hash-gate.ps1`，同会话前后对比） |
| Windows | D3D11 渲染 + GDI 纯 CPU 回退；真机 RTX 4060 @144Hz 空帧 6.885ms（= vsync 间隔） |
| Linux/X11 | CPU 合成 + MIT-SHM；WSL 上 Xvfb 验证通过（单帧耗时有 `benchmark_shm_vs_putimage_1080p`，待补测） |
| macOS | **未实现** |
| 示例 | 4 个可运行（`hello` / `counter` / `tree_view` / `image_gallery`） |

### 已知限制

- 无合成粗体的轮廓加粗：CPU 后端用"向右 1px 涂抹"近似，非真粗体（见 `CROSS_PLATFORM_PLAN.md §8.5.1`）
- 文本在变换栈下**只平移、不缩放**
- Xvfb 下 `CreateGlyphCursor` 返回 `BadFont`，光标恒为默认箭头（真 Xorg 未验）
- 图片每张纹理一次 draw call（图集化未做）
- 图片纹理在窗口销毁路径不回收（进程退出无影响）
- Grid 只做顺序排布，不支持 `grid_cell(col,row)` / 跨格 span


## 许可证

内部项目，暂未选定开源许可证。
