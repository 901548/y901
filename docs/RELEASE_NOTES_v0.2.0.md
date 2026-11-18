# Y901 v0.2.0 发布说明

> 版本：`0.2.0` · 日期：2026-11-18 · 上一版说明：[`RELEASE_NOTES_v0.1.20.md`](RELEASE_NOTES_v0.1.20.md)

本版有**一个破坏性变更**（`Options` 语义），其余是渲染正确性修复与门禁恢复。

## 破坏性变更：DPI 语义统一（选项 A）

之前框架是"一半缩放、一半不缩放"：控件度量乘 `dpi_scale`（行高、圆点、按钮高度），
而容器 `padding` / `gap` 与窗口客户区**不缩放**——三者只在 100% 下自洽。
125% 缩放下实测表格行距 28→35px，而卡片左内边距仍是 24px，内容被挤在一起；
一条 GPU 用例（固定 20px 行装不下 20px 字形 → 字形溢出脏区）就是它的直接产物。

| 变更 | 旧行为 | 新行为 |
|---|---|---|
| `Options.width/height` | 物理像素 | **DIP**：客户区 = `round(DIP × dpi_scale)` |
| `Flex::padding()` / `gap()` | 固定像素 | **DIP**，实际值 × `metrics.scale()` |
| `Flex::padding_fixed()` / `gap_fixed()` | — | **新增**：不缩放的物理像素（1px 描边内缩、命中阈值） |
| `Flex::padding_scaled()` / `gap_scaled()` | 显式缩放 | 保留为**与默认等价的别名**（既有调用点零改动） |

详见 [`design/dpi-policy.md`](design/dpi-policy.md)。

## 修复

| 类别 | 内容 | 证据 |
|---|---|---|
| **表格** | 表头先于数据行绘制导致被盖住；命中测试从底部倒数导致"点第 N 行选中第 N−1 行" | 实测表头 y=212、首行 y=244、行距恒 28px；点击首行像素变化 34.6%，点表头 0% |
| **字形度量** | `GlyphInfo::bearing_y` 在两个后端语义不一致（y-up 底边 vs 顶部下沉）；基线不随字号缩放；L0 分数缩放采样漏最右/最下一行 | 混排 14 号标签与 16 号输入值底边同为 y=365；软/硬后端同一条整数基线 |
| **文字发糊** | 字形四边形落在分数像素上被线性采样重采样：125% 下竖笔 `74 143 90`（峰值仅正文字色 57%）。改为**整像素吸附 + 行级基线取整** | 修后竖笔两步到底；峰值亮度 249→**255**；亮部像素 **+13%**；边缘梯度 p95 **+12.8%** |

详见 [`design/text-sharpness.md`](design/text-sharpness.md)。

## 门禁与工具

此前 `scripts/gate.ps1` **红在第一步**（`cargo fmt --check` 报 69 个文件），红久了就没人看。
本版把它恢复成能说实话的门禁：

- `style:` 独立提交跑 `cargo fmt --all`（纯空白/排序，`git show -w` 为空）；
- 测试基线 `751 → 960`（旧值自 v0.1.17 起未刷新，"测试数不得下降"形同虚设）；
- `numbers.txt` / `test-inventory.txt` 重新生成（129 个二进制 / 1018 条用例）；
- `missing_docs`（`-W missing_docs` 构建）**33 条 → 0**，全部集中在 WASM 绑定；
- 修 `sync-doc-numbers.ps1 -Check` 的假失败：`$baseline` 在 `-Check` 分支从未赋值，
  文档里只要写"基线 **N**"就永远 stale；
- 修 125% 下必红的 GPU 用例 `invalidate_rect_partial_equals_full`（脏区按实际字形高度标）；
- 新增 `scripts/hw-smoke.ps1`：把默认套件**不跑**的 `#[ignore]` 用例真跑一遍并落盘。

## 新增测试

| 用例 | 抓什么 |
|---|---|
| `backend_parity::d3d_and_soft_agree_on_ink_rows` | 软/硬后端**墨迹包围盒逐边对齐**与逐行墨迹带（防"错一行"，MAD 抓不到平移） |
| `dpi_layout::*`（3 条） | 留白随 DPI 缩放 / `*_fixed` 不缩放 / 后设置者优先 |
| `glyph_geometry_tests::*`（3 条） | 基线取整、落点吸附、软硬同一条整数基线（去掉 `.round()` 立即红） |
| `gap_scaled_precedence::*`（4 条） | gap 家族后设置者优先 + `gap_fixed` 反例 |
| `table` 绘制/命中（5 条） | 表头/行区域与命中索引一致 |

## 验证

- `cargo test --workspace`：**960 passed / 0 failed**（`ignored=61`）
- `cargo test --workspace -- --include-ignored`：**1021 passed / 0 failed**
- 硬件冒烟 `scripts/hw-smoke.ps1`：**61 passed / 0 failed / 0 SKIP**
  （证据 `docs/evidence/hw-smoke-2026-11-18.log`；本机 GPU 是 GameViewer 虚拟适配器，
  仅功能正确性可引用）
- `cargo clippy --workspace --all-targets -D warnings`：0 警告
- 像素哈希：`0xF7455CCE7781A864` / 400×220（字形落点变化必然改哈希，基线已按脚本既有约定刷新并记录原因）
- `missing_docs`：0；`cargo audit`：0 漏洞
- `scripts/gate.ps1`（含 audit）：**GATE OK**

## 已知债务

| # | 项 | 说明 |
|---|---|---|
| 1 | **ClearType gamma 未定** | `pow(cov, 1/2.2)` 把整帧亮度 +5.8%、亮像素 +26%；A/B 数据已记录（`Y901_GAMMA_AB=1`），默认值保持不动，属风格取舍 |
| 2 | **不如 GDI 锐** | GDI 在 16px 用字体**点阵位图**（实测两种 hint 都只有 1 种灰度），我们走 DirectWrite 轮廓 + AA；要硬边需切 `DWRITE_RENDERING_MODE_GDI_CLASSIC`，属独立提案 |
| 3 | **x11 未接 HiDPI** | x11 侧 `Options` 同样按 DIP 解释，但 `scale` 恒为 1.0；接上只需在窗口创建处补一个乘法 |
| 4 | **两后端字形引擎不同** | Windows 默认 DirectWrite、CPU 侧 MyFontEngine；`backend_parity` 通过 `Y901_FONT=self` 对齐后再比较 |

## 升级注意

1. **`Options` 语义变更**：100% 缩放下行为完全一致；125% 下客户区会大 1.25 倍（这是本版要的行为）。
2. **`padding()` / `gap()` 现在随 DPI 缩放**：绝大多数调用点**不需要改**；
   只有"必须恰好 n 个物理像素"的地方改成 `padding_fixed()` / `gap_fixed()`。
   已检查：`1px` 分隔线走的是 `FlexItem::height(1.0)`（本来就随缩放），不受影响。
3. 像素哈希门禁需固定 `Y901_TEST_DPI_SCALE=1.0`（`scripts/hash-gate.ps1` 已经这么做），
   否则换显示器会误报渲染回归。
4. PowerShell 脚本必须是 **UTF-8 带 BOM**：`hash-gate.ps1`、`sync-doc-numbers.ps1`、
   `hw-smoke.ps1` 都踩过"PS 5.1 按 ANSI 读 UTF-8 → 中文注释变乱码 → 语法错误"。
