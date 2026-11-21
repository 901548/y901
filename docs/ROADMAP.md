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
6. **必要时从零用纯 Rust 重写**（已获用户授权）：当现成 crate 或现有抽象与目标冲突时，
   允许重写我们自己的实现（例如图集从 `char` 键改为 `GlyphKey`、Web 呈现从 `putImageData`
   改为纹理上传），而不是硬套；但“重写”必须服从零 C 依赖、MSRV 1.92 与门禁，不能为重写而重写。

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

## 9. 进度记录

### S0（2026-11-21）：定位与入口 —— 已完成主体

- 英文 `README.md`（公开门面）+ 中文 `README.zh-CN.md`（互链）；英文 `docs/QUICKSTART.md`
  （5 分钟：建工程 → 依赖 → 25 行 hello → 运行 → 排错）。
- 英文 `docs/PLATFORMS.md`：T1 Windows / T2 X11 / T3 Web / macOS+Wayland 出范围；
  `docs/WASM.md` 顶部标注 Tier 3 与非目标；`CONTRIBUTING.md` 同步平台分层与双语策略。
- `scripts/check-semver-bump.ps1`：`!` 破坏性提交必须相对最近 tag 升 minor/major；
  已在本地验证“历史 v0.2.1 场景会被判红”。
- CI 新增 `semver-guard` 与 `semver-checks`（`cargo-semver-checks 0.49`，基线=最近 tag）；
  本地对 11 个 crate 跑 `--baseline-rev v0.2.5`：196 项检查全过。
- `scripts/check-consumer.ps1` + `.github/workflows/consumer.yml`：从 crates.io 拉**已发布版**
  编译下游工程（不用 path）；本地用 `y901 0.2.5` 实测通过，定时任务每周一跑。
- 待补（S0 尾部）：示例截图/GIF 与 `examples/README.md`（需要实际截图，不能伪造）。

### S1（2026-11-21）：组件与语义地基 —— 已完成主体

- **Table 重写为虚拟滚动**（`crates/y901-widgets/src/table.rs`）：表头固定、独立滚动条、
  只画可见行（5000 行时只画 ~7 行）；三态排序（数字按数值、文本忽略大小写）；
  ↑↓/Home/End/PageUp/PageDown；选中按**数据下标**，排序不漂移；`set_rows/select/sort_by/sort_command`。
- **NumberInput**（新）：数字/单点/前导负号、Enter 或失焦提交并规范化、Esc 放弃、
  ↑↓ 步进（Shift ×10）、min/max 钳制、小数位。
- **MenuBar**（新）：标题布局 + 命中测试，点击经 `ctx.request_popup` 弹 `Menu`；
  ←/→ 移动、Enter/↓ 打开、Esc 关闭；Alt 助记键明确不做（后端不提供）。
- **无障碍语义地基**：`Semantics` 新增 description/expanded/checked/selected/level/
  value_range/value_step/labelled_by/described_by/live，并标记 `#[non_exhaustive]`；
  `hash_semantics` 纳入新字段（否则状态变化不触发语义失效）；checkbox/switch/slider/
  tabs/alert/error_state/progress/number_input 全部接线。
- **破坏性变更集中在 0.3.0 起步**：workspace 版本 `0.2.6 → 0.3.0`，迁移说明见
  [`MIGRATION_0.3.md`](MIGRATION_0.3.md)；CI 的 semver-checks 允许"升了 minor 的破坏性变更"，
  但版本仍是 patch 时报破坏性变更必红。
- **顺手修掉的真 bug**：`sync-doc-numbers.ps1` 里 `cargo test` 的赋值语句早已丢失
  （`$defaultOut` 恒空 → 实测 always 0），导致文档数字**永远不会更新**且门禁不会红。
  已补回、加"解析到 0 就失败"保护、把计数规则从 `\d{3}` 放宽到 `\d{3,}`
  （通过数已过 1000），并排除四位年份与端口号避免误报。

### S2（2026-11-21）：文本管线 —— 已落地基础设施，控件接入未开始

- 新增 `crates/y901-render/src/text/`：`GlyphKey`、bidi 方向判定与视觉重排、
  集群切分、`shaping` feature 下的 rustybuzz shaping（详见 §3 与代码注释）。
- 依赖全纯 Rust；`shaping` 是可选 feature，**默认构建与像素基线完全不变**。
- CI 新增 `text-shaping` job（编译 + 跑 feature 测试 + 复核依赖树无 C/C++ 构建依赖）。
- **未做**：图集键真正切到 `GlyphKey`；`measure_with(avail_width)`；
  Label/RichText/TextInput 接入 shaped 路径；字体回退链。

### S3（2026-11-21）：无障碍桥 —— 映射层 + 两个平台适配器已落地

- 新 crate **`crates/y901-a11y`**（第 12 个）：
  - `map_role`：Y901 角色名 → AccessKit `Role`（**布局容器映射成 `GenericContainer`
    而不是 `Group`**，否则读屏器会多念一层"分组"；未知角色落到 `Unknown`，不猜）；
  - `to_tree_update`：`UiTree::snapshot()` → AccessKit `TreeUpdate`，含 label / description /
    value / 数值范围与步进 / checked（`Toggled`）/ expanded / selected / level / live /
    动作（click / toggle / focus / set_value / scroll / expand / collapse）/ bounds / focus；
  - **未暴露节点（`exposed_to_ai == false`）整棵子树跳过**，与外部工具网关同一规则；
  - `platform::PlatformBridge` trait + `NodeIdMap`（AccessKit u64 ↔ Y901 字符串 id 双向映射，
    防止动作打到上一帧的节点上）。
- **Windows UIA**（feature `windows-uia`）：`accesskit_windows::SubclassingAdapter`
  挂在 HWND 上；`ActionHandler::do_action` **只入队不处理**（回调可能在工作线程），
  应用线程用 `poll_action` 取；队列上限 256 条防刷爆。**必须在 `ShowWindow` 之前挂载**
  （库的硬约束，已在文档与注释里标明）。
- **Linux AT-SPI**（feature `unix-atspi`）：`accesskit_unix::Adapter`（zbus 纯 Rust D-Bus），
  支持根窗口 bounds、焦点状态、动作队列。
- 依赖：`accesskit 0.25`（映射层，纯 Rust）；平台适配器按目标平台 feature 门控。
  `accesskit_windows` 只依赖 `windows` 纯绑定，`accesskit_unix` 走 `zbus` —— 两者都不破零 C。
  新增 3 条 `zvariant`/`zbus_names` 维护性公告已在 `deny.toml` 与
  [DEPENDENCIES.md](DEPENDENCIES.md) 记明理由。
- 门禁：facade 增加 `a11y` feature（`pub use y901_a11y as a11y`），默认关；
  `check-packaging.ps1` / `publish-crates.ps1` 的依赖序已加 `y901-a11y`（**12 个 crate 全部可打包**）；
  CI 新增 `a11y-bridge` job（Windows 编 UIA、Linux 编 AT-SPI，并复核依赖树无 C/C++ 构建依赖）。
- 测试：19 条（角色映射 / 未暴露子树跳过 / checked+expanded / 数值范围 / live region /
  动作映射 / 焦点 / bounds / 空树 / 队列上限 / 初始树单次取用）。
- **未做**：真机验收（讲述人 / Orca 手工清单）；Web 的 DOM/ARIA 镜像；
  从后端（`y901-backend-win32` / `-x11`）自动挂载并每帧 `push` —— 目前是库层已就绪、
  后端接线待做。

### S4（2026-11-21）：后端接线完成；0.3.0 收敛未完成

已做：

- **两个后端真的推树了**（此前只有库层）：
  - `y901-backend-win32`（feature `a11y`）：`Window` 在 `CreateWindowExW` 之后、
    `ShowWindow` **之前**建 `WindowsUia`（库的硬约束），`after_frame` 末尾每帧推树 +
    轮询读屏器 `Click` 动作并回派；
  - `y901-backend-x11`（feature `a11y`）：`X11Window::create` 建 `UnixAtspi`，
    `render_tree` 结束（含"画面没变"的早退分支）推树 + 回派动作 + 同步焦点与根窗口 bounds。
- 动作反查用 `nth_node_id`（前序遍历 + 与 `to_tree_update` 相同的跳过规则），
  不长期持有会过期的 `NodeIdMap`。
- 修复：`Cargo.toml` 被写入 BOM 导致零 C 守卫把本地 crate 当 registry crate
  （报"源码缺失"假红）——已剥 BOM，并让守卫解析时剥掉。
- 验证：gate GATE OK；Windows `--features a11y` 测试通过；
  Linux Xvfb 下 `real_window` 17 passed / 2 ignored，a11y 21 条通过。

**未做**（0.3.0 收敛的剩余项）：

1. ~~`Options` 等公开结构体的批量 `#[non_exhaustive]`~~ —— 已做：`Options` 已标记，
   仓库内 46 处字面量统一成 `..Default::default()` 兜底（clippy 的 `needless_update`
   逐处清理，0 警告）。其余公开结构体（事件枚举、语义子结构）留给 0.4.0 一次性处理，
   理由：它们数量更多且多为只读消费，收益低于 `Options`。
2. 真机读屏器验收（Windows 讲述人 / Linux Orca 手工清单）—— 需要人操作，机器做不了。
3. Web 的 DOM/ARIA 镜像 —— 属 Tier 3 升级项，不在 0.3 关键路径。

- 新增 `crates/y901-render/src/text/`：
  - `GlyphKey { face, glyph }` —— 图集键的正确形状（`char` 作键会让不同上下文的
    同一字符共用位图）；`GlyphKey::from_char` 保留 cell 路径兼容；
  - `bidi`：段落方向判定（Unicode P2/P3）与视觉重排（`unicode-bidi`，
    数字在 RTL 段落里保持 123、RTL run 内部反转）；
  - `cluster`：字形集群边界与视觉→逻辑映射（组合记号不单独成界、脚本切换成界）；
  - `shape`（`shaping` feature）：`rustybuzz` 真 shaping，产出
    `ShapedRun { range, rtl, glyphs{x,y,advance,cluster}, width }`。
- 依赖全部纯 Rust（`rustybuzz 0.20` 复用已钉的 `ttf-parser 0.25` + `unicode-bidi` +
  `unicode-script`），`shaping` 是可选 feature：**默认构建与像素基线完全不变**。
- CI 新增 `text-shaping` job：编译并跑 feature 下的测试，同时复核依赖树无 C/C++ 构建依赖。
- 测试：22 条（方向判定 / 视觉重排 / 集群 / 拉丁 shaping / 阿拉伯 RTL / 坏字体报错 / face 键）。
- **未做**（下一步）：图集键真正切到 `GlyphKey`；`Widget::measure` 增加可用宽度重载；
  Label/RichText/TextInput 接入 shaped 路径；字体回退链。