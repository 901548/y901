# 路线图：从 0.2.x 到“可交付”

> 日期：2026-11-21 · 依据：对 7 项差距的代码级核对（复杂文本 / 无障碍 / bus factor /
> API 稳定 / 生态 / 入门体验 / Web 天花板）。本文是**方案**，不是已完成工作的声明。

## 0. 约束与原则

任何方案先过这五条，否则不立项：

1. **零 C/C++ 依赖**（`crates/y901-shell/tests/no_c_deps.rs` 守卫）：shaping、无障碍桥都只能选纯 Rust 实现。
2. **单维护者 + AI 辅助**：每周可投入的高质量工程时间有限，方案必须能按“单人周”切分，
   每一片都能独立提交、独立回滚。
3. **API 0.x**：破坏性变更集中在 minor（0.3.0），patch 只修 bug；已发布版本号不复用。
4. **保留等宽格契约**：终端/Grid 路径不能为了“世界文字”而改坏；复杂排版走**并行管线**。
5. **门禁是唯一事实来源**：每个阶段都要能进 `gate.ps1` / `linux-verify.sh`，不靠人肉记忆。

## 1. 七项差距的处置结论

| # | 差距 | 性质 | 处置 | 目标版本 |
|---|---|---|---|---|
| 1 | 无 shaping / 可变字体 / bidi | 架构级 | **新增并行文本管线**，终端/Grid 不动 | 0.3.0（MVP）→ 0.4.0 |
| 2 | 无障碍无 OS 桥 | 合规级 | 语义模型扩展 → AccessKit 三平台适配 | 0.3.0（Win）→ 0.4.0 |
| 3 | bus factor = 1 + 平台税 | 组织级 | **降级平台承诺**（Tier 制）+ 自动化 + runbook | 立即（0.2.x 文档） |
| 4 | semver 纪律未受考验 | 流程级 | semver-checks + 公开 API 快照 + 下游冒烟 + `!` 校验 | 0.2.7 |
| 5 | 生态/组件缺口 | 产品级 | Table v2 / MenuBar / NumberInput；**不做插件 ABI** | 0.3.0 |
| 6 | 入门是内部视角 | 展示级 | 英文 README + 5 分钟教程 + docs.rs 引导 + 截图 | 0.2.7 |
| 7 | Web 后端天花板 | 定位级 | **明确降级为 Tier 3“演示/触达”**；升级另立项 | 0.2.7 文档 |

## 2. 平台分层（先止损，再谈扩张）

| Tier | 平台 | 承诺 | CI |
|---|---|---|---|
| **T1** | Windows / D3D11 | 完整验证、发布阻断、a11y 优先 | `gate.ps1` 全量 |
| **T2** | Linux / X11（Xvfb + 真机抽检） | 尽力支持，回归可见 | `linux-verify.sh` |
| **T3** | Web / wasm32 | **演示与触达**：单 canvas、整帧、IME 尽力 | 编译 + 冒烟 |
| 出范围 | macOS、Wayland | 不承诺；保留 XWayland 兜底 | 无 |

收益：把“写完即腐烂”的维护面从 5 个平台压到 2.5 个；README/CI 与代码承诺一致。
代价：明确劝退部分用户——但比“看起来支持、实际没人维护”更诚实。

## 3. 阶段计划（按单人 + AI 辅助估）

| 阶段 | 内容 | 工期 | 验收 |
|---|---|---|---|
| **S0 定位与入口** | 英文 README/QUICKSTART、Tier 矩阵、semver 守卫、下游冒烟、Web/T1 边界 | 1–2 周 | 英文入口可跑通；CI 新增 semver/consumer 两项 |
| **S1 组件与语义地基** | Table v2（排序/虚拟滚动/键盘）、MenuBar、NumberInput；语义模型扩展（name/description/value 范围/关系/live region） | 3–4 周 | 新组件有像素/交互测试；语义快照覆盖所有角色 |
| **S2 文本管线（关键路径）** | 分层文本布局 + rustybuzz + bidi + 脚本切分 + 回退链；Label/RichText 先接入；TextInput 后接入 | 6–8 周 | Unicode Bidi 一致性用例 + HarfBuzz 参考 fixture 逐字形比对；像素基线更新 |
| **S3 无障碍桥** | AccessKit 树 → Windows UIA、Linux AT-SPI；Web DOM 镜像 | 4–6 周 | Narrator/Orca 手动验收清单 + AccessKit 树快照测试 |
| **S4 收敛 0.3.0** | 批量破坏性变更（`#[non_exhaustive]`、measure 约束、文本 API）、变量字体、Web 渲染升级决策 | 3–4 周 | 迁移指南 + semver-checks 基线切换 |

总计约 **17–24 单人周**（4–6 个月）。若砍掉 Web 升级与变量字体，可压到 **14 周**左右。

## 4. 专题方案

### 4.1 复杂文本：并行管线，不改 cell 契约

**为什么不能“补一个 HarfBuzz”**：当前文本是“逐字符查 cmap + 等宽 cell 推进”，
而 shaping 需要**整段**上下文（脚本、方向、连字、字距）。两者是不同抽象，硬合会把终端路径也拖下水。

**分层（自下而上）**：

1. `y901-render` 新增 `text` 模块：
   - `GlyphKey { face, glyph_id }` 取代字形图集的 `char` 键（**当前 atlas 用 `HashMap<char, Slot>`，
     是 shaping 的隐藏阻塞点**：同一字符在不同上下文会映射到不同字形）。
   - `TextShaper` trait：`shape(&Paragraph) -> ShapedParagraph`，默认实现仍走 cell 路径。
   - `RustybuzzShaper`（`rustybuzz 0.20` + `unicode-bidi 0.3` + `unicode-script 0.5`，全部纯 Rust，
     MIT/Apache，MSRV 均低于 1.92；`rustybuzz` 依赖的 `ttf-parser 0.25` 与现有钉版**同版本**）。
2. `y901-ui` 文本布局：
   - 段落切分（bidi levels）→ 脚本 itemization → 字体回退选面 → run shaping → 行断点（UAX #14 简版）。
   - 布局约束：新增 `Widget::measure_with(avail_width)`（默认委托旧 `measure`），文本控件覆盖；
     **这是 0.3.0 的破坏性变更**，集中在 S4 一次做。
3. 控件接入顺序：`TextLabel`/`Button` → `RichText` → `TextInput`（光标/选区按**cluster** 映射，
   逻辑偏移 ↔ 视觉位置双向表；首版可整行重 shape，编辑性能后置）。
4. 字体回退：`FontStack` 有序面表 + 逐字符 cmap 覆盖检查；平台默认链
   （Win32: 系统 UI 字体；X11: fontconfig 等价物的纯 Rust 实现或自带配置）。
5. 变量字体：等 `ttf-parser` 暴露 variation 坐标后在 S4 接（先只做 weight/width 两轴）。

**性能**：shaping 结果按 `(face, size, features, text hash)` 缓存；图集命中率不变；
cell 路径零改动、零回归。

**测试**：
- bidi：Unicode `BidiTest.txt` / `BidiCharacterTest.txt` 子集；
- shaping：提交 HarfBuzz 参考输出 fixture（JSON，离线生成、随仓库分发），逐 glyph id/advance/offset 比对；
- 回归：现有像素哈希基线整体重刷一次，并在 PR 中人工确认“变化只发生在文本控件”。

**风险**：布局 API 变更波及所有控件（S4 集中处理）；shaping 放大图集压力（按 glyph 键后条目更多，
需要 LRU/压缩策略）；字体缺失时行为要可预测（回退链 + `.notdef` 可见告警）。

### 4.2 无障碍：先把语义模型做对，再接桥

**现状**：`Semantics` 是给外部工具/AI 用的（label/value/role/actions/state），不是 a11y 模型；
缺 accessible name 计算、description、value 范围/步进、selected/expanded/checked 的规范表达、
labelled-by/described-by 关系、live region、屏幕坐标。

**步骤**：

1. **模型扩展（S1，2 周）**：`Semantics` 增补上述字段；`Role` 补齐 AccessKit 角色映射表；
   `UiTree` 输出**带父子和顺序**的快照（现有语义 patch 是扁平 id，需要树形投影）。
2. **AccessKit 适配（S3）**：
   - Windows：`accesskit` + `accesskit_windows`（依赖 `windows` crate，纯绑定、不编译 C；
     MSRV 1.87 < 1.92）挂到 HWND，响应 `WM_GETOBJECT`，每帧语义变化发 `TreeUpdate`；
   - Linux：`accesskit_unix`（`zbus` 纯 Rust D-Bus；项目已有自研 D-Bus 客户端，注意**两套 D-Bus 栈共存**的体积/线程成本）；
   - Web：canvas 无法承载 a11y 树，做**隐藏 DOM 镜像**（`role`/`aria-*` + 焦点代理），
     与语义树共用同一份数据；这条与 Tier 3 定位一致，工作量单独评估。
3. **合规边界写清**：桥只解决“屏幕阅读器能读到/能操作”；WCAG 还要求键盘可达、对比度、缩放、
   错误提示。README 给“已实现/未实现”清单，不做 VPAT 承诺。

**验收**：AccessKit 树 JSON 快照测试（跨平台）；Windows Narrator + Linux Orca 手动清单；
键盘 only 走查（Tab 顺序、焦点可见、对话框陷阱）。

### 4.3 API 稳定：用工具把纪律变成门禁

1. **CI 加 `cargo-semver-checks`**，基线固定为**已发布版本 0.2.5**（crates.io 上只有它）；
   签名级破坏直接红。
2. **公开 API 快照**：`cargo public-api` 生成文本快照，PR diff 必须人工确认。
3. **下游冒烟**：独立 job 用 `cargo new` + `y901 = "0.2.5"`（registry，不是 path）构建最小应用——
   这能抓住“本地 workspace 能编、发布版 re-export 缺失”的问题（历史上已发生过一次）。
4. **`!` 与版本号一致性检查**：脚本读 `git log <last-tag>..HEAD`，若出现带 `!` 的提交而
   workspace 版本没有升 minor，则失败。**先修现状**：`fix(widgets)!` 曾落进 patch 版 0.2.1，
   与当版“无破坏性变更”的说明矛盾。
5. **弃用策略**：`#[deprecated]` 至少保留一个 minor 再删；`#[non_exhaustive]` 在 0.3.0 一次性加到
   易扩张的公开 enum/struct（`Options`、`Theme`、事件与语义结构），避免以后反复破坏。

### 4.4 组件与生态：补“桌面应用真正会用到”的，不做插件 ABI

- **Table v2（2–3 周）**：客户端排序（点击表头、稳定多键可选）、虚拟滚动（复用 `ListView` 的
  虚拟化骨架，先固定行高）、键盘导航（↑↓/Home/End/PageUp/Down）、列宽拖拽、粘性表头、
  grid/row/cell 语义角色。
- **MenuBar（1–1.5 周）**：基于现有 `Menu` 浮层 + 键盘（Alt 助记、方向键、Esc）。
- **NumberInput（1 周）**：复用 `TextInput`，加 step/clamp/格式/滚轮；语义暴露 value/range。
- **插件策略：明确不做**。Rust 下动态插件 ABI 的维护成本远高于收益；扩展路径 = 组合 + 自定义
  `Widget`，用 `examples/custom_widget.rs` 与文档把“第三方控件怎么写”讲透。
- 生态真正的入口是**参考应用**（文件管理器/终端），不是组件数量；等 S0/S1 完成后做一个
  `y901-shell` 模板仓库（`cargo generate`）比再加 5 个控件更有用。

### 4.5 入门体验：英文为公开面，中文留内部

- **README 双语**：`README.md` 英文（crates.io/GitHub 门面），`README.zh-CN.md` 中文；
  crate-level 文档至少 facade + widgets 两处英文。
- **5 分钟路径**：`cargo new` → 加依赖 → 25 行 hello（带截图）→ 三平台运行说明；
  放进 `docs/QUICKSTART.md`，并由 CI 编译该片段防止腐烂。
- **docs.rs 引导**：crate 根文档改成“安装 → 最小示例 → 下一步读什么”的结构；
  内部设计文档（本仓库大量高质量 ADR）保持中文，但加英文索引页。
- **示例画廊**：`examples/README.md` + 截图/GIF；把“18 个示例”变成可发现的能力地图。
- 顺带修过时文档（facade 版本号、x11 crate 仍写“只做编码层”等），否则英文入口会把旧信息放大。

### 4.6 Web：先诚实降级，升级另立项

- **0.2.7**：README + `docs/WASM.md` 明确 Tier 3：单 canvas、整帧 `putImageData`、
  无 a11y 树、IME 候选窗不可控、无多窗口/FileDrop；不建议做产品。
- **若要升级（另立 4–6 周项目）**：WebGL2/WebGPU 纹理上传替代 `putImageData`（1–2 周）；
  脏矩形 `texSubImage2D`（1 周）；DOM a11y 镜像（与 S3 共用，2–3 周）；IME 候选窗仍无解。
- **不做**：把 Web 追到与桌面同能力——维护税与收益不成比例。

### 4.7 Bus factor：把“一个人的知识”变成“仓库里的约束”

1. **平台分层**（§2）：减少必须同时维护的面。
2. **每个平台一份 runbook**：`docs/runbooks/{win32,x11,web}.md`——怎么跑、怎么抓日志、
   常见故障、协议字段对照、如何加测试。目标：新人/AI 只读文档就能复现一次修复。
3. **平台补丁的 Definition of Done**：必须有“去掉修复就变红”的测试；必须有 Linux/Windows
   对应 CI 证据；否则不合并。
4. **自动化优先**：协议编码/解析继续用 golden 用例锁死；像素/语义/视觉回归用快照；
   发布流程脚本化（已有 `publish-crates.ps1`，补 semver/consumer）。
5. **AI 杠杆的正确用法**：让 AI 做“有确定性验收”的活（协议实现、测试生成、文档同步、
   跨平台移植初稿），人来定架构与验收；一周样本不足以证明可持续，但门禁越硬，杠杆越稳。
6. **退出/交接**：`ARCHITECTURE.md`（模块边界 + 不变量）、`SECURITY.md`、发布 checklist
   放在仓库根，任何继任者按文档能发版。

## 5. 如果只有两周

按性价比排序，先做这四件（都在 S0，风险低、外部感知强）：

1. 英文 README + `docs/QUICKSTART.md` + 示例截图（1 周内可完成）；
2. Tier 矩阵写进 README/CI，Web/macOS/Wayland 的承诺降级；
3. `cargo-semver-checks` + 下游 consumer 冒烟 + `!` 校验脚本；
4. Table 排序 + 虚拟滚动先行版（用户感知最强的一个组件缺口）。

## 6. 明确不做

- 不自研 HarfBuzz 替代品（用 `rustybuzz`），不做字体渲染的 hinting/亚像素级复刻；
- 不做动态插件 ABI、不做 macOS/Wayland 后端、不追 Web 产品级；
- 不在 0.2.x 里塞破坏性变更（统一 0.3.0）；
- 不为“看起来支持更多平台”而扩大 CI 维护面。

## 7. 风险与缓解

| 风险 | 影响 | 缓解 |
|---|---|---|
| 文本布局改动波及全部控件 | 大面积回归 | 并行管线 + feature flag；cell 路径零改动；像素基线分批重刷 |
| 图集改 glyph 键后容量/命中率下降 | 性能退化 | LRU + 压缩 + 分面图集；基准门禁 |
| AccessKit 引入异步/D-Bus/Windows 绑定 | 稳定性与体积 | 每平台 feature 门控；适配层薄；树快照测试 |
| 单人体力被平台适配吃掉 | 主线延期 | Tier 降级 + WIP 限制（同时最多 1.5 条线） |
| 新依赖许可证/MSRV | 门禁/发布失败 | `cargo deny check` + MSRV CI 前置验证（rustybuzz/accesskit 已核对为 MIT/Apache、MSRV ≤1.92） |
| 0.3.0 破坏性变更吓跑潜在用户 | 生态更难起步 | 目前下游为零，是**最低成本窗口**；配迁移指南与 codemod 示例 |

## 8. 里程碑

- **M0（约 2 周）**：0.2.7——英文入口、Tier 声明、semver/consumer 门禁。**先止血。**
- **M1（约 8–10 周）**：0.3.0——Table v2、MenuBar/NumberInput、语义模型、shaping MVP（Label/RichText）、
  Windows UIA；集中破坏性变更与迁移指南。
- **M2（约 16–20 周）**：0.4.0——TextInput cluster 编辑、字体回退链、Linux AT-SPI、变量字体；
  Web 决定“升级或维持 Tier 3”。