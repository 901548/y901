# 项目状态

- **HEAD**: `bf9968e` + 本轮（dispatch 分配基准）
- **tags**: v0.1.0 … v0.1.9
- **测试**: 818（基线 818；`#[ignore]` 45，含 ignore 863）
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
- 门禁六步：MSRV / 测试 **824**）/ clippy / 像素哈希 / `missing_docs` / `cargo audit`
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
- **824 测试全绿**（基线 824；`#[ignore]` **52**；`--include-ignored` **876**）
- **审查报告**：12 项可追踪并已处理；11 项原始报告未落盘（见上方更正）
- **SAFETY 100% 调用点覆盖**（d3d11 / font / window）+ vtable 契约 + context 三段式
- **文本缩放**（CPU + D3D）与**图片 mipmap LOD** 均有像素/解析真值验证
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
- 已验证结果：**11 passed / 0 failed / 2 ignored**
