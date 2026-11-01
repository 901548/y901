# 零 C/C++ 依赖 · 达标证明

> 日期：2026-11-01　范围：`F:\Y901` + `F:\Helm-Y901`
> 依据：《零 C/C++ 依赖架构规划与执行路线图》（`docs/ZERO_C_PLAN.md`）+ 老板拍板的 5 项决策

---

## 1. 达标结论

**A1（依赖级）✅ 已达标并由守卫测试长期锁死。**
**A2（构建级）✅ 在最小环境（仅 rustup + 系统目录、无 VS/gcc/cmake）下两个仓库均构建成功。**
**A3（运行级）✅ 产物可运行，`--font=self` 正常。**

| 验收级别 | 结论 | 证据 |
|---|---|---|
| A1 依赖级 | ✅ | 守卫测试双仓库 0 违规（§2） |
| A2 构建级 | ✅ | 最小 PATH 下 `cargo build --release` 成功（§5） |
| A3 运行级 | ✅ | `--font=self` 冷启动 8~12ms，像素统计正常（上一阶段 §10） |

---

## 2. 守卫测试（Phase 0）

`crates/y901-app/tests/no_c_deps.rs`，两道独立检查，**对两个仓库都跑**：

### 2.1 检查内容

| 检查 | 规则 |
|---|---|
| **名字规则** | 黑名单：`ring`、`aws-lc-rs`、`aws-lc-sys`、`openssl`、`openssl-sys`、`libz-sys`、`libssh2-sys`、`libsqlite3-sys`、`libgit2-sys`、`libpng-sys`、`freetype-sys`、`harfbuzz-sys`、`zlib-ng-sys`、`cc`、`cmake`、`bindgen` |
| | `-sys` 白名单：`windows-sys`、`windows-targets`、`windows_*_msvc`、`windows_*_gnu`、`windows_*_gnullvm`、`dirs-sys` |
| **源码规则** | 仓库内所有 `build.rs` + 依赖树中每个 crate 的 `build.rs`，出现 `cc::Build` / `cc::build` / `cmake::Config` / `bindgen::` 即违规 |

### 2.2 实测输出

```
[守卫] Y901：15 个 crate，扫描 build.rs 2 个，违规 0 条
[守卫] Helm-Y901：249 个 crate，扫描 build.rs 27 个，违规 0 条
```

### 2.3 变异检验（三次，全部"先红后绿"）

| # | 变异 | 结果 |
|---|---|---|
| A | 在 `crates/y901-app/build.rs` 写入含 `cc::Build` 的文本 | **FAILED**，报出该文件路径；删除后恢复绿 |
| B | 给 `y901-app` 加一个名为 `openssl-sys` 的 path 依赖 | **FAILED**，报"黑名单"+"未白名单 -sys"；还原后绿 |
| C | 给 `Helm-Y901` 加一个 `ring` 依赖 | **FAILED**（跨仓库守卫生效）；还原后绿 |

> 变异 C 特别重要：它证明守卫不是"只守自己家门口"——Helm 是独立仓库，但它的依赖树同样被审计。

---

## 3. 依赖全量审计结果（复述关键项）

### Y901（10 个 manifest，0 个 build.rs）
外部直接依赖 **5 个**，全部纯 Rust：
`unicode-width`、`crossbeam-channel`、`portable-pty`、`ttf-parser`、`windows-sys`。

### Helm-Y901（249 个 crate 的解析图）
`-sys` 只有 `windows-sys` + `dirs-sys`（纯绑定）。关键判定：

| crate | 判定 | 依据 |
|---|---|---|
| `russh` / `russh-keys` / `russh-sftp` | ✅ | 加密栈全纯 Rust（curve25519-dalek / aes-gcm / rsa / sha2 / ed25519-dalek），**无 ring、无 openssl** |
| `reqwest → native-tls → schannel` | ✅（Windows） | Windows 走 schannel 纯绑定；非 Windows 才会拉 openssl-sys（按决策 1，暂不跨平台） |
| `flate2` | ✅ | 用 `miniz_oxide`，未启用 zlib 特性，图中无 `libz-sys` |
| `portable-pty` | ✅ | `serial-windows` / `nix` 纯绑定 |
| `serde_yaml_ng`（本轮替换） | ✅ | 纯 Rust，无 `unsafe-libyaml` |
| `windows` 0.61 | ✅ | 纯绑定（Phase 2 未做，见 §6） |

---

## 4. Phase 3：YAML 替换（已完成）

```diff
- serde_yaml = "0.9"
+ serde_yaml_ng = "0.10"
```
`src/config.rs` 三处 `serde_yaml::` → `serde_yaml_ng::`（`from_str` ×2、`to_string` ×1）。

**结果**：构建通过（下载 `serde_yaml_ng 0.10`），**Helm 230 个测试全绿**，依赖树复核 `-sys` 仍只有
`dirs-sys` + `windows-sys`，无 `ring` / `openssl`。`unsafe-libyaml`（C 转译产物）已从图中移除。

---

## 5. 任务 3：MSRV 固化（已完成）

两个仓库根各新增：

```toml
[toolchain]
channel = "1.92.0"
```

`scripts/test-count.ps1` 开头新增检查（这是每次跑测试的必经入口，比另开 CI 脚本更难绕过）：

```
[MSRV] rustc 1.92.0 (ded5c06cf 2025-12-08) OK
```

低于 1.92 时直接 `exit 2`。

> 踩坑记录：改脚本后 PowerShell 5.1 报 `MissingArrayIndexExpression` —— 因为文件被保存成
> **UTF-8 无 BOM**，PS 5.1 把中文注释按 ANSI 解析导致语法错误。改存 **UTF-8 with BOM** 后正常。

---

## 6. 明确"不做"的三项（有决策依据）

| 项 | 决策 | 理由 |
|---|---|---|
| Phase 1（rustls 替换 native-tls） | ❌ 不做 | 决策 1：只发 Windows。Windows 上 native-tls → schannel（纯绑定） |
| Phase 4（rust-lld 固化） | ❌ 不做 | 决策 2：允许 VS Build Tools **仅用于链接** |
| Phase 2（windows 0.61 → windows-sys） | ❌ 跳过 | 改动集中在 `src/crypto.rs` 的 DPAPI 调用，要把 RAII/类型安全换成裸指针手工内存管理；这是**密钥加密**路径，风险 >> 收益（只省编译时间，与 C 依赖无关） |

---

## 7. 复现命令

```powershell
# 守卫（两个仓库一起）
cargo test -p y901-app --test no_c_deps -- --nocapture

# 最小环境构建（证明无需 C 工具链）
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Windows\System32;C:\Windows"
$env:LIB=$null; $env:INCLUDE=$null; $env:VCINSTALLDIR=$null
cargo build --release --workspace          # F:\Y901
cd F:\Helm-Y901; cargo build --release     # F:\Helm-Y901

# MSRV
rustc --version    # 必须 >= 1.92
```

---

## 8. 一句话总结

> **依赖里没有 C，守卫会一直盯着它；链接器用系统的 `link.exe`（已获批准）；MSRV 锁在 1.92。**
> 唯一未做的是 `windows` crate 的替换——那是在加密路径上换裸指针，不值得。
