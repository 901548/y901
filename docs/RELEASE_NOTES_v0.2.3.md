# Y901 v0.2.3 发布说明

> 版本：`0.2.3` · 日期：2026-11-20 · 上一版说明：[`RELEASE_NOTES_v0.2.2.md`](RELEASE_NOTES_v0.2.2.md)

本版**无破坏性变更**（patch）。内容分两部分：**对 v0.2.2 的复审修复**，
以及**业务工程拆分后的仓库清理**。

## 1. 复审发现并修复：邻居局部刷新会擦掉越界墨迹

v0.2.2 为修"`StyledText` 自裁剪切掉大字号下伸笔画"而放宽了它的纵向裁剪，
但绘制裁剪（`render_pass`）判断"要不要重绘某个节点"时只看节点 **rect**。

- **症状**：富文本的下伸笔画越出自身 rect、压到下方邻居的矩形里；
  当**只标脏邻居**时，脏区清屏把那几笔擦掉，而富文本因为 rect 不相交没被重绘
  → 局部刷新后笔画消失（与整窗重绘逐像素不一致）。
- **实测**：构造 48px 富文本 + 紧贴其下的 16px 标签，只 `invalidate("below")`，
  局部重绘与整窗重绘差 **102 像素**（首个差异 `(16,71)`）。
- **修法**：`render_pass` 增加"**rect ∪ 墨迹**与脏区相交"的判据 ——
  墨迹压进脏区的节点也必须重绘。扩展量通常 2~14px，绘制量增加可忽略。
- **回归**：`neighbour_invalidate_does_not_erase_overhanging_ink`
  （变异验证：去掉新判据立即红，102 像素 / `(16,71)`）。
- **顺带守住性能契约**：per-node 边距只在**真的重排**时重采；
  若放在 `layout_tree` 的 `is_fresh` 快路径外面，每帧都会白走一遍全树（O(n)），
  与"约束未变时 layout 是 O(1)"的契约冲突。新增用例在无变化 layout 后仍断言边距正确。

## 2. 业务工程拆分后的仓库清理

业务工程已迁移到独立仓库，本仓不再包含它的任何代码、脚本、文档与证据：

| 类别 | 处理 |
|---|---|
| 代码 | `secret.rs` 不再提示某业务专用环境变量（改为通用提示，测试同步）；`table.rs` / `no_c_deps.rs` 注释去业务仓举例 |
| 脚本 | 删除业务专用的 WSL 冒烟脚本；`sync-doc-numbers.ps1` 去掉按业务仓整行豁免的规则（豁免只剩 `Linux` / `历史` / `旧口径` / `当轮` / 显式标记） |
| 文档 | `ZERO_C_PLAN.md` / `ZERO_C_PROOF.md` **重写为单仓范围**（含最新守卫位置与变异验证）；`STATUS` 的去 AI 段、已知未覆盖表、跨仓迁移债段改为"业务工程已独立"；`CROSS_PLATFORM_PLAN` / `DEPENDENCIES` 改为通用表述 |
| 证据 | 删除业务专用的 Xvfb 日志与两份引用了已删除测试名的旧计数日志；证据索引同步 |
| 守卫 | 零 C 依赖守卫只覆盖本仓；解析失败**报错而不是静默绿** |

> 说明：历史 `.log` 属"原始输出"证据，原则不改写；本轮删除的两份是**已被取代**的旧计数日志，
> 删除后 `docs/evidence/README.md` 与 `ACCEPTANCE.md` 的引用同步更新。

## 3. 验证矩阵（2026-11-20）

### Windows（`scripts/gate.ps1`）

| 检查 | 结果 |
|---|---|
| 默认测试 | **984 passed / 0 failed / 61 ignored**（150 个测试目标） |
| `--include-ignored` | **1045 passed / 0 failed** |
| clippy / fmt / `missing_docs` | 0 warning / clean / 0 |
| 像素哈希 | `0xF7455CCE7781A864` / 400×220 |
| `cargo audit` | 0 漏洞 |

### Linux（CentOS 7.9 + Xvfb，虚拟机）

| 检查 | 结果 |
|---|---|
| 默认测试（`xvfb-run cargo test --workspace`） | **885 passed / 0 failed / 27 ignored**（150 targets） |
| `scripts/linux-verify.sh` | **7 个步骤全 0**（check / golden / real_window / stroke / sgr_reverse / ui+widgets / full workspace），并写回 `linux-numbers.txt` |

证据：[`linux-full-2026-11-19.log`](evidence/cross-platform/linux-full-2026-11-19.log)、
[`linux-verify-2026-11-19.log`](evidence/cross-platform/linux-verify-2026-11-19.log)、
[`linux-numbers.txt`](evidence/linux-numbers.txt)。

## 4. 已知限制（未变）

| # | 项 | 说明 |
|---|---|---|
| 1 | **X11 HiDPI 依赖 mm 报告** | 不准时用 `Y901_DPI` / `Y901_X11_SCALE` 覆盖 |
| 2 | **干净 VM 的"只装 Rust"验证** | 本机装了 VS 2022，只能证明"不显式依赖 C 工具链" |
| 3 | **ClearType gamma / GDI 锐度 / 两后端字形引擎差异** | 设计取舍（v0.2.0 起记录） |
| 4 | **各向异性图片缩放 / 多窗口 / macOS** | 长期项，需外部条件或独立提案 |

## 5. 升级注意

1. **无破坏性变更**：`0.2.2 → 0.2.3` 可直接升级。
2. 若你的自定义控件有"墨迹越出 rect"的行为，请实现 `Widget::ink_overflow()`
   返回真实边距 —— 脏区外扩与"邻居擦除保护"都会用它。
3. `secret.rs` 在不支持平台上的报错文案改为通用提示（不再包含某业务仓的环境变量名）。
