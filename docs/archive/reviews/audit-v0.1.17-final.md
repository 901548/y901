# Y901 v0.1.17 最终审查报告（历史档案）

> **历史档案免责声明**：此报告记录的是 v0.1.17 时期的审查与旧依赖/代码状态，仅供历史追溯；其结论、建议、数字及路径可能已经过时，不能作为当前安全、发布或维护决策依据。当前信息请以仓库现行文档、代码和自动化检查为准。


审查基准：`F:\Y901` HEAD `b6173f7`（2026-11-05 16:50:15 +0800）。  
本轮只复核 P1-01 ACCEPTANCE:13 与 P1-03 日志时间；P1-02 Grid、P1-04 record_skip 沿用上一轮已验证结果。

## 结论

Y901 v0.1.17 可发布，框架进入稳定维护状态。P0=0、P1=0、P2=0（sync 不覆盖派生表述，不阻塞）、P3=1（误报）。

## P1-01：已关闭

`docs/ACCEPTANCE.md` 当前内容：

- :12 默认：`**825 passed / 0 failed**`
- :13 include-ignored：`**877 passed / 0 failed**，825 默认 + 52 标记 #[ignore]`
- :15 总数：`**877** = 825 默认 + 52 ignored`

`825` 和 `52` 已不再加粗，`877` 是 include-ignored 的真实总数。与 `docs/evidence/numbers.txt` 的 `passed=825 / ignored=52 / include_ignored=877 / baseline=825` 一致。P1-01 关闭。

## P1-03：已关闭

- HEAD：2026-11-05 16:50:15
- `target/test-workspace-HEAD.log`：2026-11-05 16:53:49，825 passed / 0 failed / 52 ignored
- `target/test-workspace-HEAD-all.log`：2026-11-05 16:54:40，877 passed / 0 failed

两个日志的 LastWriteTime 均晚于 HEAD；计数为 825 / 877，与文档一致。P1-03 关闭。

## P1-02 / P1-04：上轮已验证通过

- Grid：`crates/y901-core/src/lib.rs:297-298、509、725` 使用 `.max(1)`；`zero_sized_grid_does_not_panic`（:1151）覆盖 new(0,0)、reflow(0)、set_rows(0)，日志中该测试 ok。
- record_skip：`crates/y901-backend-win32/tests/d3d_text_scaling.rs:176-179` 的 open 失败会 panic，`writeln!` 使用 `.expect("SKIP log write failed -- gate will not see this skip")`。

## P2-01：sync 不覆盖派生表述，P2

`docs/ACCEPTANCE.md:13/15` 仍有 `825 默认` 和 `52 ignored` 这类派生表述。`scripts/sync-doc-numbers.ps1` 的 `-Check` 规则只覆盖：
- 包含 `include-ignored` 的 `**N**`
- 包含 `含 ignore` 的 `**N**`
- `基线 **N**`
- `**N** passed`

它没有“默认 **N**”或“N 默认”的规则，所以派生表述不会被自动改写或校验。这是质量/覆盖面问题，不是当前发布数字错误，故为 P2。

建议下一轮：优先删掉 `825 默认` 这类派生表述，只保留三个明确字段（passed / ignored / include-ignored）；如果保留，则给 `-Check` 增加“默认 **N**”规则。

## P3-01：误报

`docs/PERFORMANCE.md:17` 的 `846 µs` 是延迟微秒，不是测试数量。按 `\b846\b` 搜索会产生命中，但不属于 P1-01。

## 门禁

用户报告使用 Windows PowerShell 5.1 运行 gate 并得到 `GATE OK`。当前环境没有 `pwsh` 可执行文件，无法独立复跑该命令；本轮结论基于 HEAD、ACCEPTANCE 和两份 HEAD 日志的只读取证。

## 最终判定

Y901 v0.1.17 可发布，框架进入稳定维护状态。P0=0、P1=0、P2=1（sync 不覆盖派生表述，不阻塞）、P3=1（误报）。审计报告已归档至 `docs/archive/reviews/audit-v0.1.17-final.md`。
