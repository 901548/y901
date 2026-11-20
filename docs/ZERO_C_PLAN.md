# 零 C/C++ 依赖审计（Y901 单仓）

> 版本：v0.2.4（2026-11-20 更新）　范围：**仅本仓库**（`F:\Y901`）
>
> 历史版本曾把另一个业务工程一起审计；该工程已拆分为独立仓库，
> 本文件与 [`ZERO_C_PROOF.md`](ZERO_C_PROOF.md) 的范围同步收窄为 Y901 自身。

## 0. 结论

**本仓库源码与依赖图不显式要求 C/C++ 编译器；Windows 侧"只装 Rust 即可构建"已在本机实测成立，
但运行期默认产物需要 VC 运行库**：

| 证据 | 结果 |
|---|---|
| 仓库内 `build.rs` | **0 个**（`scripts/gate.ps1` 之外的 `Get-ChildItem -Recurse -Filter build.rs` 无输出） |
| 依赖树中带 `custom-build` 目标的 crate | **0 个**（当前审计） |
| workspace 所有目标的 `-sys` crate | 仅 Windows 系统绑定（如 `windows-sys`）；Linux 另使用 libc ABI（不等于编译 C 源码） |
| 本机是否装了 VS/SDK | **没有 Win10/11 SDK**（`C:\Program Files (x86)\Windows Kits\10` 不存在）；`link.exe` / `cl.exe` 不在 PATH；`vswhere -latest -property installationPath` **为空**；`VSINSTALLDIR`/`VCINSTALLDIR`/`WindowsSdkDir` 全空 |
| 干净环境构建（清空 `LIB`/`INCLUDE`/`VC*`，PATH 只留 rustup+System32，`-C linker=rust-lld`） | **exit 0**（`scripts/win-clean-probe.ps1` 实测，独立 target 目录强制重链） |
| 链接命令行里的 VS/SDK 路径 | **0 处**（用自写 wrapper 记录 argv：无 `/libpath`、无 MSVC/Kits 路径；默认产物**一个导入库都不带**，全是 `raw-dylib`） |
| 默认产物的运行时依赖 | 导入 `VCRUNTIME140.dll` + `api-ms-win-crt-*` → **需要 VC 运行库**；`+crt-static` 产物**零 CRT 导入** |

> **边界（诚实记录）**：本机有 VS 安装目录，但**没有 Win10/11 SDK、没有在 PATH 上的 link.exe/cl.exe**，
> 而 clean-env 构建仍然成功 —— 说明这条路径不依赖它们。仍未做的只有一步：
> 在**全新 Windows VM**（连 VS 目录都没有）上跑一次 `scripts/win-clean-probe.ps1` 复核（见 §3 R3）。
> 另外：运行期默认要 VC 运行库（`VCRUNTIME140.dll` 在 System32 里，本机有、干净机可能没有），
> 要"解压即用"请用 `+crt-static`。Linux 侧另需 libc/CRT 与 Xvfb，不属于"只装 Rust"。

## 1. 依赖审计（本仓）

本仓 10 个 manifest，外部直接依赖**不要求编译 C/C++ 源码**（系统绑定单独标明）：

| crate | 外部直接依赖 | 判定 |
|---|---|---|
| `y901-core` | `unicode-width` | 纯 Rust（查表，无 build.rs） |
| `y901-ui` | **无**（只依赖内部 `y901-core`） | 纯 Rust |
| `y901-widgets` / `y901-extras` | 内部路径依赖 | 纯 Rust |
| `y901-render` | `ttf-parser`、`unicode-width`、`web-time` | 纯 Rust |
| `y901-runtime` | 内部 + `web-time` | 纯 Rust |
| `y901-backend-win32` | `windows-sys`、`ttf-parser` | 纯绑定 |
| `y901-backend-x11` | `rustbus`、`nix` | Rust 依赖；通过 libc ABI 调用系统接口（不编译 C 源码） |
| `y901-backend-web` | `wasm-bindgen` | 纯 Rust（绑定生成在编译期，无 C） |
| `y901-shell` | `y901-extras`、`windows-sys`、`russh` 等 | 见下 |

依赖树里的 `-sys` 只有 `windows-sys`；`flate2` 走 `miniz_oxide`（未启用 zlib 特性）；
`serde_yaml_ng` 取代 `serde_yaml` 后无 `unsafe-libyaml`；`russh` 的加密栈全纯 Rust。

## 2. 守卫（长期锁死）

`crates/y901-shell/tests/no_c_deps.rs::no_c_or_cpp_toolchain_dependencies`：

- 用 `cargo tree --workspace --target all --prefix none --offline` 解析 workspace 全成员/所有目标依赖图，禁止黑名单 crate
  （`ring`、`aws-lc-rs`、`openssl`/`openssl-sys`、`libz-sys`、`cc`、`cmake`…）；
- `-sys` crate 必须在白名单（`windows-sys`、`windows-targets`、`windows_*` 等）；
- 扫描仓库内所有 `build.rs` 与依赖树的 `build.rs`，出现 `cc::Build` / `cmake::Config` / `bindgen::` 即失败。

**变异验证**（任一必须变红，恢复后变绿）：见 [`ZERO_C_PROOF.md`](ZERO_C_PROOF.md) §2。

## 3. 已知风险与决策

| # | 风险 | 决策 |
|---|---|---|
| R1 | 非 Windows 目标若启用 `native-tls` 会引入 `openssl-sys` | 本仓只承诺 Windows 桌面；Linux 走 `rustls`/系统能力，不启用 native-tls 后端 |
| R2 | 目标平台仍需链接器/系统运行时与 SDK 元数据；Windows 使用 `windows-sys`，Linux 使用 libc ABI | 不将其混同为 C 源码编译依赖。**已实测 Linux 侧**：`x86_64-unknown-linux-gnu` 上去掉 `cc` 后，rust-lld 要么找不到 `-lc/-lm/-lgcc_s`，要么产出**无 `PT_INTERP`** 的二进制并 `SIGSEGV`（见 R2 下方表格）——即 GNU 目标**必须有系统 C 工具链驱动来链接**，尽管它不编译任何 C 源码 |
| R3 | 干净 VM 验证 | **只剩一步**：本机已无 Win10/11 SDK、无 PATH 上的 `link.exe`/`cl.exe`，clean-env 构建已通过（§0 表格）；仍建议在**全新 Windows VM** 上跑一次 `scripts/win-clean-probe.ps1` 复核。Linux 侧的等价证明在 §5（WSL2，非全新镜像） |

### R2 实测：`cc` 缺失时会发生什么（Linux x86_64，2026-11-20）

| 尝试 | 结果 |
|---|---|
| 默认（`cc` 存在） | `cargo build --release --workspace` 成功 |
| PATH 里去掉 `cc`/`gcc`/`ld`/`as`/`ar`，`-C linker=rust-lld` | **失败**：`rust-lld: unable to find library -lgcc_s / -lutil / -lrt / -lpthread / -lm / -ldl / -lc`（rust-lld 不读 gcc 的库搜索路径） |
| 上一行 + 显式 `-L/usr/lib/x86_64-linux-gnu -L/usr/lib/gcc/x86_64-linux-gnu/11` | **编译过了、程序崩了**：产出 ELF **没有 `PT_INTERP`**（`readelf -l` 无 interpreter 行），运行即 `SIGSEGV` |
| 最小 `rustc` 单文件对照（同上三种配置） | 同样：默认成功、仅 `rust-lld` 链接失败、加 `-L` 后 `run_exit=139` |

结论（精确口径）：**"不编译 C/C++ 源码" 成立，且由守卫长期锁死；"只装 Rust 就能构建" 在 GNU/Linux 上不成立** ——
GNU 目标需要系统 C 工具链提供的 `cc` 驱动（它负责 `-dynamic-linker` 与库搜索路径），
以及 libc/CRT 本体。这与"依赖里没有 C 源码"不矛盾，但**不能**对外宣称"零系统依赖"。
Windows 侧结论相反（见 §0 与 `scripts/win-clean-probe.ps1`）：不需要 VS/SDK，但默认产物运行时要 VC 运行库。

## 4. 复现命令

```powershell
# 守卫（同时扫描本仓 build.rs 与依赖树）
cargo test -p y901-shell --test no_c_deps -- --nocapture

# Windows 侧完整探针（机器上有什么 / 干净环境构建 / 产物 CRT 依赖 / +crt-static 对照）
powershell -NoProfile -File scripts/win-clean-probe.ps1

# 等价的单条命令（探针的第 2 步就是它）
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Windows\System32;C:\Windows"
$env:LIB = $null; $env:INCLUDE = $null; $env:VCINSTALLDIR = $null
$env:RUSTFLAGS = "-C linker=rust-lld"
cargo build --release --workspace --offline --target-dir target\clean-probe

# 所有 workspace members 与目标（--target all）依赖树里的 -sys
cargo tree --workspace --target all --offline --prefix none | Select-String -- '-sys'
```

## 5. Linux 侧"没有 C 工具链会怎样"的实测（2026-11-20，WSL2 Ubuntu 22.04）

```bash
# 1) 环境清单：确认只装了 rustup 组件，但系统里有 cc/gcc（发行版自带）
rustc -vV | sed -n 's/^host: //p'
rustup component list --installed

# 2) 默认链接器：成功（这是发布验收用的配置）
cargo build --release --workspace --offline

# 3) 造一个不含 C 工具链的 PATH（去掉 cc/gcc/ld/as/ar/cmake/pkg-config），改用 rust-lld
#    → 链接阶段失败：找不到 -lc/-lm/-lgcc_s/...（rust-lld 不继承 gcc 的库搜索路径）
RUSTFLAGS="-C linker=rust-lld" cargo build --release --workspace --offline

# 4) 再补上显式 -L → 能链接，但产出没有 PT_INTERP，运行即 SIGSEGV
readelf -l ./target/release/<bin> | grep interpreter   # 无输出
```

> 这三步的原始输出见本次会话记录；结论已写入 §3 R2 表格。
> 之所以仍然保留 Windows 侧的 rust-lld 记录：Windows 目标不走"gcc 驱动"，它的
> 链接器行为不同（`windows-sys` 走 `raw-dylib`/系统导入库路径），因此
> **不能把 Linux 的结论直接套到 Windows**，反之亦然。
