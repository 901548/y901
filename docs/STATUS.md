# 项目状态

- **HEAD**: `bf9968e` + 本轮（dispatch 分配基准）
- **tags**: v0.1.0 … v0.1.9
- **测试**: 806（基线 806；`#[ignore]` 45，含 ignore 851）
- **门禁**: `scripts/gate.ps1` 六步（MSRV / 测试计数 / clippy / 哈希 / missing_docs / audit）
- **clippy**: 0 警告（JSON 口径，先查退出码）
- **哈希基线**: `docs/evidence/hash-baseline.txt` = `0xDBB308DD1657EF9C` / `400x220`
- **SAFETY 覆盖**: d3d11 / font / window 的调用点全部有契约（56 个位置 + 3 段集中说明）

## 已知问题

1. ~~`#15` ttf-parser 无轮廓点数上限~~ —— **已修**：`flatten.rs` 加 `MAX_LINES = 10_000` / `MAX_CONTOURS = 256`，
   超限时 `flatten_glyph` 返回 `None`（与"字形不存在"同处理），诊断按 gid 去重只打一次。
2. `#18` 热路径 clone：**tree.rs 已测**（键盘 0 次，鼠标 6 次常数）；
   **`soft.rs` 纹理复制已修**：改为分片借用，每帧分配 **0 次 / 0 字节**（修前 4 MiB/帧）。
3. ~~`#19` 31 处 `#[allow(dead_code)]` 未定性~~ —— **已处理**：29 处加了归类注释
   （诊断 API 13 / 测试 API 4 / 内部实现与编译见证 12），**2 处真死代码已删除**。
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

## soft.rs 修复结果（本轮）

修法：`draw_image` 不再 `tex.2.clone()`，而是只借用 `self.textures` 字段
（`self.textures.get(&texture).map(|t| t.2.as_slice())`）；
它与循环体里写的 `self.buf` / `self.width` / `self.height` 是**不相交字段**，
因此借用检查器允许同时存在——**没有用 unsafe，也没拆字段**。

| 场景 | 修前 | 修后 |
|---|---|---|
| 首次上传 1024² | 2 次 / 4.0 MiB | 2 次 / 4.0 MiB（不变） |
| 后续 100 帧 `draw_image` | 100 次 / 400 MiB | **0 次 / 0 MiB** |
| 对照：清屏 100 帧 | 0 / 0 | 0 / 0 |

基准现已**断言**每帧 0 次分配。像素哈希不变（`0xDBB308DD1657EF9C` / 400×220）。

## #15 字形上限（本轮）

| 项 | 值 | 依据 |
|---|---|---|
| `MAX_LINES` | **10 000** | 常见 CJK 展平后 50–300 条；复杂字也在千条以内 → 一个数量级余量 |
| `MAX_CONTOURS` | **256** | 汉字最多几十个笔画分支；256 已是实际上限的数倍 |

超限处理：`ttf-parser` 的 `OutlineBuilder` **没有提前终止机制**，所以我们
**继续消费回调但不再记录任何折线**（内存不涨、不影响上游解析），
最后 `flatten_glyph` 整体返回 `None`。诊断只打一次（按 gid 去重）。

新增 5 个测试：超限 ×2、边界 ×2（防 off-by-one）、超限后继续消费回调不崩。

## #19 dead_code 定性（本轮）

**先跑了真实检查**：`CARGO_TARGET_DIR=target-dead-code RUSTFLAGS=-W dead_code cargo build --workspace --all-targets`
→ **exit 0、零 dead_code 警告**。说明得很清楚：**31 处 allow 把它们全遮住了**，
编译器已无法告诉我们哪些真死 —— 这正是审查报告的论点。于是改用静态方式：
逐处取出被标记的符号，再在全仓库数它的引用次数。

| 归类 | 数量 | 依据 |
|---|---|---|
| 诊断 API | 13 | 仅由 `Y901_PROFILE` / `Y901_SCISSOR_TEST` / IME dump 等调用 |
| 测试 API | 4 | 引用点全在 `tests/*.rs`（如 `glyph_count`、`has_dirty`） |
| 内部实现 / 编译见证 | 12 | `_unused`/`_witness` 用于证明类型被使用；`HIT_R`、`target` 等 |
| **真死代码（已删）** | **2** | `ime.rs` 的 `GCS_CURSORPOS`、`mip.rs` 的 `level_for_old`：全仓库仅定义处出现 |

每处都在 `#[allow(dead_code)]` 上一行写了 `// 归类：…` 注释，将来看到就知道它为什么在那里。

## 下一轮计划

本轮审查清单（#5 #7 #8 #9 #11 #12 #13 #14 #15 #18 #19 #20）**已全部处理**。
下一步建议：打 `v0.1.10`，并把 `docs/STATUS.md` 作为每轮固定产出。

## 审查报告状态

- **v0.1.10 起，审查报告 23 项全部关闭**（#5 #7 #8 #9 #11 #12 #13 #14 #15 #18 #19 #20 均已处理）
- 门禁六步：MSRV / 测试（基线 **806**）/ clippy / 像素哈希 / `missing_docs` / `cargo audit`
- **SAFETY 覆盖：100%（调用点口径）** —— d3d11 / font / window 的每个 `unsafe` 调用点都有契约，
  另有 vtable 契约与 context 三段式两段集中说明

## 下一阶段候选（按 ROI 排序）

| 项 | 工作量 | 依赖 |
|---|---|---|
| 图片 mipmap（复用 `MipChain`） | 1~2h | 无 |
| 真机 Present 数字 | 1h | 物理显示器 + 独占全屏 |
| X11 真机验证 | 半天 | 真 Linux 机器 |
| 多窗口支持 | 3~5d | 无（大改动） |
