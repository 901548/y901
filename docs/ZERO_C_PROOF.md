# 零 C/C++ 依赖 · 达标证明（Y901 单仓）

> 日期：2026-11-20（v0.2.5）　范围：`F:\Y901`
> 依据：[`ZERO_C_PLAN.md`](ZERO_C_PLAN.md)。历史版本包含另一业务仓的联合审计，
> 该仓已拆分为独立仓库，本证明只覆盖 Y901 自身。

## 1. 达标结论

| 验收级别 | 结论 | 证据 |
|---|---|---|
| A1 依赖级 | ✅ | 守卫覆盖所有 workspace crates 与 target 条件的依赖图；0 违规（§2）；`-sys` 为平台绑定 |
| A2 构建级 | ✅ Windows / ⚠️ Linux | **Windows 已实测**：本机无 Win10/11 SDK、`link.exe`/`cl.exe` 不在 PATH、`vswhere` 空，clean-env + rust-lld 构建 exit 0，链接命令行无 VS/SDK 路径（§3b）。**Linux 相反**：去掉 `cc` 后无法产出可运行二进制（§3c） |
| A3 运行级 | ⚠️ 默认需 VC 运行库 | 默认产物导入 `VCRUNTIME140.dll` + `api-ms-win-crt-*`；`+crt-static` 产物零 CRT 导入（§3b 第 4 步）。要"解压即用"就用静态 CRT |

## 2. 守卫测试与变异验证

位置：`crates/y901-shell/tests/no_c_deps.rs::no_c_or_cpp_toolchain_dependencies`。
三道独立检查：黑名单 crate、`-sys` 白名单、`build.rs` 源码扫描（`cc::Build` /
`cmake::Config` / `bindgen::`）。依赖树使用
`cargo tree --workspace --target all --prefix none --offline`，覆盖 workspace 所有成员与
目标专属依赖，而非仅默认成员/当前主机目标。

**实测输出**：

```
[守卫] Y901: <n> 个 crate，扫描 build.rs <m> 个，违规 0 条
```

**变异验证**（每条都必须先红后绿）：

| # | 变异 | 结果 |
|---|---|---|
| A | 在仓库内新增一个含 `cc::Build` 的 `build.rs` | **FAILED**，指出文件路径与命中模式；删除后绿 |
| B | 给某个 manifest 加一个名为 `openssl-sys` 的 path 依赖 | **FAILED**，报"黑名单"+"未白名单 -sys"；移除后绿 |

> 变异不写进仓库（否则守卫永远红）；复现方式见 §4 的手工步骤。
> 电子记录：`docs/evidence/correctness/` 下保留了当时的测试计数日志。

## 3. 依赖审计结果（本仓）

- 守卫按 workspace 全成员、全部 target 查询 78 个 crate；当前无未授权的 C build 依赖。
- `-sys` 绑定包含 Windows API bindings、WASM `js-sys` 以及平台条件 crate；它们不因此需要编译 C 源码。
- X11 代码使用 libc ABI 调用系统服务；GNU Linux 链接仍依赖系统运行时/链接环境。
- 关键判定：
  - `flate2` 用 `miniz_oxide`，未启用 zlib 特性 → 无 `libz-sys`；
  - `serde_yaml_ng` 取代 `serde_yaml` → 无 `unsafe-libyaml`；
  - `russh` 加密栈纯 Rust → 无 `ring` / `openssl-sys`；
  - `ttf-parser` 纯 Rust → 无 FreeType 绑定。

### 3b. "只装 Rust" 到底能不能构建（Linux 实测，2026-11-20）

这不是推测，是四组对照实测（WSL2 Ubuntu 22.04 / rustc 1.92.0 / x86_64-unknown-linux-gnu）：

| # | 配置 | 结果 |
|---|---|---|
| 1 | 默认（系统有 `cc`） | ✅ `cargo build --release --workspace` 成功 |
| 2 | PATH 去掉 `cc`/`gcc`/`ld`/`as`/`ar`/`cmake`/`pkg-config`，`-C linker=rust-lld` | ❌ `rust-lld: unable to find library -lgcc_s / -lutil / -lrt / -lpthread / -lm / -ldl / -lc` |
| 3 | 同 2，再补 `-L/usr/lib/x86_64-linux-gnu -L/usr/lib/gcc/x86_64-linux-gnu/11` | ⚠️ 链接"成功"，但 ELF 无 `PT_INTERP` → 运行 `SIGSEGV`（`run_exit=139`） |
| 4 | 最小单文件 `rustc main.rs` 重复 1–3 | 结论完全一致（1 成功 / 2 链接失败 / 3 `SIGSEGV`） |

**结论**：GNU/Linux 目标的链接**需要系统 C 工具链驱动**（它提供 `-dynamic-linker` 与库搜索路径）
加 libc/CRT 本体；`rust-lld` 不能替代它。因此"依赖图里没有 C/C++ 源码"（A1，✅）
**不等于**"只装 Rust 就能构建"（Linux A2，⚠️）。

### 3c. Windows 侧实测（`scripts/win-clean-probe.ps1`，2026-11-20）

| # | 配置 | 结果 |
|---|---|---|
| 1 | 机器清单 | **无** `Windows Kits\10`（没装 Win10/11 SDK）；`link.exe`/`cl.exe` 不在 PATH；`vswhere -latest -property installationPath` 为空；`VSINSTALLDIR`/`VCINSTALLDIR`/`WindowsSdkDir` 全空 |
| 2 | 清空 `LIB`/`INCLUDE`/`VC*`，PATH 只留 rustup+System32，`-C linker=rust-lld` | ✅ `cargo build --release --workspace --examples` **exit 0**（独立 target 目录，强制重链） |
| 3 | 用自写 wrapper 记录传给 rust-lld 的 argv | 无 `/libpath`、无 MSVC/Kits 路径、**没有任何导入库**（`windows-sys` 与 std 全走 `raw-dylib`） |
| 4 | 默认产物的 PE 导入表 | 17 个 DLL，CRT 相关：`VCRUNTIME140.dll` + `api-ms-win-crt-{heap,locale,math,runtime,stdio}-l1-1-0.dll` → **运行需 VC 运行库** |
| 5 | 同代码 `+crt-static` | ✅ 构建 exit 0；产物**零 CRT 导入** → 可"解压即用" |

**结论**：Windows 目标的构建**不需要 VS/SDK**（本机实测：没有 SDK 也能构建，链接器是工具链自带
`rust-lld`，链接输入里没有 SDK 库）；但**默认产物运行时要 VC 运行库**，要发布绿色版请加
`-C target-feature=+crt-static`。干净的正式证明仍需在一台连 VS 目录都没有的 VM 上跑同一脚本。

## 4. 复现命令

```powershell
# 守卫
cargo test -p y901-shell --test no_c_deps -- --nocapture

# Windows 侧完整探针（推荐：一条命令给出 §3c 的全部证据）
powershell -NoProfile -File scripts/win-clean-probe.ps1

# 等价单条（探针第 2 步）
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Windows\System32;C:\Windows"
$env:LIB = $null; $env:INCLUDE = $null; $env:VCINSTALLDIR = $null
$env:RUSTFLAGS = "-C linker=rust-lld"
cargo build --release --workspace --offline --target-dir target\clean-probe

# 手工变异（复现“先红后绿”，做完务必删除临时文件）
"fn main() { cc::Build::new(); }" | Out-File crates/y901-core/build.rs -Encoding utf8
cargo test -p y901-shell --test no_c_deps   # 必须 FAILED
Remove-Item crates/y901-core/build.rs
cargo test -p y901-shell --test no_c_deps   # 必须 ok
```

## 5. 一句话总结

> **依赖图里没有任何要求编译 C/C++ 源码的 crate（守卫锁死）。Windows 构建只需 Rust
> （本机无 SDK/link.exe 实测通过，`raw-dylib` 让链接不需要任何导入库），但默认产物运行时要
> VC 运行库——静态 CRT 可去掉这个依赖。GNU/Linux 相反：链接必须有系统 `cc` 与 libc/CRT，
> 全量验收另需 Xvfb。**
> 对外表述请用"**不编译 C/C++ 源码**"：Windows 可以写"构建不需要 VS/SDK"（已实测），
> 但不要写"零系统依赖"——运行要用系统 DLL，默认产物还额外要 VC 运行库；Linux 则必须装 `cc` 与 libc。
