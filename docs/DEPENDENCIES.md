# 依赖决策记录

> 只记录**需要解释**的依赖。普通依赖见各 `Cargo.toml`。
> 审计方式：`scripts/gate.ps1` 的第 6 步（`cargo audit`）。

## `ttf-parser 0.25.1` —— unmaintained

**状态**：`RUSTSEC-2026-0192`，作者不再维护。**不是漏洞**（`cargo audit` 退出码 0）。

**为什么钉死这个版本**：

- `y901-render` 的自研字形引擎用它解析 TTF 轮廓（`crates/y901-render/src/font_engine/`）；
  这是"零 C 依赖"承诺的关键一环 —— 换成 FreeType 就破了承诺。
- **攻击面是"本机字体文件"**（`C:\Windows\Fonts\` 下的系统字体），不是"远程恶意输入"。
- 当前**没有**接入远程字体预览或 SFTP 字体上传；字体路径来自 `Options.font` 或平台默认。

**重新评估的条件**（任一条成立就该重新评估，而不是继续"接受风险"）：

1. 接入**远程字体预览**（从网络拉字体字节）；
2. 接入**SFTP 字体上传**（用户上传任意字体文件）；
3. `ttf-parser` 出现**实际 CVE**（不只是 unmaintained）；
4. 需要解析不可信来源的字体（例如插件市场分发的字体包）。

**退路（B 方案，按成本递增）**：

1. `cargo vendor` 把 `ttf-parser 0.25.1` 源码收进仓库，锁定版本并**自己维护**；
2. 在 vendored 源码上打补丁：轮廓点数上限、嵌套深度限制、单字形解析超时
   （恶意字体最常利用的是"点数爆炸"导致的 CPU/内存耗尽，而不是内存越界）；
3. 迁移到另一个纯 Rust 解析器（工作量约 1–2 天，需要重跑全部字形像素测试）。

**当前结论：选 A（接受风险）**。理由：攻击面是本机系统字体；且 B 有明确退路，随时可切换。

## `serial 0.4.0` —— unmaintained（lock 残留）

**状态**：`RUSTSEC-2017-0008`，2017 年起无人维护。

**为什么出现在审计里**：`cargo audit` 扫描整个 `Cargo.lock`，而 `serial` 是**历史解析残留**
（可能是早期接串口相关功能时引入的传递依赖），但 `cargo tree -i serial` 报告
`package ID specification 'serial' did not match any packages` —— **它不属于任何可达的 crate**。

**处理（实测，未成功清除）**：`cargo update` 之后 `Cargo.lock` 里**仍然有** `serial` 条目
（`grep 'name = "serial"'` = 1 处），`cargo tree -i serial` 依旧报告"不在依赖图里"。

也就是说：它是一条**不可达但被 lock 保留**的条目，`cargo update` 并不会清理它
（lock 会保留已解析过的包，除非它们不再被任何 manifest 需要且触发了重解析）。

**要真正清掉，需要**（未执行）：

1. 允许网络，用 `cargo update --precise` 重新解析整图；或
2. 找到引入它的 manifest（可能在 `Helm` 侧而非本仓库），移除该依赖后再 update；或
3. 手动删除 lock 条目并 `cargo build` 让 cargo 重新生成（需网络校验）。

> **当前决策**：接受 lock 里的残留（不可达，不参与编译）。有网络环境时可用 `cargo update --workspace`
> 或手工删条目后重建锁文件。**不阻塞发布。**

**当前结论**：`serial` 只是审计输出里的噪声，**不属于任何会被编译的 crate**；
本轮**不强行清除**，因为 `cargo update` 会顺带升级 3 个依赖且需要联网下载，与"离线可复现"冲突。

## 审计基线

```
cargo audit
  → 0 vulnerabilities
  → 2 allowed warnings（本节记录的两条，均已决策）
  → exit code 0
```

`scripts/gate.ps1` 在 `cargo-audit` **未安装**时打印 `skipped`，而不是当作通过 ——
"没审计"不等于"审计通过"。

## `miniz_oxide` 双版本（`0.8.9` + `0.9.1`）

`cargo tree --workspace --duplicates` 显示两个版本同时存在：

```
miniz_oxide v0.8.9 <- png v0.17.16 <- y901-shell
miniz_oxide v0.9.1 <- flate2 v1.1.10 <- png v0.17.16 (*)
```

**原因**：`png 0.17.16` 对 `miniz_oxide` 与 `flate2` 各有约束，两条路径选中了不同大版本。
**不属于我们能单独统一的依赖**：要统一需等 `png`/`flate2` 发布新版。
**当前决策**：接受（只增二进制体积与编译时间，无行为差异）。
