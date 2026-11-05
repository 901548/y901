# Y901 v0.1.17 最终审查报告

> 审查基准：`F:\Y901` HEAD `7e7cc2b`（tag `v0.1.17`）。
> 实测：Windows **824 passed / 0 failed / 52 ignored / 876 include-ignored**；
> WSL + Xvfb :99 x11 **11 passed / 0 failed / 2 ignored**；门禁 **GATE OK**（七步）。
> 本报告基于当前代码与当前脚本取证，不引用旧报告的结论。

---

## P0（阻断）

**无。**

三个"近期修复是否真落地"的检查全部通过（详见 §1）：

| 修复 | 结论 | 证据 |
|---|---|---|
| `EnvGuard` 不再持有锁 | ✅ **已落地** | `real_window.rs` 的 `struct EnvGuard { key, prev }` **没有** `MutexGuard` 字段；`set`/`remove` 体内无 `lock(`（该处唯一的 `MutexGuard` 是 `connect()` 的返回类型，与 guard 无关）；`static ENV_LOCK` 已不存在 |
| `read_reply` 5s 超时 | ✅ **已落地** | `conn.rs` `read_exact` 内 `const REPLY_TIMEOUT = 5s` → `set_read_timeout(Some(...))` → `read_exact` → `set_read_timeout(None)` |
| `sync-doc-numbers.ps1` 无时间戳、无字面转义 | ✅ **部分落地** | 脚本内 `\\uXXXX` 计数 **0**；`numbers.txt` 为纯数字（`passed/ignored/include_ignored/baseline`，无 `Auto-generated`）；连续两次 sync 的文件哈希相同（幂等）。**但见 P1-01：脚本的"数字同步"覆盖面不完整** |

---

## P1（必须修）

### [P1-01] `docs/STATUS.md:5`、`docs/ACCEPTANCE.md:13,15` — 纯文本形式的旧数字仍在，`-Check` 抓不到

**证据**

```
STATUS.md:5      - **测试**: 818（基线 818；`#[ignore]` 45，含 ignore 863）
ACCEPTANCE.md:13 | 加上 `--include-ignored` | **824 passed / 0 failed**（801 默认 + 45 标记为 `#[ignore]`）|
ACCEPTANCE.md:15 至少 **45**，846 = 801…（`#[ignore]` 部分）GPU … harness …
numbers.txt      passed=824  ignored=52  include_ignored=876  baseline=824
```

**问题**：`STATUS.md` 是状态主文档，第一屏就写着 818/45/863 —— 与实测 824/52/876 **直接矛盾**。
`ACCEPTANCE.md` 的括号里还保留 `801 默认 + 45` 的旧算术。

**根因**：`sync-doc-numbers.ps1` 的替换与 `-Check` 都**只覆盖"加粗的 3 位数"**
（为了避开历史数字而收窄，见 `7e7cc2b` 之前的提交说明）。
于是任何**非加粗**形式的旧数字都同时逃过"生成"和"检查"。

**这属于与旧报告 P1-01 完全同类的问题，因此 P1-01 并未真正关闭。**

**修复建议**（约 15 分钟，三选一或组合）：

1. 把这三处改成加粗/表格形式，让脚本覆盖（最快，但只是"迁就工具"）；
2. 扩展脚本的比对口径：对 `STATUS/ACCEPTANCE` 里含 `测试|passed|ignored|ignore` 的行，
   把**所有** 3 位数与 `{passed, ignored, include_ignored, baseline}` 比对，命中非法值即报漂移，
   同时允许显式的历史标注（例如行内带 `历史` 关键字则跳过）；
3. 删掉"含 ignore 863""846 = 801+45"这类**推导性数字** —— 它们是旧口径的残留算式，
   留着只会持续漂移。

**我倾向 2 + 3**：口径由工具统一，且删掉不可维护的推导算式。

---

## P2（建议修）

### [P2-01] `docs/FRAMEWORK.md:243`、`docs/PERFORMANCE.md:40` — 其余旧数字

```
FRAMEWORK.md:243   cargo test --workspace   # 801 项（含 7 项 GPU harness，--ignored）
PERFORMANCE.md:40  …默认 818 项…（在解释 #[ignore] 时引用总数）
```

非加粗，故同样逃过 `-Check`。与 P1-01 同根因，只是影响面小（说明性文字，不是状态主文档）。

### [P2-02] `docs/FRAMEWORK_GUIDE.md` — 索引只列到 L9，正文已有 L11

**证据**

```
正文 ### 标题： L1 L2 L3 L4 L5 L6 L7 L8 L9 L10 L11   （无重复）
索引条目：     L1 L2 L3 L4 L5 L6 L7 L8 L9            （缺 L10、L11）
```

**修复建议**：把 L10/L11 补进索引（两行）。

### [P2-03] `docs/evidence/numbers.txt` 头部注释仍提示"可再生成"，但未说明 `include_ignored` 的口径

**说明**：`include_ignored=876` 依赖"全量跑无失败"；一旦并行 fixture 再次污染分配计数器，
它会被写成 `n/a`。当前无问题，但**读者无法从文件本身判断 876 是"全量"还是"部分"**。
**建议**：在该行旁加固定标注（如 `include_ignored=876 # full run, 0 failed`）。

---

## P3（可接受 / 记录）

### [P3-01] `serial 0.4.0` 是 `portable-pty 0.8.1` 的硬依赖

`DEPENDENCIES.md` 已如实记录（含 manifest 引用 `[dependencies.serial] version = "0.4"`），
并写明"非残留、需升级上游才可清"。**没有谎称已清除**。✅

### [P3-02] `miniz_oxide` 双版本（`0.8.9` + `0.9.1`）

`cargo tree --duplicates` 仅此一项，记录为 `png 0.17.16` 的上游约束。✅

### [P3-03] 两条 x11 用例被 `#[ignore]`

```
real_window.rs:274  #[ignore = "探测：真实 Xvfb 下…已记录…（附数据）"]
real_window.rs:637  #[ignore = "调试"]
```

13 个 `#[test]`，2 个 ignored，**11 passed** —— 与 WSL 实测一致，
**没有用 ignore 掩盖失败**。✅（L637 的 `"调试"` 理由略含糊，建议改写成具体原因，但不阻塞）

### [P3-04] `hash-gate` 对客户区尺寸变化判 exit 2 / 门禁判失败

设计取舍（宁可红也不静默重建基线）。✅

---

## 逐维度结论

| 维度 | 结论 |
|---|---|
| **1. 旧报告 23 项** | P1-02 ✅（`STATUS.md` 已改为"12 项可追踪 + 11 项缺失"，并查证原始报告确实未落盘）<br>P1-03 ✅（代码 + WSL 运行时双证）<br>P2-01 ✅（更正为上游约束）<br>P2-02 ✅<br>P2-03 ✅（ignore 统一为 52……但 `STATUS.md:5` 仍写 45 → 见 P1-01）<br>P2-04 ⚠️（L1–L11 存在且无重复，**索引缺 L10/L11** → P2-02）<br>**P1-01 ❌ 未真正关闭** → P1-01 |
| **2. 近三轮修复质量** | ✅ 三项全部落地（`EnvGuard` 无锁字段、`read_exact` 5s、脚本无转义）。唯一问题是"脚本覆盖不完整"，已单列 P1-01 |
| **3. 门禁七步** | ✅ MSRV / 测试 / 指纹 / clippy / 哈希 / docs / audit + 数字检查，各有 `Fail`/`exit`；`NOT RELEASABLE`(exit 2) 与 `DEV`(exit 0) 分离；`sync-doc-numbers.ps1 -Check` 已接入并带 `HINT`（`gate.ps1:133-136`） |
| **4. 测试有效性** | ✅ `present_sync_tests` 三条有反向验证记录（1→0 使断言 FAIL，实测 `[0]`）；`image_mip_quality` 为闭式真值（127.5 / `8i+2`）+ 变异对照；`texture_draw_is_zero_alloc_quick` **非 ignored** 且默认套件中运行；`test-inventory` 两次故意触发均有记录（`expected 25, got 26`）；`real_window` 11 passed 无 ignore 掩盖 |
| **5. 文档一致性** | ❌ **有 4 处旧数字**（STATUS 1、FRAMEWORK 1、PERFORMANCE 1、ACCEPTANCE 2 处算式）；Guide 索引缺 L10/L11；`numbers.txt` 纯数字 ✅；`STATUS.md` 的 Linux 验证口径 ✅（明确 `-p y901-backend-x11`，并说明 `--workspace` 不可行） |
| **6. 架构一致性** | ✅ `y901-ui` 生产依赖仅 `y901-core`；`y901-render` 无 `windows-sys`；重复依赖仅 `miniz_oxide`（上游）；无环 |

---

## 汇总

| 级别 | 数量 | 明细 |
|---|---:|---|
| **P0** | **0** | — |
| **P1** | **1** | P1-01 纯文本旧数字（STATUS 首屏 818/45/863 + ACCEPTANCE 旧算式），`-Check` 覆盖不到 |
| **P2** | **3** | P2-01 FRAMEWORK/PERFORMANCE 旧数字；P2-02 Guide 索引缺 L10/L11；P2-03 `include_ignored` 口径未标注 |
| **P3** | **4** | serial 上游 / miniz 上游 / 两条 ignore 有据 / hash-gate 策略 |

### 分类

- **文档与代码不一致**（与旧报告同类）：**P1-01、P2-01、P2-02**。
  根因统一：**"数字由工具生成"只在工具覆盖的形态上成立**；
  非加粗、括号内推导式、索引列表，都是工具够不到的地方。
- **环境受限、不阻塞**：无新增（serial 需升级上游；miniz 需上游发布）。
- **可立即修、纯文档**：P1-01、P2-01、P2-02、P2-03（合计约 20 分钟，无代码风险）。

---

## 最终结论

**Y901 v0.1.17 不能发布（阻断项：P1-01）。**

理由：`docs/STATUS.md:5` 是状态主文档首屏，其"818 / 45 / 863"与实测"824 / 52 / 876"直接矛盾，
且 `docs/ACCEPTANCE.md` 仍保留 `846 = 801 + 45` 的旧算式。
这与旧报告的 P1-01 属同一类问题、同一根因（数字未被工具全覆盖），
因此**"P1=0" 的说法在取证层面不成立**。

**修复 P1-01 + P2-01 + P2-02 后（纯文档，约 20 分钟，无需改代码），P0/P1 将为 0，
届时 v0.1.17 可发布并进入稳定维护状态。**

> 说明：本轮**没有发现任何代码缺陷**。全部阻断项都在"文档数字"这一层，
> 且都不是新引入的错误 —— 而是**上一轮收窄 `-Check` 覆盖面时留下的盲区**。
