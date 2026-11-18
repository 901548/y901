# Y901

Rust 跨平台 GUI 框架，自带 Win32/D3D11 与 X11 后端。文本模型是**等宽 cell**
（`0.5em × 1em`），适合终端、日志、文件管理等桌面工具类应用。

## 状态

**实验性（experimental）**。Windows D3D11 路径经过完整验证（130 个测试目标、像素门禁）；
X11 后端可在 Xvfb 下跑通，**未做真机验收**；macOS 无后端。

## 组件

| 类别 | 组件 |
|---|---|
| 基础输入 | `Button` `TextInput` `Checkbox` `RadioGroup` `Switch` `Slider` |
| 容器/布局 | `Flex`（含 `radius`/`border`）`Grid` `Stack` `ScrollView` `SplitView` `Tabs` |
| 数据展示 | `Table` `ListView` `TreeView` `Image` `RichText` `TextLabel` |
| 反馈/状态 | `Spinner` `ProgressBar` `EmptyState` `ErrorState` `Alert` |
| 导航/浮层 | `Menu` `Dialog` `ContextMenu` `Tooltip` |

## 快速开始

```bash
cargo run --example static_page     # 一屏静态页面：排版/颜色/圆角
cargo run --example table           # 多列表格 + 选中行
cargo run --example alert           # 四种 Alert
```

## 仓库结构（9 个 crate）

```
y901-core            纯逻辑：终端 Grid / 格宽 / 区间
y901-ui              框架：Widget / UiTree / 布局 / 事件 / 主题 / 动画 / 语义树
y901-widgets         组件：Button / Table / Alert / Switch / ...
y901-extras          扩展：TerminalWidget（Grid + VT + PTY）
y901-render          软光栅器（SoftBackend）+ 自研 TTF 字形引擎 + 图集
y901-runtime         平台无关契约：Host / PlatformRuntime / 帧 / 字体配置
y901-backend-win32   Win32 + D3D11 + DirectWrite + IME + DPI
y901-backend-x11     X11 后端（Xvfb 下验证）
y901-shell           应用壳：create_runtime() + 剪贴板 + 密钥 + 截图
```

## 构建与测试

```bash
cargo check --workspace --all-targets      # 0 warning
cargo test --workspace                     # 130 个测试目标
powershell -File scripts/hash-gate.ps1     # 像素门禁（基线 **956**x39AE41C1697CBFD0）
```

## 已知限制

- **Linux 输入法（IME）未实现**：中文只能显示、不能输入。
- **macOS 后端未实现**。
- **无障碍只有语义树**（`Widget::semantics`），**没有 OS 桥接**（UIA / AT-SPI）。
- **Grid + Image 组合在 D3D cached-RT 路径下整帧空白**：已排除 clip/blit/scissor/脏区/实例超限，
  根因未定位，需要 PIX/RenderDoc（详见 `docs/FRAMEWORK.md` §16）。
- **组件内无法启动动画**：`UiTree::animate(..)` 在树侧，`EventCtx` 没有入口；
  因此 `Switch` 的滑块是即时移动（滚动/弹层动画不受影响）。
- **文本模型要求等宽字体**：`cell_metrics_for` 固定按 `0.5em × 1em` 出 cell。
  默认字体 `NSimSun`（等宽、中英文齐全）。`FontConfig::modern()` 可切到
  `Microsoft YaHei UI`，但**只建议做视觉试验**——比例字体按固定步进排布会字距不均。
- 组件库**未发布到 crates.io**（当前只支持 `path` 依赖）。

## 文档

- `docs/FRAMEWORK.md`：架构、组件模型、坐标系与裁剪、已知限制
- `docs/CROSS_PLATFORM_PLAN.md`：X11 / macOS 路线
- `docs/evidence/hash-baseline.txt`：像素门禁基线