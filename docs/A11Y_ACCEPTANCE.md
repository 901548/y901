# Y901 无障碍验收清单

> **2026-11-28 更新：清单里"机器可核对"的部分已用 UIAutomationClient 自动跑过一遍。**
> 读屏器读的就是 UIA 树，所以"它要念什么"不必靠人耳 —— 取证见
> [`evidence/cross-platform/a11y-uia-windows-2026-11-28.log`](evidence/cross-platform/a11y-uia-windows-2026-11-28.log)。
> 那次运行抓到 **6 个真缺陷**（角色报成按钮/分组、名字里念出内部 id、按钮 value 泄漏命令名、
> 列表与告警不在树里、输入框只有内容没名字、窗口无名字），以及第 7 个"空闲后读屏器读到空树"
> —— 全部已修，见 [STATUS.md](STATUS.md)「续十」。
>
> **仍然需要真人**的只剩两件：
> 1. 讲述人/NVDA 的**实际播报**（语音引擎念得对不对、重音与停顿是否合理）；
> 2. 告警的 **assertive 主动播报**（dump 取不到 `LiveSetting`）。
>
> ## 怎么复跑机器验收（一条命令）
>
> ```powershell
> powershell -NoProfile -File scripts/dump-a11y-uia.ps1 -OutFile docs/evidence/cross-platform/a11y-uia-<日期>.log
> ```
>
> 它会**带 `a11y feature` 重建示例**、启动、dump 整棵 UIA 树、关窗、写日志。
> 为什么要重建：`a11y_demo.exe` 与所有 `--all-targets` 构建共用，
> 一次不带 feature 的 `cargo test/clippy` 就会把它覆盖成"没有无障碍桥"的版本
> （现象：窗口能开，但 UIA 里 `DESCENDANTS=0`）——本轮踩过这个坑。
> ## 机器验收结果（2026-11-28，`a11y_demo`）
>
> | 项 | 期望 | 实测 | 结论 |
> |---|---|---|---|
> | 窗口 | 名称 = 标题 | `name=[Y901 无障碍验收]` | PASS |
> | 标题 | 文本 | `Text name=[Y901 无障碍验收]` | PASS |
> | 按钮 | Button + 名称 | `Button name=[保存（Ctrl+S）] invoke` | PASS |
> | 复选框 | CheckBox + 勾选态 | `CheckBox name=[启用同步] toggle=Off` | PASS |
> | 开关 | Toggle + 开/关 | `Button name=[启用同步] toggle=On`（UIA 无 switch 类型，这是 Windows 惯例） | PASS |
> | 滑杆 | Slider + 值 + 范围 | `Slider name=[音量] value=0.50 min=0 max=1` | PASS |
> | 数值输入 | Edit + 名字 + 值 + 范围 | `Edit name=[重试次数] value=42 min=0 max=100` | PASS |
> | 文本框 | Edit + 名字 + 内容 | `Edit name=[备注] value=[可编辑文本]` | PASS |
> | 列表 | List + 项数 + 逐项 | `List name=[list(3)]` + 列表项 1/2/3 | PASS |
> | 告警 | 名字 = 标题 | `Text name=[这是一个告警]` | PASS（assertive 待人工） |
> | 状态行 | 文本 | `Text name=[尚未保存]` | PASS |
> | 无未命名控件 | — | 21 个元素全部有名字 | PASS |
> | 不念内部 id | — | `switch switch-ctl` 这类已消除 | PASS |
>
> ## 欠账（2026-11-28 已清）
>
> 上一版记的 13 个组件（`badge`/`combo_box`/`context_menu`/`empty_state`/`error_state`/
> `image`/`menu_bar`/`popover`/`progress`/`radio`/`skeleton`/`spinner`/`tree_view`）
> 加上 `table`/`list_view`/`list`/`combobox list`，**全部**改成了
> "有 `.label(..)` 用 label、没有就留空" —— 没有任何组件的名字里再出现内部 id。
> 复验日志：[`a11y-uia-windows-2026-11-28-labels.log`](evidence/cross-platform/a11y-uia-windows-2026-11-28-labels.log)
> （21 个 UIA 元素，无一含 id）。
>
> 约定：装饰性组件（`Skeleton`、`Popover` 锚点）**刻意不提供** `.label()` ——
> 给它们名字只会让读屏器念一堆没有信息的节点。
# 无障碍手工验收清单（10 分钟）

> 目的：验证 **读屏器真的能读到并操作** Y901 的界面。
> 自动测试能证明"语义树里有这个字段"，**不能**证明"讲述人/Orca 会把它念对" ——
> 后者只能人来做。本清单把这件事压缩到 10 分钟。

## 0. 前置

代码里已有：语义树（`Widget::semantics`）→ AccessKit 映射（`y901-a11y`）→
平台适配（Windows UIA / Linux AT-SPI）。**默认构建不启用**桥接，必须显式开 feature：

```bash
cargo run -p y901-shell --features a11y --example a11y_demo
```

| 平台 | 适配器 | 读屏器 |
|---|---|---|
| Windows 10/11 | `accesskit_windows`（UIA） | 讲述人：`Win + Ctrl + Enter` |
| Linux / X11 | `accesskit_unix`（AT-SPI over D-Bus） | Orca：`orca` 或桌面设置里开启 |
| 浏览器（wasm） | 隐藏 DOM 的 ARIA 镜像 | 任意浏览器读屏器（**未纳入本清单**，见 §5） |

启动后应看到标题「Y901 无障碍验收」与若干带标签的控件。
**控制台会打印一行**说明桥是否启用：没看到 `UIA 桥已启用` / `AT-SPI 桥已启用`
就说明忘了 `--features a11y`。

## 1. 必验项（逐条打勾）

用读屏器的"浏览/扫描"模式逐项走过，核对**念出来的内容**：

| # | 控件 | 期望播报 | 通过 |
|---|---|---|---|
| 1 | 标题 | 「Y901 无障碍验收」，角色是文本/标题 | ☐ |
| 2 | 保存按钮 | 角色「按钮」+ 名称「保存（Ctrl+S）」 | ☐ |
| 3 | 复选框 | 「复选框」+「启用同步」+ **未勾选** | ☐ |
| 4 | 开关 | 「开关」+ **开**（初值是开） | ☐ |
| 5 | 滑杆 | 「滑杆」+ 当前值（0.5 附近）+ 最小/最大（0/1） | ☐ |
| 6 | 数值输入 | 「数值输入」+ 值 42 + 范围 0–100 | ☐ |
| 7 | 文本框 | 「文本框」+ 内容「可编辑文本」 | ☐ |
| 8 | 列表 | 「列表」+ 项数，逐项念出「列表项 1/2/3」 | ☐ |
| 9 | 告警 | 「告警」+ 标题「这是一个告警」+ 描述；应为**主动播报**（assertive） | ☐ |
| 10 | 状态行 | 初始「尚未保存」 | ☐ |

## 2. 交互项（验证状态变化会被重新播报）

| # | 操作 | 期望 | 通过 |
|---|---|---|---|
| 11 | 用读屏器**激活**保存按钮（讲述人：`Caps+Space`；Orca：`Space`） | 状态行变为「已保存 1 次」，读屏器应播报该变化（polite 实时区域） | ☐ |
| 12 | 勾选复选框（聚焦后按 `Space`） | 再聚焦时应念 **已勾选**（`aria-checked` 由 false → true） | ☐ |
| 13 | 切换开关 | 再聚焦时应念 **关** | ☐ |
| 14 | 滑杆聚焦后按 `→` / `←` | 值变化且被念出新值 | ☐ |
| 15 | 数值输入聚焦后按 `↑` / `↓` | 42 → 43 / 41，且念出新值 | ☐ |
| 16 | 文本框里输入字符 | 字符进入正文（不是被读屏器吞掉） | ☐ |
| 17 | 用 Tab 走一圈 | 焦点顺序：按钮 → 复选框 → 开关 → 滑杆 → 数值 → 文本框，**不跳、不丢** | ☐ |
| 17b | （列表场景）用读屏器删掉中间一行，再点"删除" | 删掉的是**你指的那一行**，不是邻居 | ☐ |

> 17b 的意义：`NodeId` 在 2026-11-27 之前是"前序遍历序号"，删掉一行会让后面的控件 id
> 整体前移，而读屏器动作是**异步**到达的 → 点"删除"会删到邻居。现在 `NodeId` 是
> `NodeInfo.id` 的稳定哈希，已删除节点的动作会被**丢弃**（而不是错误派发）。
> 自动化测试覆盖"id 查得对"，但"讲述人念的是哪一行"只能人工看。

## 3. 反例（不该发生的事）

| # | 情况 | 期望 | 通过 |
|---|---|---|---|
| 18 | 读屏器念出「未命名按钮」「未知」 | **不该出现**（出现说明某个控件没给 label） | ☐ |
| 19 | 读屏器念组件内部实现的 id（如 `save-btn`） | **不该**被当作名称念出来 | ☐ |
| 20 | 焦点落在 canvas 上但读屏器无任何反馈 | **不该**（说明镜像/桥没生效） | ☐ |

## 4. 记录结果

把结果填进下表（按版本存档，便于回归对比）：

| 字段 | 内容 |
|---|---|
| 日期 | |
| 平台 / 发行版 | |
| 读屏器与版本 | |
| Y901 版本 / commit | |
| 通过项 / 总数 | / 20 |
| 失败项与现象 | |
| 备注（字体/缩放/多显示器） | |

失败项请按「控件 → 期望 → 实际」写成 issue，并附上 `--features a11y` 的启动日志。

## 5. 明确不在本清单内（已知边界）

- **浏览器 ARIA 镜像**：代码在 `y901-backend-web/src/aria.rs`，但 Web 属 Tier 3（演示级），
  且需要真实读屏器 + 真实浏览器组合，未纳入本轮验收；
- **焦点代理**：目前 `data-y901-focused` 只是标记，**不调用 `element.focus()`**
  （canvas 自己管焦点，两边抢焦点会让输入法错乱）；
- **文本光标位置**：canvas 无此语义，输入框只暴露 `aria-valuetext`，不暴露真实光标；
- **live region 播报调度**：交给读屏器读 `aria-live`，我们不主动控制播报时机；
- **复杂文本**：读屏器念的是**逻辑字符**（正确），但屏幕上是 shaping 后的字形，
  两者对不上属正常（与其它 GUI 框架一致）。

## 6. 如果发现失败，先查这三处

1. **忘了 feature**：控制台没有「桥已启用」那行 → 加 `--features a11y`；
2. **控件没 label**：`Widget::semantics()` 里 `label` 为空 → 映射层不会凭空造名字
   （这是刻意的，避免"看起来通过但不可用"）；
3. **树没推出去**：Windows 上 UIA 桥必须在 `ShowWindow` **之前**挂载
   （`crates/y901-backend-win32/src/window.rs` 的 `create_inner`），
   Linux 上由 `X11Window::render_tree` 每帧推。
