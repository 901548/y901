# 参与贡献

先读这两条，能省掉来回：

1. **这是 0.x 实验性框架**，API 会变。大改动先开 issue 说清动机与影响面，
   别直接提几百行的 PR。
2. **门禁是硬要求**：PR 必须让 CI 五个 job 全绿（Windows 门禁 / Linux Xvfb / 打包校验 / 依赖策略 / semver）。
   "CI 红了但本地能跑"不算过 —— 那通常意味着你改了 `#[cfg(unix)]` 或平台专属代码。

> 平台承诺分三层（见 [`docs/PLATFORMS.md`](docs/PLATFORMS.md)）：Windows 是 T1（发布阻断），
> Linux/X11 是 T2（尽力支持），Web 是 T3（演示级，不承诺兼容）。macOS 与 Wayland 明确出范围。
> **公开入口文档用英文**（`README.md`、`docs/QUICKSTART.md`、`docs/PLATFORMS.md`）；
> 内部设计文档保持中文（`docs/FRAMEWORK.md`、`docs/STATUS.md` 等）。

## 开发环境

| 需要 | 说明 |
|---|---|
| Rust **1.92** | `rust-toolchain.toml` 已固定；MSRV 由 `scripts/gate.ps1` 卡 |
| Windows | 主开发平台。D3D11 测试需要真 GPU（无 GPU 的机器上这些测试会 SKIP） |
| Linux | 只用于 `#[cfg(unix)]` / X11 验证：`sudo apt install xvfb xauth xdotool` |
| 可选 | `cargo-audit`（`cargo install cargo-audit`），否则门禁里审计会被跳过 |
| 可选 | `cargo-deny` 0.21（`cargo install cargo-deny --version 0.21.0 --locked`），用于验证 `deny.toml` 的许可与来源策略 |

## 提交前自查（与 CI 同一份脚本）

```powershell
# Windows
powershell -File scripts/gate.ps1            # 期望 GATE OK（无 GPU 时用 -SkipGpu，结论会写 SKIP）

# Linux（WSL / VM / CI）
bash scripts/linux-verify.sh                 # 期望 LINUX VERIFY: OK（八步）
```

两条都过了再推。**不要**只跑 `cargo test` —— 门禁还包含像素哈希、测试计数基线、
test-inventory 指纹、doc 数字一致性、版本一致性。

依赖策略单独校验（`deny.toml`：安全公告、许可、来源、重复版本）：

```powershell
cargo deny check                 # 需要 cargo-deny 0.21（更高版本要求 rustc 1.95+）
```

`deny.toml` 里的 `ignore` 不是"已修复"：`RUSTSEC-2017-0008`（`serial`）和
`RUSTSEC-2026-0192`（`ttf-parser`）的理由与再评估条件见 [docs/DEPENDENCIES.md](docs/DEPENDENCIES.md)。

## 代码约定

| 约定 | 为什么 |
|---|---|
| **文档必须写"为什么"**，不只写"做了什么" | 这个仓库的历史坑都在注释/文档里；删掉动机等于删掉下次不踩坑的依据 |
| 不留"静默绿灯" | 测试跳过必须写 `y901-test-skips.txt` 留痕；解析失败要 panic 而不是 return |
| 性能断言默认不强制 | 共享机器上墙钟时间不可靠；需要时用 `Y901_PERF_GATE=1` 显式开启 |
| 新增 path 依赖必须写 `version` | 否则 `cargo publish` 直接失败（见 `docs/CRATES_IO.md`） |
| `categories` 只能用固定 slug | 同上，上传阶段才报错 |
| 新 crate 的 re-export 只能用"对端已发布版本里存在的路径" | 否则 `cargo publish` 的校验构建会对着 registry 里的旧版本编译并失败 |
| `.ps1` 必须带 UTF-8 BOM | Windows PowerShell 5.1 按 ANSI 读无 BOM 文件会解析失败 |

## 目录约定

```
crates/y901             单一入口（re-export + prelude）。新能力先进子 crate，再在这里 re-export
crates/y901-*           子 crate；`y901-shell` 是应用壳，18 个示例在它的 examples/
docs/                   设计文档 + 证据；数字由 scripts/sync-doc-numbers.ps1 生成，不要手写
docs/evidence/          原始日志（隐私：只脱敏用户名/家目录，其余不改）
scripts/                门禁与发布脚本，CI 直接复用
```

## 新增平台的步骤

1. 新建 `crates/y901-backend-<平台>` 实现 `PlatformRuntime` + `WindowBackend`；
2. 在 `crates/y901-shell/src/lib.rs` 的 `create_runtime()` 加一个 `#[cfg]` 分支；
3. 在 `crates/y901/Cargo.toml` 加对应 `[target]` 依赖，`crates/y901/src/lib.rs` 加 `backend` 别名；
4. 在 `scripts/publish-crates.ps1` 与 `scripts/check-packaging.ps1` 的依赖序里加一项。

## 许可

提交即表示同意以 MIT 或 Apache-2.0 双许可发布（与本仓库一致）。
