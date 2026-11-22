# 安全策略

## 支持范围

Y901 目前处于 **0.x 实验阶段**。安全修复只保证打在**最新发布版**上
（crates.io 上的最新 `y901` 及同版本子 crate）；更早的版本请先升级。

## 报告漏洞

**请不要开公开 issue。** 用 GitHub 的私密渠道：

- 私密报告：在 https://github.com/901548/y901 的 **Security → Report a vulnerability**
  提交（GitHub Private Vulnerability Reporting）。
- **兜底**：若该入口对你不可见（维护者可能尚未在仓库设置里开启），请开一个标题为
  `[SECURITY]`、正文不含漏洞细节的 issue，维护者会用私密渠道联系你。
  目前仓库未公开邮箱，因此 issue 是唯一兜底通道。

报告里请包含：

1. 受影响的 crate 与版本（`y901` / `y901-*` + 版本号）；
2. 最小复现（代码片段或仓库链接）；
3. 影响面（能读到什么、能执行什么、是否需要本地交互）；
4. 你希望公开的时间点（如有）。

## 已知的非安全问题（不属于漏洞）

以下都是**设计取舍**，已在文档里写明，不要当漏洞报：

| 现象 | 说明 |
|---|---|
| 渲染需要 GPU（Win32 D3D11 路径） | 没有 D3D11 设备时部分测试/路径不可用；这是平台能力问题，不是安全问题 |
| X11 通过 libc ABI 调系统接口 | 不编译 C 源码，但确实依赖系统 libc/ABI（见 `docs/ZERO_C_PROOF.md`） |
| 剪贴板/密钥模块按平台能力工作 | 平台不支持时返回明确错误，不静默降级 |
| `#![deny(missing_docs)]` 之外的 API 变动 | 0.x 语义允许 minor 版本里破坏性变更，见 README |

## 依赖与供应链

- 每次 CI 都跑 `cargo audit`（`rustsec/audit-check`），0 漏洞才算绿；
- 零 C/C++ 依赖守卫：`crates/y901-shell/tests/no_c_deps.rs`（黑名单 crate +
  `-sys` 白名单 + `build.rs` 源码扫描），CI 与本地门禁都会跑；
- 发布前用 `scripts/check-packaging.ps1` 校验 12 个 crate 的打包产物。
