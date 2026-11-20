# 零 C/C++ 依赖 · 达标证明（Y901 单仓）

> 日期：2026-11-20（v0.2.3）　范围：`F:\Y901`
> 依据：[`ZERO_C_PLAN.md`](ZERO_C_PLAN.md)。历史版本包含另一业务仓的联合审计，
> 该仓已拆分为独立仓库，本证明只覆盖 Y901 自身。

## 1. 达标结论

| 验收级别 | 结论 | 证据 |
|---|---|---|
| A1 依赖级 | ✅ | 守卫测试 0 违规（§2）；`-sys` 仅 `windows-sys` |
| A2 构建级 | ✅ | 最小 PATH（无 VS/gcc/cmake）+ 清空 MSVC 变量下 `cargo build --release --workspace` 成功（§4） |
| A3 运行级 | ✅ | 示例/截图路径可运行；`--font=self` 冷启动 8~12ms，像素统计正常 |

## 2. 守卫测试与变异验证

位置：`crates/y901-shell/tests/no_c_deps.rs::no_c_or_cpp_toolchain_dependencies`。
三道独立检查：黑名单 crate、`-sys` 白名单、`build.rs` 源码扫描（`cc::Build` /
`cmake::Config` / `bindgen::`）。

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

- 外部直接依赖全部纯 Rust：`unicode-width`、`crossbeam-channel`、`portable-pty`、
  `ttf-parser`、`windows-sys`、`rustbus`、`nix`、`wasm-bindgen`。
- 依赖树 `-sys`：仅 `windows-sys`（纯绑定）。
- 关键判定：
  - `flate2` 用 `miniz_oxide`，未启用 zlib 特性 → 无 `libz-sys`；
  - `serde_yaml_ng` 取代 `serde_yaml` → 无 `unsafe-libyaml`；
  - `russh` 加密栈纯 Rust → 无 `ring` / `openssl-sys`；
  - `ttf-parser` 纯 Rust → 无 FreeType 绑定。

## 4. 复现命令

```powershell
# 守卫
cargo test -p y901-shell --test no_c_deps -- --nocapture

# 干净环境构建（证明无需 C 工具链）
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Windows\System32;C:\Windows"
$env:LIB = $null; $env:INCLUDE = $null; $env:VCINSTALLDIR = $null
cargo build --release --workspace

# 手工变异（复现"先红后绿"，做完务必删除临时文件）
"fn main() { cc::Build::new(); }" | Out-File crates/y901-core/build.rs -Encoding utf8
cargo test -p y901-shell --test no_c_deps   # 必须 FAILED
Remove-Item crates/y901-core/build.rs
cargo test -p y901-shell --test no_c_deps   # 必须 ok
```

## 5. 一句话总结

> **依赖里没有 C，守卫会一直盯着它；链接器用系统的 `link.exe`，
> 这是"只装 Rust"唯一需要说清楚的边界。**
