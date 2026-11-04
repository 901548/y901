# 项目状态

- **HEAD**: `bf9968e` + 本轮（dispatch 分配基准）
- **tags**: v0.1.0 … v0.1.9
- **测试**: 801（基线 801；`#[ignore]` 45，含 ignore 846）
- **门禁**: `scripts/gate.ps1` 六步（MSRV / 测试计数 / clippy / 哈希 / missing_docs / audit）
- **clippy**: 0 警告（JSON 口径，先查退出码）
- **哈希基线**: `docs/evidence/hash-baseline.txt` = `0xDBB308DD1657EF9C` / `400x220`
- **SAFETY 覆盖**: d3d11 / font / window 的调用点全部有契约（56 个位置 + 3 段集中说明）

## 已知问题

1. `#15` **ttf-parser 无轮廓点数上限** —— 恶意/损坏字体可让展平器吃满 CPU/内存。未做。
2. `#18` 热路径 clone：**tree.rs 已测**（键盘 0 次，鼠标 6 次常数）；
   **`soft.rs:461` 的 `tex.2.clone()` 已测：每次 `draw_image` 复制整张纹理（`1024²` → 4 MiB/帧）——**待修**。
3. `#19` **31 处 `#[allow(dead_code)]`** 未定性（诊断 API / 测试 API / 真死代码）。
4. `cargo update` 会顺带升级 3 个依赖且需联网，`Cargo.lock` 里 `serial` 残留未清（已记录在 `DEPENDENCIES.md`）。
5. audit 依赖联网刷新指南库；离线时结论带 `STALE AUDIT` 标记，**不算发布门禁**。

## #18 tree.rs 测量结果（本轮）

基准：`crates/y901-ui/tests/perf_dispatch_alloc.rs`（计数分配器，**必须 `--test-threads=1`**）

| 场景（100 节点树） | 每帧分配 |
|---|---|
| 键盘事件 dispatch | **0.00** |
| 同位置 MouseMove（目标不变） | **6.00** |
| 空白处 MouseMove（无命中） | **6.00** |

- 三次运行**完全一致**（单线程后）。
- 键盘路径**零分配**：`focus.clone()` / `id.to_string()` 都不在键盘派发路径上。
- 鼠标路径的 6 次分配与"是否命中控件"无关（空白处同样 6 次）→ 属于**固定开销**，
  不是随节点数增长的 per-node 克隆。

**结论**：

| 候选 | 占比 | 判定 |
|---|---|---|
| `focus.clone()` | 6 次中至多 1 个 `String`（≤ 1/6 ≈ 17%），且**只在鼠标路径** | **不改**（见理由） |
| `id.to_string()` | 键盘路径 **0**；鼠标路径与命中无关 → 不在热路径 | **不改** |
| `path.clone()` | 键盘/鼠标均未触发（只在 focus_next / modal 等结构操作里） | **不改** |

理由：6 次/帧是**常数**，与树规模无关（100 节点与 10k 节点的整轮布局也仅 1004 次分配）。
把 `focus.clone()` 换成借用需要改 `EventCtx` 的生命周期，收益是每帧 1 次小分配 ——
**远低于"占比 > 10% 且可确认无回归"的改动门槛**。

## #18 soft.rs 测量结果（本轮）

基准：`crates/y901-render/tests/perf_texture_alloc.rs`（计数分配器，**必须 `--test-threads=1`**）

| 场景 | 分配 |
|---|---|
| 首次上传 1024² | **2 次 / 4.0 MiB** |
| 后续 100 帧 `draw_image` | **100 次 / 400.0 MiB**（**每帧 4.0 MiB**） |
| 对照：清屏 100 帧 | 0 次 / 0 MiB |

**结论：严重，必须改。** clone 位置 `crates/y901-render/src/soft.rs:461`
（`let tex_bytes = tex.2.clone();`）在**每次 `draw_image` 都执行**。

## 下一轮计划

修 `soft.rs:461` 的整张纹理复制（用分片借用替代 clone），并用本基准验证"每帧 0 次分配"。
