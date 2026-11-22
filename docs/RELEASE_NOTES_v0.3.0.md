# Y901 v0.3.0 发布说明

> 版本：`0.3.0` · 发布日：2026-11-22 · 上一版：[`RELEASE_NOTES_v0.2.5.md`](RELEASE_NOTES_v0.2.5.md)
>
> **已发布到 crates.io**：12 个 crate 全部上传完毕（见 §6 的实际记录）。
> 仍**未**完成的是真机读屏器验收（docs.rs 构建已确认成功），见 §6 的“仍未完成”一节。

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
- 平台适配（`y901-a11y` crate 的 target-specific feature）：Windows `windows-uia`（accesskit_windows），
  Linux `unix-atspi`（accesskit_unix / zbus）；两个 backend crate 对外暴露的开关均命名为 `a11y`；
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

## 4. 自动验证记录（Windows 本地与 Linux WSL，2026-11-22）

[`scripts/check-release-readiness.ps1`](../scripts/check-release-readiness.ps1) 汇总自动检查与人工/授权阻塞项。本轮运行结果：自动项全部通过；工作区已提交干净，仍有 4 项人工/授权 `BLOCKED`（crates.io 凭据、`v0.3.0` tag、0.3.0 是否已上传、真机读屏器验收）。数量会随凭据、tag、registry、工作区和验收状态变化。退出码 0 只表示没有自动项 `FAIL`，不表示发布阻塞项已清除。下表记录本轮执行的检查。

| 检查 | 结果 |
|---|---|
| `scripts/gate.ps1` | **GATE OK**（本机完整门禁：MSRV / fmt / 默认测试基线 / Windows 测试清单 / clippy / 像素哈希 / missing_docs / 版本一致 / 文档数字与自测 / audit；audit 使用当时可用的 advisory DB，非本轮联网刷新） |
| 默认测试（`cargo test --workspace`，2026-11-22 重新测量） | **1112 passed / 0 failed / 61 ignored / 155 个测试目标**（旧文档停留在 1053；`docs/evidence/numbers.txt` 已重新生成） |
| 全量测试（`--include-ignored`，2026-11-22） | **1173 passed / 0 failed**（含 30 条性能基准与 GPU 用例） |
| GPU 快照套件（`snapshot_harness --ignored`，真机 D3D11，150% DPI） | **10 passed / 0 failed** |
| `scripts/check-packaging.ps1` | **12 个 crate 全部可打包** |
| `scripts/check-version-consistency.ps1` | 通过（0.3.0，12 个 workspace crate） |
| `scripts/check-semver-bump.ps1` | 通过（v0.2.5 → 0.3.0 = minor） |
| `scripts/check-consumer.ps1 -Version 0.3.0` | 通过（发布后实测）：独立临时目录里从 registry 拉 `y901 0.3.0` 编译最小工程成功 |
| docs.rs 构建 | 通过（发布后实测）：`docs.rs/y901/0.3.0/y901/` 返回真实 rustdoc 页面，无构建失败提示 |
| `scripts/check-consumer.ps1 -Version 0.2.5`（旧版对照） | 通过：仅证明发布链路可用，不覆盖 0.3.0 新 API |
| `cargo semver-checks --workspace --exclude y901-a11y --baseline-rev v0.2.5` | 通过（新 crate 无基线，故排除） |
| `cargo deny check` | advisories/bans/licenses/sources 全 ok（使用缓存 advisories，不代表在线实时审计） |
| 像素哈希 | `0xF7455CCE7781A864`（**全 S2 改动后未变**） |
| `shaping` feature | `cargo test -p y901-render --lib --features shaping --offline` 126 passed / 1 ignored；wasm check passed |
| `a11y` feature | `y901-a11y` 32 tests + doctest passed；Win32/X11 backend `a11y` feature suites passed (the backend feature is named `a11y`) |
| Accessibility demo | `cargo check -p y901-shell --example a11y_demo` with and without `--features a11y` passed |

Linux（WSL Ubuntu 22.04 + Xvfb），本轮 2026-11-22 执行 `CARGO_FLAGS=--offline bash scripts/linux-verify.sh`：8/8 阶段通过；全 workspace `155` 个测试目标，1031 passed / 0 failed / 27 ignored。详细输出见 [`linux-verify-2026-11-22.log`](evidence/cross-platform/linux-verify-2026-11-22.log)。

> **GPU 套件的 DPI 假设已修正**：本机窗口缩放为 150%，`snapshot_harness` 的流式行框原先写死 20px
> （按 125% 写的），比 150% 下的 24px 物理行高矮 4px，导致 `invalidate_rect_partial_equals_full`
> 出现 740 字节差异。已改为按 `TextMetrics::line_height` 取行框高度，10 条 GPU 用例复跑全绿；
> 这项修正只改了测试，不改渲染行为。细节见 [`ACCEPTANCE.md`](ACCEPTANCE.md) 的“GPU 像素验收用例”一节。

## 5. 已知边界（不承诺）

- **复杂文本**：shaping 需显式开启 `shaping` feature；**默认不折行的组件**（只有
  `TextLabel::wrap(true)` 会折）；不做连字符断词、两端对齐、避头尾标点；
  emoji ZWJ 序列的集群判定未覆盖。
- **无障碍**：Windows UIA / Linux AT-SPI 原生后端有推树接线；Web ARIA 映射与 DOM helper 存在，但尚无 runtime 调用点。CI 的 `web-aria` 仅编译 wasm `a11y` feature，不是浏览器测试；未做浏览器 DOM 或人工读屏器验证。Windows 讲述人 / Linux Orca 真人验收**未完成**；Web 焦点代理亦未做。
- **平台**：macOS、Wayland 无后端；Web 是 T3（演示级）。
- **0.x API**：仍是 0.x 语义，破坏性变更集中在本次 0.3.0。

## 6. 发布记录与仍未完成项

### 已完成的发布动作（2026-11-22）

1. **crates.io 上传**：12 个 crate 按依赖序全部 `Uploaded`（`scripts/publish-crates.ps1`，`PUBLISH: OK`）。
   顺序：`core → ui → render → a11y → widgets → runtime → extras → backend-win32 → backend-x11 → backend-web → shell → y901`。
2. **下游验证**：`scripts/check-consumer.ps1 -Version 0.3.0` 通过 —— 独立临时目录里从 registry 拉
   `y901 0.3.0` 编译最小工程成功，证明发布链路可用（path 依赖测不到这一点）。
3. **tag 与 push**：`v0.3.0` 附注 tag 打在本次发布的对应提交上，并推送 `origin/main`。

### 仍未完成

1. **真机读屏器验收**：Windows 讲述人 / Linux Orca / NVDA 手工清单（机器无法替代）。
   无障碍桥能推树，但**没有任何真人验证过读屏效果**。

> **发布后（尚未发版）的进展**：Web 的 ARIA 镜像原来只有 helper、没有 runtime 调用点；
> 0.3.0 发布后已在 `main` 上接入帧循环（仅在上屏帧推送，语义未变则跳过 DOM 重建），
> 并在 `y901-a11y` 补了指纹去重的单测、CI `web-aria` 补了默认 feature 的对照编译。
> 该改动**不在 0.3.0 的产物里**，会随下一个版本发布；浏览器与读屏器验证仍未做。

### 已知的发布过程波折（不影响产物）

- 首个 token 只成功上传了 `y901-core`，其余全部 `403 authentication failed`（疑似 crate scope 限制或被撤销），已换新 token。
- 中途出现过一次 `SSL connect error (schannel)`，重试即成功（网络抖动，非认证问题）。
- `y901-core 0.3.0` 先于 README 的“反馈渠道”修订发布，因此它的 crates.io 页面**不含**新增的
  “反馈与问题报告”小节；其余 11 个 crate（含入口包 `y901`）都包含。
