# Y901 框架验收凭证

本验收矩阵汇总的是带日期的运行记录，不表示每次提交均重新执行。Windows 数字口径最近更新于 2026-11-22：默认测试 **1135 passed / 0 failed / 63 ignored**；包含 ignored 的完整运行 **1198 passed / 0 failed**。唯一计数数据源为 [numbers.txt](evidence/numbers.txt)，baseline 见 [test-baseline.txt](evidence/test-baseline.txt)。Linux 数字在 2026-11-26 X11 正确性收口后复测为 **1246 passed / 0 failed / 34 ignored / 188 targets**，证据见下方新日志及 [linux-numbers.txt](evidence/linux-numbers.txt)。

## 结果

| 口径 | 数量 |
|---|---|
| `cargo test --workspace --offline`（默认，跳过 `#[ignore]`） | **1283 passed / 0 failed / 70 ignored** |
| `cargo test --workspace --offline -- --include-ignored` | **1353 passed / 0 failed** |

`include_ignored=1353`（唯一来源 `docs/evidence/numbers.txt`；该数来自实际 `--include-ignored` 运行，而非默认通过数与 ignored 数相加的推算）。

`#[ignore]` 用例的构成见下表（总数以 `docs/evidence/numbers.txt` 为准）。

| 类别 | 已核对数量 | 用例 |
|---|---:|---|
| GPU / D3D11 依赖 | **13** | `snapshot_harness` ×10、`soft_matches_d3d_within_antialiasing_tolerance`、`pixel_statistics_and_png_baseline`、`rebuild_reuploads_atlas` |
| DirectWrite 对拍 | **2** | `compare_with_directwrite_l1_l2`、`l2_best_shift_diagnostic` |
| 性能基准 | **30** | ui 12（`perf_bench` ×6、`perf_dispatch_alloc` ×3、`layout_bench`、`perf_alloc`、`rev_snapshot`）、win32 8（`perf_bench` ×3、`present_bench`、`breakdown_rasterize_vs_atlas_insert`、`cold_start_500_cjk_glyphs`、`all_phases_reference_cost`、`soft_fill_rate_1280x800`）、render 10（`cjk_cold_start` ×3、`mip_latin` ×3、`perf_texture_alloc` ×3、`which_method_is_closer_to_supersampled_truth`） |
| 调试探针 | **5** | `debug_print_glyphs`、`debug_crossings_for_a`、`debug_print_cjk_frames`、`occupancy_matches_glyph_count_sanity`、`probe_orientation_and_channels` |
| 压力测试 | **1** | `forced_eviction_thrash_measurement`（图集 eviction 抖动） |
| **本表已分类合计** | **51** | 依据当前列出的名称计算；与全仓 ignored=63 相差 12 条未在本分类表盘点（其中 2 条是 `layout_profile` 的剖析用例）。总数以 `docs/evidence/numbers.txt` 为准。 |

## GPU 像素验收用例（真机 D3D11，全部 ok）

> **三个口径别混淆**（此前标题只写"8 个/7 个"，与另外两个数互相矛盾）：
> 本表列的是 **8 条按像素/解析真值断言的用例**；`snapshot_harness.rs` **文件里共 10 条
> `#[ignore]`**（另有 2 条是纯诊断/确定性自检）；全仓 `ignored=70` 是**所有 crate 的
> `#[ignore]` 总数**（含性能基准、DirectWrite 对拍、调试探针），不是本表。


| 用例 | 验证内容 |
|---|---|
| `snapshot_harness_is_deterministic` | 同一棵树两次绘制哈希相等 |
| `gradient_matches_math_at_45_degrees` | 45° 渐变逐像素对齐解析解（1106 点，≤0.0050） |
| `round_rect_sdf_matches_math` | 圆角矩形 SDF（内部 22840 / 边界带 708） |
| `round_stroke_draws_ring_only` | 描边只在轮廓带（1430），内部不被填充（3494） |
| `gradient_round_rect_matches_math` | 圆角渐变叠加（2652 点，≤0.0089 + 四角裁切） |
| `invalidate_rect_partial_equals_full` | 局部标脏重绘 == 整窗重绘（逐字节 0 差异） |
| `two_disjoint_dirty_rects_match_full_redraw` | 两块不相邻脏区 == 整窗重绘（逐字节 0 差异） |
| `grid_with_images_is_not_blank_on_gpu` | Grid + 4 图片不再整帧空白（62 381 种颜色；四象限主色命中） |

2026-11-22 复跑（Windows 本机 150% DPI）：`snapshot_harness` 的 10 条 `#[ignore]` 用例全部 ok。
其中 `invalidate_rect_partial_equals_full` 在复跑时先红（740 字节差异，首个差异在行底线下一行）：
用例把流式行框写死为 20px，而 150% 下 16 号字的物理行高是 24px，行框比字形矮 4px，
溢出的像素落在框架 2px 最小外扩之外。根因是**用例的 DPI 假设过期**（注释按 125% 写的），
不是渲染错；现已改为按 `TextMetrics::line_height` 取行框高度，100%/125%/150% 通用。

## 双平台验证矩阵（Windows 2026-11-26；Linux 2026-11-26）

Windows 门禁与 Linux 验收是**两套独立证据**：`gate.ps1` 只跑在 Windows 上，
`#[cfg(unix)]` 代码与 X11 运行时只能靠 `scripts/linux-verify.sh`（Xvfb）覆盖。
两张表分开写，是为了让"哪边的数就是哪边的"一眼可查（混在一行会被 doc 检查判为口径不明）。

### Windows（gate.ps1）

| 检查 | 结果 |
|---|---|
| 默认测试 | **1283 passed / 0 failed / 70 ignored**（188 个测试目标） |
| `--include-ignored` | **1353 passed / 0 failed** |
| clippy / fmt / `missing_docs` | 0 warning / clean / 0 |
| 像素哈希 | `0xF7455CCE7781A864` / 400×220 |
| `cargo audit` | 0 漏洞 |

### Linux（Ubuntu 22.04 / WSL2 + Xvfb；scripts/linux-verify.sh）

| 检查 | 结果 |
|---|---|
| Linux 默认测试（`xvfb-run cargo test --workspace`） | **1246 passed / 0 failed / 34 ignored**（188 个测试目标） |
| Linux x11 `check --all-targets` | exit 0 |
| Linux x11 `golden` | 36 passed / 0 failed |
| Linux `xvfb-run real_window` | 17 passed / 0 failed / 2 ignored |
| Linux x11 `clippy --all-targets -- -D warnings` | exit 0（补上 Windows 门禁看不到的 `#[cfg(unix)]` 代码） |
| Linux `render stroke_round_aa` / `extras sgr_reverse` / `ui+widgets` / `full workspace` | 全部 exit 0（脚本汇总 8/8） |

> 该表为 2026-11-22 **第三次**复测结果（TextArea 加入后重跑）：比第二次的 1035 多 19 条
> （18 条 TextArea 用例 + 1 条 doctest），因此 Linux 计数从 1035 变为 1054。
> 更早的首次 1031（Web ARIA 接线前）见 git 历史与本文件旧版本。

Linux 本轮证据：[`linux-verify-2026-11-27-collector.log`](evidence/cross-platform/linux-verify-2026-11-27-collector.log)。上一轮 [`linux-verify-2026-11-27-ci-fix.log`](evidence/cross-platform/linux-verify-2026-11-27-ci-fix.log)。上一轮 [`linux-verify-2026-11-27-atlas-overflow.log`](evidence/cross-platform/linux-verify-2026-11-27-atlas-overflow.log)。上一轮 [`linux-verify-2026-11-26-popup-hit.log`](evidence/cross-platform/linux-verify-2026-11-26-popup-hit.log)。上一轮 [`linux-verify-2026-11-26-instance-capacity.log`](evidence/cross-platform/linux-verify-2026-11-26-instance-capacity.log)。上一轮 [`linux-verify-2026-11-26-soft-stats.log`](evidence/cross-platform/linux-verify-2026-11-26-soft-stats.log)（RenderStats A6：Win32 `--force-soft` 软路径埋点；Linux 侧用于确认无回归）。
同日的更早记录（按时间倒序，均在同目录）：`linux-verify-2026-11-26-web-stats.log`（A5 Web）、`linux-verify-2026-11-26-x11-stats.log`（A4 X11）、`linux-verify-2026-11-26-x11-encode.log`（X11 编码器长度）、`linux-verify-2026-11-26-atlas.log`（图集帧边界）、`linux-verify-2026-11-26-x11-deadline.log`（读取层 deadline）、`linux-verify-2026-11-26-x11-read.log`（半包重组 / `bytes_after`）、`linux-verify-2026-11-26-x11.log`（resize）、`linux-verify-2026-11-23.log`、`linux-verify-2026-11-22-s6.log`、`linux-verify-2026-11-22.log`。
命令：`CARGO_FLAGS=--offline bash scripts/linux-verify.sh`（Ubuntu 22.04 / WSL2 + Xvfb）。
2026-11-20 的历史记录保留为 [`linux-full-2026-11-20.log`](evidence/cross-platform/linux-full-2026-11-20.log)、
[`linux-verify-2026-11-20.log`](evidence/cross-platform/linux-verify-2026-11-20.log)，不可视为本轮证据。更早的 CentOS 7.9 记录见 [`linux-verify-2026-11-19.log`](evidence/cross-platform/linux-verify-2026-11-19.log)。

## Clippy

`cargo clippy --workspace --all-targets --offline`：**0 error / 0 warning**

## 性能

详见 **[PERFORMANCE.md](PERFORMANCE.md)**（官方白皮书）：
脏区合并 −230×、布局 −47%、图集与实例提交经实测确认不是瓶颈。

## 浮层架构（最终形态）

- 唯一机制：`UiTree::popups` + `ctx.request_popup` / `request_modal_popup`
- `Layer` 枚举、`Widget::layer()`、`layer_present`、`deferred` 回放**已删除**
- 绘制顺序 = 树序（父先子后、兄弟按序）；"画在最上层"由 popups 提供
- 模态语义 = `Widget::is_modal()`（与绘制层级解耦）
- 闭合环：浮层内容 `is_open() == false` → 浮层层自动塌
