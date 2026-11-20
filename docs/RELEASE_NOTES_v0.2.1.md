# Y901 v0.2.1 发布说明

> 版本：`0.2.1` · 日期：2026-11-19 · 上一版说明：[`RELEASE_NOTES_v0.2.0.md`](RELEASE_NOTES_v0.2.0.md)

本版**无破坏性变更**（patch）：修掉一个"整帧空白"的 D3D 渲染 bug、
把脏区墨迹外扩从"树级常数"改成"按节点字号"，并首次把 **Linux 侧验收**做成
可复现的脚本 + 证据。`v0.2.0` 的 DPI 语义变更（`Options` 按 DIP）在本版保持不变。

## 修复：Grid + Image 在 D3D 下整帧空白

`examples/image_gallery.rs`（Grid 里放 Image）曾经**整帧只剩清屏色**；Flex+Image、
Grid+文字都正常，所以长期只在示例里暴露。

- **根因**：`Context::draw_instances` 把批次起点传给了 `DrawInstanced` 的第 4 参数
  `StartVertexLocation`；实例缓冲是按实例步进的槽位，正确参数是第 5 个
  `StartInstanceLocation`。于是图片批次用 `ps_image` 重画了**实例 0（背景）**，
  后续每个批次也都从实例 0 开始 —— 整帧被涂成错误颜色。
- **修复前后**：同一棵树快照从 **1 种颜色**（`(12,0,0)` 整帧）变为 **62 381 种**，
  四象限主色各出现 9k~15k 像素；`Y901_SNAPSHOT=1` 两次重画哈希相等。
- **回归**：`crates/y901-backend-win32/tests/grid_image_gpu.rs`（无 GPU 时打印跳过）。
- 诊断开关保留：`Y901_DEBUG_BATCHES=1`（打印批次与每次 draw 的实例区间）、
  `Y901_NO_IMAGE_BATCH=1`（强制单次 draw，用于二分"批次 vs 实例"问题）。

## 修复：大字号文本局部重绘残影（脏区墨迹边距）

脏区必须比控件矩形多扩一点，才能清掉下伸字（g/y/j）与圆字母 overshoot 的墨迹。
本版把这个外扩从"一个常数"改对了：

| 阶段 | 行为 | 后果 |
|---|---|---|
| v0.2.0 | 无外扩 | 局部重绘留下残影（j→o 同宽改字，125% 下 164 像素） |
| 上一轮 | 按 `TextStyle::default()`（16px）算**一个树级常数** 4.63px | 16px 文本正确；24/32/48px 标题仍留残影（实测差 72 / 136 / 312 像素） |
| **本版** | `Widget::ink_overflow()` 按**节点自身字号**返回；`UiTree` 布局时缓存 `id → margin` | 16/24/32/48px 全部逐像素一致 |

- 覆盖路径：`invalidate_rect()`、`invalidate(id)`、布局新旧矩形并集（`mark_moved`）、
  Signal 失效、浮层关闭、广播矩形 —— 全部走同一个 `add_dirty_rect_with_margin()`。
- 回归 `node_invalidate_ink_margin`：四个字号各做一次"同宽同高 j→o + 只 `invalidate(id)`
  + 局部清屏重绘 vs 整窗重绘"，并断言**外扩量随字号增长**；
  变异验证：把 `TextLabel::ink_overflow` 改回 `None`，24px 立刻红。
- `StyledText` 按**最大 span 字号**声明边距；它自己 `push_clip(self.rect)`，
  墨迹不可能越界，所以该条是单元级断言（STATUS.md 已记录这个边界）。

## 修复与加固（其他）

| 类别 | 内容 |
|---|---|
| 图集 | D3D11 整图 reset 的 `UpdateSubresource` 按 RGBA 传 `row_pitch=size×4` / `slice_pitch=size²×4`（原来按像素算，少 4 倍），创建与 reset 共用同一个 helper |
| 帧预算 | 忙档等待 = `16ms − 上一帧工作耗时`（`last_tick` 差值），不再固定睡 16ms；profiler 的 `present_ms` 不再恒为 0；口径改为**纯工作耗时**（不含事件等待） |
| 组件 DPI | `VerticalList` 的 `padding`/`gap` 与 Flex 同口径（DIP 缩放），新增 `padding_fixed()`/`gap_fixed()` |
| 渲染回归 | `sgr_reverse` 改用"格内背景众数"断言，对字体回退（Segoe UI / DejaVuSans）不敏感，Windows/Linux 双绿 |
| 测试确定性 | `y901-shell` 剪贴板用例共用**一把进程锁**（原先两个模块各一把，等于没锁）+ 可注入假实现；Xvfb 下 20 次里 17 次红 → 20/20 绿 |
| 缩放回归 | `d3d_text_scaling` 的墨量比/像素比区间从 ~4 倍宽收到 `0.30..=0.44 / 0.33..=0.47`（实测 0.372 / 0.400） |

## 门禁与工具

- **新增 `scripts/linux-verify.sh`**（版本化）：x11 `check`（all-targets）/ `golden` /
  `xvfb-run real_window` + `render stroke_round_aa` + `extras sgr_reverse` + `ui+widgets`，
  任一失败 exit 1。此前 Linux 验收脚本只存在于虚拟机里，且漏掉 Soft 描边用例。
- **doc 数字检查器加严**：`-Check` 现在校验"含计数语义的行里**所有**三位数"，
  不再只认加粗写法；新增 `targets=` 字段；平台/历史数字必须**在文档里**显式标记才豁免；
  并新增**反自伤**规则（`**N**xF…`、`字母**N**` 直接判 stale —— 这是脚本自己曾造成的脏数据）。
- **新增 `scripts/test-doc-numbers-check.ps1`**：向文档注入 7 类漂移并断言全部被抓到，
  接入 gate —— 检查器本身也有了覆盖证明。
- `gate.ps1`：缓存审计库 ≤7 天时结论打印 `GATE OK (CACHED DB N.Nd; offline audit)`，
  并把天数写进 `target/audit.log` 标题行；`.ps1` 统一 UTF-8 with BOM（无 BOM 时
  PowerShell 5.1 会把某个汉字的 `0x5C` 尾字节当反斜杠，吞掉字符串终止符）。
- `.gitattributes` 固定 `*.sh` 为 LF（`git archive` 在 Windows 上曾把脚本导出成 CRLF，
  Linux 直接报 `$'\r': 未找到命令`）。

## 验证矩阵（2026-11-19）

| 检查 | Windows | Linux（CentOS 7.9 / Xvfb，虚拟机） |
|---|---|---|
| 默认测试 | **982 passed / 0 failed / 61 ignored**（150 个测试目标） | **881 passed / 0 failed / 27 ignored**（150 targets） |
| `--include-ignored` | **1043 passed / 0 failed** | —（差异来自平台 cfg 门控） |
| x11 专项 | — | `check` / `golden 32` / `real_window 12` / `stroke_round_aa` / `sgr_reverse` / `ui+widgets` **六项 EXIT_*=0** |
| clippy / fmt / missing_docs | 0 警告 / clean / 0 | — |
| 像素哈希 | `0xF7455CCE7781A864` / 400×220 | — |
| `cargo audit` | 0 漏洞 | — |
| `scripts/gate.ps1` | **GATE OK** | — |

证据：
[`docs/evidence/cross-platform/linux-full-2026-11-19.log`](evidence/cross-platform/linux-full-2026-11-19.log)、
[`linux-verify-2026-11-19.log`](evidence/cross-platform/linux-verify-2026-11-19.log)；
计数唯一来源 [`docs/evidence/numbers.txt`](evidence/numbers.txt)。

## 已知限制（未变或新记录）

| # | 项 | 说明 |
|---|---|---|
| 1 | **x11 未接 HiDPI** | `scale` 恒 1.0（沿用 v0.2.0 记录） |
| 2 | **Linux 的计数行被 doc 检查豁免** | 该环境的计数无法在 Windows 复现，只能靠 `linux-verify.sh` 日志核对（v0.2.2 起改为独立数据源校验） |
| 3 | **`StyledText` 自裁剪** | 墨迹不会越界，因此 per-node 边距只做了单元级断言；取消自裁剪时需补像素回归 |
| 4 | **ClearType gamma / 不如 GDI 锐 / 两后端字形引擎不同** | 同 v0.2.0，属风格取舍或既有设计 |

## 升级注意

1. **无破坏性变更**：`0.2.0 → 0.2.1` 可直接升级。
2. 若你依赖"脏区总是精确等于控件矩形"的内部断言型测试，请改为按字号外扩
   （或直接用 `Widget::ink_overflow()` 的返回值作为期望）—— 这是修复残影的必要行为。
3. `VerticalList::padding()`/`gap()` 现在是 DIP；需要固定物理像素请用
   `padding_fixed()`/`gap_fixed()`。
