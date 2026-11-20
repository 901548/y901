# Y901 v0.2.2 发布说明

> 版本：`0.2.2` · 日期：2026-11-20 · 上一版说明：[`RELEASE_NOTES_v0.2.1.md`](RELEASE_NOTES_v0.2.1.md)

本版**无破坏性变更**（patch），收掉 v0.2.1 发布说明里列出的三条"遗留"：

1. Linux 行在 doc 数字检查里**不再整行豁免**；
2. `StyledText` 不再用自己的裁剪切掉大字号下伸笔画；
3. **X11 接上 HiDPI**（此前 `scale` 恒 1.0）。

## 1. X11 DPI：从屏幕物理尺寸推缩放

此前 X11 侧 `Options` 按 DIP 解释、`scale` 却恒为 1.0，等于"窗口永远 100%"。

- `ServerInfo` 现在解析 Setup 响应里的 **`width-mm` / `height-mm`**（X11 协议偏移 24/26）。
- `dpi_scale_from_screen()`：`dpi = px × 25.4 / mm`，再**吸附到桌面档位**
  （1.0 / 1.25 / 1.5 / 2.0，上限 4.0）。
  为什么不是连续值：Xvfb/部分驱动报的 mm 只是近似（实测 1280×800 报 325mm ≈ 100 DPI），
  连续值会得到 1.04 这种半吊子缩放，把像素哈希与既有截图全部扰动。
  `< 1.2` 一律按 1.0 —— **因此既有 Linux 像素证据不受影响**。
- 环境覆盖：`Y901_X11_SCALE=1.5` 或 `Y901_DPI=144`（测试 / 特殊显示器）。
- 接入点：`X11Runtime` 在建窗**之前**探一次屏幕信息，然后
  - 字形引擎按**物理字号**（`font.size × scale`）创建；
  - `SoftBackend::new` 用**物理尺寸**分配缓冲，并 `set_dpi_scale(scale)`；
  - 窗口尺寸 = `round(DIP × scale)`；日志会打印"逻辑 WxH（物理 WxH，scale=…）"。
- 单测（仅 Linux 编译）：档位吸附（1.04→1.0、1.2→1.25、1.66→1.75、9.0→4.0、NaN→1.0）
  与 mm 推算（100/150/192 DPI）。

## 2. `StyledText` 的裁剪：纵向按墨迹放宽

`RichText`/`StyledText` 之前 `push_clip(self.rect)`——横向是对的（长行不能盖邻居），
但**纵向会把大字号的下伸笔画（g/y/j）切掉**：行高是"行距"，墨迹可以落到行框之下。

- `measure` 里按**最大 span 字号**算好 `ink_margin` 并缓存；`render` 的裁剪框改为
  `rect` 纵向上下各放 `ink_margin`（横向仍裁到自身宽度）。
- 新增**像素级**回归 `mixed_size_rich_text_invalidate_matches_full_redraw`：
  48px 富文本、同宽同高 j→o、只 `invalidate(id)`、局部清屏 vs 整窗重绘逐像素比对；
  变异验证：把裁剪改回 `push_clip(self.rect)`，用例立刻红（节点下方墨迹消失）。
- 顺带记录一个框架契约：`StyledText::update()` 会清空内部排版，调用方必须
  `request_layout()`（真实主循环会做），否则 render 拿到空排版。

## 3. Linux 计数：从"豁免"改为"独立校验"

此前 `sync-doc-numbers.ps1 -Check` 把含 `Linux` 的行**整行豁免**——等于"Linux 的数字永远不会错"。

- 新增独立数据源 [`docs/evidence/linux-numbers.txt`](evidence/linux-numbers.txt)
  （`passed / ignored / targets`），由 `scripts/linux-verify.sh` 在 Linux 上跑完全量
  workspace 测试后**自动写回**（脚本第 7 步；`SKIP_FULL=1` 可跳过）。
- `-Check` 对"含 Linux 且谈计数"的行逐数字校验；自测脚本的变异用例从 7 条扩到 **8 条**
  （新增"Linux 计数漂移"）。
- 豁免只剩"历史口径"与"当轮口径"（必须在文档里显式标记），不再有按业务仓整行放行的规则。

## 验证矩阵（2026-11-20）

### Windows（`scripts/gate.ps1`）

| 检查 | 结果 |
|---|---|
| 默认测试 | **983 passed / 0 failed / 61 ignored**（150 个测试目标） |
| `--include-ignored` | **1044 passed / 0 failed** |
| clippy / fmt / `missing_docs` | 0 warning / clean / 0 |
| 像素哈希 | `0xF7455CCE7781A864` / 400×220 |
| `cargo audit` | 0 漏洞 |

### Linux（CentOS 7.9 + Xvfb，虚拟机）

| 检查 | 结果 |
|---|---|
| 默认测试（`xvfb-run cargo test --workspace`） | **884 passed / 0 failed / 27 ignored**（150 targets） |
| x11 `check --all-targets` | exit 0 |
| x11 `golden` / `real_window` | 32/0、12/0（2 ignored） |
| `stroke_round_aa` / `sgr_reverse` / `ui+widgets` | 全部 exit 0 |
| `linux-verify.sh` 汇总 | **7 个步骤全 0**（新增 `FULL_WORKSPACE`，并写回 `linux-numbers.txt`） |

证据：[`linux-full-2026-11-19.log`](evidence/cross-platform/linux-full-2026-11-19.log)、
[`linux-verify-2026-11-19.log`](evidence/cross-platform/linux-verify-2026-11-19.log)。
（文件名保留首次落盘日期，内容已按本版 HEAD 重跑刷新。）

## 已知限制（更新）

| # | 项 | 说明 |
|---|---|---|
| 1 | **X11 HiDPI 的 DPI 来源是 mm** | 依赖服务器报的物理尺寸；不准时用 `Y901_DPI` / `Y901_X11_SCALE` 覆盖 |
| 2 | **历史口径的数字不由检查器校验** | 需要在文档里写"历史/当轮"标记；这类数字不再自动纠偏 |
| 3 | **ClearType gamma / 不如 GDI 锐 / 两后端字形引擎不同** | 同 v0.2.0 |
| 4 | **各向异性图片缩放 / 多窗口** | 同 v0.2.0 的长期项 |

## 升级注意

1. **无破坏性变更**：`0.2.1 → 0.2.2` 可直接升级。
2. X11 下若你的显示器 mm 报告偏大/偏小导致缩放档位不对，用 `Y901_DPI=96|120|144…` 覆盖；
   需要精确到非 0.25 档位时用 `Y901_X11_SCALE`。
3. `linux-verify.sh` 现在默认会跑全量 workspace 测试并**写** `docs/evidence/linux-numbers.txt`；
   CI 里若不想重复跑，设 `SKIP_FULL=1`（但那样不会更新计数文件）。
