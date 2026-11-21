# Y901 v0.2.6 发布说明（草案）

> 版本：`0.2.6` · 日期：2026-11-21 · 上一版：[`RELEASE_NOTES_v0.2.5.md`](RELEASE_NOTES_v0.2.5.md)
>
> **当前尚未发布，草案不代表发布就绪。** 本次仓库变更主要是版本号从 `0.2.5` 升至 `0.2.6`、X11 后端增加 opt-in XRandR 输出物理 DPI 查询，并修订发布运维文档；Windows 完整门禁已通过，但 Linux 真服务器/Xvfb 验证、cargo-deny 策略验证、公开 API 兼容性审查及 crates.io 上传仍未完成。

## 变更

- 所有 workspace crate 版本与内部 path 依赖版本更新至 `0.2.6`。
- README 安装示例改用 `y901 = "0.2.6"`，并明确 facade 示例应通过 `y901` 包运行。
- crates.io 运维文档中的 docs.rs 构建链接改为版本占位符。
- `y901-shell` 的 `hello` 示例改用唯一 target 名 `shell_hello`，避免与 facade crate 中的同名示例在 workspace 测试时争用输出文件。
- 发布脚本增加目标版本检查，并避免把 crates.io API 网络/服务错误误判成 crate 尚未发布。
- X11 后端新增 `randr` 模块与 `Connection::active_output_dpis()`：用原始协议查询 XRandR 1.3+，返回各活动输出的 CRTC 位置/尺寸与物理 DPI。`randr::dpi_at()` 可按窗口位置取所在显示器的 DPI；全局缩放仅在**恰好一个**活动输出时用其物理 DPI 替代整屏毫米估算，多输出时不猜屏（详见 [docs/design/dpi-policy.md](design/dpi-policy.md) §7.1）。
- 构建治理：新增 Dependabot、cargo-deny 策略（`deny.toml`，CI 新增 `dependency-policy` job）、issue/PR 模板；历史审查报告归档到 `docs/archive/reviews/`。

## 已完成的验证

- `powershell -NoProfile -File scripts\gate.ps1`：**GATE OK**（Windows）。含 MSRV、fmt、workspace 测试、测试清单指纹、clippy、像素哈希、missing_docs、版本一致性、文档数字同步与自测、`cargo audit`（0 漏洞、3 条已记录警告）。
- workspace 测试：`passed=996 failed=0`（基线 993）；测试清单 138 个二进制 / 1053 个测试与指纹一致。
- `scripts/check-version-consistency.ps1`：通过，workspace、crate manifests 与 lockfile 一致为 `0.2.6`。
- `scripts/check-packaging.ps1`：11 个 crate 均可打包。该脚本不等同于完整发布内容审核。
- `cargo deny check`（cargo-deny 0.21.0，与 MSRV 1.92 兼容）：`advisories ok, bans ok, licenses ok, sources ok`，无警告。`deny.toml` 的 3 条 ignore 均在 [docs/DEPENDENCIES.md](DEPENDENCIES.md) 有理由与再评估条件。
- Linux（WSL Ubuntu 22.04 + Xvfb）：
  - `cargo check -p y901-backend-x11 --all-targets` 通过（`#[cfg(unix)]` 代码真正编译）；
  - `xvfb-run -a cargo test -p y901-backend-x11 --test real_window`：17 passed / 2 ignored，含新增 `randr_active_output_dpi_query_is_safe_on_xvfb`（真服务器接受了 XRandR 请求）；
  - `cargo test -p y901-backend-x11 --lib`：23 passed，含新增单一输出缩放策略测试；
  - `xvfb-run -a cargo test --workspace --no-fail-fast`：通过（清理磁盘后完整复跑）。

## 尚未完成

- crates.io 实际发布、凭据确认、发布后安装验证及 tag 均未完成（本机未找到 `~/.cargo/credentials.toml`，也未设 `CARGO_REGISTRY_TOKEN`）。
- 不作公开 API 无破坏性变更的保证；已检查 `git diff v0.2.5` 的 `crates/*/src/*.rs` **无删除行**（只增不改），但仍建议发布前复核。

在这些检查完成前，不应将此文件作为正式发行说明或据此发布。