# DPI 策略：现状、证据与三个选项

> 状态：**已按选项 A 实现**（2026-11-18）。实现摘要见 §5，迁移注意见 §6。
> 相关：[`text-sharpness.md`](text-sharpness.md)（字形落点）、
> [`../design/grid-span.md`](../design/grid-span.md)（格宽/字宽契约）。

## 0. 一页结论

| 问题 | 现状 | 后果 |
|---|---|---|
| `Options.width/height` | 当作**物理像素**，客户区不随 DPI 变（640×320 在 100% 与 125% 下都是 640×320） | 125% 屏上窗口相对变小；UI 内容却按 1.25 倍长 |
| 控件内部度量 | **乘 `self.scale`**（全仓 85 处，如 `ROW_H * self.scale`） | 行高、圆点直径、按钮高度在 125% 下 +25% |
| 容器的 `padding` / `gap` | **固定不缩放**（全仓 54 处 `.padding()/.gap()`，`dpi_padding/dpi_gap` **0 处使用**） | 控件变大而留白不变 → 越来越挤，文字可能溢出固定高度的行 |
| 字号 | `px × dpi_scale`（`physical_font_size`） | 与控件度量一致，但与容器留白不一致 |

一句话：**框架一半按 DPI 缩放（控件、字号），一半不缩放（容器留白、窗口尺寸）**，
三者只在 100% 下自洽。

## 1. 证据

### 1.1 代码事实

| 位置 | 行为 |
|---|---|
| `crates/y901-backend-win32/src/window.rs` `physical_font_size()` | `size × dpi_scale`；`apply_dpi()` 在 `WM_DPICHANGED` 与首次显示后各校准一次 |
| `crates/y901-widgets/src/table.rs:151,195,202` | `HEADER_H * self.scale`、`ROW_H * self.scale`、命中测试同式 |
| `crates/y901-widgets/src/*.rs` | 85 处 `* self.scale`（alert/badge/table/…） |
| `crates/y901-ui/src/layout.rs` | 存在 `gap_scaled()` / `padding_scaled()` API —— **全仓 0 处使用** |
| `crates/y901-shell/examples/table.rs:44` | `Flex::padding(Edges::all(24.0))` —— 固定值 |

### 1.2 实测

来源有两块，**分开标注**，不混为一谈：

**(a) 真实窗口观测**（本机显示缩放曾在 100% 与 125% 之间切换，两次都抓了同一份 `--example table`）：

| 观测量 | 100% 时 | 125% 时 | 谁在缩放 |
|---|---|---|---|
| 窗口外框（UIA 物理像素） | 656×359 | 658×367 | 边框随 DPI 变（DWM 行为） |
| 客户区 | **640×320** | **640×320** | 不缩放（`Options` 被当物理像素） |
| 表格行基线 y | 244 / 272 / 300 / 328 / 356 → **行距 28px** | 199 / 231 / 266 / 299 / 334 → **行距 ≈35px**（= 28 × 1.25） | `ROW_H * self.scale` |
| 表头字高 | 14px | 15px（字号 16 → 物理 20 的可见部分） | 字号 × 1.25 |
| 卡片左内边距 | 24px | **24px**（未变） | `Flex::padding(24.0)` 固定 |

**(b) 代码事实**（不依赖机器 DPI，可复查）：

- 控件内部度量乘 `self.scale`：全仓 **85 处**（`table.rs:151,195,202` 等）；
- 容器 `padding/gap` 固定：`y901-widgets/src` 与 `y901-shell/examples` 共 **54 处** `.padding()/.gap()`；
- `padding_scaled()` / `gap_scaled()`（`layout.rs`）：**0 处使用** —— 当年留的显式缩放口子，没人用。

即：**纵向被放大 1.25 倍，横向留白没有** —— 表格内容在 125% 下明显更挤。

> 复现方式（两种都行）：① 系统显示缩放切到 125%，跑 `cargo run -p y901-shell --example table`，
> 抓 5 行文字的基线 y；② 用 `Y901_TEST_DPI_SCALE` 固定缩放跑同一示例。
> 注意 ② 的边界：本机实测该变量会改变 **字号**（`ascent` 13.75 → 17.19），
> 但**不会**改变 `Options` 的客户区像素数 —— 这正是本文要说的问题本身。


### 1.3 已暴露的 bug（同一个根因）

`snapshot_harness::invalidate_rect_partial_equals_full` 在 125% 下曾经红：
测试行固定 20px 高、文字从 `+2` 起笔，而 16 号字在 125% 下物理高度是 20px →
**字形溢出这一行**，脏区外的那两行像素既是旧帧也没被清掉，局部刷新与全量重绘必然不一致。
已按"脏区覆盖实际字形高度"修好测试，但根因（固定高度 + 缩放字号）仍在框架里。

## 2. 三个选项

### 选项 A：DIP 化（向 Windows 标准靠拢）——**推荐**

- `Options.width/height` 语义改为 **DIP**：客户区 = `round(DIP × dpi_scale)`；
- `Flex` 的 `padding` / `gap` **默认乘 scale**，另加 `fixed_padding()` / `fixed_gap()` 逃生舱；
- 既有 `padding_scaled/gap_scaled`（0 使用）退化为**与默认等价的别名**，避免两套语义；
- 根变换仍为 1.0：布局直接产出物理像素，后端不再乘一次。

| 维度 | 评价 |
|---|---|
| 用户预期 | ✅ 与系统其他应用一致：125% 下窗口与文字一起变大 |
| 一致性 | ✅ 控件、字号、留白、窗口同源缩放 |
| 改动面 | ⚠️ 窗口层 1 处 + `Flex` 语义 1 处 + 全仓 54 处 `padding/gap` 需要 review（多数会自动正确，少数"1px 边框 / 命中阈值"要显式 fixed） |
| 风险 | ⚠️ 中高：动布局核心，但**像素门禁固定 `Y901_TEST_DPI_SCALE=1.0`**，哈希基线不受影响 |
| 分步 | 1) 窗口 DIP 换算 + 测试；2) `Flex` 默认缩放 + 逃生舱；3) 逐控件 review；4) 125%/150% 视觉验收 |

### 选项 B：统一为"纯物理像素"（DPI 无关）

- 控件去掉 `* self.scale`，字号也不乘 `dpi_scale`，窗口尺寸 = `Options`。
- 优点：语义最简单、像素级可复现、哈希最稳。
- 缺点：**125% 屏上 UI 整体小 20%**，与 Windows 其它应用并排会明显偏小；
  高 DPI（150%/200%）下不可用。
- 适用：游戏/仪表盘式固定分辨率 UI，与"桌面 GUI 框架"定位冲突。**不推荐**。

### 选项 C：保持现状 + 文档化"混合缩放"

- 不改代码，只写清楚"控件缩放、容器不缩放"，让调用方自己用 `dpi_padding` 补救。
- 优点：零风险、零改动。
- 缺点：每次新增控件/布局都要重新判断一次；54 处固定 `padding` 里"哪些该缩放"没有客观标准；
  今天这个脏区溢出就是它的直接产物。**长期成本最高**。

## 3. 若选 A：验收标准

1. **窗口层**：`Options{640,320}` 在 125% 下 `GetClientRect == 800×400`；
   100% 下仍是 640×320（像素门禁不受影响）。
2. **布局层**：同一棵树在 `scale=1.0` 与 `1.25` 下，`header_rect`/`row_rect`/`padding` 全部按 1.25
   等比（新增一条布局单测，不需要 GPU）。
3. **视觉回归**：`--example table`、`--example static_page` 在 125% 下
   "行高、留白、字号"三者比例与 100% 一致（截图像素测量，不是肉眼）。
4. **门禁**：`scripts/gate.ps1` 仍 **GATE OK**（像素哈希在 `DPI=1.0` 下不变）。


## 4. 需要你拍板的问题

1. ~~选 **A / B / C** 哪条？~~ → **已选 A**（2026-11-18）。
2. 若选 A：`Options.width/height` 改语义是**破坏性变更**（现有调用方以为传的是物理像素）。
   是否接受升 minor 版本（`0.1.20 → 0.2.0`）并写迁移说明？
3. 若选 A：`Flex::padding` 默认缩放后，是否保留 `dpi_padding` 作为**无操作别名**（减少调用方改动），
   还是直接删除、统一用 `fixed_padding`？

> 我的建议：**A + 升 0.2.0 + 保留 `dpi_padding` 别名**（过渡期零改动可用，文档标注"默认即缩放，此别名已冗余"）。

## 5. 实现摘要（选项 A）

| 位置 | 改动 |
|---|---|
| `Options.width/height` | 语义改为 **DIP**：`y901-runtime` 文档更新，`describe` 里写明 `round(DIP × dpi_scale)` |
| `window.rs::create_inner` | 客户区 = `round(DIP × scale)`，再 `AdjustWindowRectExForDpi` 反算外框 |
| `window.rs::create_time_dpi_scale()` | 新增：创建期缩放比，**优先级与后端初始化同源**（`Y901_TEST_DPI_SCALE` 可覆盖，否则系统 DPI） |
| `Flex::padding()` / `gap()` | 量的单位改为 **DIP**，实际值 × `metrics.scale()` |
| `Flex::padding_fixed()` / `gap_fixed()` | 新增逃生舱：**不缩放**的物理像素（1px 描边内缩、命中阈值） |
| `Flex::padding_scaled()` / `gap_scaled()` | 保留为**与默认等价的别名**（既有调用点零改动） |
| 测试 | 新增 `crates/y901-ui/tests/dpi_layout.rs`（3 条：缩放 / 不缩放 / 后设置者优先）；`gap_scaled_precedence.rs` 改为新契约并补 `gap_fixed` 用例；`d3d_text_scaling.rs` 固定 `DPI=1.0`（它断言的是像素数与墨量比，与 DPI 无关） |

**为什么 `create_time_dpi_scale` 必须尊重 `Y901_TEST_DPI_SCALE`**：门禁的像素哈希按
`400×220` 巡检，若客户区按系统 DPI 放大，150% 机器上会变成 `600×330`，
哈希基线立刻失效——那不是渲染回归，是环境差异。

## 6. 迁移注意（破坏性变更）

1. `Options.width/height` 从"物理像素"变成"DIP"：在 100% 缩放下**完全一致**，
   在 150% 缩放下客户区会大 1.5 倍（这是本次要的行为）。
   需要升 **minor**（`0.1.20 → 0.2.0`）并在 RELEASE_NOTES 写迁移说明。
2. `Flex::padding()` / `gap()` 现在随 DPI 缩放：绝大多数调用点**不需要改**；
   只有"必须恰好 n 物理像素"的地方要改成 `padding_fixed()` / `gap_fixed()`。
   已全仓检查：`1px` 分隔线走的是 `FlexItem::height(1.0)`（本来就 ×scale），不受影响。
3. 像素哈希门禁不受影响（`scripts/hash-gate.ps1` 固定 `Y901_TEST_DPI_SCALE=1.0`）。

## 7. 平台差异（当前）

| 平台 | `Options` 语义 | 缩放来源 | 状态 |
|---|---|---|---|
| Windows（win32） | DIP，客户区 = `round(DIP × scale)` | `GetDpiForSystem` / `WM_DPICHANGED`；`Y901_TEST_DPI_SCALE` 可覆盖 | 已实现 |
| Linux（x11） | **同样按 DIP 解释**；客户区 = `round(DIP × scale)` | `Y901_X11_SCALE` → `Y901_DPI` → XSettings（`Xft/DPI` / `Gdk/WindowScalingFactor`）→ 根窗口 `Xft.dpi` → **单一 XRandR 活动输出的物理 DPI** → setup 的屏幕毫米估算（吸附 0.25 档位） | 已实现（v0.2.2 起；v0.2.3 补 `Xft.dpi`、XSettings 与**运行期跟随缩放**；v0.2.6 补 XRandR 输出几何/物理 DPI 查询） |

结论：契约跨平台一致（"Options 是 DIP"）。运行期改缩放会在约 1 秒内被窗口发现
（轮询 XSettings/Xft.dpi，不订阅 `PropertyNotify`），随后重建光栅器并按新物理尺寸调整客户区。
XSettings 与 `Xft.dpi` 都不做 0.25 吸附（它们是用户意图），只有毫米估算与 XRandR DPI 吸附。

### 7.1 每显示器 DPI（v0.2.6）

X11 没有"每窗口 DPI"窗口属性；全局缩放只能在建窗前定一次，而那一刻还不知道窗口会落在哪块屏。
因此 v0.2.6 的处理是**分开两件事**：

1. **全局缩放（建窗前）**：只有**恰好一个** XRandR 活动输出时，用它的物理 DPI 代替整屏毫米估算
   （单屏下更准，多屏时整屏毫米会把几块屏的物理尺寸加在一起）。多输出时**不猜屏**，保持原毫米估算。
2. **按位置查询（每显示器）**：`Connection::active_output_dpis()` 返回各输出的 CRTC 位置/尺寸与
   物理 DPI；窗口位置确定后用 `randr::dpi_at(&outputs, x, y)` 取所在屏的 DPI。这是唯一不猜的做法。

已知边界：镜像模式（两个 output 共用同一 CRTC）会返回两条记录，`dpi_at` 取先匹配的一条；
两者 DPI 相同，结果一致。热插拔导致的资源变化会让单次输出查询被跳过，而不是整体报错。
