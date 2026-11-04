# v0.1.15 全量审查报告

> 审查基准：`F:\Y901` HEAD `a1fd4cd`（v0.1.15 + 审计收口），实测 **824 passed**。
> 本报告**基于当前代码重新审查**，不引用旧报告的 23 项（旧报告原文未落盘，其中 11 项无法核对）。
> 每项格式：`[严重度-编号] 文件:行号 — 问题` / 证据 / 修复建议 / 需运行时验证。

---

## P0（阻断）

**无。**

---

## P1（必须修）

### [P1-01] `README.md:127`、`docs/STATUS.md:5,117,202` — 文档声明的测试数与实测不符

**证据**

```
README.md:127    | 测试 | **818**（cargo test --workspace），0 失败 |
STATUS.md:5      - **测试**: 818（基线 818；#[ignore] 45，含 ignore 863）
STATUS.md:202    - **818 测试全绿**（基线 818；#[ignore] **52**；--include-ignored **870**）
实测              actual passed=824
docs/evidence/test-baseline.txt  824
```

**问题**：三处声明停留在 818，实测已是 824（`#6` 指纹 +5、图片质量 +2、零分配回归 +1…）。
更严重的是 `STATUS.md` **内部自相矛盾**：同一份文件里 `#[ignore] 45` 与 `#[ignore] 52` 并存。

**修复建议**：以 `docs/evidence/test-baseline.txt` 为准（当前 824），
统一 README 与 STATUS；ignore 数由 `cargo test` 汇总得出（当前 52），删除 45 的说法。

**需运行时验证**：否（数字已实测）。

### [P1-02] `docs/STATUS.md` — “审查报告 23 项全部关闭”与“12 项可追踪”并存的历史残留

**证据**：`STATUS.md:113` 已改为更正说明（12 项 + 11 项缺失），
但同文件 `:200` 一带仍有“23/23 关闭”的旧表述（本轮已一并改为“12 项可追踪；11 项原始报告未落盘”）。

**状态**：本轮已修（`a1fd4cd`）。保留在此，因为**这类“文档自相矛盾”正是旧报告同类问题的复发**。

**修复建议**：任何“总数”类声明必须能被文件内的明细清单加总验证；否则不写总数。

**需运行时验证**：否。

### [P1-03] `crates/y901-backend-x11/tests/real_window.rs:485,535,547` — 环境变量未受 EnvGuard 保护

**证据**

```
L485  std::env::set_var("Y901_NO_SHM", "1");
L535  std::env::remove_var("Y901_NO_SHM");
L547  std::env::set_var("Y901_NO_SHM", "1");
（L495/496 已有“恢复原值”逻辑，说明部分位置改过，仍有三处裸调用）
```

**问题**：`set_var`/`remove_var` 是进程全局状态；中途 panic 会把污染留给后续用例。
`font_3c_pixels.rs` 与 `y901-widgets` 侧已用 `EnvGuard`（拿锁 + 恢复原值），这里没有。

**修复建议**：复用 `EnvGuard` 模式，且**必须整段 `#[cfg(target_os = "linux")]`** ——
上一轮尝试在 Windows 宿主插入非门控辅助结构，直接让 workspace 编译失败（`the item is gated here`）。

**需运行时验证**：**是** —— 需 WSL/Linux 环境编译并运行 `cargo test -p y901-backend-x11`。

---

## P2（建议修）

### [P2-01] `Cargo.lock:225` — `serial` 残留条目

**证据**：`cargo tree -i serial` → `package ID specification 'serial' did not match any packages`；
`Cargo.lock:225` 仍有 `name = "serial"`。

**问题**：不可达条目，不参与编译；但会让 `cargo audit` 多报一条 unmaintained。
**修复建议**：联网后 `cargo update`（注意：上一轮实测它会顺带升级 `crc32fast`/`log`/`unicode-width`）。
**需运行时验证**：是（需联网）。

### [P2-02] `miniz_oxide` 双版本（`0.8.9` + `0.9.1`）

**证据**

```
miniz_oxide v0.8.9 <- png v0.17.16 <- y901-shell
miniz_oxide v0.9.1 <- flate2 v1.1.10 <- png v0.17.16 (*)
```

**问题**：上游 `png`/`flate2` 的版本约束决定，非本项目可控；代价是体积与编译时间。
**修复建议**：接受，已记入 `docs/DEPENDENCIES.md`；等上游发布再统一。
**需运行时验证**：否。

### [P2-03] `docs/STATUS.md` 的 `#[ignore]` 计数与实测口径不同步

**证据**：`STATUS.md` 同时出现 `#[ignore] 45`、`#[ignore] 52`、`--include-ignored 870`；
实测默认 `passed=824`、`ignored=52`。
**修复建议**：三个数字（默认 / ignored / include-ignored）一次性重测后统一写入。
**需运行时验证**：否（已实测）。

### [P2-04] `docs/FRAMEWORK_GUIDE.md §0.1` — 九条教训的编号连续性

**证据**：教训以“观测工具会撒谎 / 位置按控制流 / 锚点来自搜索 / 分组自证 / 守恒先行 /
字段归属 / cfg(test) 边界 / 恢复≠重建 / 离线显式承认”九条形式并列，
但**没有编号**，后续引用（如本报告）只能按标题引用。
**修复建议**：加编号（L1…L9），便于 commit message 与报告引用。
**需运行时验证**：否。

---

## P3（可接受 / 记录价值）

### [P3-01] `gate.ps1` 对“客户区尺寸变化”判 FAIL（exit 2）

**证据**：`scripts/hash-gate.ps1:71 exit 2`；`gate.ps1:113` 把 exit 2 也当失败。
**说明**：`Options.width/height` 在 v0.1.8 起已是**客户区**尺寸（`AdjustWindowRectExForDpi` 反算），
本会话内尺寸已稳定为 400×220；但仍可能随 DPI/主题变化。
**判断**：设计取舍（宁可红也不静默重建基线）。可接受。

### [P3-02] `Image` 没有 `from_rgba` / `from_bytes` 构造器

**证据**：`crates/y901-ui/src/image.rs` 只有 `ImageStyle`、`ImageFit`、`TextureId`；
`grep "fn from_rgba"` 无命中。
**说明**：旧审查项“`Image::from_rgba` 溢出”**指称的 API 在当前代码中不存在** ——
像素上传走 `RenderCtx::upload_texture(w, h, rgba)`，其中已有长度校验
（`rgba.len() < (w*h*4)` 时返回 INVALID）。
**判断**：旧项已失效；无需修。

### [P3-03] `TextBuffer` 无显式长度上限

**证据**：`grep "MAX_LEN|max_len"` 无命中。
**说明**：文本输入长度由调用方/控件自行管理，框架层不设硬上限。
**判断**：设计取舍；若将来接 RPC/剪贴板大文本，需在应用层设限。记录备查。

### [P3-04] `#[cfg(test)]` 字段在生产构建中的可见性

**证据**：`backend/mod.rs:76,135,284` 全部带 `#[cfg(test)]`；`window.rs:802` 同。
`cargo build --release --workspace` 通过（`#9` 已验）。
**判断**：✅ 无问题。

---

## 逐维度结论

| 维度 | 结论 |
|---|---|
| 1. 代码正确性 | ✅ 生产代码（`cfg(test)` 之前）**无 `unwrap`/`panic!`**：全仓 6 处命中里 5 处是注释或自定义 `Parser::expect`，1 处是说明性注释。四个边界项：Grid 零尺寸 **已修**（`debug_assert` + `max(1)`，`lib.rs:297/511/728`）；Slider NaN **已修**（`slider.rs:18` 显式 `is_nan`）；`Image::from_rgba` **不存在**（旧项失效）；`TextBuffer` 上限 **未设**（设计取舍）。 |
| 2. 门禁可靠性 | ✅ 六步齐全，各自有 `Fail`/`exit`：基线缺失、不编译、计数下降、skip、clippy 退出码、clippy 警告数、hash、docs、audit（未安装 / 需联网 / 失败）。`NOT RELEASABLE` exit 2 与 `DEV: STALE AUDIT` exit 0 分离正确。指纹脚本有 **超时**（`WaitForExit`）与 **守恒检查**（`sum != total` → PARSE ERROR）。hash-gate 基线缺失 **exit 3**（不自动创建）。 |
| 3. 测试有效性 | ✅ 抽查的 5 组均有可失败断言与变异/反向对照：`d3d` 运行时断言（1→0 反向验证 FAIL）、`image_mip_quality`（闭式解 + 变异对照）、`perf_texture_alloc` 快速回归（`assert_eq!(a,0)`）、`text_scale_pixels`（墨量比 + 变异）、`flatten` 上限（超限后继续消费不 panic）。`#[ignore]` 均有理由（基准 / 需 GPU）。`present_sync_tests` 为**单元测试**，因为 `#[cfg(test)]` API 对 integration test 不可见。 |
| 4. 文档一致性 | ❌ **发现 3 处不一致**：测试数 818 vs 824（P1-01）、`#[ignore]` 45 vs 52（P2-03）、`23 项关闭` 残留（P1-02，本轮已修）。其余抽查（能力清单、DEPENDENCIES 三条、PERFORMANCE §5 DWM 结论）与代码一致。 |
| 5. 架构一致性 | ✅ `y901-ui` 生产依赖仅 `y901-core`（间接引入 `unicode-width`）；`y901-render` **无 `windows-sys`**；重复依赖只有 `miniz_oxide` 双版本（上游）。 |
| 6. 遗留与已知限制 | ✅ 逐项判断：`#11` x11 EnvGuard = **真缺陷**（P1-03，环境受限）；`serial` 残留 = 可接受（P2-01）；`miniz_oxide` = 上游约束（P2-02）；图片非 2 的幂 = **测试覆盖缺口**（已知限制，非缺陷）；3c 前缀 = **0 命中，无问题**。 |

---

## 汇总

| 级别 | 数量 | 明细 |
|---|---:|---|
| **P0** | **0** | — |
| **P1** | **3** | P1-01 测试数文档漂移 / P1-02 23 项声明残留（已修） / P1-03 x11 EnvGuard（需 WSL） |
| **P2** | **4** | serial 残留 / miniz_oxide 双版本 / ignore 计数不同步 / Guide 教训缺编号 |
| **P3** | **4** | hash exit 2 策略 / `Image::from_rgba` 旧项失效 / TextBuffer 无上限 / cfg(test) 边界（✅） |

### 其中

- **文档与代码脱节**（与旧报告同类）：**P1-01、P1-02、P2-03** —— 三处都是"文档写了旧数字/旧结论"。
  共同根因：**没有任何机制保证文档数字随代码变化**。建议：把关键数字（测试数、ignore 数）
  从 `docs/evidence/*.txt` **生成**到文档，而不是手写。
- **环境受限、不阻塞发布**：**P1-03**（WSL）、**P2-01**（联网）。
- **可立即修**：P1-01、P2-03（改数字）、P2-04（加编号）—— 纯文档。

---

## 与旧报告的关系

旧报告 23 项中**只有 12 个编号可追踪**（#5 #7 #8 #9 #11 #12 #13 #14 #15 #18 #19 #20）。
本报告**未复查旧编号**，而是重新取证，发现：

- 旧项“`Image::from_rgba` 溢出”所指的 API **不存在**（P3-02）—— 说明旧报告至少有一项基于过时/错误的前提；
- 旧项“生产路径 `[dbg]` 前缀”实测 **0 命中**（3c 已证）；
- 旧项 `#11` 仍是**真缺陷**（P1-03）。

**结论**：旧报告不可作为当前工作的依据；以本报告为新基线。
