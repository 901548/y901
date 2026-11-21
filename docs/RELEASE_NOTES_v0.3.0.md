# Y901 v0.3.0 发布说明（草案 · 未发布）

> 版本：`0.3.0` · 日期：2026-11-21 · 上一版：[`RELEASE_NOTES_v0.2.5.md`](RELEASE_NOTES_v0.2.5.md)
>
> **尚未发布。** 本文是发布候选的变更清单与验证记录，**不是公布承诺**：
> crates.io 上传、tag、真机读屏器验收都还没做（见 §6）。

## 1. 版本语义

`0.2.6` 的改动（版本号收敛、XRandR 输出 DPI、发布脚本加固）与后续 S1–S4 的破坏性变更
合并为 **0.3.0**：按 0.x 约定，破坏性变更升 minor。
`0.2.6` 这个版本号**从未发布**，因此不会出现在 crates.io 上。

迁移做法见 [`MIGRATION_0.3.md`](MIGRATION_0.3.md)（含 2b 节：为什么 `Options` 没有用
`#[non_exhaustive]`）。

## 2. 破坏性变更

- `Semantics` 新增无障碍字段（`description`/`expanded`/`checked`/`selected`/`level`/
  `value_range`/`value_step`/`labelled_by`/`described_by`/`live`）并标记 `#[non_exhaustive]`；
- `TableColumn` 新增 `sortable` 并标记 `#[non_exhaustive]`；
- `Table` 重写为虚拟滚动实现（表头固定、独立滚动条、只画可见行）；
- `y901-a11y` 是**新 crate**（第 12 个 workspace 成员）。

## 3. 新能力

### 3.1 文本管线（S2）

- 新增 `crates/y901-render/src/text/`：`GlyphKey`、bidi 方向判定与视觉重排、
  字素簇切分、`shaping` feature 下的 rustybuzz shaping；
- 图集与字形缓存键从 `char` 换成 `GlyphKey`，引擎新增 `glyph_keyed()`（按 glyph id 光栅化）；
- 字体回退链：`push_fallback()` / `face_for()`，按字符选面；
- 按字体面**分段**渲染：一段里混排多字体各自 shaping；
- 文本折行：`y901_ui::wrap` + `TextLabel::wrap(bool)`；
- 集群簇光标：`y901_ui::cluster`，光标不落簇中、Backspace 整簇删除；
- `shaping` 是**可选 feature**：默认构建的依赖树、像素基线与二进制体积不变。

### 3.2 无障碍（S3）

- 新 crate `y901-a11y`：`map_role` / `to_tree_update` / `PlatformBridge` / `NodeIdMap`；
- 平台适配（feature 门控）：Windows `windows-uia`（accesskit_windows），
  Linux `unix-atspi`（accesskit_unix / zbus）；
- 两个后端都**已经会推树**：`y901-backend-win32`（`ShowWindow` 之前挂载 UIA）、
  `y901-backend-x11`（`render_tree` 末尾推树 + 回派 Click 动作）；
- facade 增加 `a11y` feature（`pub use y901_a11y as a11y`），默认关。

### 3.3 组件（S1）

- `NumberInput`、`MenuBar`/`MenuEntry`；
- `Table`：三态排序、键盘导航（↑↓/Home/End/PageUp/PageDown）、虚拟滚动、`set_rows/select/sort_by`。

### 3.4 工程化

- 英文 `README.md` + 中文 `README.zh-CN.md`、`docs/QUICKSTART.md`、`docs/PLATFORMS.md`
  （T1 Windows / T2 X11 / T3 Web / macOS+Wayland 出范围）；
- `scripts/check-semver-bump.ps1`（`!` 提交必须升 minor/major）、
  `scripts/check-consumer.ps1` + `.github/workflows/consumer.yml`（从 crates.io 拉**已发布版**编译下游）；
- `deny.toml` + CI `dependency-policy`、`text-shaping`、`a11y-bridge`、`semver-guard`、
  `semver-checks` 五个 job；
- 历史审查报告归档到 `docs/archive/reviews/`。

## 4. 本次验证（Windows 本地实测）

**一条命令复核**：[`scripts/check-release-readiness.ps1`](../scripts/check-release-readiness.ps1)
（自动项全绿，并列出 6 项 `BLOCKED` 的人工/授权项）。

| 检查 | 结果 |
|---|---|
| `scripts/gate.ps1` | **GATE OK**（MSRV / fmt / 1053 tests / inventory 139 二进制 1156 用例 / clippy 0 警告 / 像素哈希 / missing_docs / 版本一致 / doc 数字 + 自测 / audit 0 漏洞 7 条已记录警告） |
| `scripts/check-packaging.ps1` | **12 个 crate 全部可打包** |
| `scripts/check-version-consistency.ps1` | 通过（0.3.0，11 个 lock 条目） |
| `scripts/check-semver-bump.ps1` | 通过（v0.2.5 → 0.3.0 = minor） |
| `cargo semver-checks --workspace --exclude y901-a11y --baseline-rev v0.2.5` | 通过（新 crate 无基线，故排除） |
| `cargo deny check` | advisories/bans/licenses/sources 全 ok |
| 像素哈希 | `0xF7455CCE7781A864`（**全 S2 改动后未变**） |
| `shaping` feature | `cargo test -p y901-render --lib --features shaping` 125 passed |
| `a11y` feature | Windows `--features windows-uia` 21 passed；Linux `unix-atspi` 21 passed |

Linux（WSL Ubuntu 22.04 + Xvfb）：`linux-verify.sh` 各步通过；全 workspace
`155` 个测试二进制全绿（`CARGO_INCREMENTAL=0`，磁盘清理后复跑）。

## 5. 已知边界（不承诺）

- **复杂文本**：shaping 需显式开启 `shaping` feature；**默认不折行的组件**（只有
  `TextLabel::wrap(true)` 会折）；不做连字符断词、两端对齐、避头尾标点；
  emoji ZWJ 序列的集群判定未覆盖。
- **无障碍**：真机读屏器（讲述人 / Orca）**未验收**；Web 的 DOM/ARIA 镜像未做。
- **平台**：macOS、Wayland 无后端；Web 是 T3（演示级）。
- **0.x API**：仍是 0.x 语义，破坏性变更集中在本次 0.3.0。

## 6. 发布前仍缺（阻塞发布）

1. **凭据**：本机无 `~/.cargo/credentials.toml`、无 `CARGO_REGISTRY_TOKEN` —— 需 `cargo login`；
2. **授权**：上传 crates.io、打 `v0.3.0` tag、推送 GitHub 均需明确授权；
3. **真机读屏器验收**：Windows 讲述人 / Linux Orca 手工清单（机器无法替代）；
4. **发布后验证**：`scripts/check-consumer.ps1 -Version 0.3.0` 与 docs.rs 构建。

在这些完成前，本文件不得作为正式发行说明。
