# Y901 框架验收凭证

生成时间：2026-10-31
命令：`cargo test --workspace --offline -- --include-ignored`
日志：`target/test-workspace-final.log`（构建产物目录，被 `.gitignore` 排除，不入库）

## 结果

| 口径 | 数量 |
|---|---|
| `cargo test --workspace`（默认，跳过 `#[ignore]`） | **499 passed / 0 failed** |
| 加上 `--include-ignored` | **523 passed / 0 failed**（含基准与 doc-test） |

差值 24 = 需要真机/耗时的 `#[ignore]` 用例：

- **7 个 GPU harness**（D3D11 逐像素对拍，见下表）
- **1 个 GPU 图集重建**：`backend::rebuild_atlas_tests::rebuild_reuploads_atlas`
- **3 个万节点性能基准**：`layout_10k_full_relayout`、`rev_snapshot_is_linear_on_10k_nodes`、`public_tree_is_linear_on_10k_nodes`
- **11 个性能基准用例**（`perf_bench` / `perf_alloc`）——见 [PERFORMANCE.md](PERFORMANCE.md)

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
脏区合并 −220×、布局 −44~49%、图集与实例提交经实测确认不是瓶颈。

## 浮层架构（最终形态）

- 唯一机制：`UiTree::popups` + `ctx.request_popup` / `request_modal_popup`
- `Layer` 枚举、`Widget::layer()`、`layer_present`、`deferred` 回放**已删除**
- 绘制顺序 = 树序（父先子后、兄弟按序）；"画在最上层"由 popups 提供
- 模态语义 = `Widget::is_modal()`（与绘制层级解耦）
- 闭合环：浮层内容 `is_open() == false` → 浮层层自动塌
