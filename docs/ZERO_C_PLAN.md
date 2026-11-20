# 零 C/C++ 依赖审计（Y901 单仓）

> 版本：v0.2.3（2026-11-20 更新）　范围：**仅本仓库**（`F:\Y901`）
>
> 历史版本曾把另一个业务工程一起审计；该工程已拆分为独立仓库，
> 本文件与 [`ZERO_C_PROOF.md`](ZERO_C_PROOF.md) 的范围同步收窄为 Y901 自身。

## 0. 结论

**本仓库在 Windows 上满足"只装 Rust 即可完整构建"**：

| 证据 | 结果 |
|---|---|
| 仓库内 `build.rs` | **0 个**（`scripts/gate.ps1` 之外的 `Get-ChildItem -Recurse -Filter build.rs` 无输出） |
| 依赖树中带 `custom-build` 目标的 crate | **0 个** |
| 依赖树中的 `-sys` crate | 仅 `windows-sys`（纯绑定；Windows 官方元数据 crate） |
| 干净环境构建 | 最小 `PATH`（无 VS / gcc / cmake）+ 清空 `LIB/INCLUDE/VCINSTALLDIR` → `cargo build --release --workspace` **exit 0** |
| `RUSTFLAGS=-C linker=rust-lld` | 同样 **exit 0**（rustup 自带 lld） |

> **边界（诚实记录）**：本机装了 VS 2022，只能证明"不显式依赖 C 工具链"，
> 不能 100% 排除 rustc 通过注册表/vswhere 找到 `link.exe`。所以"只装 Rust"
> 的严格证明仍需一台**干净 VM**（见 §3 的 R3）。

## 1. 依赖审计（本仓）

本仓 10 个 manifest，外部直接依赖**全部纯 Rust**：

| crate | 外部直接依赖 | 判定 |
|---|---|---|
| `y901-core` | `unicode-width` | 纯 Rust（查表，无 build.rs） |
| `y901-ui` | **无**（只依赖内部 `y901-core`） | 纯 Rust |
| `y901-widgets` / `y901-extras` | 内部路径依赖 | 纯 Rust |
| `y901-render` | `ttf-parser`、`unicode-width`、`web-time` | 纯 Rust |
| `y901-runtime` | 内部 + `web-time` | 纯 Rust |
| `y901-backend-win32` | `windows-sys`、`ttf-parser` | 纯绑定 |
| `y901-backend-x11` | `rustbus`、`nix` | 纯 Rust / 纯绑定 |
| `y901-backend-web` | `wasm-bindgen` | 纯 Rust（绑定生成在编译期，无 C） |
| `y901-shell` | `y901-extras`、`windows-sys`、`russh` 等 | 见下 |

依赖树里的 `-sys` 只有 `windows-sys`；`flate2` 走 `miniz_oxide`（未启用 zlib 特性）；
`serde_yaml_ng` 取代 `serde_yaml` 后无 `unsafe-libyaml`；`russh` 的加密栈全纯 Rust。

## 2. 守卫（长期锁死）

`crates/y901-shell/tests/no_c_deps.rs::no_c_or_cpp_toolchain_dependencies`：

- 用 `cargo metadata` 解析依赖图，禁止黑名单 crate
  （`ring`、`aws-lc-rs`、`openssl`/`openssl-sys`、`libz-sys`、`cc`、`cmake`…）；
- `-sys` crate 必须在白名单（`windows-sys`、`windows-targets`、`windows_*` 等）；
- 扫描仓库内所有 `build.rs` 与依赖树的 `build.rs`，出现 `cc::Build` / `cmake::Config` / `bindgen::` 即失败。

**变异验证**（任一必须变红，恢复后变绿）：见 [`ZERO_C_PROOF.md`](ZERO_C_PROOF.md) §2。

## 3. 已知风险与决策

| # | 风险 | 决策 |
|---|---|---|
| R1 | 非 Windows 目标若启用 `native-tls` 会引入 `openssl-sys` | 本仓只承诺 Windows 桌面；Linux 走 `rustls`/系统能力，不启用 native-tls 后端 |
| R2 | 链接器仍需系统 `link.exe` | 允许 VS Build Tools **仅用于链接**；"只装 Rust"的严格边界见干净 VM 验证（未做，需外部环境） |
| R3 | 干净 VM 验证 | **待做**：需要一台无 VS 的 Windows VM（外部条件） |

## 4. 复现命令

```powershell
# 守卫（同时扫描本仓 build.rs 与依赖树）
cargo test -p y901-shell --test no_c_deps -- --nocapture

# 干净环境构建（最小 PATH，清空 MSVC 环境变量）
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Windows\System32;C:\Windows"
$env:LIB = $null; $env:INCLUDE = $null; $env:VCINSTALLDIR = $null
cargo build --release --workspace

# 依赖树里的 -sys
cargo tree --workspace --offline --prefix none | Select-String -- '-sys'
```
