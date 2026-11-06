# Y901 框架验收凭证

生成时间：2026-11-02
命令：`cargo test --workspace -- --include-ignored`
日志：`docs/evidence/correctness/final-760-include-ignored.log`（**历史记录**，文件名沿用旧口径 760）。
当前口径以 `scripts/gate.ps1` 为准：**712 passed / 0 failed**（baseline 见 `docs/evidence/test-baseline.txt`）。

## 结果

| 口径 | 数量 |
|---|---|
| `cargo test --workspace`（默认，跳过 `#[ignore]`） | **712 passed / 0 failed** |
| 加上 `--include-ignored` | **763 passed / 0 failed**（含默认测试集与 `#[ignore]` 测试） |

含 ignore 的总数是 **763**（默认测试集 + `#[ignore]` 测试的全量结果），不是单独跑的测试集。

差值 **51**（763 − 712）= `#[ignore]` 用例。源码里共有 **55** 处 `#[ignore]`，与 51 的差额有两个来源：

- **2 处在 example**（`y901-ui/examples/hash_bench.rs`）：`cargo test` 不执行 example 里的测试；
- **2 处在 `y901-backend-x11`**：该 crate 在 Windows 上 suite 存在但 `running 0 tests`（整 crate 被 cfg 掉）。

| 类别 | 数量 | 用例 |
|---|---|---|
| GPU / D3D11 依赖 | **13** | `snapshot_harness` ×10、`soft_matches_d3d_within_antialiasing_tolerance`、`pixel_statistics_and_png_baseline`、`rebuild_reuploads_atlas` |
| DirectWrite 对拍 | **2** | `compare_with_directwrite_l1_l2`、`l2_best_shift_diagnostic` |
| 性能基准 | **30** | ui 12（`perf_bench` ×6、`perf_dispatch_alloc` ×3、`layout_bench`、`perf_alloc`、`rev_snapshot`）、win32 8（`perf_bench` ×3、`present_bench`、`breakdown_rasterize_vs_atlas_insert`、`cold_start_500_cjk_glyphs`、`all_phases_reference_cost`、`soft_fill_rate_1280x800`）、render 10（`cjk_cold_start` ×3、`mip_latin` ×3、`perf_texture_alloc` ×3、`which_method_is_closer_to_supersampled_truth`） |
| 调试探针 | **5** | `debug_print_glyphs`、`debug_crossings_for_a`、`debug_print_cjk_frames`、`occupancy_matches_glyph_count_sanity`、`probe_orientation_and_channels` |
| 压力测试 | **1** | `forced_eviction_thrash_measurement`（图集 eviction 抖动） |
| **合计** | **51** | |

## 7 个 GPU harness（真机 D3D11，全部 ok）

| 用例 | 验证内容 |
|---|---|
| `snapshot_harness_is_deterministic` | 同一棵树两次绘制哈希相等 |
| `gradient_matches_math_at_45_degrees` | 45° 渐变逐像素对齐解析解（1106 点，≤0.0050） |
| `round_rect_sdf_matches_math` | 圆角矩形 SDF（内部 22840 / 边界带 708） |
| `round_stroke_draws_ring_only` | 描边只在轮廓带（1430），内部不被填充（3494） |
| `gradient_round_rect_matches_math` | 圆角渐变叠加（2652 点，≤0.0089 + 四角裁切） |
| `invalidate_rect_partial_equals_full` | 局部标脏重绘 == 整窗重绘（逐字节 0 差异） |
| `two_disjoint_dirty_rects_match_full_redraw` | 两块不相邻脏区 == 整窗重绘（逐字节 0 差异） |

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
