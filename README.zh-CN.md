# Y901（中文说明）

[English README](README.md)

Rust 跨平台 GUI 框架，自带 Win32/D3D11、X11 与 Web（wasm）后端。文本模型是**等宽 cell**
（`0.5em × 1em`），适合终端、日志、文件管理等桌面工具类应用。

## 安装：只需要一个 crate

```toml
[dependencies]
y901 = "0.4.0"
```

`y901` 是**单一入口**，内部把下面 10 个子 crate 全部 re-export 好：

```rust
use y901::prelude::*;          // 启动应用与画界面必用的类型与基础控件

fn main() -> Result<(), String> {
    // 业务代码里不出现任何 `#[cfg(target_os)]`：平台分叉在 y901 内部完成。
    let runtime = y901::create_runtime();
    let _ = runtime;           // 真正启动需要窗口环境，见 examples/
    Ok(())
}
```

| 想做什么 | 用什么 |
|---|---|
| 画界面 | `use y901::widgets::*;`（Button / Table / TextInput / Alert …） |
| 自定义控件 | 实现 `y901::ui::Widget` + 往里画 `y901::ui::RenderCtx` |
| 直接开窗口跑循环 | `y901::create_runtime()` → `.run(host, build, options)` |
| 只用布局/文本原语（不装平台后端） | 直接依赖 `y901-core` / `y901-ui` |

> 子 crate 都是**实现细节**：保留它们是为了编译期并行、依赖裁剪与平台条件依赖，
> 不是为了让你一个个装。需要更细控制时才直接依赖某个子 crate。

## 状态与平台支持

**实验性（experimental）**。平台分层见 [docs/PLATFORMS.md](docs/PLATFORMS.md)，简要版：

| Tier | 平台 | 承诺 |
|---|---|---|
| T1 | Windows / D3D11 | 完整验证、发布阻断 |
| T2 | Linux / X11 | 尽力支持，Xvfb 回归 + 真机抽检 |
| T3 | Web / wasm32 | **演示与触达**，非产品级 |
| 出范围 | macOS、Wayland | 不承诺；Wayland 走 XWayland |

## 组件

| 类别 | 组件 |
|---|---|
| 基础输入 | `Button` `TextInput` `TextArea` `Checkbox` `RadioGroup` `Switch` `Slider` |
| 容器/布局 | `Flex`（含 `radius`/`border`）`Grid` `Stack` `ScrollView` `SplitView` `Tabs` |
| 数据展示 | `Table` `ListView` `TreeView` `Image` `RichText` `TextLabel` |
| 反馈/状态 | `Spinner` `ProgressBar` `EmptyState` `ErrorState` `Alert` |
| 导航/浮层 | `Menu` `Dialog` `ContextMenu` `Tooltip` |

## 快速开始

5 分钟上手路径见 [docs/QUICKSTART.md](docs/QUICKSTART.md)（英文；公开入口统一英文）。

```bash
# 只看单一入口怎么写（只依赖 y901 一个 crate）
cargo run --example hello -p y901

# 组件画廊（在 y901-shell 里，所以要 -p）
cargo run -p y901-shell --example static_page     # 一屏静态页面：排版/颜色/圆角
cargo run -p y901-shell --example table           # 多列表格 + 选中行
cargo run -p y901-shell --example alert           # 四种 Alert
```

> 为什么画廊要 `-p y901-shell`：workspace 默认成员是单一入口 `y901`，而 18 个示例
> 都在应用壳 crate 里（`crates/y901-shell/examples/`）。`cargo run --example static_page`
> 不带 `-p` 只会找 `y901` 自己的 examples。

## 仓库结构（12 个 crate）

```
y901                 单一入口：re-export 全部子 crate + prelude + create_runtime()
y901-core            纯逻辑：终端 Grid / 格宽 / 区间
y901-ui              框架：Widget / UiTree / 布局 / 事件 / 主题 / 动画 / 语义树
y901-a11y            语义树 → AccessKit 桥（Windows UIA / Linux AT-SPI，feature 后）
y901-widgets         组件：Button / Table / Alert / Switch / ...
y901-extras          扩展：TerminalWidget（Grid + VT + PTY）
y901-render          软光栅器 + 自研 TTF 字形引擎 + 图集 + 文本 shaping
y901-runtime         平台无关契约：Host / PlatformRuntime / 帧 / 字体配置
y901-backend-win32   Win32 + D3D11 + DirectWrite + IME + DPI
y901-backend-x11     X11 后端（Xvfb 下验证）
y901-backend-web     浏览器后端（wasm32，单 canvas）
y901-shell           应用壳：create_runtime() + 剪贴板 + 密钥 + 截图
```

## 已知限制

- **国际文字：部分支持，需显式开启**。默认构建仍是原来的 cell 管线（逐字符查 cmap 字形 +
  等宽格排布）。`shaping` feature（默认关）才加入 rustybuzz shaping、Unicode 双向文本
  （P2/P3 方向判定 + 视觉重排）、字素簇切分与平台回退字体链（Windows：Segoe UI/Tahoma/
  YaHei/SimSun/Nirmala；Linux：DejaVu/Noto 等）。**仍未做**：连字符断词、两端对齐、
  避头尾标点、emoji ZWJ 序列、按 run 切多字体面，以及任何真实语料上的复杂文种验证。
  CJK 能正常显示，是因为汉字/假名/谚文大多能落进“整字 2 格、宽 1 em”的等宽格契约，
  **不代表已支持完整复杂排版**。
- **无障碍：语义树 + AccessKit 桥**。`Widget::semantics` 已是完整语义模型，另有可选的 `y901-a11y` crate（facade 的 `a11y` feature）把它映射成 AccessKit，平台适配器 Windows UIA / Linux AT-SPI 都在 feature 后。**尚未做**：后端自动接线与读屏器验收。
- **macOS 与 Wayland 后端未实现**；Wayland 用户走 XWayland。
- **Web 后端是 Tier 3**：单 canvas、整帧 `putImageData`、隐藏 `<input>` 承接 IME（候选窗不可钉位）、
  无多窗口/FileDrop。DOM 语义只在可选的 `a11y` feature 下提供（隐藏 ARIA 镜像，已接入帧循环），
  且**未在真实浏览器与读屏器上验证过**。单入口 `y901` 现在可以构建 `wasm32`；
  `TerminalWidget` 也能用，但 `attach_shell` 会返回错误 —— 浏览器里没有进程可 fork。
- **文本模型要求等宽字体**：`cell_metrics_for` 固定按 `0.5em × 1em` 出 cell。
  默认字体 `NSimSun`（等宽、中英文齐全）。`FontConfig::modern()` 可切到
  `Microsoft YaHei UI`，但**只建议做视觉试验**——比例字体按固定步进排布会字距不均。
- **组件内无法启动动画**：`UiTree::animate(..)` 在树侧，`EventCtx` 没有入口；
  因此 `Switch` 的滑块是即时移动（滚动/弹层动画不受影响）。
- **Grid + Image 组合整帧空白**：已在 v0.2.1 修复（`draw_instances` 的
  `StartInstanceLocation`），回归用例 `crates/y901-backend-win32/tests/grid_image_gpu.rs`。
- **API 仍是 0.x 语义**：破坏性变更会升 minor（0.3.0…）并在
  `docs/RELEASE_NOTES_v*.md` 里给迁移说明；已发布的版本号不会被复用。

## 反馈与问题报告

0.x 项目不承诺响应时间，但报告都会被看：

- **Bug / 功能请求** —— 在 <https://github.com/901548/y901/issues> 开 issue（有模板）；
  请附上 `y901` 版本、系统与后端（Win32 / X11 / Web）、最小复现。
- **安全问题** —— **不要开公开 issue**。走仓库的 **Security → Report a vulnerability**
  （见 [SECURITY.md](SECURITY.md)）；若你看不到该入口，请开一个标题为 `[SECURITY]`
  且不含细节的 issue，我会私下联系你。
- **使用问题** —— 同上 issue 区，打 `question` 标签。

## 构建与测试

数字与证据以 [`docs/STATUS.md`](docs/STATUS.md) 和 [`docs/evidence/`](docs/evidence/) 为准（由脚本生成，不手写）。

```bash
cargo check --workspace --all-targets      # 0 warning
cargo test --workspace                     # 1295 passed / 0 failed / 70 ignored（188 个测试目标）
powershell -File scripts/hash-gate.ps1     # 像素门禁（基线 0xF7455CCE7781A864，400x220）
powershell -File scripts/gate.ps1          # 完整门禁（发布前必跑）
```

> 上面 `cargo test` 那行的数字由 `scripts/sync-doc-numbers.ps1` 生成并校验，**不要手改**
> （门禁会比对 `docs/evidence/numbers.txt`）。

## 文档

- [docs/QUICKSTART.md](docs/QUICKSTART.md)：5 分钟上手（英文）
- [docs/PLATFORMS.md](docs/PLATFORMS.md)：平台分层与出范围声明（英文）
- [docs/ROADMAP.md](docs/ROADMAP.md)：复杂文本/无障碍/API/组件的整改路线图
- [docs/RELEASE_NOTES_v0.4.0.md](docs/RELEASE_NOTES_v0.4.0.md)：0.4.0 发布说明（已发布）
- [docs/RELEASE_NOTES_v0.3.0.md](docs/RELEASE_NOTES_v0.3.0.md)：0.3.0 发布说明（已发布）
- [docs/MIGRATION_0.3.md](docs/MIGRATION_0.3.md)：0.2.x → 0.3.0 迁移说明
- [docs/CRATES_IO.md](docs/CRATES_IO.md)：发布流程与踩过的坑
- [docs/FRAMEWORK.md](docs/FRAMEWORK.md)：架构、组件模型、坐标系与裁剪、已知限制
- [docs/CROSS_PLATFORM_PLAN.md](docs/CROSS_PLATFORM_PLAN.md)：X11 / macOS 路线
- [docs/evidence/hash-baseline.txt](docs/evidence/hash-baseline.txt)：像素门禁基线

## 许可

双许可，任选其一：[MIT](LICENSE-MIT) 或 [Apache-2.0](LICENSE-APACHE)。