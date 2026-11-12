# hash-gate 场景范围（实测结论）

> 本文修正╠之前若干轮报告里“场景不含某控件，所以 hash 未变”的错误推断。

## 1. `scripts/hash-gate.ps1` 跑的是什么

```powershell
cargo test -p y901-backend-win32 --offline --test font_3c_pixels -- --ignored
```

对应 `crates/y901-backend-win32/tests/font_3c_pixels.rs`。

## 2. 场景里实际渲染的 widget

**只有一个测试内自定义的 `Scene` widget**（`impl Widget for Scene`），它手绘：
`ctx.fill_rect` 色块 + `ctx.draw_text` 文字。

| 框架控件 | 场景里有吗 |
|---|---|
| `Radio` / `Checkbox` / `Tooltip` / `Tabs` / `Menu` / `Slider` / `TreeView` | **全都没有** |
| 自绘 `Scene` | 只有它 |

**因此：修改 `y901-widgets` 里任何控件的尺寸常量，`hash-gate` 都不会变。**

## 3. 之前各轮的错误解释（已被实验推翻）

| 提交 | 当时的说法 | 事实 | 结论 |
|---|---|---|---|
| `c0639cf` overlay | “demo 不打开菜单（`open == false`）” | 场景里**没有 `Menu`** | 结论对，**理由错** |
| `2f9e395` tabs | “demo 不用 `Tabs`” | 场景不含任何框架控件 | 碰巧对 |
| `8dda88b` tooltip/checkbox | “demo 不含这两个控件” | 同上 | 碰巧对 |
| （当时未提交）radio | “demo 确实渲染了 Radio” | **stash 掉 `radio.rs` 后 hash 不变**：与 radio 无关 | **错** |

## 4. 本轮的 bisect 实证（关键）

固定前（未固定 DPI）下逐个 checkout 跑门禁：

| 提交 | 跑出的 hash |
|---|---|
| `75af5b1`（DPI × scale） | **`0xFC83652FBBC7188B`** |
| `c0639cf` | `0xFC83652FBBC7188B` |
| `2f9e395` | `0xFC83652FBBC7188B` |
| `8dda88b` | `0xFC83652FBBC7188B` |

**结论：跃变就发生在 `75af5b1`，且之后不变。**

但 `75af5b1` 提交时记录的基线是 `0x87DECF871BB90AFF` ——
**同一份代码、不同时间，算出两个不同值** → **环境因素**。

## 5. 根因：hash-gate 从 `75af5b1` 起变成了 **DPI 相关**

- 场景画的就是**文字**；
- `75af5b1` 起，字号 = `FontConfig.size × GetDpiForWindow(hwnd)/96`；
- 本机实测 `GetDpiForSystem()=96`、`GetDpiForWindow(desktop)=120` —— **两个值不同**；
- 于是**同一份代码在不同 DPI 上会算出不同哈希**。
- `0x87DECF871BB90AFF` 既不是 DPI=1.0（`0xDBB3…`）也不是 DPI=1.25（`0xFC83…`）
  → **那个基线是从一个未能重现的环境里记下的**。

## 6. 修复：固定 DPI = 1.0

- `window.rs`：`Y901_TEST_DPI_SCALE` 环境变量优先于真实 DPI（仅测试/诊断用）；
- `hash-gate.ps1`：运行前设 `$env:Y901_TEST_DPI_SCALE = '1.0'`。

**修复后实测：hash = `0xDBB308DD1657EF9C`，与 `75af5b1` 之前的原始基线逐位一致** ——
这反过来证明：**DPI 缩放是当时 hash 变化的唯一原因**，渲染本身没有回归。
基线已改回 `0xDBB308DD1657EF9C`（按"固定 DPI=1.0"重记）。

## 7. 教训

1. **“hash 未变”只能证明"这次运行没变"**，不能推断"场景不含某控件"。
2. 要证明"场景不含 X"，**必须读源码**。
3. 要证明"hash 变化的原因"，**必须做 checkout 重跑实验**（本轮就是靠 stash/bisect 才排除了 radio）。
4. **引入依赖环境（DPI/字体）的改动时，必须同时固定门禁的环境变量** ——
   否则门禁会变成"谁跑谁改基线"。

## 门禁场景扩展（2026-11-12）

### 新增内容

- `Flex::row("gate-bar")` + `align::Center` + `gap(6.0)`
- 3 个 `TextLabel`：`t16`（默认 16px）、`t18`（`size(18.0)`）、`t13`（`size(13.0)`）
- 放在原 `Scene` 绘制区域的空白带（`r.y + 146`，高 40），**避开原绘制**

### 新旧 hash

| | 值 | 说明 |
|---|---|---|
| 旧 | `0xDBB308DD1657EF9C` | 只有 `Scene` 自绘，无框架控件 |
| **新** | **`0x39AE41C1697CBFD0`** | + `Flex` 布局 + 三档 per-text 字号 |

**客户区尺寸不变**（`400x220`）→ 变化完全来自新增布局段。

### 现在门禁覆盖

- **字形光栅化**（中英混排、SDF 圆角、渐变）
- **Flex 布局**（`align::Center` + `gap`）
- **per-text 字号**（16 / 18 / 13 三档）

### 仍不覆盖

- **DPI 常量**（场景 DPI 固定 1.0）
- **`WM_DPICHANGED`**（运行期 DPI 变化）
- **`wrap`**（场景未使用）
- **`cell_span` 零宽字符**（场景无零宽/组合/控制字符）

### 可复现性

`Y901_TEST_DPI_SCALE=1.0` 固定；扩展后连跑 **4 次**，hash 均为 `0x39AE41C1697CBFD0`。
