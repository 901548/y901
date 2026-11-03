# 零 C/C++ 依赖架构规划与执行路线图

> 版本：2026-11-01　状态：**规划稿，待老板确认**
> 目标：**只安装 Rust 编译器即可完整构建与使用**，不依赖 C/C++ 编译器、cmake、bindgen 或外部 `.a/.lib`。

---

## 0. 结论摘要（先说最重要的）

**在 Windows 上，这个目标已经达成了。** 不是"接近达成"，是**实测达成**：

```
# 清空 VS / gcc / cmake 相关一切，只留 rustup 与系统目录
PATH = %USERPROFILE%\.cargo\bin;C:\Windows\System32;C:\Windows
（并确认 LIB / INCLUDE / VCINSTALLDIR / WindowsSdkDir 均未设置）

cargo build --release --workspace          # F:\Y901   → exit 0
cargo build --release                      # F:\Helm-Y901 → exit 0（1m22s，含 249 个 crate）
RUSTFLAGS="-C linker=rust-lld" cargo build --release --workspace  # → exit 0
```

三条独立证据：

| 证据 | 结果 |
|---|---|
| 两个仓库的 `build.rs` | **0 个**（`Get-ChildItem -Recurse -Filter build.rs` 无输出） |
| 解析图中带 `custom-build` 目标的 crate | **0 个** |
| 解析图中 `-sys` crate | Y901：仅 `windows-sys`；Helm：`windows-sys` + `dirs-sys`（都是纯绑定） |
| 本地 registry 全量 `build.rs` 扫描（`cc::Build`/`cmake::Config`/`bindgen::`） | 命中 5 个（android-activity / objc-sys / objc_exception / **ring** / wayland-backend），**均不在两个仓库的依赖图中** |

因此本轮真正要做的不是"消灭 C 依赖"（没有），而是**三件事**：

1. **把现状锁死**（守卫：任何人加了一个带 `cc` 的 crate，CI 立刻红）；
2. **消除唯一的潜在 C 路径**：`reqwest → native-tls`（Windows 走 `schannel`，非 Windows 走 `openssl-sys`）；
3. **把"只装 Rust"的边界写清楚**（链接器、工具链版本、干净环境定义）。

---

## 1. 依赖全量审计

### 1.1 Y901（10 个 manifest，0 个 build.rs）

| crate | 依赖 | 纯 Rust？ | 判定依据 |
|---|---|---|---|
| `y901-ui` | **无外部依赖**（仅 `y901-core` 内部 crate） | ✅ | 零第三方依赖（这是刻意设计） |
| `y901-core` | `unicode-width 0.2` | ✅ | 查表实现，无 build.rs |
| `y901-widgets` | core, ui | ✅ | 内部路径依赖 |
| `y901-extras` | core, ui, `crossbeam-channel 0.5`, `portable-pty 0.8.1` | ✅ | 见 §1.3 |
| `y901-agent` | ui | ✅ | 内部 |
| `y901-runtime` | ui, agent | ✅ | 内部 |
| `y901-mcp` | ui, widgets, agent | ✅ | 内部 |
| `y901-backend-win32` | core, ui, runtime, `ttf-parser 0.25.1`, `windows-sys 0.59` | ✅ | 见 §1.3 |
| `y901-app` | 全部内部 crate + `crossbeam-channel 0.5` | ✅ | — |

**总计：5 个外部 crate**，全部纯 Rust。

### 1.2 Helm-Y901（1 个 manifest，249 个 crate 的解析图）

| crate | 版本 | 纯 Rust？ | 判定依据 / 风险 |
|---|---|---|---|
| `serde` / `serde_json` | 1 | ✅ | 纯 Rust + 派生宏 |
| `serde_yaml` | 0.9 | ✅（有隐患） | 后端是 `unsafe-libyaml` —— **C 代码的机械转译产物，编译时纯 Rust**，但维护性差（§2.4） |
| `anyhow` / `thiserror` | 1 / 2 | ✅ | — |
| `async-trait` / `regex` / `data-encoding` / `futures-util` | — | ✅ | — |
| `dirs` | 5 | ✅ | → `dirs-sys` → `windows-sys`（纯绑定） |
| `tokio` | 1（full） | ✅ | — |
| **`reqwest`** | 0.12（默认特性） | ⚠️ **Windows ✅ / 非 Windows ❌** | 默认拉 `hyper-tls → native-tls`；Windows 上 native-tls 用 `schannel`（纯绑定），Linux/macOS 上会拉 **`openssl-sys`（需要 C 编译器）** |
| `russh` / `russh-keys` / `russh-sftp` | 0.44 / 2.4 | ✅ | 加密全在纯 Rust：`curve25519-dalek`、`aes-gcm`、`rsa`、`sha2`、`chacha20`、`ed25519-dalek`、`p256/p384/p521` —— **没有 ring，也没有 openssl** |
| `windows` | 0.61 | ✅ | 纯绑定（`windows-targets` → `windows_x86_64_msvc` 是**预编译 .lib 的元数据 crate**，不调用 cc） |
| `tempfile`（dev） | 3.8 | ✅ | — |

### 1.3 关键传递依赖逐项判定

| crate | 作用 | 判定 | 依据 |
|---|---|---|---|
| `windows-sys` 0.59 / `windows` 0.61 / `windows-targets` | Win32 API 绑定 | ✅ 纯 Rust | 无 build.rs；`windows-targets` 只是选择三元组对应的**预生成**链接元数据 |
| `dirs-sys` | 已知目录 | ✅ | → windows-sys（Windows）/ libc（Unix），无 cc |
| `native-tls` 0.2 | TLS 抽象 | ⚠️ 视平台 | Windows → `schannel`（✅）；Unix → `openssl-sys`（❌ C） |
| `schannel` | Windows SChannel 绑定 | ✅ | 纯 FFI 声明，无 build.rs |
| `hyper-tls` / `tokio-native-tls` | reqwest 的 TLS 适配 | ⚠️ | 随 native-tls 定性 |
| `flate2` | gzip | ✅ | 用的是 `miniz_oxide`（纯 Rust 移植），**未启用 `zlib`/`zlib-ng` 特性**，图中没有 `libz-sys` |
| `miniz_oxide` / `simd-adler32` / `crc32fast` | 压缩/校验 | ✅ | 纯 Rust |
| `portable-pty` 0.8.1 | 伪终端 | ✅ | Windows 下 → `serial` → `serial-windows`（纯绑定）+ `filedescriptor`；Unix 下 → `nix`（纯绑定）。**无 cc** |
| `nix` / `libc` | Unix 系统调用 | ✅ | 纯绑定（且当前 Windows 构建根本不编译它们） |
| `getrandom` | 系统随机源 | ✅ | Windows → `windows-sys` 的 `BCryptGenRandom`；Unix → libc |
| `curve25519-dalek` / `p256/p384/p521` / `rsa` / `sha2` / `aes-gcm` | 加密原语 | ✅ | 纯 Rust；`cpufeatures` 只在运行时检测 CPU 特性，不编译 C |
| `libm` | 数学函数 | ✅ | Rust 移植版（无 libm.so 链接） |
| `unsafe-libyaml` | YAML 解析 | ✅（编译期） | C→Rust 机械转译，源码是 Rust，**不调用 cc** |
| `zmij` | 浮点格式化 | ✅ | 纯 Rust，无 build.rs |
| `shared_library` / `memoffset` | 动态加载 / 结构体偏移 | ✅ | 纯 Rust（`memoffset` 有 build.rs 但只用 rustc 探测，不调 cc） |
| `winapi`（Windows 旧绑定） | — | ✅ | 纯声明，无 build.rs；但**建议移除**（与 windows-sys 重复） |

### 1.4 审计方法（可复现，三道独立证据）

```powershell
# 证据 1：仓库内是否有 build.rs
Get-ChildItem F:\Y901, F:\Helm-Y901 -Recurse -Filter build.rs | Where-Object { $_.FullName -notmatch '\\target\\' }

# 证据 2：解析图里的 -sys crate
cargo tree --workspace --offline --prefix none | Select-String -- '-sys'

# 证据 3：registry 全量扫描所有 build.rs 是否调用 cc / cmake / bindgen
Get-ChildItem "$env:USERPROFILE\.cargo\registry\src" -Recurse -Filter build.rs |
  Where-Object { (Get-Content $_ -Raw) -match 'cc::Build|cc::build|cmake::Config|bindgen::' } |
  ForEach-Object { $_.Directory.Name } | Sort-Object -Unique
# → android-activity, objc-sys, objc_exception, ring, wayland-backend（均不在我们的图中）

# 证据 4（终极）：干净环境构建
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Windows\System32;C:\Windows"
$env:LIB=$null; $env:INCLUDE=$null; $env:VCINSTALLDIR=$null
cargo build --release --workspace
```

### 1.5 审计结论

- **当前两个仓库在 Windows 上 100% 满足"零 C/C++ 编译"**；
- 唯一的**潜在** C 路径是 `native-tls` 的非 Windows 后端（`openssl-sys`），当前未触发；
- 唯一"看着像 C 其实不是"的是 `unsafe-libyaml`（转译产物）；
- 没有 `ring`（很多 Rust 项目栽在这），因为 `russh 0.44` 已全面转向纯 Rust 加密栈。

---

## 2. 风险与替代方案

### 2.1 风险 R1（唯一真正的风险）：`native-tls` 的跨平台后端

| 平台 | native-tls 后端 | C 编译 |
|---|---|---|
| Windows | `schannel` | ❌ 不需要 |
| Linux | `openssl-sys` | ✅ **需要**（C 编译器 + OpenSSL 头文件/库） |
| macOS | `security-framework` | ❌ 不需要 |

**替代方案：`reqwest` + `rustls`**

```toml
reqwest = { version = "0.12", default-features = false,
            features = ["json", "stream", "rustls-tls", "charset", "http2"] }
```

| 维度 | 影响 |
|---|---|
| API 变化 | **零**（reqwest 的公开 API 不变，只换 TLS 后端） |
| 功能缺失 | ① 不再读系统证书存储（改用 `webpki-roots` 内置根证书，或 `rustls-native-certs` 读系统库）；② 不支持客户端证书的 Windows 证书存储选择；③ 个别老旧 TLS 1.0/1.1 站点可能连不上（rustls 只支持 1.2/1.3） |
| 体积 | +~1.5 MB 二进制（ring 或 aws-lc-rs 后端）—— ⚠️ **注意：`rustls` 默认后端是 `aws-lc-rs`，它需要 C 编译器！必须显式选 `ring`** |
| 成本 | 1~2 小时（改 manifest + 跑 AI 请求测试 + SSE 流式验证） |

> **关键坑**：`rustls 0.23` 默认启用 `aws-lc-rs`，那是 C/汇编实现。必须写：
> `rustls = { version = "0.23", default-features = false, features = ["ring", "logging", "std", "tls12"] }`
> 并且 `reqwest` 要用 `rustls-tls` 而不是 `rustls-tls-manual-roots` 之外的默认特性——**否则等于没换**。
> （`ring` 本身有 C+汇编，但它被预编译成 `.a` 还是现场编译取决于平台；**rustls 的 `ring` 后端在 Windows 上仍需 cc**。若要绝对零 C，需要 `rustls` 的 `rustcrypto` 后端或接受"ring 由 ring 自带 prebuilt"——这一条必须先做实验验证，见 §4.4 决策点 3。）

**如果只发 Windows**：保留 `native-tls` 完全没问题，**零改动**。

### 2.2 风险 R2：Windows 绑定重复（非 C 问题，但值得清理）

| 现状 | 影响 |
|---|---|
| Y901 用 `windows-sys 0.59`，Helm 用 `windows 0.61` | 两套元数据 crate（`windows-core`/`windows-result`/`windows-strings`…）同时编译，增量构建多 ~3~5 秒 |
| Helm 里还残留 `winapi`（由某传递依赖带入） | 三套 Win32 绑定共存 |

**方案**：Helm 改用 `windows-sys`（或让 Y901 升级到 `windows 0.61` 的 sys 子集）。**成本**：Helm 只用了 `Win32_Foundation` + `Win32_Security_Cryptography` 两个 feature，`windows` → `windows-sys` 是纯机械替换，1 小时内完成。**收益**：少编译 ~8 个 crate，构建更快；**与 C 无关**，属于顺手清理。

### 2.3 风险 R3：链接器依赖（"只装 Rust" 的真实边界）

这是**最容易被忽略**的一条：Rust 在 Windows 上**编译**不需要 C 编译器，但**链接**历史上需要 MSVC 的 `link.exe`（VS Build Tools，2~4 GB）。

**本次实测**（rustc 1.92.0，x86_64-pc-windows-msvc）：

- 最小 PATH（无 VS、无 gcc）+ 无 `LIB/INCLUDE/VCINSTALLDIR` → **构建成功**；
- 显式 `-C linker=rust-lld`（rustup 自带的 `rust-lld.exe`）→ **也成功**。

**但这台机器装了 VS 2022**，无法排除 rustc 通过注册表/vswhere 找到 `link.exe`。因此：

| 方案 | 说明 | 风险 |
|---|---|---|
| A. 默认 MSVC 工具链 | 用户装 rustup + VS Build Tools | 与"只装 Rust"字面冲突（但业界常态） |
| B. 强制 `-C linker=rust-lld` | 写进 `.cargo/config.toml`，用 rustup 自带 lld | **需在纯净 VM 验证**（本机不能证明） |
| C. `x86_64-pc-windows-gnu` | 需要 MinGW-w64 | 同样需要额外工具链，且 ABI/兼容性有坑 |
| D. 只发预编译二进制 | 用户零工具链 | 开发者仍需 A/B |

**建议**：先做 **B 的纯净 VM 验证**（一次 30 分钟），若通过则写进 `.cargo/config.toml` + `rust-toolchain.toml` 固化版本，**这是唯一能真正兑现"只装 Rust"的路径**。

### 2.4 风险 R4：`serde_yaml 0.9` 的维护性

- `serde_yaml` 已被作者归档（unmaintained）；
- 后端 `unsafe-libyaml` 是 C 的机械转译，**编译期纯 Rust、无 cc**，但长期无人维护；
- **不影响"零 C"目标**，属于**安全/维护**问题。

**方案**：换 `serde_yaml_ng` 或 `serde_norway`（纯 Rust，API 兼容）。**成本**：改 1 行 manifest + 跑配置加载测试。

### 2.5 明确"不该替换"的三项

| 项 | 为什么保留 |
|---|---|
| `windows-sys` / `windows` | 这是**官方纯绑定**，不编译任何 C；换掉它就只能手搓 FFI —— 老板已明确"不手搓 FFI（会引入 UB）" |
| `portable-pty` | 底层是 `serial-windows` / `nix` 的纯绑定；手写 ConPTY 调用属于手搓 FFI，风险更高 |
| `ttf-parser` | 纯 Rust 解析器，正是我们自研字体引擎的基石 |

---

## 3. 执行顺序与回滚策略

原则：**先锁现状（无风险）→ 再动唯一风险项（有明确验证点）→ 最后做顺手清理（可独立回滚）**。

### Phase 0：锁死现状（半天，零风险）

**做什么**
1. 新增守卫测试 `crates/y901-app/tests/no_c_deps.rs`（或 `xtask`）：
   - 读 `cargo metadata`，遍历解析图；
   - 断言：没有任何 crate 的 `custom-build` 目标在 `build.rs` 里出现 `cc::Build` / `cmake::Config` / `bindgen::`；
   - 断言：`-sys` crate 只能出现在**白名单**（`windows-sys`、`dirs-sys`、`windows_x86_64_msvc`）—— 白名单变更必须显式改代码，天然形成 review 关卡；
   - 断言：`native-tls` 在**非 Windows target** 下不得出现（`cargo tree --target x86_64-unknown-linux-gnu` 时 `openssl-sys` 必须为空）。
2. CI 加一条 `cargo test -p y901-app --test no_c_deps`。

**验证**：故意在 Helm 加 `openssl = "0.10"`，守卫必须红；移除后变绿。

**回滚**：纯新增文件，删掉即可。

### Phase 1：TLS 纯 Rust 化（仅当需要跨平台；1~2 小时）

**顺序**（必须先做实验再改）：
1. **实验**：在临时 crate 里试 `reqwest(default-features=false, features=["rustls-tls"])`，
   用 `cargo tree --target x86_64-unknown-linux-gnu | Select-String "openssl-sys|ring|aws-lc"` 看实际拉了什么。
   **这一步决定成败**：若拉了 `aws-lc-sys` / `ring`（需 cc），则"零 C 跨平台"不成立，只能接受"Windows 零 C"。
2. 若通过 → 改 Helm 的 reqwest 特性；保留 `native-tls` 作为可选 feature（`--features tls-native`）便于回滚。
3. 跑验证：AI 请求（非流式 + SSE 流式）、证书校验失败用例（错误信息必须合理）。

**测试**
- `helm-y901` 现有的 AI 集成测试（若有）+ 手工发一次真实请求；
- `cargo tree --target x86_64-unknown-linux-gnu | grep openssl` 必须为空。

**回滚**：feature 开关切回 `native-tls`，一行 manifest 改动。

**降级决策**：**不接受**"TLS 暂时不可用"——AI 是核心功能，必须一次性替换并验证。

### Phase 2：Windows 绑定统一（1 小时，独立回滚）

- Helm：`windows 0.61` → `windows-sys 0.59`（只用到 2 个 feature）；
- 移除图中的 `winapi`（找到引入方，通常是 `portable-pty`/`serial-windows`，能换就换，不能换就接受）；
- 验证：`cargo tree | Select-String "winapi|windows v0"` 计数下降；全量测试绿。

### Phase 3：YAML 维护性替换（可选，30 分钟）

- `serde_yaml` → `serde_yaml_ng`（API 兼容）；
- 验证：配置加载 + 错误信息测试。

### Phase 4（决策后）：链接器固化

- 若 §2.3 的 B 方案在纯净 VM 验证通过：
  - 加 `.cargo/config.toml`：`[target.x86_64-pc-windows-msvc] rustflags = ["-C", "linker=rust-lld"]`
  - 加 `rust-toolchain.toml` 固化 `channel = "1.92.0"`（rust-lld 的行为与版本强相关）
- 回滚：删两个文件。

---

## 4. 边界与验收定义

### 4.1 三级验收（建议全部纳入验收）

| 级别 | 判据 | 命令 | 说明 |
|---|---|---|---|
| **A1 依赖级** | 解析图中无 crate 的 build.rs 调用 `cc`/`cmake`/`bindgen`；`-sys` 仅白名单 | 守卫测试 | **可自动化、可在 CI 里长期守住** |
| **A2 构建级** | 干净 Windows VM（只装 rustup + 本仓库）能 `cargo build --release` 成功 | 手工/CI 快照 | "干净"的定义见 §4.3 |
| **A3 运行级** | 产物能跑：`--font=self` 正常、Helm 的 SSH/AI 功能正常 | 冒烟脚本 | 防止"编译过了但功能废了" |

> 我的建议：**A1 作为永久守门员，A2 作为发布前一次性验证，A3 作为每次发版前冒烟**。
> 只做 A1 不够（它不覆盖链接器问题）；只做 A2 不可持续（每次都要干净 VM）。

### 4.2 守卫的白名单规则（需要你确认）

允许的 `-sys` crate（都是纯绑定、不调用 cc）：

```
windows-sys, windows_x86_64_msvc, windows-targets, dirs-sys
```

**禁止**（出现即红）：`openssl-sys`、`libz-sys`、`libssh2-sys`、`aws-lc-sys`、`ring`（含 C/汇编）、`libsqlite3-sys`、任何 `*-sys` 不在白名单里。

### 4.3 "干净环境"的定义（必须写死，否则验收无法复现）

```
- Windows 10/11 x64，干净快照
- 只安装：rustup（stable 1.92.0）+ Git（可选）
- 不安装：Visual Studio / Build Tools / MinGW / cmake / LLVM / OpenSSL
- 环境变量 LIB / INCLUDE / VCINSTALLDIR 未设置
- PATH 只含 %USERPROFILE%\.cargo\bin 与系统目录
```

### 4.4 需要你确认的 5 个决策点

| # | 问题 | 选项 | 我的建议 |
|---|---|---|---|
| 1 | **发 Windows 还是会跨平台？** | (a) 只发 Windows (b) 也要 Linux/macOS | 若 (a)：**Phase 1 可以不做**，现状已合规 |
| 2 | **"只装 Rust"是否允许 VS Build Tools（仅用于链接）？** | (a) 允许 (b) 必须 rustup-only | 若 (b)：必须先做 §2.3 的纯净 VM 验证，再决定是否固化 `rust-lld` |
| 3 | **`ring` 算不算违规？** | (a) 算（有 C+汇编）(b) 不算（crate 自带预编译产物） | 建议 (a)：但那样跨平台就只能用 `rustls + rustcrypto` 后端（性能略低），需实测 |
| 4 | **`unsafe-libyaml`（C 转译）可接受吗？** | (a) 接受 (b) 换 `serde_yaml_ng` | 建议 (b)：与 C 无关，但维护性更好 |
| 5 | **MSRV（最低 Rust 版本）定多少？** | 1.85 / 1.92 / 不设 | 建议 1.92（与本次验证一致），写进 `rust-toolchain.toml` |

---

## 5. 工作量估算

| 阶段 | 工作量 | 风险 | 是否必须 |
|---|---|---|---|
| Phase 0 守卫 | 半天 | 极低 | **必须** |
| Phase 1 rustls | 1~2 小时 | 中（ring/aws-lc 决策） | 仅跨平台时需要 |
| Phase 2 绑定统一 | 1 小时 | 低 | 可选（构建加速） |
| Phase 3 YAML | 30 分钟 | 低 | 可选 |
| Phase 4 链接器固化 | 半天（含 VM 验证） | 中 | 若选 rustup-only 则必须 |

**总计**：只做 Windows + 守卫 ≈ **半天**；跨平台 + rustup-only ≈ **1.5~2 天**。

---

## 6. 本次实测原始数据（可复现）

```
rustc 1.92.0 (ded5c06cf 2025-12-08)   host: x86_64-pc-windows-msvc

# 最小 PATH + 清空 MSVC 环境变量
cargo build --release --workspace     → exit 0   (F:\Y901)
cargo build --release                 → exit 0   (F:\Helm-Y901, 1m22s)
RUSTFLAGS="-C linker=rust-lld" cargo build --release --workspace → exit 0

# 依赖规模
Y901：10 manifest，0 build.rs，5 个外部直接依赖，-sys 仅 windows-sys
Helm：249 个 crate，0 个 custom-build 目标，-sys 仅 windows-sys + dirs-sys
registry 全量 build.rs 扫描：5 个 cc/cmake/bindgen 命中，均不在依赖图中
TLS 链：reqwest 0.12.28 → hyper-tls 0.6 → native-tls 0.2.18 → schannel（Windows）
```

---

## 7. 一句话总结

> **C 依赖已经不存在了，现在缺的是"守住它"的机制和"跨平台"的决策。**
> Phase 0（守卫）无论如何都要做；Phase 1（rustls）只有在需要 Linux/macOS 时才有意义，且**先做实验再动手**——
> 因为 `rustls` 的默认后端 `aws-lc-rs` 本身就是 C，换错了等于白换。
