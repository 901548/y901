# 项目状态

- **状态（2026-11-23）**: workspace 版本 `0.4.0`（**已发布**）。12 个 crate 均已上传 crates.io 且未 yank，tag `v0.4.0` 已推送；上一版是 `0.3.0`，0.4.0 起因于发布后两个带 `!` 的提交与一处真实破坏性变更（`WasmState` 增 `pub` 字段，`constructible_struct_adds_field`），按 0.x 约定升 minor。变更见 [RELEASE_NOTES_v0.4.0.md](RELEASE_NOTES_v0.4.0.md)。
- **tags**: v0.1.0 ~ **v0.4.0**（0.4.0 已发布到 crates.io）；历史版本说明见 `RELEASE_NOTES_v0.2.1.md` ~ `v0.2.5`；**crates.io 发布手册见 [docs/CRATES_IO.md](CRATES_IO.md)**。
- **测试记录**: Windows 2026-11-26 复测 `passed=1378` / `ignored=71` / `targets=208`；`--include-ignored` 全量 `passed=1378` / `failed=0`（含 GPU 用例，本机 150% DPI）（[numbers.txt](evidence/numbers.txt)）。Linux 2026-11-28 复测 `passed=1336` / `ignored=35` / `targets=208`（[linux-numbers.txt](evidence/linux-numbers.txt)，日志 [linux-verify-2026-11-28-tab-indent.log](evidence/cross-platform/linux-verify-2026-11-28-tab-indent.log)）。均为有日期的平台证据，非每次提交即时运行。
- **门禁**: `scripts/gate.ps1`（MSRV / **fmt** / 测试计数 / **test-inventory** / clippy / 像素哈希 / missing_docs / **doc 数字** / audit）；CI 另有 Linux Xvfb、打包校验与 `cargo deny check`（`deny.toml`）
- **clippy**: 0 警告（JSON 口径，先查退出码）
- **哈希基线**: `docs/evidence/hash-baseline.txt` = `0xF7455CCE7781A864` / `400x220`
- **SAFETY 覆盖：**96 个调用点 / 96 覆盖 / 0 缺口**（v0.1.14 起）；历史：v0.1.13 前 **96 / 93 / 3**

## 2026-11-28（续九）：`TextArea` 的 Tab 缩进 —— 以及框架必须先给一条"键盘出路"

PLAN 批次 5 的最后一项。当初挂起的理由写的是"要先定 Tab 属于编辑器还是焦点遍历的产品规则，
与 `focus_tab` 的现有契约冲突"。查下来发现：**框架早有 `Widget::wants_tab()` 这个钩子**
（终端用它做 shell 补全），所以真正要做的不是"改规则"，而是**装上钩子 + 补一条逃生口**。

### 设计（三条，缺一不可）

| 决定 | 理由 |
|---|---|
| **默认不接管**，要显式 `tab_indents(true)` | `Tab` 是键盘用户在大界面里移动的主要手段，组件不能随便吃掉 |
| 插**空格**（对齐到下一个制表位），不插 `\t` | `TextBuffer` 逐字符量宽，`\t` 没有"跨到下一个制表位"的语义，插进去会画歪 |
| **`Ctrl+Tab` 永远走焦点遍历**（框架保证，不看 `wants_tab`） | 否则开了缩进的编辑器就是**键盘陷阱**：进了文本框再也出不来 |

第三条是本轮**框架侧**的改动（`y901-ui/src/tree.rs`）。这不是锦上添花 ——
"组件能吃掉 Tab"和"键盘用户永远出得去"必须同时成立，否则那个能力就是无障碍缺陷。

### 踩到并记下的一个坑

`wants_tab` 第一次被我写进了 `impl TextArea { … }`（固有 impl）。编译通过、只有一条
"方法从未被使用"的警告 —— 因为框架通过 `&dyn Widget` 调的是 **trait 的默认实现**（`false`），
固有方法只是静静躺着。表现就是"缩进完全没生效"。改成 `impl Widget for TextArea` 才真正生效。
**这条写进了方法的文档注释**：同名方法放错 impl 块 = 静默失效。

### 验证

`tab_indent`（4 条）：默认 Tab 切焦点且不动正文；开启后 Tab 补到制表位、Shift+Tab 只吃空格；
制表位**按显示列**算（第 2 列按 Tab 只补 2 个；中文占 2 列同理）；**`Ctrl+Tab` 必须切走焦点**。

变异验证：

| 变异 | 结果 |
|---|---|
| 框架去掉 `!modifiers.ctrl` 判断 | **立即红**（`Ctrl+Tab` 被缩进吃掉 → 键盘陷阱） |
| 制表位按字符数而非显示列 | **立即红**（`中` 后补 4 个空格，应为 2 个） |

workspace **1378 passed / 0 failed / 71 ignored / 208 目标**（`--include-ignored` 1449）；
像素哈希不变；Linux Xvfb 1336 / 35 / 208。

## 2026-11-28（续八）：两件事 —— ①文本输入多状态属性测试 ②`box_levels_rgba` 零拷贝

### ① `TextInput`：密码 × 占位 × IME 组合态（属性测试，**没有新 bug**）

三个状态单独看都简单，叠在一起才是事故现场。四条不变量：

| 性质 | 为什么重要 |
|---|---|
| 密码模式的**语义 value** 只含 `*`，且**掩码长度按字符数** | 外部工具只读语义；按字节数打码会泄漏"这字段里有几个字节中文" |
| 密码模式的**绘制文本**也只含 `*` | 只挡语义不挡像素 = 没挡 |
| 占位提示不得进语义 label/value/description | 敏感应用的提示会被读走（`password_placeholder.rs` 的既定要求） |
| IME 预编辑串只画不提交；组字中 Backspace 不碰正文 | 否则输入法组字会被"删掉一半" |

结论：**零红**。变异验证：掩码改按字节数 → 2 条红；语义不掩码 → 红（`value 里出现了非掩码字符 "hunter2"`）。

### ② `box_levels_rgba` 零拷贝（批次 6 最后一个遗留项）

旧实现每次上传做**两份额外全量拷贝**：`base.to_vec()`（只为给第一轮一个 `&[u8]`）+
每级 `next.clone()`（`next` 既要当下轮输入又要交给调用方）。大图约 15 MB/次。

改成"先算 `next`、再整体 `push`、下一轮借 `out.last()`"后，**分配次数从 `1+2×levels` 降到 `levels`**，
且每次分配都是最终产物本身。

**怎么在没有 GPU 的 CI 里守住**（这正是当初把它挂起的原因）：`box_levels_rgba` 是**纯 CPU** 路径，
所以用**计数分配器**（`#[global_allocator]`）直接量出来：

```text
4 MiB 大图上传，累计分配：
  改前  3.31 × L0
  改后  1.313 × L0     ← 1.3125 = 1 + 1/4 + 1/16，正好是"只留产物"的理论值
```

护栏：`image_mip_alloc` 断言比例 < 1.45×（并把实测值打印出来）。变异验证：把 `base.to_vec()`
加回去 → 比例跳到 **3.313×**，断言立即红。

**验证**：当轮 workspace **1374 passed / 0 failed / 71 ignored / 207 目标**（`--include-ignored` 1445）；
像素哈希 `0xF7455CCE7781A864` 不变；Linux Xvfb 1332 / 35 / 207。

## 2026-11-28（续七）：`frame_step` 状态机属性测试 —— **没有新 bug**

最后一层：事件循环。`FrameStats` 的四个布尔量（`had_work` / `need_layout` / `presented` /
`close_requested`）各自驱动不同行为，**组合错了分别是"关不掉窗口"、"空闲烧 CPU"、
"后台数据不刷新"**——单看每一行代码都对，只有组合起来才错。

随机混 6 种 `RawEvent`（关窗 / 改尺寸 / 鼠标移动 / 按下 / 字符 / 光标形状）× 200 组 × 15 帧，
每帧检查四条不变量：

1. `close_requested` **恰好**由 `Close` 事件触发（多一个 → 莫名退出；少一个 → 关不掉）；
2. 有 `Resize` 就必须 `need_layout` 且真的调了 `relayout`；
3. `presented` 为真 ⇒ 后端确实渲染过；
4. 空闲帧（无事件、无脏区、无请求）不得上屏。

另加两条定向回归：**外部标脏同帧重排 + 上屏**（审计 #7 的运行时侧）、**空闲帧一次都不画**。

结论：**零红**。

变异验证有一个**有价值的部分命中**：

| 变异 | 结果 |
|---|---|
| `close_requested` 永远不置真 | **立即红**（`close_requested 与事件不符`） |
| 去掉 `drain_external_invalidations` 里的 `layout_requested = true` | **runtime 用例不红** —— 因为 `frame_step` 自己用 `external > 0` 判定要不要重排；但 **`y901-ui` 的 `single_node_invalidation_requests_a_relayout` 立刻红** |

第二行是这轮最值得记的一条：**同一处状态被两个消费方用不同方式使用** ——
`frame_step` 走返回值，直接调 `UiTree` 的宿主走 `take_layout_request()`。
覆盖点在后者，不在前者。变异存活不等于没测到，得先去别处确认；确认后才知道这层是安全的。

**验证**：当轮 workspace **1368 passed / 0 failed / 71 ignored / 205 目标**（`--include-ignored` 1439）；
像素哈希不变；Linux Xvfb 1326 / 35 / 205。

## 2026-11-28（续六）：选区抽取 / 剪贴板清洗属性测试 —— **没有新 bug**

推到终端最被信任的一条链路：**选中 → 复制 → 粘贴**。

四组性质（各对应一种用户会真的踩到的失败）：

| # | 性质 | 为什么重要 |
|---|---|---|
| 1 | `Selection::text()` 与 `Selection::contains()` **逐格一致** | 两者不一致时，界面高亮的格子和复制出来的内容不是同一块 |
| 2 | **复制→粘贴→再复制无损**（同列宽目标网格） | 少一个字符就是"看不见的数据丢失" |
| 3 | `paste::sanitize()` 后**不留任何控制字符**（除 `\r`/`\t`） | `\x1b` 能伪造转义序列、`\x08` 能改写行内内容 —— 安全红线 |
| 4 | `sanitize()` **幂等** | 否则"先清洗再转发"的链路会二次变化 |

覆盖：`text()` vs `contains()` 800 组随机网格×选区；复制粘贴往返 400 组；
`sanitize` 控制字符 2000 组、幂等 1500 组（输入里掺 ESC / BS / NUL / DEL / LF / CR / TAB）。

结论：**零红**。

变异验证（证明断言有牙）：

| 变异 | 结果 |
|---|---|
| `text()` 不再跳过 `WideTail` | **2 条立即红**（宽字符被重复输出 → 往返丢数据） |
| `sanitize()` 放行 `< 0x20` 控制符 | **立即红**（`残留控制字符 '\u{8}'`，输出里带着能改行的 BS） |

**验证**：当轮 workspace **1365 passed / 0 failed / 71 ignored / 204 目标**（`--include-ignored` 1436）；
像素哈希不变；Linux Xvfb 1323 / 35 / 204。

## 2026-11-28（续五）：`Table` 定宽列超宽时，"按比例压缩"被跳过 —— 列画到可视区外

第 6 个真 bug，出在 `Table::column_layout()`。

### 症状

列布局算完后有一段"超宽按比例压缩"，判据写的是 `flex_total_w > 0.0`：

```rust
let flex_total_w = if flex_weight > 0.0 { rest } else { 0.0 };   // rest = 分给 flex 列的剩余空间
if total > avail && total > 0.0 && flex_total_w > 0.0 { ...压缩... }
```

但 `rest = (avail - fixed - percent_total).max(0.0)` —— **定宽列本身就超宽时 `rest` 恰好是 0**，
于是 `flex_total_w == 0`，压缩被跳过：

```text
列：Px(7) Flex Flex Flex Px(119)，表格宽 93
fixed = 126 → rest = 0 → flex 列宽 0、flex_total_w = 0 → 不压缩 → 总宽 126 > 93
```

后果：最后一列画在行区域之外（被裁掉、看不见），而 `column_layout()` 同时供**命中测试**用 →
表头点击会指向那块看不见的区域。这正是这段代码注释里说要修的那一类问题
（"第二列被裁、表头命中还会指向被裁的区域"）—— 判据选错了代理变量，同一个 bug 又回来了。

### 修法

判据改成 **`flex_weight > 0.0`**（"存在 flex 列"这个真正的事实），而不是"分给 flex 的空间 > 0"。
同例：`fixed=126`、`flex_weight=3` → 按 `93/126` 压缩 → 总宽正好 93。
顺带删掉已无人使用的 `flex_total_w`。

### 验证

`table_invariants`（2 条）：随机列组成（定宽/flex 混合）× 200 组，
断言列首尾相接不重叠、除"显式定宽本身超宽"外总宽不超出行区域；外加随机 `set_rows`/`sort`/
`select` 下"选中下标永远合法"。

变异验证：判据改回 `flex_total_w > 0.0` → **立即红**
（`[case30] 列总宽 126 超过表格宽 93（fixed_sum=126 has_flex=true）`）。

**验证**：当轮 workspace **1361 passed / 0 failed / 71 ignored / 203 目标**（`--include-ignored` 1432）；
像素哈希不变；Linux Xvfb 1319 / 35 / 203。

## 2026-11-28（续四）：`ScrollView` 粘底在"摆完内容之后"改偏移 —— 流式输出会一顿一顿跳

这是第 5 个真 bug，来自第三套独立实现的滚动逻辑（`scroll.rs`，与 `ListView`/`TreeView` 完全不同）。

### 症状

`arrange()` 的顺序是：

```text
钳制偏移 → 摆内容（kid.arrange 用 offset 算 y） → 处理粘底（把 offset 推到新底部）
```

粘底那一步**在摆完内容之后**改 `offset`，于是同一帧里"子节点位置"和 `offset()` 各说各话：

```text
流式追加一行（粘底开着）：
  摆内容用 offset=300  →  再把 offset 推到 360
  子节点按 300 摆好    →  最后 60px 落到视口外  →  下一帧才跳上来
```

表现就是**流式输出（日志、进度）时画面一顿一顿地跳**。这个文件自己在钳制那段注释里
写着"旧实现先按旧 offset 摆好子节点、之后才钳制……看起来像抖一下" —— 同一个错误在
粘底路径上又犯了一次。

### 修法

把粘底挪到 `kid.arrange()` **之前**，让"本帧最终 offset"在摆内容前定稿。规则可以一句话
概括：**凡是会影响子节点位置的 `arrange` 内状态，都必须在子节点 arrange 之前算完。**

### 验证

- `scrollview_invariants`（2 条）：滚动容器每帧必须满足 `子节点 y == viewport.y - offset()`，
  包含"随机变高/变矮 + 滚动 + 粘底"120 组 × 20 步；
- 变异验证：把粘底挪回"摆完内容之后" → **两条都立即红**（`kid.y=-300` vs 期望 `-360`）。

**验证**：当轮 workspace **1359 passed / 0 failed / 71 ignored / 202 目标**（`--include-ignored` 1430）；
像素哈希不变；Linux Xvfb 1317 / 35 / 202。

## 2026-11-28（续三）：虚拟滚动属性测试 —— **没有新 bug**

继续推到 `ListView` / `TreeView`。这类控件的错误表现是"看得见却点不中"或"点中邻行"：
`visible_range()` 决定画哪几行，`row_at()` 决定点在哪一行，`row_rect()` 决定脏区 ——
三者只要有一个用了不同的偏移，键盘选择、鼠标点击、局部重绘就各说各话。

**`ListView`**（随机行数/行高/视口/滚动，150 组）：

1. `visible_range` 是合法子区间；
2. 区间内**每一行的中心点**用 `row_at` 反查必须还是它；
3. 区间内行矩形按 y 单调、不重叠；
4. 反复上下滚后区间始终合法。

**`TreeView`**（随机展开/折叠 + 滚动，120 组 + 一条专项）：

1~3 同上；外加 **"滚到底后把视口拉高，偏移必须夹回 0"** —— 这是审计 #34 的同一族问题
（折叠/拉高后偏移仍指向内容之外 → 内容整体偏上留白、`visible_range` 变空、点击全失效）。

结论：**零红**。变异验证证明断言有牙：

| 变异 | 结果 |
|---|---|
| `ListView::row_at` 丢掉滚动偏移 | **立即红**（`row_at(...) 应为 60，实得 Some(0)`） |
| `TreeView::arrange` 不调 `clamp_scroll()` | **立即红**（`row0.y=-64`，正是"内容偏上"） |
| `TreeView::visible_range` 去掉 `first.min(len)` | **不红** —— 因为 `clamp_scroll` 保证了 `first < len`，这是不可达态下的冗余保护（记录下来：它不是 bug，但也不是可测行为） |

**验证**：当轮 workspace **1357 passed / 0 failed / 71 ignored / 201 目标**（`--include-ignored` 1428）；
像素哈希不变；Linux Xvfb 1315 / 35 / 201。

## 2026-11-28（再续）：X11 请求编码属性测试 —— **没有新 bug**

继续把属性测试推到协议编码层。不解析协议，只检查**服务端收到包后第一件会做的事**：

1. 整包 **4 字节对齐**；
2. 头部 `length` 字段（字节 2..4，小端）必须**精确等于** `总字节 / 4`；
3. 总长不超过单请求上限；
4. 大 payload 必须被 `request_fits` 判为"装不下"（调用方据此分块），而不是编码出一个回绕的小长度。

覆盖 11 个编码器 × 300 组随机参数，外加 `PutImage` 200 组随机尺寸（断言 `总长 == 24 + w×h×4`）。

结论：**零红**。长度守卫此前已有 5 条定向用例（`length_guard_tests`），这轮把"每个编码器 +
随机参数"都过了一遍，没发现新缺陷。

一条**变异验证**确认断言有牙：把 `length_units` 改成少算 1 个单位 →
`intern_atom` 的 `length=4 (16 字节) vs 实际 20`、`put_image` 的 `1263 vs 5056` 立刻报出来。

**验证**：当轮 workspace **1353 passed / 0 failed / 71 ignored / 199 目标**（`--include-ignored` 1424）；
像素哈希不变；Linux Xvfb 1311 / 35 / 199。

## 2026-11-28（续）：`ZWJ` 之后的制表符/空白被粘进 emoji 簇 —— 退格会连带删掉

这条是**跨实现比对**发现的：`y901-render` 和 `y901-ui` 各有一套"集群边界"实现，
我本来想断言两者相等，结果发现它们**按设计就不同**（一个服务 shaping 分段、一个服务
光标/删除粒度）—— 但那轮比对顺手暴露了 UI 侧一个真问题。

### 症状

`y901-ui::cluster` 的规则是"每个字符成界，除非它是组合记号，**或前一个是 `ZWJ`**"。
"前一个是 ZWJ" 被无条件应用，于是 **`ZWJ` 后跟任何字符都粘在一起**：

```text
👨 + ZWJ + TAB     → 被当成一个簇
Backspace 一次     → 连制表符一起删掉
←/→                → 跨过制表符/换行
```

UAX #29 里 `ZWJ` 只对 Extended_Pictographic 生效（GB11），而**控制符与空白永远成界**
（GB4/GB5）。所以 `👨‍\t` 必须是两簇：emoji 一簇（含 ZWJ）、制表符一簇。

### 修法

新增 `joins_after_zwj(ch)`：只有"非控制、非空白、非 ASCII"的字符才在 ZWJ 后继续同簇
（emoji 与 CJK 都在这个范围内；拉丁字母/标点应当断开）。`cluster_boundaries`、
`is_boundary_at`、`next_boundary` 三条路径全部走同一个判定 ——
顺带把 `next_boundary` 里那份**手抄的旧规则**换成调用 `is_boundary_at`，
消除"同一规则写两遍"的分叉源（这正是本轮 bug 的温床）。

**验证**：新增 2 条具名回归（`ZWJ + 制表符/换行/空格` 必须断开；`👨‍👩` 仍是一簇）。
变异验证：把 `joins_after_zwj` 改回恒 `true` → 用例立即红。
另新增 `bidi_invariants`（3 条）钉住双向文本：视觉顺序必须是逻辑字符位置的一个**排列**
（不重不漏）、纯 LTR 不变、纯 RTL 整段反转、`visual_runs_of` 的区间连续覆盖且无空段。

当轮 workspace **1350 passed / 0 failed / 71 ignored / 198 目标**（`--include-ignored` 1421）；
像素哈希不变；Linux Xvfb 1308 / 35 / 198。

**记一条方法论**：跨实现比对时先确认"两者是否**应当**相同"。这次断言错了，
但错误断言把注意力引到了真 bug 上 —— 前提写错不白费，只要愿意读失败原因。

## 2026-11-28：属性测试扫到 Flex 布局层 —— **没有新 bug**（这本身就是结论）

同一套"随机化 + 不变量"继续往布局层推：随机容器宽高、padding、gap、子项数量与 flex 权重，
钉三条不变量：

1. **兄弟不重叠**（沿主轴比较矩形区间，200 组）；
2. **flex 均分**：所有子项宽度 + 全部 gap 必须等于内容宽度（误差 < 0.5px，200 组）；
3. **放得下时不越界、放不下时允许溢出但不得重叠**，且首个子项永远从内容区左上角开始。

结论：**一条都没红**。布局层前几轮已经有过针对性用例（min/max 的 DPI 解析、
百分比跨阶段一致、`Stack` 无界、`Table` 列宽压缩等），随机化只是补覆盖，没发现新缺陷。

一条**测试自己的坑**值得记：最初把"子项必须留在父内容区内"写成无条件断言，结果随机用例
在两处失败 —— 查下去都不是 bug：

- `Flex::wrap(false)`（默认）**允许溢出**，这正是 `layout.rs` 里
  `default_no_wrap_still_overflows` 钉住的既定语义；
- `min_width` 之和超过可用宽度时，**守住 min 就必然溢出**，溢出才是对的。

所以断言改成分两种情形（放得下 → 必须在框内；放不下 → 只要求不重叠且起点正确）。
把"设计意图"误判成 bug 是属性测试最常见的假阳性来源，这条记在这里免得下次重复。

**验证**：当轮 workspace **1345 passed / 0 failed / 71 ignored / 197 目标**（`--include-ignored` 1416）；
像素哈希不变；Linux Xvfb 1303 / 35 / 197。变异验证：让 `leftover` 不减 `gaps`
（即 gap 不参与可用宽度计算）→ flex 均分用例**立即红**，说明这三条断言是真有牙的。

## 2026-11-27（续十一）：属性测试继续推进 —— `DirtyRegion::bounds` 的 f32 舍入

同一套"随机操作 + 不变量"打到脏区层，抓到第三个 bug。

### 症状（API 契约被违反一个 ULP）

`bounds()` 的契约是"覆盖全部脏块"。实现是 `Rect::new(x0, y0, x1 - x0, y1 - y0)`，
而 `Rect` 的右边界是**由 `x + w` 重算**的 —— f32 下 `x0 + (x1 - x0)` 不保证回到 `x1`：

```text
x0 = 54.8, x1 = 424.72  →  w = 369.91998  →  x0 + w = 424.71997  <  424.72
```

于是脏块右边缘恰好等于包围盒右边缘时，`bounds().contains_rect(块) == false`。
拿来当 scissor 就是**最右一列像素不重绘**（残影）；拿来做断言就是随机红。
属性测试里几百个随机脏区序列，第 2 个就命中。

修法：算完 `w`/`h` 后检查 `x0 + w < x1`，是则把 `w` 抬到**下一个可表示的 f32**
（`next_up`）。契约是"覆盖"，宁可宽一个 ULP，也不能窄一个。

### 顺带确认没问题的两处

同一轮还写了两套属性测试，**没有发现新 bug**（这类结论同样要记下来，否则下轮会重写一遍）：

- `atlas_packing_invariants`（随机若干张图集 × 每次 80 次分配、尺寸 32~96）：`SkylinePacker`
  的每次分配都完整落在图集内、任意两块不重叠、零尺寸/超尺寸请求被拒绝。
- `dirty_region_invariants` 的其余不变量：空矩形不入表、块数不超 `MAX_RECTS`
  （超了退化 `full`）、`clear()` 之后既非 `full` 也无残块。

**一处测试自身的坑**（值得记）：覆盖性最初用"矩形角点"断言，被 f32 舍入判成失败 ——
那不是 bug，是断言选错了对象。改成**按像素中心**采样（后端转 scissor 时是 `ceil` 向外取整，
"最后一个像素覆盖没覆盖"才是真语义），随机红消失。

**验证**：当轮 workspace **1342 passed / 0 failed / 71 ignored / 196 目标**（`--include-ignored` 1413）；
像素哈希不变；当轮 Linux Xvfb 1300 / 35 / 196。变异验证：去掉 ULP 外扩 → 属性测试与具名回归**都立即红**。

## 2026-11-27（续十）：用属性测试扫出 `y901-core` 两个真 bug

前几轮都是"读代码找问题"。这轮换成**随机操作 + 不变量断言**（确定性伪随机，不引依赖），
先在 `y901-core` 的终端网格上跑 300 个网格 × 120 步 × 14 种 VT 操作，抓到两个：

### A. 擦除会把宽字符擦成半个

`blank_from` / `blank_upto` 逐格填空白。宽字占两格（`WideHead` + `WideTail`），
只要擦除边界落在中间，就会留下"有尾无头"或"有头无尾"的半格：

```text
擦前:  a 中[尾]        列 1 是宽字头、列 2 是宽字尾
ESC[1K 到列 2:  a [空] 尾   ← 列 0/1 被填空白，列 2 的 WideTail 成了孤儿
```

后果：渲染时 `WideTail` 被当作半格标记跳过 → 画面上**少半个字**；
`line_text()` / `text()` 多出一个无头字符位；下次往这行写字时 `clear_wide_pair`
又按"列 2 是尾"去清列 1/2，**把相邻内容一起吃掉**。
触发方式很普通：`ESC[1K` / `ESC[K` / `ESC[J` 时光标恰好停在中文/emoji 上。

修法：所有擦除走同一条 `blank_span()`，边界落在宽字中间时**整字向外扩**
（VT 语义就是"擦到一部分 = 整个字被擦掉"）。

### B. `reflow` 改列宽时可能 `u32` 下溢 panic

裁剪到视口高度时，旧实现给"同一逻辑行的每一行"都减去 `base_off`：

```rust
for r in new_rows.iter_mut() { if r.line == base_line { r.line_start -= base_off; } }
```

它隐含假设"同一 `line` id 的行在序列里连续且偏移单调"。这条假设**不成立**：
`soft_wrap()` 会把写到的下一个物理行改写成当前逻辑行的续行（终端的正常行为），
于是同一个 id 可能在屏幕上**非连续地出现两段**，其中一段的 `line_start` 是 0 ——
减去 `base_off` 就下溢（debug 直接 panic，release 静默错位到错误的行首，
点击/光标定位全偏）。

修法：只修正**从行 0 开始的连续前缀**（那才是被切开的同一块），并且用**累加**重算
偏移而不是减法 —— 与 `line_start` 的定义逐字一致，不再依赖 id 唯一性。

### 顺带补的两个属性测试（未发现新 bug，但有护栏价值）

- `wrap_invariants`（4000 + 1500 组随机文本）：`wrap()` 的 `start` 必须是 char 边界且
  指向该行内容、多字符行宽度放得下、**不丢字符**（按字符而非按词比 —— CJK 允许断字）。
- `text_caret_invariants`（8 组种子 × 300 轮 × 25 步随机编辑）：光标/锚点永远落在
  **字素簇边界**上（`e` + 组合重音不能被劈开），包括"把光标设到任意字节位置"和撤销/重做。

**验证**：当轮 workspace **1342 passed / 0 failed / 71 ignored / 194 目标**；像素哈希不变；
两处修复都做了**变异验证**（把实现改回旧写法 → 属性测试立即红，还原后全绿）。

## 2026-11-27（续九）：发布就绪检查结果 —— 下一版必须是 **0.5.0**，不能是 0.4.1

批次 1–7 全部落地后跑 `scripts/check-release-readiness.ps1`（只读脚本，不发布）：

| 项 | 结果 |
|---|---|
| fmt / 版本一致性 / semver 纪律 / `gate.ps1` / `check-packaging`（12 crate）/ `cargo deny` | **PASS** |
| `cargo semver-checks`（对 `v0.4.0` 基线） | **FAIL**：`constructible_struct_adds_private_field` — `Present.collector` |
| crates.io 凭据 | **BLOCKED**（本机无 `~/.cargo/credentials.toml`） |
| 工作区干净 | **BLOCKED**（发布时要求） |
| 真机读屏器验收 | **BLOCKED**（只能人工） |

**为什么不能发 0.4.1**：`Present`（`y901-backend-web`，crate 根 `pub use present::Present`）
在 0.4.0 可以用结构体字面量构造（`pub canvas` + `pub ctx`）；本轮 A5 给它加了一个私有字段
`collector` 存分阶段统计，于是外部 `Present { canvas, ctx }` 不再编译 —— 这是**破坏性变更**。
本仓库的规矩是"破坏性变更走 minor"（0.4.0 正是这么来的：`WasmState` 增 `pub` 字段），
所以下一版按 **0.5.0** 规划，见 [PLAN_2026-11-25.md](PLAN_2026-11-25.md) 批次 8。

**没有为了 patch 号好看去绕 API**：把 `collector` 挪进 `WasmState`、或给 `WasmWindow` 加字段，
都会破坏另一个公开结构体 —— 那只是把 break 换个位置。诚实升 minor 比扭曲设计便宜。

## 2026-11-27（续八）：Win32 关窗可否决 + IME 候选窗真的会跟随光标（审计 #4 #5）

审计里最后两条"未修"的 Win32 项。

### A. `WM_CLOSE` 不再偷偷销毁窗口

旧行为：点标题栏 X → `DefWindowProcW` → **同步 `DestroyWindow`**。宿主拿到
`RawEvent::Close` 时 HWND 已经没了 —— "还有未保存的改动，确定退出？"这种提示根本做不到，
`Window::close()` 也没有参与的机会。

现在 `WM_CLOSE` **只投递事件**（窗口继续存在）：否决 = 什么都不做，同意 = 调
`Window::close()`（改为幂等，`IsWindow` 短路）。`run_loop` 语义没变（收到 Close 就返回，
`Window` 随后 drop 时兜底销毁），所以"点 X 关窗"的用户体验与以前一致。

`tests/wm_close.rs` 2 条钉住：`WM_CLOSE` 之后窗口**仍在**且事件队列里有 `Close`；
否决之后窗口不会被偷偷销毁。变异：改回 `DefWindowProcW` → 2 条立刻红。

### B. IME 候选窗用错了 style 位 + 坐标系

`CandidateForm` 里传的是 `CFS_POINT`(0x0002) 和客户区坐标 —— 两个都错：

| API | 正确的 style | 正确的坐标 |
|---|---|---|
| `ImmSetCompositionWindow` | `CFS_POINT` | 客户区 |
| `ImmSetCandidateWindow` | `CFS_CANDIDATEPOS`(0x0008) | **屏幕** |

`CFS_POINT` 不是 `CandidateForm` 的合法位，于是**候选窗根本不动**（永远停在默认位置）。
拆出纯函数 `ime_forms()` 让两套口径各自正确，并用 `ClientToScreen` 换算出候选窗那份。

**验证**：`ime::ime_form_tests` 断言两个表单的 style 与坐标各自正确（且断言
`candidate.style != CFS_POINT`）；变异回 `CFS_POINT` → 立即红。

**诚实边界**：IME 只验到"表单参数正确"。**真实输入法把候选框画在光标旁边**仍需人工看
（仓库里没有可自动化的 IMM 环境）。

当轮 workspace **1331 passed / 0 failed / 71 ignored / 191 目标**；当轮 Linux Xvfb **1289 / 35 / 191**。

## 2026-11-27（续七）：后台线程一次标脏不再等于整窗重绘（审计 #7）

`InvalidationHandle` 是给后台线程用的"该重绘了"通知。旧实现里 `drain` 把 `One(id)` 和
`All` **一视同仁**地升级成 `DirtyRegion::full()`（`whole_window` 变量赋值后就是
`let _ = whole_window;`，纯粹的死代码）。后果很直接：

- 后台每来一条数据（一行日志、一次下载进度、一个列表项）→ **整窗重绘 + 整帧纹理上传**；
- 而这个项目自己的 `invalidate_all()` 注释里写着"不要每帧调用——它会让脏区剪枝完全失效"。

**现在**：`All` → 整窗；`One(id)` → 只标该节点矩形（含 `Widget::ink_overflow` 外扩）
并请求本帧重排。正确性不靠"整窗最稳妥"这句托词，而是三条已有机制：

1. `invalidate(&id)` 按该节点自己的矩形标脏；
2. 后台改的数据只有组件下次 `render` 才读得到，所以同时置 `layout_requested`；
3. 重排后 `mark_moved_map()` 会补"旧矩形 ∪ 新矩形"——尺寸变化、甚至节点消失（旧位置要擦）
   都被覆盖。

真正说不清是哪块的场景（换主题、整表换数据）由调用方显式调 `invalidate_all()` 表达，
不再让框架去猜。

**验证**：+4 条（`One` 不整窗 / `One` 请求重排 / 同帧 `All` 压过 `One` / 未知 id 不产生脏区）；
变异验证：把 `drain` 改回"一律整窗" → 2 条立刻红，还原后全绿。像素哈希不变。
当轮 workspace **1328 passed / 0 failed / 71 ignored / 190 目标**（`--include-ignored` 1399）；
当轮 Linux Xvfb **1289 passed / 35 ignored**。

## 2026-11-27（续六）：WARP 上的 `STATUS_HEAP_CORRUPTION` 已定位并修复

这条风险挂了好几天（`instance_overflow` 的 GPU 用例因此被 `#[ignore]`）。**根因不是 WARP**，
是一处"两份状态各自记忆"的经典错误：

| 事实 | 存在哪 |
|---|---|
| 实例缓冲实际多大 | `Pipeline::instance_buffer`（`byte_width = cap * 96`） |
| 程序认为它多大 | `Gfx::instance_capacity`（元素数） |

`rebuild()`（设备丢失后的恢复路径）会重建 `Pipeline` → 缓冲回到 `INSTANCE_CAPACITY`，
**但没把 `Gfx::instance_capacity` 改回去**。下一帧：

```text
ensure_instance_capacity(10002) → 10002 <= 20000（旧值）→ "够大"，扩容被跳过
count = min(10002, 20000) = 10002
Map(DISCARD) 一个 10000 容量的缓冲 → copy 10002 * 96 字节 → 写过了映射区尾部
```

WARP 上稳定 `STATUS_HEAP_CORRUPTION`（0xc0000374，进程级崩溃、连用例名都不打印）；
真实 GPU 上则是驱动内存被悄悄踩坏 —— 更糟，因为看不见。

**修法**：把容量搬进 `Pipeline`，让"容量"与"缓冲"只能同生共死（`create_pipeline` 一起初始化，
`rebuild` 一起重建）。`Gfx` 上不再有第二份。另加两条源码守卫用例钉住这个结构。

顺带发现并修掉同类第二处：`rebuild()` 没有清 `images`（图片纹理表）——
重建后仍会把**旧设备**的 `ShaderResourceView` 绑到新上下文。release 下不报错、只是图不显示，
debug layer 下是 `DEVICE_MISMATCH`，还可能把新设备再踢一次。现在重建时丢弃并打一行日志
（代价：设备丢失后图片需重新上传，已记录）。

**验证**：
- 修复前：`Y901_D3D_WARP=1 cargo test ... --ignored` → `0xc0000374`（稳定复现）
- 修复后：同一命令不再崩溃；WARP 仍会在 `Present` 返回 `0x887A0005`（**100 个实例也一样**，
  说明这是本机 WARP/交换链的既有问题，与扩容无关）→ 用例按"环境不可用"留痕跳过
- 真实 GPU：`instance_overflow` / `font_3c_pixels` 全部通过；像素哈希 `0xF7455CCE7781A864` 不变

**诚实边界**：设备丢失后的恢复路径仍然没有**自动化**覆盖（触发它需要拔显卡/驱动更新）。
这次靠的是"WARP 上恰好能稳定复现崩溃"这条偶然路径定位的 —— 已把结论写成结构守卫，
但"下次 real GPU 上设备真的丢了会怎样"仍属人工验证范围。

## 2026-11-27（续五）：卫生批 —— 把"假装在测的用例"和"没人用的字段"清掉（PLAN 批次 7）

这一批没有新功能，全是"看起来有、其实没有"的东西：

| 位置 | 问题 | 现在 |
|---|---|---|
| `primitives.rs` 渐变主导轴 | 用 **40×40 正方形**，逐行与逐列条数相同，主导轴判断错了也照样绿 | 改用 64×16：竖直占优 → 16 条、水平占优 → 64 条，外加单调性/渐变跨度/精确面积 |
| `table.rs` PageDown | 只断言 `选中 >= 3`：page_rows 恰为 3 时**恒真** | 断言**精确等于一页**，并补对称的 PageUp |
| `combo_box.rs` 键盘高亮 | 只断言私有字段 `highlight`，画没画出来不知道 | 渲染层用例：高亮行必须真被涂色；用**偶数行**做对照（斑马纹只涂奇数行） |
| `Theme.slots` | 字段存在但 `style()` 走编译期常量表 → **死数据** | `style()` 走实例 `slot_index()`；加"槽位改名真生效"用例 |
| `fasthash::write_u64` | 文档说 FNV-1a，实际只有**一轮**；且高位不参与低位 | 文档如实说明"快路只需分布、不必与标准一致"；**加 `n ^ (n >> 32)`** 把高位混进低位 + 回归用例（去掉即红） |
| `ImageFit::None` | 源图大于目标时只裁目标、源仍整张 → **采样被拉伸** | 按交集**两边同时裁**，保证面积相等 |
| `Query` 的 `~=` | 对 `disabled`/`exposed` 恒不匹配，静默返回空结果 | 解析期报错并提示改用 `=`（"查不到"会被误读成"没有禁用节点"） |

`color/geometry/command` 的"过期文档"复核后**没有**需要改的表述，因此没动 —— 记录在此，
避免下一轮重复怀疑。

**验证**：+6 条新用例、加强 3 条；当轮 workspace **1322 passed / 0 failed / 71 ignored / 190 目标**
（`--include-ignored` 1393）；像素哈希 `0xF7455CCE7781A864` 不变；Linux Xvfb 1285 passed（当轮数）。

## 2026-11-27（续四）：文本编辑的产品缺口 —— 撤销/重做、按词移动、双击选词（PLAN 批次 5）

三个"用户一定会按"的操作此前都不存在：`Ctrl+Z` 没反应、`Ctrl+←/→` 不动、双击不选词。
现在两个文本控件（`TextInput` / `TextArea`）都有，且**逻辑只写在 `TextBuffer` 里一份**。

| 能力 | 语义（刻意选的约定） |
|---|---|
| `Ctrl+Z` / `Ctrl+Y` / `Ctrl+Shift+Z` | 撤销 / 重做。**连续打字合并为一步**（间隔 < 700ms），停顿后另起一步 —— 撤一次退回一段话，而不是一个字 |
| 历史容量 | 条数上限 200 **且** 总字节上限 4 MiB（只按条数限，4 MB 文档的 200 步就是 800 MB；只按字节限，短文本会攒几千步） |
| `Ctrl+←` / `Ctrl+→` | 按词移动，**对称约定**：`←` 落到词首，`→` 落到下一个词首（最后一个词的下一站 = 文本末尾）。加 `Shift` 扩展选区 |
| 双击 | 选中指针下的词；点在空白/标点上**优先选右边的词**，右边没有才选左边；全是标点时选这串标点 |

两条容易做错、所以专门钉住的细节：

1. **不能拆字素簇**：`cafe` + `U+0301` 的词移动/选词必须落在整个簇的外侧。
   `word_range_never_splits_a_grapheme_cluster` 与控件层的 `word_moves_never_split_a_grapheme_cluster` 各钉一遍。
2. **撤销要还原光标**：`undo_restores_the_cursor_where_the_edit_started` —— 否则撤销后光标跳到开头，
   用户下一次输入就插错位置（比"不能撤销"更糟）。

双击识别不用系统双击消息，而是"**时间 + 距离**"（400ms / 4 逻辑像素）：
三个后端都要各自翻译一遍系统消息，而这两个量在任何后端都拿得到，判定逻辑只写一份。

**验证**：`y901-ui` 新增 9 条、`y901-widgets/tests/text_editing.rs` 新增 10 条（端到端：事件进、文本出）；
workspace **1316 passed / 0 failed / 71 ignored / 190 目标**（`--include-ignored` 1387）；Linux Xvfb 全量见 evidence 日志。

**诚实的边界**：撤销历史是"整份文本快照"，不是基于操作的 diff —— 4 MiB 上限下最坏约 4 MiB 额外内存；
换成 rope/CRDT 才能做到真正的 O(编辑量)，那属于 NEXT 阶段（见 [ROADMAP.md](ROADMAP.md)）。

## 2026-11-27（续三）：光标移动不再物化整份边界表（PLAN 批次 6 后半）

`TextBuffer` 的 `snap` / `prev_boundary` / `next_boundary` 每次调用都先
`cluster_boundaries()` 建一个**全文本**的 `Vec<usize>`，再二分/线性找。一次光标移动
要调 3 次，等于每帧物化 3 份边界表。

真实感的一个数字（`crates/y901-ui/tests/cluster_perf.rs`，`#[ignore]` 剖析用例）：
**20 万字符的单行文本，1000 次右移 = 327 µs（改后）vs 41.2 s（改前），约 12.6 万倍**。

改法：`snap` / `prev` / `next` 改成**按簇游走**——
- `snap`：退到字符起始，不在簇边界就接着往回走；
- `prev_boundary`：往回走一个字符再判边界；
- `next_boundary`：往前一个字符一个字符判边界（簇内字符不可能是边界，所以真实步数是簇长）；
- 判定条件与 `cluster_boundaries` **逐字对应**（`is_boundary_at`）。

三者都不再分配内存；`cluster_boundaries()` 保留（公开 API，测试与外部用户仍可用）。

**正确性靠对拍**：新增 `allocation_free_walk_matches_the_materialised_reference`，
把旧实现原样抄成参照物，在 10 组文本（ASCII / 组合重音 / 天城文 / 阿拉伯文 /
ZWJ emoji 序列 / 空格混合）× **每个字节位置**上逐一比对 snap/prev/next 三个函数。
另加一条超长簇用例（"基字 + 500 组合记号"，验证一次跨簇而不是按字符走）。

**验证**：当轮 workspace **1324 passed / 0 failed / 71 ignored / 190 目标**；像素哈希
`0xF7455CCE7781A864` 未变（纯实现优化）；Linux Xvfb 全量见 evidence 日志。

顺带修掉一个**门禁脚本自身的坑**：`gate.ps1` 与 `sync-doc-numbers.ps1` 都用
`Get-Content` 读 `test-baseline.txt`，而 PowerShell 5.1 默认按 ANSI 解码 —— 中文注释
遇到"尾字节 + 换行"的歧义时会把末尾的数字行**粘进上一行注释**，于是报
"test baseline has no number"。两处都改成 `[System.IO.File]::ReadAllLines(..., UTF8)`
（`sync-doc-numbers.ps1` 里读 `linux-numbers.txt` 早就踩过同一个坑并写下了注释）。

## 2026-11-27（续二）：无障碍动作路由 —— `NodeId` 改稳定哈希（PLAN 批次 3）

**症状（真实可复现的误操作）**：读屏器/自动化点"删除第 2 行"，删掉的却是第 3 行。

根因有两层，缺一不可：

1. `y901-a11y::to_tree_update` 按**前序遍历序号**分配 `NodeId`（`NodeId(next_id)`）。
   列表删掉一行，它后面所有控件的 id **整体前移一位**；
2. 两个后端的 `pump_a11y` 收到动作后，用 `nth_node_id(tree, target.0 as usize)`
   按**当下这棵树**的序号反查。而 UIA / AT-SPI 的动作都要过一层 IPC，
   到达时树**已经不是**当初推出去的那棵了 —— 序号 + 异步 = 张冠李戴。

修法（三处一起改才有意义）：

| 层 | 改法 |
|---|---|
| id 分配 | `NodeId = FNV-1a 64(NodeInfo.id)`；同树内碰撞**线性探测**。同一控件**跨帧 id 不变**，UIA 的自动化节点缓存与读屏器的焦点记忆才靠得住 |
| 映射 | `to_tree_update_with_map()` 返回 `(TreeUpdate, NodeIdMap)`（旧入口 `to_tree_update()` 保留）；`NodeIdMap` 此前是**从未被填充的死代码** |
| 派发 | 窗口持 `a11y_node_ids`（最近一次推送的映射）；动作进来直接反查。**查不到 = 节点已删除 → 丢弃 + 每进程告警一次**（不静默误派，也不刷屏）。两个后端的 `nth_node_id` 已删除 |

顺带补上浮层的语义树断言：`snapshot()` 把浮层作为根的最后几个孩子追加，而
AccessKit 的 `bounds` 就来自这个 `rect` —— 现在钉住"语义树矩形 == 命中测试矩形"
（锚点贴视口右下角，强制 `place_popup` 翻转）与"浮层排在基础树之后"。

**验证**：`y901-a11y` +10 条、`y901-ui/popup_hit` +2 条；当轮 workspace **1297 passed / 0 failed**（此后批次 6/5/7 依次增至 1322）
（ignored 71 / 190 目标，`--include-ignored` 1387）；Linux Xvfb **1336 passed / 35 ignored**，
且 `y901-backend-x11 --features a11y`（AT-SPI/D-Bus）可编。
**变异验证**：把 `IdAllocator::allocate` 退回遍历序号 → 4 条新用例立即红，还原后 43 条全绿
（文件 SHA256 比对确认字节一致）。
**诚实边界**：自动化只覆盖到"映射表把 id 查对了"；"讲述人真的点中那一行"仍需按
[A11Y_ACCEPTANCE.md](A11Y_ACCEPTANCE.md) 人工验收。

## 2026-11-27（续）：整理 —— RenderStats 三个软后端统一到 `Collector`

A4–A6 落地时，X11 / Web / Win32 软路径**各自手写了 6 个几乎相同的辅助方法**
（`begin_frame` / `mark` / `record` / `dirty` / `mark_skipped` / `finish`），
合计约 120 行重复代码。重复的不只是行数：**"关闭时不读时钟""取走语义"
"CPU 后端没有的量写 0" 这类规则各写一遍，迟早有后端写错。**

现在把规则收进 `render_stats::Collector`（`y901-runtime`，+121 行）：

| 后端 | 整理前 | 整理后 |
|---|---|---|
| X11 | 6 个私有方法（≈65 行） | 字段 + 直接调 `self.stats.*` |
| Web（`Present`） | 6 个 pub 方法（≈60 行） | 6 个一行转发（保留 pub 面不变） |
| Win32 软路径 | 1 个私有方法 + 3 段内联块 | 字段 + `self.soft_stats.*` |

净变化：**+197 / −185**（净 +12 行，其中 121 行是带完整语义注释的共享实现 +
4 条新单测；重复代码减少约 120 行）。

新增 4 条规则单测（`render_stats::tests`，进程开关两种模式都跑过）：
关闭时 `mark()` 不读时钟且 `take()` 不交付；开启时**复用同一对象**、帧号每帧 +1、
`finish()` 把 `atlas_upload_bytes`/`instance_*`/`batch_count` 写 0；
`mark_skipped()` 只置标志、**任何阶段都不得有测量值**；`discard()` 后不交付（失败帧）。

**零行为变化**：四个统计测试二进制（`render_stats_enabled` / `_soft` / `_headless` /
`render_stats_x11`(+`_disabled`)）逐条仍绿，像素哈希不变。
顺带修掉一处会打印错误信息的细节：扩容日志现在报的是**真实旧容量**
（此前 `10000 → 40000` 会把旧的 20000 报成 10000）。

保留的"后门"：`Collector::stats_mut()` 供 `record_upload` 这类批量写入与单测使用 ——
否则单测也得先打开进程级开关（`OnceLock`，一开全进程都变），反而更容易写出依赖
执行顺序的测试。已在方法注释里写明用途。
## 2026-11-27（续）：issue #7 的 CI 收口 —— 用例合并 + 记录 WARP 崩溃风险

`instance_overflow` 首次进 CI 时连续三次红，每次原因不同，逐一查清：

1. **两个 GPU 用例并行跑** → 同进程两个 D3D11 窗口，WARP 上
   `STATUS_HEAP_CORRUPTION`（0xc0000374），进程直接崩、不打印用例名。
   改用例：两段检查（计数器 + 像素回读）**合并成一个用例、同一个窗口**串行做，
   断言一条不少。仓库本来就有"同一进程只允许一个 D3D 窗口"的约定
   （`grid_image_gpu.rs` / `font_3c_pixels.rs`），这里只是遵守它。
2. **设备丢失重试**：WARP 在扩容后报 `0x887A0005`（DEVICE_REMOVED）。
   重试前必须 `tree.invalidate_all()`（上一轮 render 已消费脏区，否则重试只会得到
   `Skipped`）。这条修复保留在用例里。
3. **但仍会 HEAP_CORRUPTION**（单用例也复现）→ 定位为**WARP 专有**：
   本地真实 GPU 跑同一用例全绿（"实例缓冲扩容 10000 → 20000" + `drawn == pushed`
   + 尾部像素为红）。在 WARP 上崩溃点未定位，因此该用例改为
   **默认 `#[ignore]`**（仓库对需要真 GPU 的用例一律如此），默认门禁只保留
   纯函数 `capacity_for_rounds_up_to_powers_of_two`。

**~~遗留风险~~ → 已定位并修复（2026-11-27 续六）**：当时猜的两个方向都不对。真正原因是
`rebuild()` 之后"容量记在 `Gfx`、缓冲在 `Pipeline`"二者不同步，`copy_nonoverlapping`
写过了映射区尾部。详见本文顶部「续六」；同一段落里的"`#[ignore]`"结论保留（CI 无 GPU）。
## 2026-11-27：PLAN 批次 6 前半 —— 图集面积溢出加固 + blend 限制显式记录

两项都属"审计已点名、但当时没修"的项（`docs/AUDIT_2026-11-25.md` #11 #12）。

### A. 图集像素数用 `u64` 计算（审计 #12 前半）

`Atlas` 里三处 `size * size`（初始化缓冲、`occupancy()`、`compact()` 的新缓冲）都是
**`u32` 乘法**：`size > 65535` 时溢出（`65536² == 0`）→ 分配出一个几乎为空的缓冲，
随后字形写入越界/错位，画面整片花屏**且没有任何错误日志**。
统一改用 `pixels_of(size)`：`u64` 计算 + `usize` 转换 + 越界 `debug_assert`。
新增用例 `pixels_of_does_not_overflow_u32` 直接钉住 65536 / 100000 两个边界。

### B. `blend()` 的适用条件写清楚（审计 #11）

该函数是"目标不透明时正确"的 source-over：省掉了 `dst_c * da` 与 `/ out_a` 两步。
软后端每帧 `clear()` 成不透明清屏色，所以**正常渲染路径上恒等且正确**；只有把半透明
像素画到**已经半透明**的缓冲上（离屏合成）才会偏亮。
修它要按像素做除法、影响整屏填充率（当前整屏文字 1.24 ms），因此在没有离屏合成
需求前**保持现状并显式记录**，而不是偷偷改公式让像素基线漂移。

### C. 修正一处与实现不符的注释（审计 #12 后半）

`atlas.rs` 原注释称"像素区域由 packer 复用" —— `SkylinePacker` 只增不减、没有 free，
被丢弃的区域是**永久空洞**，只能靠 `compact()` 回收。已按实现改写。

### 未做（明确）

- `cluster_boundaries` 的分配优化：全仓 grep 确认它**没有热路径调用方**
  （`x_for_offset` 之类并不存在），属于"审计时的推测性条目"，本轮不做无收益重构；
- `Theme.slots` 死数据、`fasthash` 文档、`ImageFit::None` 越界、`Query ~=` 布尔键、
  若干过期文档 —— 归入批次 7（卫生），尚未处理。
## 2026-11-26（续）：PLAN 批次 4 —— 浮层命中 / 光标 / 快照可见性

浮层（`UiTree::popups`：菜单、下拉、Tooltip、Dialog）**不在普通节点索引里**
（不参与树布局，`paint_order` 看不到它们），而 `hit_test` / `cursor_at` /
`snapshot_json` 都只查树。三条用户可见缺陷随之而来：

1. 浮层盖住背景时命中返回**被遮挡的背景节点** → RPC/自动化点"菜单项"点到下面；
2. 菜单项声明的光标不生效（看到的是背景的 `Default`）；
3. RPC `get_tree` 里**看不到菜单项** → 外部工具无法驱动菜单。

修法：
- `popup_hit(pos)`：按开启顺序**逆序**找第一个包含该点的浮层（后开的在上），
  再在它子树里用 `deepest_at()` 取最深命中（后序优先 = 后画在上）。
  浮层的 `arrange` 用的就是 `place_popup` 返回的**屏幕矩形**，所以不需要坐标平移。
- `hit_test`：浮层优先；**存在模态浮层时，浮层外的点返回 `None`**（事件被吞，
  不透传到底层）—— 与 `dispatch` 的模态语义保持一致。
- `cursor_at`：与 `hit_test` 同一套优先级（否则光标会指向"点不到"的元素）；
  模态浮层外部保持 `Default`（不暗示可点）。
- `snapshot_json()`：新增**独立的 `popups` 数组**，不混进 `children` ——
  已有消费者按 `children` 遍历树节点，混进去会让菜单被当成普通子控件。

- 新增 4 条用例（`crates/y901-ui/tests/popup_hit.rs`）：浮层命中不透背景、
  光标跟随浮层、模态浮层吞掉外部点击、快照含浮层子树。
- 反向验证：`hit_test` 不看浮层 → 前两条红；`snapshot_json` 不带 `popups` → 第四条红。
- **未做**：`snapshot()`（AccessKit 用的 `NodeInfo`）仍不含浮层 —— 那属无障碍批次，
  与本文的 RPC/命中路径分开处理，避免一次改两类消费者。
## 2026-11-26（续）：RENDER-BUG-01 / issue #7 —— 实例超限不再静默截断

旧行为：`let count = instances.len().min(INSTANCE_CAPACITY)`，第 10 001 个实例往后
**整块消失**（画面"后半帧控件没了"），只有一条进程级 `eprintln` 告警，默认没人看得到。

现在：`Gfx::ensure_instance_capacity()` 在提交前把实例缓冲**按 2 的幂扩容**
（`capacity_for()`：不小于 `INSTANCE_CAPACITY`，向上取 2 的幂 ⇒ 每翻一倍才重建一次），
控制流不变；只有扩容失败才退化为截断，并且**一定**打印告警。

为什么扩容而不是分批提交：`Batch::start` 是**全局实例下标**，
`draw_instances(n, b.start)` 直接把它当 first-vertex 用；分批必须重映射所有
batch 区间，改动面大且容易引入"画错区间"这类难查的 bug。

- 新增 3 条 Windows-only 用例（`instance_overflow.rs`）：
  - `capacity_for_rounds_up_to_powers_of_two`（纯函数，无需 GPU）；
  - `every_instance_is_drawn_when_the_count_exceeds_capacity`：造 10 001 个实例，
    断言 `instance_drawn == instance_pushed`（旧的 `min()` 抛掉 1 个）；
  - `pixels_from_beyond_the_old_capacity_are_actually_present`：把标记块故意排到
    实例流末尾，回读客户区右下角像素必须为红色 —— **像素级证明**超限部分真被画出来。
- 反向验证：把扩容关掉（`capacity_ok = true`）后，后两条用例分别报
  "推入 10001 个，只画了 10000 个" 与 "右下角实际 (5,5,8)"，正是 issue #7 的症状。
- 顺带修正 `render_stats_enabled.rs` 里"绘制数不得超过 INSTANCE_CAPACITY"的旧断言：
  扩容易后它不再成立，正确不变量是"推入多少就画多少"。
## 2026-11-26（续）：RenderStats A 阶段第 6 步 —— Win32 `--force-soft` 软路径埋点

软渲染路径以前**完全不交付统计**：`render_inner` 在软分支里直接 `gdi_present` 后 return，
`render_stats_backend()` 又只在 `backend.is_some()` 时返回 `D3d11`。于是
`--force-soft` 下 `take_render_stats()` 恒为 `None`，与"统计关闭"无法区分。

现在：

| 阶段 | 起止 |
|---|---|
| `render_prepare` | `tree.invalidate_all()` + `soft.clear()` |
| `cpu_raster` | `tree.render(soft)` |
| `present_blocking` | `gdi_present()`（`StretchDIBits` 是阻塞调用） |

- 统计对象由 **`Window` 自己持有**（`soft_stats`）——软路径根本没有 `D3dBackend`，
  两条路径在同一 `Window` 里二选一；`render_stats_backend()` 现在二选一自报
  `D3d11` / `Soft`，`take_render_stats()` 也从对应持有者取。
- 跳过帧也 `stats_begin_frame` + `frame_skipped=true`（与 D3D 路径一致），
  这样基准能区分"跳过"与"未接入"。
- 计数器：`damaged_pixels` = **实际**客户区面积（不是请求尺寸 —— 本机 150% DPI 下
  360×200 的请求得到 450×250 的客户区，统计报的是后者，这是正确的）。

- 新增 1 个 Windows-only 测试目标 / 2 条用例（`render_stats_soft.rs`，自己设
  `Y901_FORCE_SOFT=1` + `Y901_RENDER_STATS=1`）：三个可测阶段都被测量；
  六个不存在阶段必须 `NotApplicable`；`command_gen`/`compositor` 必须
  `Unavailable` 且原因非空；计数器自洽；三段之和 ≤ 墙钟；`presented` 恒 false；
  `validate()` 通过；一次性消费。第二例钉住"跳过帧不得有阶段测量值、且不污染下一帧"。
- 反向验证：删掉 `present_blocking` 打点 → 用例变红（文件以 SHA256 校验恢复）。
- Linux 侧：`linux-verify.sh` 全量确认无回归（软路径代码是 Windows-only）。
## 2026-11-26（续）：RenderStats A 阶段第 5 步 —— Web 上屏四段埋点

`Present`（Web 的 canvas 呈现器）现在持有统计对象并实现上屏四段，`WasmWindow`
实现 `render_stats_backend()`（自报 `BackendKind::Web`）与 `take_render_stats()`：

| 阶段 | 起止 |
|---|---|
| `render_prepare` | `soft.clear()` + `tree.invalidate_all()`（在 `WasmWindow::render` 里） |
| `cpu_raster` | `tree.render(self.soft)` |
| `pixel_convert` | `bgra_to_rgba()` 通道交换 |
| `image_alloc` | `ctx.create_image_data(w, h)` |
| `pixel_copy` | `image_data.data().set_from(&rgba, 0)`（wasm 内存 → ImageData 的批量拷贝） |
| `canvas_blit` | `ctx.put_image_data(0, 0)` |

计数器：`damaged_pixels` = 画布面积（Web 每帧整窗重绘）、`dirty_rect_count` 按 `is_full()` 记 1、
`atlas_upload_bytes`/`instance_*`/`batch_count` 写 0（CPU 后端确实没有）��

**统计对象为什么放在 `Present` 而不是 `WasmWindow`**：后者是借用式薄包装
（`&mut SoftBackend` / `&mut Present`），给它加字段会改变公开结构体的字面量构造方式；
`Present` 由 `WasmState` 长期持有，是天然所有者。

**借用冲突的解法**：`present()` 里 `ctx` 借自 `self.ctx`，同时要写 `self.stats`；
先把四段收集成值 `UploadStages`，再由**纯函数** `record_upload(&mut Option<RenderStats>, UploadStages)`
统一写入 —— 这也让映射关系可以在**宿主机**上单测（真正的 canvas 调用没有浏览器跑不了）。

- 新增 3 条宿主机用例（`y901-backend-web` 现在可在 host 跑测试）：四段各写进自己的
  `StageId` 且不误写 `upload`；统计关闭时 `record_upload` 是**纯 no-op**（不凭空造对象）；
  四段并存不互相覆盖。
- 反向验证：把 `canvas_blit` 写成 `pixel_copy`（映射错位）→ 两条用例红；
  把 no-op 改成 `get_or_insert_with` → no-op 用例红（文件随后恢复）。
- 已确认未验证项：**真实浏览器内的数字没有采集**（本环境无浏览器），
  wasm 侧只做到 `cargo check/clippy --target wasm32-unknown-unknown` 通过。
## 2026-11-26（续）：RenderStats A 阶段第 4 步 —— X11 分阶段埋点

`X11Window` 现在实现 `WindowBackend::render_stats_backend()`（自报 `BackendKind::X11`）
与 `take_render_stats()`（取走语义），并按设计表 §2 的 X11 列打四个点：

| 阶段 | 起止 |
|---|---|
| `render_prepare` | `invalidate_all()` + `soft.clear()` |
| `cpu_raster` | `tree.render(&mut soft)` |
| `pixel_copy` | `frame.extend_from_slice(soft.pixels())`（上屏前的暂存拷贝） |
| `upload` | `present()` 内部：SHM 写入 / 分块 PutImage + flush（X11 的上传与 Present 是同一个动作） |

计数器：`dirty_rect_count`（`DirtyRegion::is_full()` 记 1，否则取矩形数）、
`damaged_pixels`（整窗重绘 = 客户区面积）、`atlas_upload_bytes`/`instance_*`/`batch_count`
一律写 0（CPU 后端没有这些量，**写 0 是"确实没有"，不是"测得 0"）。

**顺带修正一处能力表错误**：A1 初稿把 `PixelCopy` 归为"只在 Web 后端存在"（NotApplicable），
但 X11 每帧确实要把 `soft.buf` 拷到 `window.frame` 才能上屏（1080p 约 8.3 MB/帧）。
`BackendKind::X11` 改为可测，并同步修正 `render_stats::tests` 里那条按旧表写的断言。

- 新增 2 个测试目标 / 3 条用例：
  - `render_stats_x11.rs`：四段都被测量；`compositor`/`command_gen` 必须 `Unavailable`
    （带原因）；`gpu_execute`/`backend_submit`/`present_blocking` 必须 `NotApplicable`；
    计数器自洽；四段之和 ≤ `window.render` 墙钟；`presented` 恒 false；`validate()` 通过；
    一次性消费；以及"跳过帧不得有阶段测量值、且不跨帧污染"。
  - `render_stats_x11_disabled.rs`：关闭统计时 `take_render_stats()` **恒为 `None`**
    （不是"有值但全 0"），且渲染行为不变（Presented / Skipped 判定照旧）。
- 反向验证：删掉 `pixel_copy` 打点 → 对应用例变红（文件以 SHA256 校验恢复）。
- 失败帧不交付统计（`present` 出错时 `stats = None`），与 D3D11 一致。
- Linux 全量（Xvfb）：X11 crate 全绿（计数以 `docs/evidence/linux-numbers.txt` 为准）；
  `linux-verify.sh` 每一步都 `EXIT_0`。
## 2026-11-26（续）：X11 传输层最后两项收口

### C. `poll_events` 错误路径丢弃已解码事件 —— 已修

`poll_events_inner` 先 `let mut events = mem::take(&mut self.pending)`，再用 `?` 传播错误。
`events` 里既有上一次遗留的 `pending`，也有本次刚解出的 `WM_DELETE_WINDOW`
—— 错误返回时随栈帧销毁，用户点关闭无反应。现在拆成
`poll_events_inner`（负责"出错就把事件放回 `pending`"）+ `poll_events_drain`
（只负责解析），并加 `Connection::pending_events_len()` 让"归还"可被断言。

### D. 编码器 `length as u16` 静默截断 —— 已修

以前每个编码器各写一行 `(X + pad4(n) / 4) as u16`，超限时 `as` 静默回绕，
服务器按错误长度切包 → 整条流错位。现在统一走 `length_units()`：
debug 下 `debug_assert!(request_fits(..))`，release 下 `try_into().unwrap_or(u16::MAX)`
（**不再回绕成小值**）。覆盖 8 处：`intern_atom` / `query_extension` /
`put_image` / `change_property` / `create_window` / `create_gc` / `change_gc` /
`configure_window`；另外补了 `SetupRequest` 两个 u16 认证字段的长度断言。

顺带修掉一个**真实溢出缺口**：`put_image_bgra` 用
`(PUT_IMAGE_MAX_BYTES / stride).max(1)` 分块，窗口宽 > 65529 px 时一行就超过单请求上限，
`.max(1)` 会把整行塞进一次请求。现在直接 `Err`（调用方可降级）。

`PUT_IMAGE_MAX_BYTES` 从 `conn`（`#[cfg(unix)]` 才编译）移到 `encode`（各平台都编译），
`conn` 保留 `pub use` 维持既有公开路径；因此这 5 条长度用例在 Windows 上也真跑。

- 回归用例 8 条：`encode::length_guard_tests`（5）、`poll_events_keeps_pending_events_when_it_errors`、
  `poll_events_delivers_and_clears_pending_on_success`、`put_image_rejects_a_row_wider_than_one_request`。
- 反向验证：分别回退"归还 pending"、`request_fits` 上限、`.max(1)` 分块，
  各导致对应用例变红（随后按 SHA256 校验恢复）。
## 2026-11-26：X11 连接层收尾 + SoftBackend 图集帧边界（RENDER-BUG-02 / issue #8）

### A. SoftBackend 从未推进字体图集生命周期 —— 已修复

`MyFontEngine` 的 `begin_frame` / `take_dirty` **只有 D3D11 后端调用**，软渲染三条路径
（X11、Win32 `--force-soft`、Web）都没有入口，于是同一份引擎在两种后端行为分叉：

1. `frame` 恒为 0 → `evict_one` 的热集判定 `last_used < cutoff`（`cutoff = frame - 512`）
   永远为假，只能走"全表最冷"回退；`evicted_last_frame` 也从不归零；
2. `dirty` 只增不减（每个光栅化过的字形一条矩形，永不释放）。

**修法**（不改任何后端调用点）：帧边界收到 `RenderCtx::begin_frame`——`UiTree::render`
在每次渲染前调一次（默认空实现，GPU 后端与测试 mock 不受影响），`SoftBackend`
在其中推进帧号并消费上一帧脏区。这样三条软路径自动都接上了。

- 新增诊断：`MyFontEngine::frame()` / `pending_dirty_len()`、`SoftBackend::font_frame()` /
  `font_pending_dirty()`（与既有 `occupancy` / `evicted_last_frame` 同风格）。
- 回归用例 `crates/y901-render/tests/soft_atlas_lifecycle.rs`（4 条）：帧号每帧 +1、
  脏区每帧被消费、30 帧不累积、**光栅化量超过图集容量后脏区仍 ≤1/帧**。
- 反向验证：分别删掉 `begin_frame()` / `take_dirty()` 调用后，对应用例逐条变红。
- 像素基线 `0xF7455CCE7781A864` **未变**；`soft_fill_rate_1280x800` 仍在原量级
  （text 1.24 ms / fill_rect 0.09 ms）。
- **为什么不测"淘汰优先级"**：图集满后 `insert_into_atlas` 会整表重打包，小图集用例
  被重打包主导；而"不推进帧号"的对照实现里所有 `last_used` 并列，淘汰靠
  `HashMap` 迭代顺序，断言必然 flaky。帧号是否推进已由用例直接钉住。

### B. X11 连接层三轮收尾（详见提交 `1576cbf` / `0aae0e1` / `e399eb9`）

半包重组与 `next_packet(0)`、`get_property` 的 `bytes_after` 分页、窗口 resize 落库、
阻塞/非阻塞读取的总 deadline、GenericEvent 附加数据 drain、`AnyPropertyType`
分页 type 一致性、`delete` 语义。Linux 计数从 1054 → 1223（同一台 Xvfb）。

## 2026-11-24：内核优化第四~六轮（全部有同机 A/B 与像素金标）

这一轮的规矩：**先量再改**，每处改动都做同机 A/B（`git stash` 前后各跑数次取中位数），
像素级改动一律用"旧实现录 `frame_hash` → 新实现逐位比对 → 钉成金标"验证。

| 项 | 优化前 | 优化后 | 证据 |
|---|---|---|---|
| Grid 整帧渲染（100×100 网格，含索引重建） | 59 300–60 900 µs | **203–215 µs** | `profile_grid_in_tree`；`children_clip` 的 O(n²) 线性查找改游标顺序查找 + 裁切矩形缓存 |
| `grid_perf`（10k，measure+arrange） | 1 196 µs | **770 µs** | 前缀和替代逐子累加、`auto_cells` 惰性 + 扁平占用位图 |
| 软光栅整屏渐变（1280×800） | 4.50–4.53 ms | **0.09–0.10 ms** | `gradient_golden.rs` 四个金标，新旧逐位相同 |
| 软光栅圆角矩形（400×300，SDF） | 0.68–0.72 ms | **0.09 ms** | `round_rect_golden.rs` 四个金标（含分数坐标/半透明/r=40） |
| shaped 文本量宽 | 按格数（`"שלום"`=32px） | **按 shaping（36.76px）** | `shaped_measure_matches_draw.rs`：量宽必须等于绘制推进 |
| shaped 文本绘制 | 每帧克隆整份字体（≈1 MB） | **零克隆** | `shaped_no_font_copy.rs`（计数分配器） |
| 软光栅整屏文字（40 行 × 55 字符） | 2.94–3.12 ms | **1.26–1.53 ms** | 覆盖率查表 + 1:1 直贴；`font_3c_pixels` 哈希未变 |
| 整帧渲染（11k 节点，空 RenderCtx） | 155.6–168.4 µs | **115.6–124.5 µs** | 整帧改走稠密 `paint_order`；`render_pass` 只留给浮层 |
| 索引表 | `rects` + `node_ink_margin` 两张表 | **一张 `nodes`** | 每节点一次插入/查询；消掉"墨迹表漏写回"这类不同步 |
| 文本度量 | 只有单行 `measure_text` | **新增 `measure_with(avail_width)`** | 默认实现就是渲染端 `wrap()`，`TextLabel` 已改用它；量=画由构造保证 |

细节与复跑命令见 [PERFORMANCE.md §8b](PERFORMANCE.md)。像素基线 `0xF7455CCE7781A864`
全程未变，真机 GPU 快照 10/10。
## 2026-11-19：第二轮审查收尾 + Grid+Image 整帧空白修复

### A. Grid + Image 在 D3D 下整帧空白 —— 已修复（根因：draw 参数传错位）

- **根因**：`Context::draw_instances` 把批次起点传给了 `DrawInstanced` 的第 4 参数
  `StartVertexLocation`；实例缓冲是按实例步进的槽位，正确参数是第 5 个
  `StartInstanceLocation`。于是图片批次用 `ps_image` 重画了**实例 0**（背景），
  后续每个批次也都从实例 0 开始 —— 整帧被涂成错误颜色，看起来"只剩清屏色"。
- **影响面**：只在"有图片批次"时出现；Flex+Image（单批次）与 Grid+文字（无图片批次）
  都正常，因此长期没被发现。
- **修复**：`draw_instances(count, start_instance)` → `DrawInstanced(6, count, 0, start_instance)`；
  同时把 `Y901_SNAPSHOT=1` 的确定性自检改成复用 `draw_scene`（原先自己拼"无批次"重画，
  含图片的帧必然 hash 不等，是假红）。
- **证据**：同一棵树快照从 **1 种颜色**（`(12,0,0)` 整帧）变为 **62 381 种**，
  四象限主色各 9k~15k 像素；`Y901_SNAPSHOT=1` 两次重画哈希相等（`0xddf6c99d30ffc70c`）。
- **回归**：[grid_image_gpu.rs](../crates/y901-backend-win32/tests/grid_image_gpu.rs)
  （Grid + 4 图片 + 文字；无 GPU 时打印跳过）。
- **诊断开关（保留）**：`Y901_DEBUG_BATCHES=1` 打印批次切分与每次 draw 的实例区间；
  `Y901_NO_IMAGE_BATCH=1` 强制单次 draw，用于二分"批次 vs 实例"。

### B. 第二轮审查的 P0 收尾（Linux + 供应链）

- `y901-extras` 的 `sgr_reverse` 两条红改为**格内背景众数**断言：仍是精确 RGB，
  但对字体覆盖范围不敏感（Windows Segoe UI / Linux DejaVuSans 双绿）。
- 新增版本化脚本 [scripts/linux-verify.sh](../scripts/linux-verify.sh)：x11 `check`
  （all-targets）/ `golden` / `xvfb-run real_window` + `stroke_round_aa` +
  `sgr_reverse` + `ui+widgets`，任一失败 exit 1。
- Linux 全量（历史记录）：见 [linux-full-2026-11-19.log](evidence/cross-platform/linux-full-2026-11-19.log)
  （当时 885 通过 / 0 失败 / 27 忽略）。当前值见上方 2026-11-20 验收，
  差异来自本轮新增的 X11 用例（Xft.dpi 协议读取、DPI 优先级）。
- `cargo audit` 恢复**真实联网刷新**（不再 STALE）：0 漏洞。
- 本机 git 全局配置里指向 `127.0.0.1:7897` 的死代理已清除（备份
  `~/.gitconfig.bak-20261119`），`main` 与全部 tag 已推送。


### D. 第三轮复审（2026-11-19）：A-1 / A-2 与一批文档/脚本卫生

**A-1 大字号局部重绘残影（上轮"统一脏区"的遗漏分支）—— 已修**

- **根因**：`UiTree::layout()` 只用 `TextStyle::default()`（16px）算**一个**树级
  `ink_margin`（实测 4.63px），然后把这个常数套到所有节点上；而
  `TextLabel::size(24/32/48)` 的真实溢出是 6.95 / 9.27 / 13.90px。
  第二轮复审用 SoftBackend 探针实测差异像素：24px **72**、32px **136**、48px **312**。
- **修法**：新增 `Widget::ink_overflow(metrics) -> Option<f32>`（默认 `None` = 用树级值），
  `TextLabel` / `StyledText` / `TextInput` 按**自身样式**返回；
  `UiTree` 在布局后顺手采集 `node_ink_margin: id → margin`（零额外遍历），
  并新增 `add_dirty_rect_for(id, rect)`；`invalidate(id)` 与 `mark_moved`（新旧矩形）
  全部改走 per-node 版本。外扩算法抽成 `ink_margin_for()`，树级默认与 per-node 共用。
- **回归**：`node_invalidate_ink_margin` 覆盖 **16/24/32/48px**（同宽同高 j→o、
  只 `invalidate(id)`、局部清屏 vs 整窗重绘逐像素比对），并断言脏区外扩量随字号增长；
  另有"树级默认对 48px 不够"的反向对照 + `StyledText` 最大 span 边距断言。
  **变异验证**：把 `TextLabel::ink_overflow` 改回 `None`，24px 立刻红（first diff 在 24px 用例）。
- **已知边界**：`StyledText::render` 自己 `push_clip(self.rect)`，其墨迹不可能越界，
  所以富文本只做"声明边距"的单元断言（像素级测不到）；将来若取消自裁剪，
  per-node 边距已经就位。

**A-2 `sync-doc-numbers.ps1` 在吃 `0x` 前缀 / 词内数字 —— 已修**

- **现象**：`(基线\s*)(\d{1,4})` 把 `基线 0xF7455...` 的 `0` 换成 `**N**`
  （→ `**N**xF7455...`）；`(?=默认)` 规则把 `win32 默认族` 的 `32` 换成 <!-- counts-exempt -->
  `win***1378** 默认族`。两处都是**脚本制造的脏数据**，而 `-Check` 当时完全看不见。 <!-- counts-exempt -->
- **修法**：两条规则都要求数字是**独立 token**（`(?<![\w.])…(?![\w.])`），
  基线规则额外**整体跳过 `0x…`**；`-Check` 增加"反自伤"规则
  （`**N**xF…` / `字母**N**` 直接判 stale）；自动改写也遵守同一套 token 边界。
- **自证**：`test-doc-numbers-check.ps1` 从 5 条变异扩到 **7 条**，新增
  "0x 被吃"与"win32 → win**N**"两类，全部被抓到；恢复后 `-Check` 通过。
- **数据修复**：README 基线恢复为 `0xF7455CCE7781A864`，STATUS 恢复 `win32 默认族`。

**B 批（文档/工程卫生）**

| # | 问题 | 处理 |
|---|---|---|
| 3 | `empty_clipboard_maps_to_none` 恒真（match 三分支都通过） | 改为**注入假剪贴板**的确定性断言（空串→None、非空→原样），另加"不可用→None"用例 |
| 4 | 门禁审计降级无痕迹 | 缓存库 ≤7 天时结论打印 `GATE OK (CACHED DB N.Nd; offline audit)`，`audit.log` 标题行加 `cached_db_days=` |
| 5 | 仓库跟踪 `scripts/__pycache__/*.pyc` | `git rm --cached` + `.gitignore` 增加 `__pycache__/`、`*.pyc` |
| 6 | `.gitignore` 第 37 行是 GBK 非法 UTF-8 | 逐行解码修复为 UTF-8（唯一非法行已还原中文注释） |
| 7 | 55 行字面 `\uXXXX` 注释 | 解码 `profile.rs` 16 / web `lib.rs` 13 / `bindings.rs` 18 / `keys.rs` 1 / 两个 `.ps1` 6 行；仅保留 STATUS 里作为**示例**的那一处 |
| 8 | `FRAMEWORK.md` 6 处"浯层"错字 | 全部改回"浮层" |
| 9 | ACCEPTANCE GPU harness 口径互相矛盾 | 标题改为"GPU 像素验收用例"，并写明三个口径（8 条断言 / `snapshot_harness` 10 条 `#[ignore]` / 全仓 `ignored=71`）；STATUS 的文档债标为已修 |
| 10 | `runtime.rs` 注释与实现脱钩（还在讲"自节流/上一帧呈现耗时"） | 改成 `last_tick` 差值口径，并写明旧 `last_frame_ms` 恒 0 的事故；`profile.rs` 注明 p50/p95 已改为**纯工作耗时**，不能与旧数字直接比 |

**C 批（测试灵敏度）**

- `d3d_text_scaling` 的墨量比/像素比区间从 `0.12..=0.45 / 0.15..=0.50` 收到
  `0.30..=0.44 / 0.33..=0.47`（本机实测 0.372 / 0.400，CPU 对照 0.246 / 0.339），
  并注明 gamma 校正使 D3D 侧天然偏高 ~1.5×。
- 新增富文本（`StyledText`）最大 span 边距的单元回归（见 A-1）。

### C. `y901-shell` 剪贴板在 Xvfb 下的偶发红 —— 已修（两把锁各锁一半）

- **现象**：`xvfb-run cargo test -p y901-shell --lib` **20 次里 17 次红**，
  失败的都是 `widget_clipboard::tests::*`（`适配器应原样转发` / `空剪贴板 == None`）。
- **定位**：三个用例各自单跑 10 次全绿 → 不是用例逻辑错，而是**并发**：
  `clipboard.rs` 与 `widget_clipboard.rs` 各有一把 `CLIPBOARD_LOCK`，
  锁不共享，于是在同一二进制里并行时互相覆盖系统剪贴板（进程外的全局资源）。
- **修法**：
  1. 锁提升到模块级 `clipboard::CLIPBOARD_TEST_LOCK`（`#[cfg(test)] pub(crate)`），
     两个测试模块共用同一把；
  2. `widget_clipboard` 的读回改成"写入 → 等待 40ms → 读，最多 5 次"，
     每次重试前重新写（X11 selection 所有权可能被抢走，只重读拿不回来）；
  3. 环境确实不支持（写失败 / 外部客户端接管）时打印跳过，不伪装成通过也不假红。
- **验证**：修后同一命令 **20/20 全绿**；`xvfb-run cargo test --workspace`
  连续 **3/3 全绿**（此前每次都可能撞到那两条）。


## 本轮（2026-11-18）：P0 渲染修复 + 门禁恢复

**四个 P0 修复**（每个都有像素证据，见各提交说明）：

1. `fix(table)` 表头/行绘制与命中顺序：表头在数据行之上，点第 N 行不再选中第 N-1 行。
2. `fix(render)` `GlyphInfo::bearing_y` 统一为 y-up 位图底边偏移 + 基线随字号缩放
   （同一表单行内 14 号标签与 16 号输入值底边同为 y=365）。
3. `fix(render)` L0 分数缩放采样补最右/最下一行，墨迹底边不再被切。
4. `fix(render)` **字形四边形整像素吸附**：位图 1:1 贴图 + 线性采样，落在分数像素上
   就会把相邻纹素混一次（125% 缩放下竖笔峰值亮度只有正文色的 57%）。修后
   文字峰值亮度 249→255、亮部像素 +13%、边缘梯度 p95 +12.8%。

**门禁恢复**（此前门禁红在第 1b 步，没人看）：

| 项 | 修前 | 修后 |
|---|---|---|
| `cargo fmt --all -- --check` | 69 个文件不符 | 0（独立 `style:` 提交，纯空白/排序） |
| 测试基线 | 停留在很早的旧值（严重过期） | 重新实测并写入 `test-baseline.txt` |
| doc 数字 | 文档里的计数长期未刷新 | 由 `sync-doc-numbers.ps1` 重新生成 |
| test-inventory | 指纹过期 | 127 个二进制 / 1013 条用例 |
| `missing_docs`（`-W missing_docs` 构建） | web 后端 33 条 `pub` 项无文档 | 0 |
| `sync-doc-numbers.ps1 -Check` | `$baseline` 在 `-Check` 分支从未赋值 → 文档里写"基线 **N**"必判 stale | 从 `numbers.txt` 读回，缺行即失败 |
| GPU 用例 `invalidate_rect_partial_equals_full` | 125% DPI 下红 | 绿（脏区按"实际字形高度"标，见测试注释） |

门禁现状：`scripts/gate.ps1`（含 audit）→ **GATE OK**（exit 0）。

**文字清晰度收尾（B）**：见 [`docs/design/text-sharpness.md`](design/text-sharpness.md)。
- gamma 校正 A/B 已量化：`pow(cov, 1/2.2)` 把整帧亮度抬 +5.8%、亮像素数抬 +26%，
  **保持默认不动**（风格取舍，非正确性），只留 `Y901_GAMMA` 做对照；
- 与 GDI 的差异定位为**渲染模式差异**：GDI 在 16px 用字体点阵位图（无抗锯齿），
  我们走 DirectWrite 轮廓 + ClearType 抗锯齿，小字号必然有过渡像素；
- 守卫用**单元测试**（`glyph_geometry_tests` 三条），已验证去掉 `.round()` 会立刻红；
  曾写过一条像素比值用例，实测**抓不到回归**（开关两种实现输出逐像素相同），已撤，
  理由写在设计文档里。


**硬件用例证据（D）**：新增 `scripts/hw-smoke.ps1`，把默认套件不跑的 **60 条 `#[ignore]`**
（GPU 回读 / 真实窗口 / DPI 探针）真跑一遍并落盘：`docs/evidence/hw-smoke-2026-11-18.log`
（**PASS 60 / 0 failed / 0 SKIP**，62s，HEAD `9a8060d`）。
口径写进日志头：SKIP ≠ 通过；本机 GPU 是 **GameViewer 虚拟适配器**，
功能正确性可引用、帧率/显存类结论不可引用。


**双后端一致性（D 收尾）**：新增 `crates/y901-backend-win32/tests/backend_parity.rs`
（`#[ignore]`，需 GPU）。同一棵树、**同一个字形引擎与字体文件**、DPI 固定 1.0，
分别在 D3D11 与 CPU `SoftBackend` 上渲染混合字号三行文字，断言：

- **墨迹包围盒逐边对齐**（±1px）与**逐行墨迹带完全相同** —— 这是"错一行"的直接判据
  （当初 `bearing_y` 语义分叉就是这个症状，而 MAD 对整体平移不敏感，抓不住）；
- 墨量比 [0.4, 2.5]（GPU 侧 gamma 校正会让它天然高约 1.5×，属已知有意差异）；
- 逐像素 MAD ≤ 0.25（实测 **0.0080**）。

实测：两侧行带 `[(18,32),(62,77),(111,122)]` 完全一致，包围盒 `(16,18)..(185,122)` 相同。
调试中踩到并记录：两侧若用**不同字体文件**（`default_test_font_path()` vs win32 默认族
NSimSun→simsun.ttc），hhea ascent 差 3.5px，会得到整行偏移——那是字体不同，不是后端错行。


## 审查整改（2026-11-18 第二批：P1-1/2/3/4 + P2-7/8/9/10）

针对代码审查报告逐条修复，每条都有回归用例或可验证的机制说明：

| # | 问题 | 修法 |
|---|---|---|
| P1-1 | `Flex` 四个 setter 的"后设置者生效"不成立（`*_fixed`/`*_scaled` 互不清空，而 fixed 优先） | 四个 setter 两两互斥清空；新增 **6×2 条顺序用例**（padding 家族 + gap 家族各 6 种顺序） |
| P1-2 | mip 路径把"该级 bearing"与"L0 几何"混用（下伸/overshoot 在 s<0.5 被缩小 2~4 倍） | bearing 先乘 `h / slot.height` 换回 L0；**几何改到映射后空间**计算（与 soft 同构）；有成对比例断言保证顶部与尺寸用同一取整高度 |
| P1-3 | mip 路径假粗体第二实例用未裁剪的 `pos/size/uv` | 改用裁剪后的 `quad.*`，与 sized 路径一致 |
| P1-4 | soft 的 `level>0` 按 slot 尺寸画、却按连续 `scale` 推进 | 统一"目标尺寸 = L0×scale"；**目标与位图尺寸差 ≤1px 时走 1:1**（否则会多一次半像素重采样，把形状 MAD 从 0.03 抬到 0.08） |
| P2-7 | `backend_parity` 只打印行带不比较、无 GPU 时静默绿灯 | 行带**直接 `assert_eq!`**（L0 三行严格相等；0.3 缩放行容差 ±1 并注明原因）；无 GPU 写 `SKIP:` 记账 |
| P2-8 | `hw-smoke.ps1` 的 `NO-OP` 退出码仍是 0 | 新增 `exit 3`：一条用例都没跑到 ≠ 通过 |
| P2-9 | `glyph_top` 在 soft/win32 各一份 | 抽到 `y901_render::font::glyph_top`，两端共用；用例不再手抄公式 |
| P2-10 | mip 路径尺寸仍是分数 | 四边形尺寸取整（位置与尺寸同时吸附） |

**新发现的真问题**（比报告多出来的）：mip 路径的基线在"未缩放空间先相加再乘 k"与
"先各自乘 k 再相加"之间差 `(k-1)·(ascent − height)`，缩小时会让 GPU 与 CPU 错 1 行——
`backend_parity` 加了 0.3 缩放行后立刻抓到（D3D `(132,136)` vs CPU `(133,135)`），已改到映射后空间计算。

验证（**当轮口径**）：`cargo test --workspace` **963 passed / 0 failed**；
`--include-ignored` **1114 passed / 0 failed**；硬件冒烟 **61 passed / 0 failed / 0 SKIP**； <!-- counts-exempt -->
`scripts/gate.ps1`（含 audit）→ **GATE OK**。

仍待处理（下一批）：**P1-5** `line_height` 契约（需在"改 1em→ascent+descent"与
"新增 `baseline_offset()`"之间选）、**P1-6** 组件内部 padding 未随 DPI 缩放、
**内容清理**（README/ACCEPTANCE/WASM/STATUS 过期数字与旧哈希、`FRAMEWORK.md` 控制字符、
`clippy.txt`+`todo.tsv` 移出仓库）。

> **收口（2026-11-19）**：这三项均已完成 —— P1-5 见下方"P1-5 收口"与第二轮审查补齐；
> P1-6 由 `d5b8c48`（`TextInput` padding 随 DPI 缩放）+ `VerticalList`/`Flex` 同一口径落地；
> 内容清理在第二轮审查中完成（README/ACCEPTANCE/WASM/STATUS/FRAMEWORK 全部对齐，
> `clippy.txt` / `todo.tsv` 已进 `.gitignore` 且不再跟踪）。


**P1-5 墨迹深度（部分完成）**：新增 `TextMetrics::ink_bounds(style) -> (above, below)`
（默认实现 = `(0, line_height)`，向后兼容）+ `FontEngine::descent()`
（DWrite metrics / ttf hhea，O(1) 不扫字形）。实测本机 CJK 字体 16px：
**行高 16、墨迹深度 21.28（溢出 5.28px）** —— 这就是 `invalidate_rect` 当初要靠加宽测试脏区才绿的根因。

**为什么不直接拿它定高**：hhea 的 `ascent+descent` 在该字体上是 **1.33em**，
按它定高会让所有 `TextLabel` 高 33%（全站密度崩、像素基线全变）——
属于"修一个脏区、赔掉整站布局"。所以本轮只落地**能力**（API + 后端实现 + 用例），
把墨迹外溢的消费点（`UiTree` 的多个标脏点改成按 `ink_bounds` 扩张）留到单独一轮
（**已在该文件下方"P1-5 收口"与第二轮审查整改中完成**）。
`line_height` 语义保持 = 行距，未改。


**P1-5 收口：脏区按墨迹外扩（本轮完成）**

- `UiTree` 在 `layout()` 里从 `metrics.ink_bounds()` 求"墨迹溢出量"，并在
  **所有局部标脏路径**统一外扩：`invalidate_rect()`、`invalidate(id)`、
  布局前后的新旧矩形并集（`mark_moved`）、Signal 失效、浮层关闭、广播矩形
  —— 全部经由同一个 `add_dirty_rect()` 辅助函数；外扩后裁回视口。
- **第二轮审查补齐**：早先只有 `invalidate_rect()` 入口外扩，节点级 `invalidate(id)`
  与布局移动路径仍按精确矩形标脏。于是"j → o"这种**同宽同高**的文本内容变更
  （125% DPI）会留下 164 个残影像素（行 56~71）。现在三条路径共用同一外扩，
  并有纯 CPU 逐像素回归 `node_invalidate_ink_margin`（去掉外扩时 96 像素不一致）。
- 外扩量 = `max(ink_depth − line_height, 2px)`。**为什么要有 2px 下限**：
  实测 SimSun 16px 的 `ascent+descent` 恰好等于行高（gap = 0），但真实墨迹仍会越过 1~2px
  （overshoot + 抗锯齿），名义度量预测不到。
- **收益可验证**：`snapshot_harness::invalidate_rect_partial_equals_full` 里那段
  "手工把脏区加宽 4px"的补丁**已删除**，用例回到最朴素的窄矩形写法并通过
  —— 即框架的脏区兜底真的接管了这个 bug。
- 受影响的精确脏区断言按新契约收紧为**显式边距断言**（不再是 `+5px` 这类宽容差）；
  视口边缘的用例按"外扩后裁到视口"夹取期望值。

## 已知问题

1. ~~`#15` ttf-parser 无轮廓点数上限~~ —— **已修**：`flatten.rs` 加 `MAX_LINES = 10_000` / `MAX_CONTOURS = 256`，
   超限时 `flatten_glyph` 返回 `None`（与"字形不存在"同处理），诊断按 gid 去重只打一次。
2. `#18` 热路径 clone：**tree.rs 已测**（键盘 0 次，鼠标 6 次常数）；
   **`soft.rs` 纹理复制已修**：改为分片借用，每帧分配 **0 次 / 0 字节**（修前 4 MiB/帧）。
3. ~~`#19` 31 处 `#[allow(dead_code)]` 未定性~~ —— **已处理**：29 处加了归类注释
   （诊断 API 13 / 测试 API 4 / 内部实现与编译见证 12），**2 处真死代码已删除**。
4. ~~`cargo update` 会顺带升级 3 个依赖且需联网，`Cargo.lock` 里 `serial` 残留未清~~
   —— **已清**：`portable-pty` 升 0.9 后 `serial*` / `termios` / `ioctl-rs` 全部移出。
5. audit 依赖联网刷新指南库；离线时结论带 `STALE AUDIT` 标记，**不算发布门禁**。
6. ~~`RENDER-BUG-02`（issue #8）SoftBackend 不调 `begin_frame`/`take_dirty`~~ —— **已修**
   （2026-11-26，帧边界收到 `RenderCtx::begin_frame`，见本文顶部 A 节）。
   ~~`RENDER-BUG-01`（issue #7）D3D11 超 10k instances 静默截断~~ —— **已修**（2026-11-26，
   改为按需扩容，见本文顶部）。

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

门禁已恢复绿色，下一步按 ROI：

1. ~~**文字清晰度收尾**~~ —— 已完成（见上方 B 小节与 `docs/design/text-sharpness.md`）。
   剩余可选项：若日后要 GDI 那种"硬边"，需切 `DWRITE_RENDERING_MODE_GDI_CLASSIC`
   或改用点阵位图，属于独立提案。
2. **DPI 策略统一** —— 已完成（选项 A，`9a8060d` + `docs/design/dpi-policy.md`）。
3. **验证覆盖**：~~补 `scripts/hw-smoke.ps1`~~ —— 已完成（见上方 D 小节）。
   剩：~~soft vs d3d 像素差回归~~ —— 已完成（见上方"双后端一致性"）。
4. **发布**：`0.2.0` 已提交并打 tag；**已推送成功**（`main` 与全部 tag 在 `origin`，
   曾阻断推送的是本机 git 里指向 `127.0.0.1:7897` 的死代理，已移除）。
   `v0.2.0` 之后累积 15 个提交（Grid+Image 修复、per-node 墨迹边距、Linux 双平台验收、
   剪贴板确定性等）—— 已发布 v0.2.1；v0.2.2 收掉遗留（X11 DPI、StyledText 裁剪、Linux 计数校验）；v0.2.3 修复审发现的墨迹擦除并清理业务工程拆分后的痕迹；v0.2.4 接 XSettings 与运行期缩放、补各向异性质量测试，并按实测改写零 C 边界。
5. ~~**杂物**：根目录 `clippy.txt` / `todo.tsv` 是生成物，建议移出仓库或进 `.gitignore`~~ ——
   已完成：两者已进 `.gitignore` 且不再被跟踪。

## 审查报告状态

> **更正**：原审查报告未落盘。**实际可追踪的是 12 个编号**
> （#5 #7 #8 #9 #11 #12 #13 #14 #15 #18 #19 #20），且每个都有修复记录与提交。
> 其余 **11 项因原始报告未保存而无法逐项核对** —— 不编造编号。
> 下一次审计应重新跑全量审查，以新报告为准。
- 门禁步骤：MSRV / fmt / 测试计数 / test-inventory / clippy / 像素哈希 / `missing_docs` / doc 数字 / `cargo audit`
- **SAFETY 覆盖：100%（调用点口径）** —— d3d11 / font / window 的每个 `unsafe` 调用点都有契约，
  另有 vtable 契约与 context 三段式两段集中说明

## 历史候选（v0.1.17 期列出，仍然有效）

| 项 | 状态 | 依赖 |
|---|---|---|
| 图片 mipmap LOD | **已完成**（解析真值验证，见下方 mip 小节） | — |
| 真机 Present 数字 | 待做 | 物理显示器 + 独占全屏 |
| X11 真机验证 | 已在 **Xvfb** 全绿；真机仍待做 | 真 Linux 机器 |
| 桌面多窗口（独立 Host/UiTree） | 已实现（Win32/X11 `run_many` + 示例；X11 事件隔离测试通过） | 不承诺跨窗共享状态、焦点/标签策略；WASM 是单 page/canvas/runtime |
| Web/WASM 多 canvas/window | 不支持（每页单 canvas/runtime） | 独立架构提案 |

## 图片 mipmap（步骤 1 完成：建链 + 坐标映射，选级强制 level=0）

**已做**（`y901-render::mip`）：
- `level_for_image(scale, max_level)`：只看缩放比、不看墨量（图片没有"重新光栅化"
  这个维度，每级只能由上一级 box 降采样得到）；取 floor，`≥ 1` / NaN / `≤ 0` 均回 0。
- `image_mip_chain(base, levels)`：复用 `MipChain` + `downsample_box`；1px 时提前停止。
- **6 + 4 = 10 个测试**：LOD 边界（1.0 / 0.75 / 0.5 / 0.25 / 非法值 / clamp）
  + box 降采样为四点平均、左黑右白保持分界、1px 不再生成、存储 **1.3125×**。

**当轮遗留项 → 后续状态**（2026-11-19 核对代码后更新，避免把已完成的工作读成 TODO）：
- ~~`draw_image` 还没接 mip 选级~~ —— **已接**：`soft.rs` 用
  `level_for_image(min(scale_x, scale_y))` 选级（见下方"步骤 2"）。
- ~~`Image` 结构体加 `mips` 字段~~ —— **设计结论：不加**。mip 链属于**渲染后端**的资源，
  由 `SoftBackend::mips`（按 `TextureId` 索引）持有；控件只上传一次字节，
  不该在 UI 层复制一份派生数据。动态/静态边界在 `upload_texture*` 两个入口上区分。
- ~~`Cover` 模式的实际倍率~~ —— **已覆盖**：`scale_x/scale_y` 取自
  `style.layout()` 返回的 `dst/src`，而 `layout()` 已经把 `Cover` 的裁切算进去了，
  所以选级用的就是"实际绘制倍率"。
- ~~存储成本没写进文档~~ —— **已写**：`soft.rs` 的 `upload_texture` 文档注明
  `L0+L1+L2 = 1.3125×`。

## 图片 mipmap 步骤 1（本轮）

- `SoftBackend::upload_texture`：静态图片就地建 mip 链（**L1、L2**）；
  **L0 不另存一份** —— 它就是 `textures` 表里的原始字节，
  `level == 0` 路径因此与改造前**逐字节相同**。
- `upload_texture_dynamic`：动态图片（视频帧）只存 L0，不建 mip。
- `draw_image`：已算出 `scale_x = dst.w/src.w`、`scale_y = dst.h/src.h`，
  `draw_scale = min(...)`，并调 `level_for_image` 得到 `_computed_level`；
  但**本步骤强制 `level = 0`**，只证明"引入选级与坐标映射不改变渲染"。
- `src` 到级坐标的映射：`src / 2^level`（L0 时即 `src` 本身）；dst 始终是屏幕坐标。

**验证**（当轮口径，非当前基线）：全量 816 全绿，像素哈希 **`0xDBB308DD1657EF9C` / 400×220 不变**。
> 当前基线是 `0xF7455CCE7781A864` / 400×220 —— 后续文字清晰度修复改变了它，以 `docs/evidence/hash-baseline.txt` 为准。

**验证**：图片 mip 选级已启用；各向同性质量用例见 `crates/y901-render/tests/image_mip_quality.rs`。

**已知限制**：布局支持分别缩放 X/Y 轴，但过滤器没有各向异性专用路径（`max(scale_x,scale_y) / min(...) > 2`）。Soft 取较小倍率选各向同性 mip 以降低强缩小轴走样风险，较少缩小轴可能偏模糊；改选较大倍率则会增加强缩小轴的走样，故目前不擅自改变策略。D3D 硬件 mip 路径未启用专用 anisotropic sampler。

**定向图案测试（2026-11-20 补）**：`crates/y901-render/tests/image_mip_quality.rs` 新增 4 条
各向异性解析用例 —— X/Y 两个方向各一条"强缩小轴不走样"（闭式解恒 127.5，实测 127..127）、
比值 > 2 的有界性、以及近各向同性对照。**变异验证**：把 `draw_scale` 的 `min` 改成 `max` 后，
两条强缩小轴用例立刻红（实测 R 0..255）；改回即绿。所以这组测试真的锁住了当前策略，
不是"跑通即通过"。

## 图片 mipmap 步骤 2（本轮）

- **选级已打开**：`draw_image` 用 `level_for_image(min(scale_x, scale_y))` 选级。
- **s=1.0 哈希不变**：`0xDBB308DD1657EF9C` / 400×220（`s >= 1` 时返回 0，路径恒等）。
- **修了一个真 bug**：选中级后采样坐标仍用 L0 坐标 → 越出级图宽度，
  实测 MAD **0.3745**；按 `÷ 2^level` 映射后降为 **0.0000**。
- **但质量并未证明**：我构造的"4×4 超采样真值"采样点**落回 2×2 源像素**，
  等价于 box 降采样 —— MAD 恒为 0 是**度量退化**，不是质量好。
  该测试已标 `#[ignore]`，理由写在属性里。**下一步需重新构造独立真值**。

## 本轮教训

- **测试全绿 ≠ 功能存在**：步骤 1 里 `box_levels_rgba` 根本没被调用，**当轮** 816 测试全绿、
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
- **去 AI 完成**（Y901 本仓）：
  - Y901 主仓：`7062463` 删除 `y901-agent` / `y901-mcp`，`28ffc88` 清 AI 痕迹；
  - 业务工程已拆分为**独立仓库**，不在本仓范围内；本仓只保留零 C 依赖守卫
    （`crates/y901-shell/tests/no_c_deps.rs`，见 `docs/ZERO_C_PROOF.md`）。
  - 验证：`no_c_deps` 守卫通过（0 违规）。

- **`sync_interval` 语义漏洞已修**：`Gfx::present` 之前写死 0，交换链无法遵守 vsync；
  现为参数，默认路径 `render_inner(tree, 1)` 由结构化测试钉死

## 需要外部条件的待办

| 项 | 阻塞 |
|---|---|
| 独占全屏 Present | 手写 COM 槽位 + 物理显示器 |
| GPU timestamp query | 更深的驱动层改动 |
| X11 真机验证 | 真 Linux 机器 |
| macOS 后端 | Mac 或 CI runner |
| Web/WASM 多 canvas/window | 需独立 runtime / canvas 生命周期设计（当前明确单 canvas） |

## 不阻塞的剩余小项

- 各向异性图片过滤质量：几何 Fill 可独立缩放 X/Y；Soft 用较小倍率选各向同性 mip，D3D 使用硬件 mip/线性采样但无专门 anisotropic sampler。比例差大时缩小轴得到保守抗混叠、另一轴可能偏模糊；独立质量基准与定向过滤仍未实现。
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
- **P2 = 0**：`serial` 残留已清（`portable-pty` 0.9）；其余已修（ignore 计数同步、Guide 编号）或接受（miniz_oxide 上游约束）
- **P3 = 4**（设计取舍 / 旧项失效）
- 数字由 `scripts/sync-doc-numbers.ps1` 生成，并由 gate 的 `-Check` 步骤拦截漂移
- 历史审查报告：[`docs/archive/reviews/audit-v0.1.15.md`](archive/reviews/audit-v0.1.15.md)；后续 v0.1.17 档案见 [`docs/archive/reviews/audit-v0.1.17-final.md`](archive/reviews/audit-v0.1.17-final.md)。这些报告仅供历史追溯，不代表当前项目状态。

### v0.1.17 前的两项核实（本轮）

- **P2-01 `serial`**：**描述更正** —— 它是 `portable-pty 0.8.1` 的**必选依赖**，不是 lock 残留；
  `cargo update --workspace` 归零个包，`cargo generate-lockfile` 也不会删它。需升级 `portable-pty` 到 0.9 才能去掉（未做，需验证 API）。
  **2026-11-22 已做**：升到 `portable-pty 0.9.0`，`serial*` / `termios` / `ioctl-rs` / `memoffset 0.6.5` 全部移出，
  `nix` 0.25 → 0.28；`cargo audit` 的 allowed warning 7 → 6。PTY API 与本仓用量兼容，双平台测试通过。
- **P1-03 x11 EnvGuard**：代码已修，**双平台编译通过**；WSL 里 `Xvfb :99` 启动成功，
  单测 `handshake_reports_screen_and_visual` **通过**；但 **13 个用例并行跑时全部卡住**（>600s 超时），
  尚未定位是 `SERVER_LOCK` 互斥、基准耗时过长，还是某个用例等事件不返回。
  **因此不打 `v0.1.17`（运行时验证不完整）。**

## Linux / WSL 验证口径

### Linux 全量测试（**交付前必跑**）

以下为历史排障记录；当前复测命令与 2026-11-22 的验收证据见 [`docs/ACCEPTANCE.md`](ACCEPTANCE.md)。
这是唯一能发现"只在 Linux 链接 / 运行时暴露"的债的探针。
`cargo check --all-targets` **不链接**；Windows 的 `cargo test` **不编译 `#[cfg(unix)]` 代码**。

```bash
cargo test --workspace --no-fail-fast 2>&1 \
  | grep -E "test result: FAILED|^error" || echo "LINUX OK"
```

- 期望：`LINUX OK`。若有输出，**贴完整错误行**。
- **不要用 `--exclude` 绕过** —— 那只是掩盖问题。
- 四条测试基建债（22 个测试无门控 / lib 的 4 处 `#[link]` /
  `file_drop` 硬编码 `C:\` / supersample 阈值绑定字体）就是靠它首次暴露的。
- 已验证（2026-11-20，Ubuntu 22.04 / WSL2 + rustc 1.92 + Xvfb）：
  Linux 全量 `xvfb-run cargo test --workspace --no-fail-fast`（2026-11-20 历史记录；**当前口径**见
  [linux-numbers.txt](evidence/linux-numbers.txt) 与 [linux-verify-2026-11-26-x11.log](evidence/cross-platform/linux-verify-2026-11-26-x11.log)）
  → 当时全绿（具体计数以该日志为准）。证据：[linux-full-2026-11-20.log](evidence/cross-platform/linux-full-2026-11-20.log)。
  与 Windows 的当轮 1328/0/71 不同 —— **差异来自平台 cfg 门控与字体回退链**，不是测试丢失。
  同日复跑 `scripts/linux-verify.sh`：八步全 `EXIT_*=0`
  （证据：[linux-verify-2026-11-20.log](evidence/cross-platform/linux-verify-2026-11-20.log)）。
- **本轮修掉一个只在 Linux 复现的真问题**：`xvfb-run` 下测试进程里**没有 `HOSTNAME` 环境变量**，
  旧的 Xauthority 匹配传空主机名 → `FamilyLocal` 条目全部匹配失败，服务端回
  `Authorization required, but no authorization protocol specified`（现象像"没发认证"，
  实际是"有 cookie 没匹配上"）。现在改用 `xauth::local_hostname()`（`$HOSTNAME` → `/etc/hostname` 兜底），
  `xvfb-run -a cargo test ...` 直接可用。
- **上一轮仅剩的两条红（已修）**：`y901-extras` 的 `sgr_reverse` 曾取"单元格右上角"像素
  断言精确 RGB，前提是 Windows 的 Segoe UI 字形不覆盖那个采样点；Linux 回退到
  DejaVuSans 后读到"前景 × 背景"的混合值（182,186,195 vs 期望 230,235,245）。
  现在改为**格内背景众数**断言：仍要求精确 RGB，但对字体覆盖范围不敏感，Windows/Linux 双绿。
- **x11 专项验收（2026-11-20）全部通过**，且脚本已进仓库
  （[scripts/linux-verify.sh](../scripts/linux-verify.sh)）：
  `check`（all-targets）、`golden 32/0`、`xvfb-run real_window 14/0`（2 ignored）、
  **`clippy --all-targets -D warnings`**、`render stroke_round_aa`、`extras sgr_reverse`、
  `ui+widgets`、`full workspace` —— 八项 `EXIT_*=0`。
  证据：[linux-verify-2026-11-20.log](evidence/cross-platform/linux-verify-2026-11-20.log)。
  新增的 clippy 步是补一个盲区：**Windows 门禁的 clippy 只 lint 宿主 target，
  `#[cfg(unix)]` 代码根本不参与编译**，本轮因此积了 6 条 lint 债（已全修）。
- 已知不稳定项：`y901-shell` 的剪贴板单测在 Xvfb 下有 X 选区 owner 竞争，可能偶发失败；
  本轮全量运行恰好 0 failed，但它不在发布门禁内（门禁在 Windows 宿主上跑）。

### X11 专项

- **DPI 缩放来源（v0.2.4 更新）**：`Y901_X11_SCALE` → `Y901_DPI` → **XSettings**
  （`Xft/DPI` / `Gdk/WindowScalingFactor`）→ 根窗口 `RESOURCE_MANAGER` 的 `Xft.dpi`
  → setup 屏幕毫米估算（吸附 0.25 档位）。XSettings 与 `Xft.dpi` 是用户/桌面写下的
  缩放意图，取 `72..=384` 且**不吸附**；都没有才回退毫米。
  XSettings 走标准协议：`GetSelectionOwner(_XSETTINGS_S0)` → 该窗口的
  `_XSETTINGS_SETTINGS` 属性 → 解析二进制块（整数 / 字符串 / 颜色三种类型，
  字符串按 4 字节补齐，未知类型整块作废以防错位）。**零新依赖**（只加了
  `GetSelectionOwner` 编码与解码，opcode 23）。
- **运行期跟随缩放（v0.2.4 新增，v0.2.5 补 SHM 重建）**：桌面改缩放后，窗口在 ≤1 秒内发现并
  **重建字形引擎 + 软后端**、按新物理尺寸 `ConfigureWindow`，随后照常重排重绘。
  检测方式是轮询（`Y901_X11_DPI_POLL_MS` 可调，默认 1000ms；设为 0 每次事件都查），
  **不是** `PropertyNotify` 订阅 —— 后者要给别人的窗口选 `PropertyChangeMask`
  并新增事件解码，成本高且漏事件会永久停在旧缩放。重建失败时保留旧缩放并停用跟随，
  不会每秒重试刷日志。
- **缩放后 SHM 也会重建**（v0.2.5）：先发 `ShmDetach` 让服务端放开旧段，再用**轮转的新 XID**
  （`0x10..0x18`，避免与仍被附着的旧 XID 撞车）按新尺寸建段；失败才回退分块 `PutImage`。
  否则改一次缩放就永久丢掉共享内存快路径——画面仍对，但每帧多一次整窗拷贝。
  回归断言在 `real_window::runtime_scale_change_rebuilds_soft_backend`：
  改缩放后 `shm_active()` 必须仍为 true、且 `present` 成功。
- **仍未覆盖**：每显示器（per-monitor）DPI —— X11 协议层没有可靠 API；
  多屏不同缩放只能靠显式环境变量。全屏状态下不主动改客户区尺寸（交给 WM）。
- `DISPLAY=:99 cargo test -p y901-backend-x11`（需 Xvfb :99）
- 平台中立 crate（`y901-core` / `y901-ui` / `y901-render` / `y901-widgets`）可单独 `-p` 跑
- x11 全量：`DISPLAY=:99 cargo test -p y901-backend-x11 --test real_window -- --test-threads=1`
  （**必须 `--test-threads=1`**：用例共享一个 X 连接与环境变量）
- 已验证结果见 `docs/evidence/numbers.txt`（Windows 侧口径）

> 旧描述（已修正）曾写"`cargo test --workspace` 在 WSL 上不可行：
> `y901-backend-win32` 在 Linux 下链接失败（预期）" ——
> 那是**crate 级 `#![cfg(windows)]` 之前**的事；现在 Linux 全量测试可行，
> 且有上方 2026-11-19 的实测记录（仅剩两条字体相关的采样假设差异）。

## 数字检查的覆盖范围

**已从"只比对加粗三位数"升级为"含计数语义的行里所有三位数都必须对"**
（2026-11-19，第二轮审查 P1-01 的根因修补）。

为什么必须升级：上一版只认 `**N** passed` 这类手写形式，
换一种写法就整条逃检 —— 实测漏掉了 README 的
`978 passed / 0 failed / 61 ignored（149 个测试目标）`（真实值已 979/150）。<!-- counts-exempt -->

现在的规则（`scripts/sync-doc-numbers.ps1 -Check`）：

| 规则 | 说明 |
|---|---|
| 手写语义规则 | 保留：`**N** passed/failed/ignored`、`基线 **N**`、`include-ignored: **N**`、`` `key=N` `` |
| **通用兜底** | 含 `测试 / passed / ignored / 基线 / 项测试 / include-ignored` 的行里，**每个三位数**必须是 `passed / ignored / targets / baseline / include_ignored` 之一 |
| 非计数后缀豁免 | 紧跟 `% / × / x / px / DPI / 色 / 点 / 个像素` 的数字按"不是计数"跳过（如 `125%`、`400×220`、`1039 色`） |
| 平台/历史豁免 | 行内显式出现 `Linux / 历史 / 旧口径 / 当轮` 或 `<!-- counts-exempt -->` 才豁免；Linux 行另有独立数据源逐数校验 |
| 反向写入 | sync 的自动改写**逐行**进行并遵守同一套豁免，绝不把 别处口径的数改成 Windows 口径 |

新增 `targets=` 字段（`test result:` 行数，见 numbers.txt 的 `targets=`）：`passed` 与"N 个测试目标"
是两个独立派生量，只校准前者会留下"通过数对了、目标数停在旧值"的漂移。

**验证方式（不靠"跑一次是绿的"）**：对检查器做了 5 组变异测试 ——
非加粗 `passed` 漂移、目标数漂移、表格内 `passed` 漂移、`FRAMEWORK` 目标数漂移、
新增一条无标记的假计数 —— **全部被抓到**；恢复后 `-Check` 通过（豁免行不误报）。

**仍存在的已知限制**（诚实记录）：

- **历史口径的数字不由检查器校验**：文档里写 "历史 / 当轮" 的行会豁免，
  它们不会随代码自动纠偏（这是刻意的：历史就该保持原值）。
- `Guide` 索引自动生成仍未做。

**Linux 行已不再豁免**（本轮改进）：新增独立数据源
[`docs/evidence/linux-numbers.txt`](evidence/linux-numbers.txt)（`passed / ignored / targets`），
`-Check` 对"含 Linux 且谈计数"的行逐数字校验；`linux-verify.sh` 在 Linux 上跑完
全量 workspace 测试后写回该文件。自测脚本新增"Linux 计数漂移"变异用例。

## 框架最终状态（v0.1.17-dev，审计与测试基建债清零）

### 测试

| 平台 | 通过 | 失败 | ignored | 备注 |
|---|---|---|---|---|
| **Windows** | **751** | 0 | 58 | **当轮口径**（当前见 `numbers.txt`） |
| **Linux** | **661** | 0 | 25 | **当轮口径**；当前 906/0/27，见上方 2026-11-20 验收 |

### 审计

- **两批全部收口，零未修项**（P1-1/2/3、P2-1/2/3/4a/4b、subscriptions 泄漏、present 签名）
- 每项都有验证手段：像素哈希 / 双向证伪 / `[diag]` 真跑 / 脚本回归

### 已知未覆盖项（诚实记录，不美化）

| # | 未覆盖 | 原因 | 怎么补 |
|---|---|---|---|
| 1 | **IME 两窗口同时组字不串** | 需真实 IME 环境 | 人工：两窗口各打一半中文，看组字条 |
| 2 | **present 的设备丢失场景** | 真实设备丢失无法构造 | 已有 `Y901_SIMULATE_PRESENT_FAIL` 验证 **Err 传播路径**（非设备丢失本身） |
| 3 | **多窗口集成语义/IME 双窗实测** | 独立 Host/UiTree + 桌面 `run_many` 已实现；共享应用状态、跨窗焦点/标签策略不属于现有契约，双窗 IME 需真机验证 | 有需求时另立协调层设计；IME 手工验收见 #1 |

### 测试开关清单（方便后来者）

| 环境变量 | 作用 | 代码位置 |
|---|---|---|
| `Y901_FULLSCREEN` | 启动即全屏（与 `Options.fullscreen` 取 OR） | 两后端的 create |
| `Y901_FORCE_SOFT` | 强制 CPU 光栅器，不建 D3D11 | win32 `create_inner` |
| `Y901_SIMULATE_RENDER_FAIL` | `WindowBackend::render` 立即 `Err`（验 `Drop` 兜底） | win32 `window.rs` |
| `Y901_SIMULATE_PRESENT_FAIL` | `D3dBackend::present` 立即 `Err`（验 present 传播） | win32 `backend/mod.rs` |
| `Y901_SUB_TRACE` | 打印订阅懒清理条数 | `y901-ui` `tree.rs` |
| `Y901_X11_DPI_POLL_MS` | X11 检查桌面缩放变化的间隔（默认 1000；`0` = 每次 `wait_events` 都查） | x11 `window.rs` `wait_events` |
| `Y901_NO_SHM` | X11 禁用 MIT-SHM（验 PutImage 回退路径） | x11 `window.rs` |
| `Y901_PAINT_TRACE` | 按真实绘制顺序打印节点矩形/裁切 | `y901-ui` `tree.rs` |
| `Y901_PROFILE` | 每次 `run_loop` 退出时打印帧耗时统计 | `backend-common` `runtime.rs` |
| `Y901_IDLE_WAIT_MS` | 空闲等待上限 | `backend-common` `runtime.rs` |
| `Y901_FONT=self` / `Y901_NO_CLIPBOARD` | 自研 TTF 字形引擎 / 禁用剪贴板 | `y901-shell` |

## 架构债

- **`plan_dirty_rects` 是后端无关的启发式，但两个后端的最优策略相反**（相差约 4 个数量级）：
  CPU 后端 `c_switch ≈ 10 像素`（逐块最优），D3D 后端每块脏区 = 一次全量实例化绘制（激进合并最优）。
  需要后端提供 `c_rect / c_pixel` 常数，由后端决定合并策略。
  当前 `MAX_DIRTY_RECTS = 16` 是**替 D3D 兜底**的临时方案 —— 它不是设计，是巧合地正确。
- `DirtyRegion::add` 的合并判据已改为"仅真实重叠"（`overlaps`），零重叠（含边/角相贴）不再合并。
  这是唯一不依赖后端常数即可确定净收益的改动。
- **零 C 依赖守卫只覆盖本仓**：业务工程已拆分为独立仓库，它的依赖审计由该仓自己负责。
  本仓守卫（`crates/y901-shell/tests/no_c_deps.rs`）解析失败会**报错而不是静默绿**。
- **Windows"只装 Rust 能否构建"已实测（v0.2.4 复核）**：本机**没有 Win10/11 SDK**
  （`Windows Kits\10` 不存在）、`link.exe`/`cl.exe` 不在 PATH、`vswhere` 返回空，
  而 clean-env（清空 `LIB`/`INCLUDE`/`VC*`）+ 工具链自带 `rust-lld` 的
  `cargo build --release --workspace --examples` **exit 0**；链接输入里没有任何 SDK 库
  （`windows-sys` 与 std 走 `raw-dylib`）。
  **但默认产物运行时要 VC 运行库**（导入 `VCRUNTIME140.dll`），要"解压即用"得加
  `-C target-feature=+crt-static`（实测产物零 CRT 导入）。
  一键复现：[scripts/win-clean-probe.ps1](../scripts/win-clean-probe.ps1)。
  以前文档写的"本机装了 VS 2022 所以证明不了"是**未测量就下的结论**，已按实测修正。
- **Linux 相反**：去掉 `cc` 后 rust-lld 找不到 `-lc/-lm/-lgcc_s`，补 `-L` 也只会产出
  没有 `PT_INTERP` 的二进制（运行即 `SIGSEGV`）—— GNU 目标必须有系统 C 工具链驱动。


- **文档债：`docs/FRAMEWORK.md` 的"需要真机的测试"一节陈旧**（去 AI 阶段 4b 记录）。
  原文称"需要真机的测试只有 `rebuild_reuploads_atlas`"，实际 GPU / D3D11 依赖的 `#[ignore]` 有 **13** 个
  （`snapshot_harness` ×10 + `soft_matches_d3d_within_antialiasing_tolerance` + `pixel_statistics_and_png_baseline` + `rebuild_reuploads_atlas`）。
  需改为 13 个，或直接指向 `ACCEPTANCE.md` 的分类表。
- ~~**文档债：`docs/ACCEPTANCE.md` 的"7 个 GPU harness"小节与分类表不一致**~~ —— 已修（2026-11-19）：标题改为"GPU 像素验收用例"并写明三个口径（8 条断言用例 / `snapshot_harness` 10 条 `#[ignore]` / 全仓 `ignored=71`）。
  `snapshot_harness` 现有 **10** 个 `#[ignore]`，小节只列 7 个；余下 3 个（`text_label_paints_visible_non_background_pixels`、
  `text_inside_scrollview_is_not_clipped_away`、`rebuild_of_cached_target_does_not_leave_black_areas`）是回归用例而非像素 harness。
  需先澄清"harness"定义，再决定是补进表还是把标题改成"7 个像素对拍 + 3 个回归"。

- **`sync-doc-numbers.ps1` 只保证数字对，不保证句子对**（已第三次改坏派生行）：
  它会改对 `769`（**历史口径**）这个数字，但会把 `--include-ignored` 行的句子结构改坏
  （写成 `718` 应为 `769`，或出现 `499` 等怪数）。
  短期：每次 commit 前人工检查 `ACCEPTANCE` 的派生行。
  长期：删掉 `ACCEPTANCE` 的派生表述，只留一行 `769`，不再列 `718 + 53` 这种二次计算。

- **`\uXXXX` 转义泄漏到新文件**（已发生多次）：
  生成含中文的新文件时，转义层会把中文写成 `\u4e2d` 形式，
  而 Rust/PowerShell 不会将它当转义处理，于是文件里出现字面串。
  **固定动作**：凡是含中文的新文件，落盘后 grep 一次 `\\u[0-9a-f]{4}`，有命中就修。

- **`.ps1` 必须存成 UTF-8 with BOM**：Windows PowerShell 5.1 对**无 BOM** 的 UTF-8
  按 ANSI 解码，中文字节会被错读；若某个汉字的第二字节恰好是 `0x5C`（反斜杠），
  它会把后面的 `"` 转义掉，报"字符串缺少终止符"（本轮 `gate.ps1` 加一行中文警告时踩到）。
  固定动作：改完 `.ps1` 后用 `read_bytes()[:3] == b'\xef\xbb\xbf'` 验证。

- ~~**Linux 验收脚本未覆盖 `y901-render` 的 Soft 描边用例**~~（已修）：
  版本化的 [scripts/linux-verify.sh](../scripts/linux-verify.sh) 现在覆盖
  x11 `check` / `golden` / `real_window`、`render stroke_round_aa`、
  `extras sgr_reverse` 与 `ui+widgets`；任一失败都会让脚本以 exit 1 结束。

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
  **注意**：`--include-ignored` 在并行负载下仍可能被 allocation-counter 污染（**当轮**实测漂到 560）。
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

- **订阅的懒清理语义**：`remove_node` **不立即清理** `subscriptions`，
  失效条目在**下一次 `poll_subscriptions`** 被剔除。
  为何不立即清：`remove_node` 只知道被挂掉的子树根，而订阅 key 是**任意层级的节点 id** ——
  在那里清要多遍历一次子树；而 `poll_subscriptions` 本来就要逐个查节点，
  顺带判存在性**零额外遍历**。延迟上界 = **一帧**（GUI 每帧 poll）；
  headless 场景不跑 poll，订阅清理不是其关切。
  测试 `crates/y901-ui/tests/subscriptions.rs` **经过证伪**（去掉清理即失败）。

- **`Y901_SUB_TRACE` 是订阅清理的诊断开关**：设置后 poll 清理了失效订阅时打印条数。
  默认静默、**零热路径成本**（只在 `dropped > 0` 时才读环境变量）。
  与 `Y901_FULLSCREEN` / `Y901_FORCE_SOFT` / `Y901_SIMULATE_RENDER_FAIL` 同类。

- **`Copy-Item` 还原源码会回退 mtime，cargo 用旧二进制跑测试**（本轮踩坑）：
  做证伪验证时用 `Copy-Item` 备份/还原源码，`Copy-Item` 会保留源文件的 `LastWriteTime`，
  还原后的文件 mtime **比构建产物还旧** → cargo 认为没变化 → **跑旧二进制**。
  表现：源码明明已还原，测试却持续失败（报的是旧代码的错）。
  修法：还原后显式 `(Get-Item $f).LastWriteTime = Get-Date` 再跑；
  或用 `git checkout -- <file>`（但工作区有未提交改动时慎用，会丢改动）。
  **这条也影响归因判断**：上一轮 subscriptions 的"先绿后红"可能就是这个陷阱，
  **不一定是 `find` 索引问题**。

- **断言用的子串不能也出现在 mock 数据里**（run_many 契约测试的教训）：
  mock 返回 `"mock 窗口 0 失败"`，断言 `contains("窗口 0")` ——
  **即使默认实现完全不加序号前缀，断言也照样通过**（mock 自己的文本里就有"窗口 0"）。
  做证伪验证时才发现（去掉序号前缀后测试仍然全绿）。
  修法：mock 错误文本改成不含断言子串的 `mock-boom-{n}`。
  **原则**：证伪验证不是"跑一遍看红不红"，而是"**去掉被测行为后必须红**"。
  如果去掉行为后仍然绿，说明断言是**空转**的 —— 测的是 mock 数据，不是被测代码。

- **subscriptions 归因补充**：上一轮记"用 `find` 判断节点存在性导致订阅清不掉，
  改用 `find_path`"——`find` 走 layout 索引、`remove_node` 不重建索引这个**事实是真的**，
  但**当时的失败也可能是 stale binary 陷阱**（Copy-Item 还原后 cargo 没重建）。
  用 `find_path` 仍然是对的（直接遍历活的控件树，不依赖索引时效），
  但**归因不唯一**。**以后遇到"源码改了测试仍红"，先确认 cargo 真的重建了**。

- **`cargo check --all-targets` 通过不代表能链接**（四条测试基建债的教训）：
  `check` 只做类型检查，**不链接**。`y901-backend-win32` 的 lib test 在 Linux 上
  因为 4 处裸 `#[link(name = "d3d11"/"d3dcompiler"/"dwrite"/"imm32")]` 而链接失败，
  但 `cargo check --all-targets` 一直是绿的。
  **`cargo test --workspace` 才是"全量通过"的硬判据** ——
  它是唯一会**链接所有测试二进制**的命令。

- **`--test <name>` 通过不代表 `cargo test --workspace` 通过**：
  给 22 个集成测试文件加 `#![cfg(windows)]` 解决了 `--test <name>` 的链接问题，
  但 **crate 自身的 lib test 目标仍会编译 `src/` 里的 `#[link]` extern 块**。
  **只有 crate 级的 `#![cfg(windows)]` 才能让 lib 本身在 Linux 上消失。**
  以后验证 `cargo test --workspace` 时，**不要用 `--exclude` 绕过** —— 那只是掩盖问题。

- **审计报告的事实前提需要核实**（present 签名改造的教训）：
  审计第 1 批写"`RenderBackend::present` 返回 `bool`，失败无法表达"——
  实际读代码后发现：**`RenderBackend::present` 本来就是 `Result<(), String>`**，
  真正的缺口在**固有方法** `D3dBackend::present`（返回 `bool`，`false` 同时表示失败和被遮挡）。
  **如果按审计字面意思去改 trait，会改错地方。**
  **教训**：审计报告也是人写的，事实性描述（"X 返回 Y"）在动手前**必须用代码核实**。
  特别是"改签名"这类动作，改错 trait 会影响所有实现和调用点。

- **`Y901_SIMULATE_PRESENT_FAIL` 是 present 异常路径的测试开关**：
  设置后 `D3dBackend::present` 立即返回 `Err`，用于验证 `render_inner` 的 `present?` 传播
  （设备丢失 / 交换链失效的真实场景无法在测试中构造）。
  开关在**任何 unsafe 操作之前**检查；与 `Y901_SIMULATE_RENDER_FAIL` 同构，`OnceLock` 缓存，零热路径成本。
