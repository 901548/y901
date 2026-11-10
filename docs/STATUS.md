# 项目状态

- **HEAD**: `56e06a9`
- **tags**: v0.1.0 ~ **v0.1.17**
- **测试**: `passed=746` / `ignored=58` / `include_ignored=804`（唯一来源 `docs/evidence/numbers.txt`）
- **门禁**: `scripts/gate.ps1` 六步（MSRV / 测试计数 / clippy / 哈希 / missing_docs / audit）
- **clippy**: 0 警告（JSON 口径，先查退出码）
- **哈希基线**: `docs/evidence/hash-baseline.txt` = `0xDBB308DD1657EF9C` / `400x220`
- **SAFETY 覆盖：**96 个调用点 / 96 覆盖 / 0 缺口**（v0.1.14 起）；历史：v0.1.13 前 **96 / 93 / 3**

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

> **更正**：原审查报告未落盘。**实际可追踪的是 12 个编号**
> （#5 #7 #8 #9 #11 #12 #13 #14 #15 #18 #19 #20），且每个都有修复记录与提交。
> 其余 **11 项因原始报告未保存而无法逐项核对** —— 不编造编号。
> 下一次审计应重新跑全量审查，以新报告为准。
- 门禁六步：MSRV / 测试 **746**）/ clippy / 像素哈希 / `missing_docs` / `cargo audit`
- **SAFETY 覆盖：100%（调用点口径）** —— d3d11 / font / window 的每个 `unsafe` 调用点都有契约，
  另有 vtable 契约与 context 三段式两段集中说明

## 下一阶段候选（按 ROI 排序）

| 项 | 工作量 | 依赖 |
|---|---|---|
| 图片 mipmap（复用 `MipChain`） | 1~2h | 无 |
| 真机 Present 数字 | 1h | 物理显示器 + 独占全屏 |
| X11 真机验证 | 半天 | 真 Linux 机器 |
| 多窗口支持 | 3~5d | 无（大改动） |

## 图片 mipmap（步骤 1 完成：建链 + 坐标映射，选级强制 level=0）

**已做**（`y901-render::mip`）：
- `level_for_image(scale, max_level)`：只看缩放比、不看墨量（图片没有"重新光栅化"
  这个维度，每级只能由上一级 box 降采样得到）；取 floor，`≥ 1` / NaN / `≤ 0` 均回 0。
- `image_mip_chain(base, levels)`：复用 `MipChain` + `downsample_box`；1px 时提前停止。
- **6 + 4 = 10 个测试**：LOD 边界（1.0 / 0.75 / 0.5 / 0.25 / 非法值 / clamp）
  + box 降采样为四点平均、左黑右白保持分界、1px 不再生成、存储 **1.3125×**。

**未完成**（下一轮）：
- `soft.rs` 的 `draw_image` **还没接** mip 选级（仍只采 L0），因此 `s=0.5` 的 MAD 对比“超采样真值"尚未做。
- `Image` 结构体尚未加 `mips: Option<MipChain>`（动态/静态边界未咮定）。
- `Cover` 模式"实际绘制倍率"与 `scale` 可能不同，选级应用前者；尚未实现与测量。
- 存储成本已测（1.3125×），但尚未写进 `Image` 的文档。

## 图片 mipmap 步骤 1（本轮）

- `SoftBackend::upload_texture`：静态图片就地建 mip 链（**L1、L2**）；
  **L0 不另存一份** —— 它就是 `textures` 表里的原始字节，
  `level == 0` 路径因此与改造前**逐字节相同**。
- `upload_texture_dynamic`：动态图片（视频帧）只存 L0，不建 mip。
- `draw_image`：已算出 `scale_x = dst.w/src.w`、`scale_y = dst.h/src.h`，
  `draw_scale = min(...)`，并调 `level_for_image` 得到 `_computed_level`；
  但**本步骤强制 `level = 0`**，只证明"引入选级与坐标映射不改变渲染"。
- `src` 到级坐标的映射：`src / 2^level`（L0 时即 `src` 本身）；dst 始终是屏幕坐标。

**验证**：全量 816 全绿，像素哈希 **`0xDBB308DD1657EF9C` / 400×220 不变**。

**未做（步骤 2）**：打开 `level = level_for_image(...)`，跑 `s=0.5` 与**超采样独立真值**的 MAD。

**已知限制**：各向异性（`max(scale_x,scale_y) / min(...) > 2`）第一版**不特殊处理**，
只按 `min` 取级（宁可略模糊，不导致走样）。

## 图片 mipmap 步骤 2（本轮）

- **选级已打开**：`draw_image` 用 `level_for_image(min(scale_x, scale_y))` 选级。
- **s=1.0 哈希不变**：`0xDBB308DD1657EF9C` / 400×220（`s >= 1` 时返回 0，路径恒等）。
- **修了一个真 bug**：选中级后采样坐标仍用 L0 坐标 → 越出级图宽度，
  实测 MAD **0.3745**；按 `÷ 2^level` 映射后降为 **0.0000**。
- **但质量并未证明**：我构造的"4×4 超采样真值"采样点**落回 2×2 源像素**，
  等价于 box 降采样 —— MAD 恒为 0 是**度量退化**，不是质量好。
  该测试已标 `#[ignore]`，理由写在属性里。**下一步需重新构造独立真值**。

## 本轮教训

- **测试全绿 ≠ 功能存在**：步骤 1 里 `box_levels_rgba` 根本没被调用，816 测试全绿、
  编译通过，是 **clippy 的 `never used`** 把它抓出来的。
- **真值必须真的独立**：我以为"四点平均就是独立真值"，但采样点分布
  凑巧退化成了被测实现的同一个块 —— MAD=0 是"两个一模一样的东西比较"。

## 图片 mipmap 质量：**已验证**（本轮，解析真值）

用**闭式解**代替上一版的采样真值（后者退化成被测实现，MAD 恒为 0）。

| 测试 | 图案 | 闭式真值 | 实测 |
|---|---|---|---|
| 1 | 1px 棋盘（64²） | 2×2 块 2 黑 2 白 → **127.5** | 灰度范围 **127..127**，0/1024 越界 |
| 2 | 斜坡 `v = x*4` | 目标列 i = `(8i + 8i+4)/2` = **8i+2** | 最大偏差 **0** |

两个测试都带**变异对照**（改一个像素到 0 / 加 10 → 必须脱离容差）。

> 真值是**算出来的**，不是采样出来的：棋盘在任何 2×2 块都是 2 黑 2 白，
> 斜坡在任何 2 列都是 `8i` 与 `8i+4`，与采样点位置无关。

## 本轮教训（第三条）

- **真值必须与实现互斥**：上一版 4×4 采样的点落回 2×2 块，恰好等于 box 平均；
  这一版的闭式解**不依赖任何采样网格**，所以不可能退化。

## 已完成的里程碑

- **v0.1.0 ~ v0.1.12**
- **测试全绿**（数字见 `docs/evidence/numbers.txt`）
- **审查报告**：12 项可追踪并已处理；11 项原始报告未落盘（见上方更正）
- **SAFETY 100% 调用点覆盖**（d3d11 / font / window）+ vtable 契约 + context 三段式
- **文本缩放**（CPU + D3D）与**图片 mipmap LOD** 均有像素/解析真值验证
- **去 AI 完成**（Y901 + Helm-Y901 两仓）：
  - Y901 主仓：`7062463` 删除 `y901-agent` / `y901-mcp`，`28ffc88` 清 AI 痕迹；
  - Helm-Y901（独立仓）：删 `y901-agent` 依赖；`host.rs` 的 `Policy` 是死字段（无读者），
    删字段 + getter；`terminal_bridge.rs` 的 2 条测试改用本地 `dispatch_type` 辅助
    （`UiTree::set_focus` + `dispatch_to(UiEvent::Text)`，替代已删除的 `dispatch_request`）；
    `no_c_deps` 守卫恢复为"必须通过"（删跳过分支 + `deleted_local_dependency` + `DELETED_LOCAL_CRATES`）；
  - 顺带修：Helm 的 `Options` 缺 `fullscreen` 字段（Y901 侧加字段后未同步，
    被 `y901-agent` 缺失的编译错误掩盖），补了 3 处 `fullscreen: false`；
  - 验证：Helm `cargo test` 230 passed / 0 failed / 3 ignored；Y901 `no_c_deps` 2 passed ——
    **Helm 侧改动已应用，等待用户 commit**（Helm 工作区当前有 6 个文件待提交）。

- **`sync_interval` 语义漏洞已修**：`Gfx::present` 之前写死 0，交换链无法遵守 vsync；
  现为参数，默认路径 `render_inner(tree, 1)` 由结构化测试钉死

## 需要外部条件的待办

| 项 | 阻塞 |
|---|---|
| 独占全屏 Present | 手写 COM 槽位 + 物理显示器 |
| GPU timestamp query | 更深的驱动层改动 |
| X11 真机验证 | 真 Linux 机器 |
| macOS 后端 | Mac 或 CI runner |
| 多窗口支持 | 3~5 天 |

## 不阻塞的剩余小项

- 各向异性图片缩放（第一版不做，已知限制：`max/min > 2` 时仍按 `min` 取级）
- 动态图片 mipmap（`upload_texture_dynamic` 已绕过）
- dead_code 的 12 处"内部实现 / 编译见证"可进一步分类

**v0.1.16-dev**：`62b1c9f` 代码质量已补跑确认（`GATE OK (DEV: STALE AUDIT)`），
提交时未显式承认离线。

## 审计收口状态（v0.1.16-dev）

- **审查报告**：12 项可追踪且已处理；11 项原始报告未落盘 → 不可核对，不编造
- **miniz_oxide 双版本**：`0.8.9`（`png` 直接依赖）与 `0.9.1`（`flate2` → `png`）并存；
  两者都由 `png 0.17.16` 的**上游约束**引入，不是我们能单独统一的；需等 `png`/`flate2` 发布新版或联网 `cargo update`
- **serial 残留**：仍在 `Cargo.lock:225`，`cargo tree -i serial` 报 "did not match any packages"（不可达）；
  清除需联网 `cargo update`（本轮离线，未尝试硬删）
- **下一步**：重新跑全量审查（新报告为准），而不是围着一份丢失的旧报告转

## 审查报告状态（v0.1.16）

- **P0 = 0**
- **P1 = 1**：P1-03 x11 `EnvGuard`（需 WSL，Windows 宿主无法编译验证）
- **P2 = 1**：`serial` lock 残留（需联网）；其余已修（ignore 计数同步、Guide 编号）或接受（miniz_oxide 上游约束）
- **P3 = 4**（设计取舍 / 旧项失效）
- 数字由 `scripts/sync-doc-numbers.ps1` 生成，并由 gate 的 `-Check` 步骤拦截漂移
- 最新审查报告：`docs/audit-v0.1.15.md`

### v0.1.17 前的两项核实（本轮）

- **P2-01 `serial`**：**描述更正** —— 它是 `portable-pty 0.8.1` 的**必选依赖**，不是 lock 残留；
  `cargo update --workspace` 归零个包，`cargo generate-lockfile` 也不会删它。需升级 `portable-pty` 到 0.9 才能去掉（未做，需验证 API）。
- **P1-03 x11 EnvGuard**：代码已修，**双平台编译通过**；WSL 里 `Xvfb :99` 启动成功，
  单测 `handshake_reports_screen_and_visual` **通过**；但 **13 个用例并行跑时全部卡住**（>600s 超时），
  尚未定位是 `SERVER_LOCK` 互斥、基准耗时过长，还是某个用例等事件不返回。
  **因此不打 `v0.1.17`（运行时验证不完整）。**

## Linux / WSL 验证口径

- `cargo test --workspace` 在 WSL 上**不可行**：`y901-backend-win32` 在 Linux 下链接失败（预期，
  `cc` 报 `input_events` / `font_3c_pixels` / `lib test`）
- Linux 侧验证命令：`DISPLAY=:99 cargo test -p y901-backend-x11`（需 Xvfb :99 运行）
- 平台中立 crate（`y901-core` / `y901-ui` / `y901-render` / `y901-widgets`）可单独 `-p` 跑
- x11 全量：`DISPLAY=:99 cargo test -p y901-backend-x11 --test real_window -- --test-threads=1`
  （**必须 `--test-threads=1`**：用例共享一个 X 连接与环境变量）
- 已验证结果见 `docs/evidence/numbers.txt`（该文件是唯一口径）

## 数字检查的覆盖范围（已知限制）

`sync-doc-numbers.ps1 -Check` 当前只比对**加粗的三位数**（为避免把历史数字误报为漂移）。
后果：**非加粗写法的旧数字会逃过检查**（v0.1.17 审查的 P1-01 就是这个）。

下一步（下一轮）：让脚本比对含 `测试|passed|ignored` 的行里**所有**三位数，
历史数字用显式标记豁免；Guide 索引由脚本从正文生成。
本轮已把所有非加粗旧数字改为当前值，`-Check` 通过。

## 架构债

- **`plan_dirty_rects` 是后端无关的启发式，但两个后端的最优策略相反**（相差约 4 个数量级）：
  CPU 后端 `c_switch ≈ 10 像素`（逐块最优），D3D 后端每块脏区 = 一次全量实例化绘制（激进合并最优）。
  需要后端提供 `c_rect / c_pixel` 常数，由后端决定合并策略。
  当前 `MAX_DIRTY_RECTS = 16` 是**替 D3D 兜底**的临时方案 —— 它不是设计，是巧合地正确。
- `DirtyRegion::add` 的合并判据已改为"仅真实重叠"（`overlaps`），零重叠（含边/角相贴）不再合并。
  这是唯一不依赖后端常数即可确定净收益的改动。
- **跨仓迁移债：`F:\Helm-Y901` 仍依赖已删除的 `y901-agent`**（去 AI 阶段 1 物理删除）。
  - 后果：`crates/y901-app/tests/no_c_deps.rs` 的兄弟仓守卫 `sibling_repo_helm_has_no_c_dependencies`
    对它跑 `cargo tree` 会失败。
  - 现状：该守卫已改为**特定错误 + 显式打印 + 跳过**——仅当 stderr 同时满足
    "`failed to read`/`failed to load source for dependency`"、"路径指向本仓 `crates/`"、
    "命中的 crate 属于 `y901-agent`/`y901-mcp`" 三个条件时才跳过；其他任何 `cargo tree` 失败仍然红。
    跳过时打印：`[守卫] Helm-Y901 依赖树无法解析：依赖的本地 crate 已被删除（y901-agent）`。
  - TODO：**下一轮 demo 会话**迁移 `F:\Helm-Y901` 去除 agent/mcp 依赖后，
    删除 `no_c_deps.rs` 里的跳过分支（代码中已标注 `TODO(去 AI 阶段 3)`），恢复对 Helm 的完整零 C 依赖审计。


- **文档债：`docs/FRAMEWORK.md` 的"需要真机的测试"一节陈旧**（去 AI 阶段 4b 记录）。
  原文称"需要真机的测试只有 `rebuild_reuploads_atlas`"，实际 GPU / D3D11 依赖的 `#[ignore]` 有 **13** 个
  （`snapshot_harness` ×10 + `soft_matches_d3d_within_antialiasing_tolerance` + `pixel_statistics_and_png_baseline` + `rebuild_reuploads_atlas`）。
  需改为 13 个，或直接指向 `ACCEPTANCE.md` 的分类表。
- **文档债：`docs/ACCEPTANCE.md` 的"7 个 GPU harness"小节与分类表不一致**（去 AI 阶段 4b 记录）。
  `snapshot_harness` 现有 **10** 个 `#[ignore]`，小节只列 7 个；余下 3 个（`text_label_paints_visible_non_background_pixels`、
  `text_inside_scrollview_is_not_clipped_away`、`rebuild_of_cached_target_does_not_leave_black_areas`）是回归用例而非像素 harness。
  需先澄清"harness"定义，再决定是补进表还是把标题改成"7 个像素对拍 + 3 个回归"。

- **`sync-doc-numbers.ps1` 只保证数字对，不保证句子对**（已第三次改坏派生行）：
  它会改对 `769` 这个数字，但会把 `--include-ignored` 行的句子结构改坏
  （写成 `718` 应为 `769`，或出现 `499` 等怪数）。
  短期：每次 commit 前人工检查 `ACCEPTANCE` 的派生行。
  长期：删掉 `ACCEPTANCE` 的派生表述，只留一行 `769`，不再列 `718 + 53` 这种二次计算。

- **`\uXXXX` 转义泄漏到新文件**（已发生多次）：
  生成含中文的新文件时，转义层会把中文写成 `\u4e2d` 形式，
  而 Rust/PowerShell 不会将它当转义处理，于是文件里出现字面串。
  **固定动作**：凡是含中文的新文件，落盘后 grep 一次 `\\u[0-9a-f]{4}`，有命中就修。

- **Linux 验收脚本未覆盖 `y901-render` 的 Soft 描边用例**：
  `/root/Y901/linux_verify_cplan.sh` 只覆盖 x11 的 `check` / `golden` / `real_window`，
  未覆盖 `y901-render` 的 `stroke_round_aa` 单测（本轮由 demo 手工补跑才发现通过）。
  应把 `cargo test -p y901-render --test stroke_round_aa` 加进脚本，
  否则
后续 Linux 验收会“看不见”这条路径。

- **字体回退链未实现（P2）**：当前 NSimSun 只缺 U+2212，已用替身映射（→ `-`）解决。
  若未来需支持任意 Unicode（emoji、其他脚本、用户输入的特殊字符），
  需要实现完整字体回退：DirectWrite 用 `IDWriteFontFallback`，自研 TTF 需多字体 atlas + source 标记。
  当前不做，因为只有 1 个已知缺失字符。替身表在 `y901-render/src/font_fallback.rs`。

- **乘数型参数的默认值必须是 1.0，不能是 0.0**（opacity 阶段踩坑）：
  `Instance.params[3]` 原来是 0（因为从没人用），变成 opacity 乘数后，
  字形 quad（`mod.rs:486/496`）绕过 `push_instance` 导致 `params.w = 0`，
  shader 里 `* 0` 让**文字整体消失**（哈希 `0xDBB308DD1657EF9C → 0xA076219E89C1EDFD`）。
  **哈希门禁是唯一能抓住“语法正确但画错了”的手段**——结构断言发现不了，普通测试也发现不了。
  规则：凡是将来会变成“乘数”的字段，默认值从一开始就设 1.0，不要设 0.0。
  反向修复也要做：新建实例的统一入口（如 `push_instance`）必须**无法被绕过**，
  否则两套保护都会在下一个新构造点上失效。
- **Linux 同步清单**：改了 `y901-ui` 的 `RenderCtx` trait（如 push_opacity/pop_opacity）后，
  Linux 侧同步必须带上 `crates/y901-ui/src/widget.rs`，否则 y901-render 会误报编译回归。
  建议：改 trait 就全仓 tar 同步 + MD5 校验，不要单文件增量。
- **tar 同步后 mtime 保留**：tar 解压保留原 mtime，cargo 会误判 fresh，check 通过可能只是缓存命中。
  建议：tar 同步后 touch 关键源文件，或 cargo clean -p <crate> 后重编。

- **`-Check` 只读，不跑测试**：`sync-doc-numbers.ps1 -Check` 只验证 docs ↔ `numbers.txt` 一致。
  “台账 ↔ 代码”一致由 gate 的 `passed vs baseline` 硬比对保证。
  **约束**：不要把任何测试测量塞回 `-Check`，否则 gate 会重新变成“每次跑都赌博”。

- **完整 sync 是唯一测量源**：只有无 `-Check` 的完整 sync 会跑测试并写 `numbers.txt`。
  **注意**：`--include-ignored` 在并行负载下仍可能被 allocation-counter 污染（实测漂到 560）。
  完整 sync 的守卫：**测试全部通过（exit 0）才写 `include_ignored`**，否则写 `n/a`。
  如果 `numbers.txt` 的 `include_ignored` 是 `n/a`，说明上次完整 sync 时有测试失败 —— 先修测试再跑完整 sync。

- **自动化验证脚本里不要出现非 ASCII 字符**（Win32 Esc 验证踩坑）：
  `FindWindow("Y901 · hello")` 在无 BOM 的 UTF-8 脚本里被 PowerShell 5.1 按 ANSI 解码，
  `·` 变成乱码，按标题找不到窗口 → `hwnd = 0` → PostMessage 发到空句柄 → **假失败**。
  修法不是"记得写 BOM"，而是**改成按 PID 找窗口**，脚本对编码完全不敏感。
  同类教训：改 `.ps1` / `.bat` 必须保留原编码 BOM（`check-test-inventory` 那次）。

- **改 `#[cfg(unix)]` 下的文件必须同时确认"引用闭合"**（第三次盲区返工）：
  Windows 的 `cargo check` 对 `#[cfg(unix)]` 代码是"全部通过"的假象。
  三次返工：
  1. X11 `RenderOutcome` 未导入；
  2. `create` 签名改了，`runtime.rs` + `real_window.rs` 调用点没改；
  3. 删 diag 时连带删掉 `WANT_EXIT_FS` 静态声明 + `create` 的
     `fullscreen_atoms` 调用漏参数。
  **固定动作**：改 `#[cfg(unix)]` 文件后，用 `grep` 列出该文件里所有 `fn X(` 的定义与调用，
  逐个核对参数个数；或让 demo 在每次改动后立即跑 `cargo check -p y901-backend-x11`。
  另：删诊断代码时**按行/按关键字删块会误伤上下文**（实际把绝对多行的
  `static` 与紧邻注释一起删掉）；应先 `cargo check` 再删，或按 AST/锚点删整块。

- **X11 读包：只有 reply（byte0=1）和 GenericEvent（byte0=35）带附加数据**：
  Error（byte0=0）的 byte4-5 是 `bad_value`，Event（byte0>=2 且 !=35）的 byte4 起是事件体，
  **都不是长度字段**。无条件按 byte4-5 当长度 drain 会多读字节、流错位。
  本轮踩坑：`sync()` 的 drain 放在 `match` 之前、对所有包执行 →
  **修前 5/20 失败变成修坏后 12/20**，real_window 从 11 passed 变 6 failed。
  修正：drain 移进 reply 分支，事件分支只对 35 drain；`read_reply_packet` 同样修。
  另：`read_exact` 的 `WouldBlock`/`TimedOut` 不应直接当致命错误——
  改为 200ms 轮询 + 5s deadline，EAGAIN 时继续等而不是报错。
  **教训：协议细节没确认就动手，会从“修一个 bug”变成“引入一个回归”。**

- **“写了但没跑过”是一个反复出现的模式**（X11 测试基建三轮的教训）：
  1. 加了 `[SKIP]` 打印，但 gate **捕获输出**，通过的测试 stderr 不进日志 → **不可追溯**；
  2. 加了 `record_skip`，但**相对路径在包根 CWD 下写不进去 + 静默吞错** → **完全失效**；
  3. 修了路径，但 `[WARN]` 分支（写失败时 eprintln）**至今从未被执行**。
  共同点：代码路径写出来之后，**没有确认它"真的跑过一次"**。
  **固定动作**：新增任何"防御性 / 错误处理 / 日志"代码时，先问一句**"这条路径怎么触发"**：
  能触发的，写一条测试或用环境变量强制触发（如 `DISPLAY=:4321` 触发连接失败）；
  不能触发的，在注释里写明**"未覆盖，触发条件是什么"**，并在 commit message 里标注。

- **改动 `#[cfg(unix)]` 文件的 `use` 清单或引入新类型，必须在 Linux 上跑
  `cargo check -p y901-backend-x11 --all-targets` 并看到 exit 0，才算完成**：
  Windows 的 `cargo check` 对该 crate 是"全部通过"的假象（`#[cfg(unix)]` 整体为空）。
  三次返工都源于此：
  1. X11 `RenderOutcome` 未导入；
  2. `X11Window::create` 签名改了，`runtime.rs` + `real_window.rs` 调用点没改；
  3. `CreateArgs` 改了 `real_window.rs` 的构造，但漏了 `use CreateArgs`。
  **结构体参数能保证"字段漏写会红"（E0560/E0063），但保证不了"名字没 import 会红"**。
  **固定动作**：改 `#[cfg(unix)]` 文件后，**立即**让 demo 跑 `cargo check --all-targets`，
  不要等下次整体验证。

- **`docs/evidence/test-inventory.txt` 是 **Windows 平台**的测试指纹**：
  `#![cfg(target_os = "linux")]` 或 `#[cfg(unix)]` 下的测试二进制在 Windows 上为空，
  **不进入指纹**。例如 `multi_window` 的 2 条测试（1 普通 + 1 ignored）存在于 Linux 侧，
  但不在 Windows 的 inventory 里，也不进 `numbers.txt`（所以 Windows 侧 ignored=58、Linux 侧=59）。
  这是**平台固有局限**，不是计数错误；要覆盖 Linux-only 测试需跨平台指纹机制，单独立项。

- **校验通过 ≠ 校验有效**（`sync-doc-numbers.ps1` 的 `-Check` 空转）：
  `-Check` 分支里 `$docFiles` 定义在 `if (-not $Check)` 内，`-Check` 模式下它是 `$null`，
  文档校验循环遍历空集合 → **一条都没查** → 恒返回 `ok`。
  这意味着 gate 的 `doc numbers match` 一步在 `-Check` 路径下**长期空转**，
  它只比对了 `numbers.txt` 自身的格式，从未校验任何文档。
  与"写了但没跑过"同构：**`ok` 曾经是"没查任何东西的 ok"**。
  **教训**：任何"校验 / 检查"代码写完后，必须**故意制造一个违规**确认它真的会报错。
  如果制造违规后仍 `ok`，那校验本身是坏的，不是被校验对象是对的。

- **VM 没有 PowerShell，`sync-doc-numbers.ps1` 在 Windows 侧验证即可**：
  该脚本服务于 Windows 侧 gate，不需要 Linux 环境。
  Linux 侧真正需要的验证是测试计数（`multi_window` / `real_window` / `golden`），已有独立证据。

- **`poll(0)` 在 Linux 下 `set_read_timeout(Some(0ms))` 返回 EINVAL**（既有 bug）：
  这是 196% CPU 空转的真正来源，不是 EOF。旧的 `Err(_) => return out` 把它当"没事件"吞掉，
  于是 `run_loop` 在"上一帧有工作"（`was_busy` → `wait_ms = 0`）时每轮都空转。
  修法：`timeout_ms == 0` 用 `set_nonblocking(true)`，非 0 用 `set_read_timeout(ms)`；
  函数末尾（**含所有 `?` 返回路径**）恢复阻塞模式 —— 拆成"外层设模式 + 内层干活"才能覆盖完整。
  `EINTR` 在 `poll_events` 里重试，不冒泡。
  **教训**：`poll_events(0)` 这条路径**从来就没工作过** ——
  **"能编译、能跑单窗口"不代表这条路径被验证过**。

- **`WM_PROTOCOLS = WM_DELETE_WINDOW` 必须在 `MapWindow` 之前设置**（ICCCM）：
  WM 在 reparent 窗口时就读取 `WM_PROTOCOLS`，**映射后再设就晚了**。
  不设的后果：WM 走 `XKillClient` 直接杀客户端，客户端只能从 EOF 得知窗口没了。
  修法：intern `WM_PROTOCOLS` / `WM_DELETE_WINDOW`，`ChangeProperty` 到窗口
  （`format=32`, `type=ATOM(4)`），并且**在 `MapWindow` 之前**。
  验证方法：外部关窗后，日志应出现"X11 会话结束"（走 ClientMessage 干净路径），
  **不应**出现"X11 连接已不可用：服务端断开连接"（EOF 兜底）。

- **`WM_NCDESTROY` 里 `PostQuitMessage` 的语义是"投递给调用线程"**：
  每窗口一线程的 `run_many` 下，`WM_NCDESTROY` 应**无条件** `PostQuitMessage(0)` ——
  它只结束**当前线程**的 `run_loop`，不影响其他窗口线程。
  之前的 `if before == 1 { PostQuitMessage(0) }` 会让**先关的窗口线程永久阻塞**（进程不退）。
  `WINDOW_COUNT` 在"每窗口一线程"下**不参与退出决策**，仅用于诊断。

- **`Y901_SIMULATE_RENDER_FAIL` 是异常路径的测试开关**：
  设置后 `WindowBackend::render` 立即返回 `Err`，用于验证 `Window::Drop` 的异常路径兜底
  （正常路径下 HWND 已被 `WM_NCDESTROY` 销毁，Drop 时 `IsWindow` 返回 0，不会重复销毁）。
  这是"把不可测变成可测"的一次投入：P2-2 的异常路径此前**零运行覆盖**，
  加这个开关后真跑到了 Drop 兜底分支（`[diag] Drop 兜底 DestroyWindow 结果：1`）。
  用 `OnceLock` 缓存，**零热路径成本**。与 `Y901_FULLSCREEN` / `Y901_FORCE_SOFT` 同类。
