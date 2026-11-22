# Y901 v0.4.0 发布说明（**开发中，尚未发布**）

> 版本：`0.4.0` · 上一版：[`RELEASE_NOTES_v0.3.0.md`](RELEASE_NOTES_v0.3.0.md)（已发布）
>
> **这个版本还没有发布**：没有打 tag、没有上传 crates.io。本文随开发推进更新。

## 为什么会有 0.4.0

0.3.0 发布之后，`main` 上落了两个带 `!` 的提交，其中一个是**真实的破坏性变更**：

- `WasmState`（`crates/y901-backend-web/src/lib.rs`）新增了两个 `pub` 字段
  （`aria` / `aria_doc`）。它是可以被结构体字面量构造的公开类型，
  所以外部写 `WasmState { .. }` 的代码会编译失败 —— `cargo semver-checks` 报
  `constructible_struct_adds_field`。

按 0.x 约定，破坏性变更升 minor，因此 workspace 版本从 `0.3.0` 升到 `0.4.0`
（`scripts/check-semver-bump.ps1` 与 CI 的 `semver-guard` / `semver-checks` 两个 job 都以此为准）。

## 目前含有的变更

### 破坏性

- `y901-backend-web`: `WasmState` 增加 `pub aria` / `pub aria_doc` 字段
  （feature `a11y` 下存在）。用 `WasmState::new(canvas)` 构造的代码不受影响；
  直接写字面量的需要补字段或 `..` 形式。

### 新增

- `y901-widgets::TextArea`：多行编辑（软换行、跨行选区、`↑/↓` 保持目标列、
  `Enter` 换行、行首退格合并、IME 预编辑、可见行虚拟化、光标滚动入视）。
  见 [`ROADMAP.md`](ROADMAP.md) 的 S6 段与 `crates/y901-shell/examples/text_area.rs`。
- 门面 `prelude` 导出 `TextArea`。

### 修复

- `y901-shell` 的 `a11y` feature 现在会同时打开 `y901-backend-web/a11y`：
  之前浏览器用户开了 feature，ARIA 镜像代码根本不参与编译。
- `y901-extras` 的 PTY 依赖按目标门控，**单入口 `y901` 因此可以构建 `wasm32`**
  （此前 `portable-pty → filedescriptor` 在 wasm32 上编不过）。wasm 上
  `attach_shell` 返回明确错误，其余能力保留。
- `snapshot_harness::invalidate_rect_partial_equals_full` 的行框高度改取
  `TextMetrics::line_height`，不再写死 20px（150% DPI 下必红）。

### 工程

- 门禁新增 rustdoc 警告检查（`RUSTDOCFLAGS=-D warnings`）与 PowerShell 脚本
  UTF-8 BOM 检查。
- `sync-doc-numbers.ps1` 不再改写同一行里 Linux 段之后的数字（此前会把 Linux 计数
  覆盖成 Windows 的，而 `-Check` 放行）。
- 新增环境开关 `Y901_D3D_WARP` / `Y901_HEADLESS` / `Y901_SNAPSHOT_DEBUG`
  （默认关闭），研究记录见 [`evidence/cross-platform/warp-feasibility-2026-11-22.md`](evidence/cross-platform/warp-feasibility-2026-11-22.md)。

### 依赖

- `portable-pty` 0.8 → **0.9**：`serial`（`RUSTSEC-2017-0008`，2017 年起无维护）
  连同 `termios` / `ioctl-rs` / `memoffset 0.6.5` 一并移出，`nix` 0.25 → 0.28。
  `cargo audit` 的 allowed warning 从 7 条降到 6 条；剩下 6 条全部阻塞在上游，
  逐条结论见 [`DEPENDENCIES.md`](DEPENDENCIES.md)。
- `unicode-width` 0.2.2 → 0.2.3。
- CI 动作 `actions/checkout` 4 → 7、`actions/upload-artifact` 4 → 7。

## 发布前仍缺

1. 真机读屏器验收（Windows 讲述人 / Linux Orca / NVDA）—— 自 0.3.0 起一直未做；
2. `Options` 改私有字段 + builder（原本就规划在 0.4.0，尚未开始）；
3. 打 `v0.4.0` tag、上传 crates.io、推送授权。
