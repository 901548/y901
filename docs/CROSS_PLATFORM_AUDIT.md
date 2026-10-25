# Y901 跨平台 v3 · 阶段 0 盘点

> 结论全部来自 grep / 源码阅读，证据格式 `文件:行号`。
> 标注 **未验证** 的条目表示"需要读更多代码或做实验才能确认"，不做猜测。

## 1. windows-sys 使用清单

### 1.1 依赖声明（只有一处）

| 位置 | 内容 |
|---|---|
| `crates/y901-backend-win32/Cargo.toml:9` | `windows-sys = { version = "0.59", features = [...] }` |

**结论**：`windows-sys` 只被 `y901-backend-win32` 依赖，符合"平台依赖只允许出现在平台层"的约束。
其余 7 个 crate 的 `Cargo.toml` 无 `windows` 字样（grep 仅命中上面一行）。

### 1.2 按功能分类

| 功能 | 文件:行号 | 用到的 API | 是否已在 `y901-runtime` 抽象 |
|---|---|---|---|
| 窗口注册 | `window.rs:259` | `RegisterClassExW` | 否 |
| 窗口创建 | `window.rs:267` | `CreateWindowExW` | 否 |
| 消息循环 | `window.rs:321` | `PeekMessageW` / `DispatchMessageW` / `TranslateMessage` | 否 |
| 窗口过程 | `window.rs:100`（单 `wndproc`） | `DefWindowProcW` + 11 种 `RawEvent` 生产 | 部分（`RawEvent` 是抽象，生产点未抽） |
| 光标形状 | `window.rs:352-359` | `LoadCursorW` / `SetCursor`（`IDC_*`） | 否 |
| 客户区尺寸 | `window.rs`（`GetClientRect`） | `GetClientRect` | 否 |
| DPI 感知 | `window.rs:236` | `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` | 否 |
| D3D11 设备/交换链 | `d3d11.rs:1490` | `D3D11CreateDeviceAndSwapChain` | 否（无 `RenderBackend` trait） |
| D3D11 呈现 | `d3d11.rs`（`present`） | `IDXGISwapChain::Present` | 否 |
| DirectWrite 工厂 | `font.rs:481` | `DWriteCreateFactory` | 否（无 `GlyphProvider` trait） |
| DirectWrite 排版 | `font.rs`（`layout`/`advance`） | `IDWriteTextLayout` 等 | 否 |
| IME 上下文 | `ime.rs:89, 126` | `ImmGetContext` / `ImmReleaseContext`（FFI 声明在 `ime.rs:24`） | 否（无 `ImeBackend` trait） |
| IME 组合串 | `ime.rs:78` | `ImmGetCompositionStringW` | 否 |
| IME 候选窗 | `ime.rs:106` | `ImmSetCompositionWindow` / `ImmSetCandidateWindow` | 否 |
| 剪贴板 | — | **未实现**：只有 `Clipboard` trait + 内存/空实现（`text_input.rs:24, 31, 42`），**没有 Win32 剪贴板后端**（无 `OpenClipboard`/`GetClipboardData`） | 是（`Clipboard` 是 UI 层 trait，已抽象） |
| 系统字体枚举 | — | **未实现**：字体名硬编码 `FONT_FAMILY`（`backend/mod.rs:53` 附近） | 否 |

## 2. `y901-runtime` 已抽象 vs 仍耦合

### 2.1 已有抽象

| 项 | 位置 | 内容 |
|---|---|---|
| `Host` | `runtime/lib.rs:23` | `idle` / `command` / `event`（默认 Ignored）/ `policy`（默认 Policy）/ `ime_position` |
| `PlatformRuntime` | `runtime/lib.rs:47` | `run(host, build, options)` —— 单一入口 |
| `RawEvent` | `runtime/lib.rs:75` | 10 个变体（键、文本、组合、鼠标、滚轮、尺寸、焦点等） |
| `translate` | `runtime/lib.rs:89` | `RawEvent + viewport → Option<UiEvent>`，纯函数 |

### 2.2 仍耦合在 `backend-win32` 内部、需要新 trait 的部分

| 缺口 | 证据 | 建议 trait |
|---|---|---|
| 渲染设备与呈现 | `d3d11.rs:1490`、`backend/mod.rs` 的 `present/resize` | `RenderBackend`（设备创建、`present`、`resize`、`draw_instances`） |
| 字形获取与度量 | `font.rs:481`、`font.rs:497-508` | `GlyphProvider`（`glyph(c) -> Glyph{a,bearing,advance}`、`metrics()`） |
| 窗口生命周期 | `window.rs:259-282` | `WindowSystem`（`create`、`pump`、`drain_events`、`close`） |
| IME | `ime.rs:78-147` | `ImeBackend`（`composition_string`、`set_position`、`handle_message`） |
| 光标形状 | `window.rs:352` | 并入 `WindowSystem::set_cursor` |
| DPI 感知 | `window.rs:236` | 并入 `WindowSystem`（启动时一次性设置） |

### 2.3 签名可能不够抽象的地方

| 位置 | 问题 |
|---|---|
| `Host::policy()` | 让平台层（backend）反向依赖 `y901-agent::Policy`，破坏 `backend → runtime → ui` 的单向依赖。**这是已知架构债**（前几轮记录过，无调用点） |
| `PlatformRuntime::run(...)` | 一次性接收 `impl FnOnce() -> UiTree`，没有给"窗口重建"留接口；macOS 的 view 重建可能需要对钩子 |
| `RawEvent` | 变体命名/语义偏 Win32（如 `ImeCommit`/`ImeCompose` 直接对应 `WM_IME_*`），需要确认 macOS/WASM 能映射（**未验证**） |

## 3. v3 "格契约" 5 条不变量可行性检查

### 3.1 `cell_span` 只来自 unicode-width

- 证据：`crates/y901-core/src/lib.rs:6` `use unicode_width::UnicodeWidthChar;`
- 写入路径：`core/src/lib.rs:372` `if w == 2 { ... }`，`w` 由 `ch.width()` 得出（同一函数内）。
- **结论：满足**。核心层不依赖字体度量计算格数。
- 注意：`backend` 侧仍会用字体 advance 做**校验**（见 3.2），但不参与格数判定。

### 3.2 CJK 2 格居中

- 证据：`backend/font.rs:503-508`
  ```rust
  let ascii = advance_of('M');
  if (ascii - cell_width).abs() > 0.01 { return Err(...); }
  let cjk = advance_of('\u{4e2d}');
  if (cjk - cell_width * 2.0).abs() > 0.01 { return Err(...); }
  ```
- 渲染定位：`backend/mod.rs:197` `x + glyph.bearing_x as f32` —— 直接摆字形，**没有**"居中偏移 = (2*cell - advance)/2"的计算。
- **结论：不满足**。当前是"**依赖**字体 advance 恰好 2:1"，并用**运行时断言**在字体不满足时直接报错（`Err`），而不是在布局层做居中补偿。
- **影响**：换到 macOS/Linux/WASM 时，常见 CJK 字体的 `'中'` advance 未必精确等于 2×ASCII（等宽字体通常满足，但后备字体链、合成字形未必）。一旦断言触发，后端直接起不来。
- **v3 前置条件**：把"居中偏移"做成 `GlyphProvider` 的契约（返回 `advance` + `bearing`，由渲染层算 `(2*cell - advance)/2`），断言降级为 `debug_assert!` 或警告。

### 3.3 绝对定位 `col * cell_width`（无 pen 累加）

- 证据：`backend/mod.rs:274-296`
  ```rust
  fn draw_text(&mut self, origin: Pos, text: &str, style: &TextStyle) -> f32 {
      let mut pen_x = origin.x;                    // 275
      for ch in text.chars() {
          let span = self.text.cell_span(ch).max(1) as f32;   // 277 ← 格数来自 unicode-width
          ... Rect::new(pen_x, origin.y, span * CELL_W, CELL_H) // 281
          self.push_glyph(pen_x, origin.y, ch, ...);          // 286
          pen_x += span * CELL_W;                             // 294 ← 以"格"为单位累加
      }
  }
  ```
- **结论：实质满足，形式上是累加**。
  - 位置来自 `span * CELL_W`（**格单位**），不是字形 advance —— 因此不会因字体比例而漂移；
  - 逐步累加与 `col * CELL_W` 在数学上等价（`span` 恒为整数格），差别只有 f32 累积误差（同一条文本内 ≤ 数十个加法，可忽略）。
- **残留风险**：
  1. 若将来把 `CELL_W` 改成非整数（如 DPI 缩放后 7.5），f32 累加会引入亚像素漂移，需改成按格索引计算 `origin.x + col as f32 * CELL_W`；
  2. 该路径只覆盖 widget 文本；**终端网格的实例生成是否同样以格为单位，本轮未验证**（`atlas.rs` 未读）。

### 3.4 其余两条（本轮无直接证据）

| 不变量 | 状态 |
|---|---|
| 光标位置 = 格坐标（非像素） | 未见反例（`Cell`/`Grid` 用 col/row），**未逐行验证** |
| 宽字符占两格且尾部为 `WideTail` | 满足：`core/src/lib.rs:374-375` `Cell::wide_head` + `Cell::wide_tail` |

## 4. 各平台工作量估算

基准（现状，Win32）：

| 模块 | 行数 |
|---|---|
| `window.rs` | 353 |
| `backend/d3d11.rs` | 1378 |
| `backend/font.rs` | 719 |
| `backend/atlas.rs` | 445 |
| `backend/mod.rs` | 275 |
| `ime.rs` | 176 |
| `runtime.rs` | 65 |

估算方法：`适配行数 ≈ 现有行数 × 抽象化比例`；每行 Win32 专有代码换平台约 1:0.6~1:1 重写。

| 平台 | 窗口/事件 | 渲染 | 字体 | IME | 剪贴板 | DPI | 合计（行） | 未知风险 |
|---|---|---|---|---|---|---|---|---|
| **Windows** | 已有 | 已有 | 已有 | 已有 | 未实现（用 `NullClipboard`） | 已有 | **0**（剪贴板另计 ~150） | — |
| **Linux** | winit 适配 ~350 行（`WindowSystem` 实现 + 事件映射） | wgpu 后端 ~700–900 行（设备/交换链/管线 ≈ d3d11 的 60%） | fontdue + 字形缓存 ~500–600 行（font.rs 的 70–80%） | fcitx/ibus 经 winit IME 事件 ~200 行 | arboard ~50 行 | winit `scale_factor` ~20 行 | **~1820–2120** | Wayland 的 IME 支持（`text-input-v3`）依赖 compositor，**未验证** |
| **macOS** | winit ~300 行（`NSView` 生命周期） | wgpu/Metal ~700–900 行 | CoreText 或 fontdue ~500–700 行 | **已知弱点**：winit 的 IME 在 macOS 上偶发丢事件，~250 行 + 调试未知 | arboard ~50 行 | `scale_factor` ~30 行 | **~1830–2230** | 重试/补偿 IME 事件的复杂度**无法估算** |
| **WASM** | wasm-bindgen + DOM ~400 行（canvas 事件） | WebGL2/WebGPU ~600–800 行 | 浏览器 `measureText` 或预烘焙图集 ~300–400 行 | 组合输入（`compositionstart/update/end`）~150 行，**尽力而为** | `navigator.clipboard` ~60 行（需用户手势） | `devicePixelRatio` ~20 行 | **~1530–1830** | 光标/选择在 canvas 上的可访问性、字体加载时序**未验证** |

**说明**：
- "行数"指新增适配代码，不含把现有 Win32 代码重构到 trait 后面的成本（那部分是一次性的，约 **300–500 行重构 + 200 行测试**）。
- 以上是**工程量**估算，不是日历时间。按"每天有效产出 150–250 行含测试"折算，Linux 约 8–12 个工作日，macOS 约 9–13 天，WASM 约 7–10 天。

## 5. 建议的抽取顺序

### 第 1 步：`RenderBackend` trait

- **抽什么**：`create_device` / `present` / `resize` / `draw_instances(instances, atlas)` / `is_occluded`。
- **为什么先抽**：零平台语义（只有"给我一组实例和图集"），且**最容易验证**——同一组实例在两个后端应产出相同像素/相同绘制调用。它也是 D3D11 与 wgpu 差异最小的一层（都是"顶点缓冲 + 纹理 + draw call"）。
- **验证**：记录型 `RenderBackend`（把 `draw_instances` 的入参存进 Vec）跑现有全部渲染测试；再对拍 Win32 真机截图（1 张终端 + 1 张 widget）。

### 第 2 步：`GlyphProvider` trait

- **抽什么**：`glyph(c) -> Glyph { atlas_rect, bearing, advance }`、`line_height()`、`cell_size()`。
- **为什么第二步**：它直接决定 §3.2 的"居中偏移"能否落地。抽完就能把 `font.rs:503-508` 的硬断言改成"返回 advance，由渲染层算偏移"，从而**消除 v3 的一个前置阻塞**。
- **验证**：断言 `glyph('M').advance == cell_w`、`glyph('中').advance == 2*cell_w` 从"后端启动即报错"变为"布局层补偿"；加一个用**非等宽字体**跑的测试，验证不再 panic/Err。

### 第 3 步：`WindowSystem`（`create_window` / `pump_events`）

- **抽什么**：窗口创建、消息泵、`RawEvent` 生产、`set_cursor`、DPI 初始化。
- **为什么第三步**：依赖前两步（窗口要先能拿到渲染上下文）；回归影响最大（现有 11 种 `RawEvent` 全在这里生产），所以放在渲染/字体稳定之后。
- **验证**：`translate` 的单测已存在（`runtime/lib.rs` 3 项）；再加"记录型 WindowSystem 产出的事件序列"对拍。

### 第 4 步：`ImeBackend`

- **抽什么**：`composition_string` / `set_position` / `handle_message`。
- **为什么最后**：最难验证（依赖真实输入法、平台差异最大、无法在 CI 断言）。前三步完成后，IME 的失败面被隔离在单个 trait 内。
- **验证**：只能手工验收（中日韩输入、候选窗定位）+ 单元测试覆盖 `composition: true/false` 的 `translate` 分支（已有）。

### 建议的附加动作（先于第 1 步）

1. **修 §3.2**：把 `font.rs:503-508` 的硬 `Err` 改成 `debug_assert!` + 记录实际 advance（否则平台一换，后端起不来）。
2. **确认 §3.3**：读 `atlas.rs` 的实例生成代码，确认终端网格是否 `col*CELL_W`（本轮未验证）。
3. **清除 `Host::policy()` 对 `y901-agent` 的依赖**（`runtime/lib.rs:23`），否则 `runtime` 无法脱离 agent 单独移植。

---

## 附：本轮证据来源

```
grep -rn "windows\|windows-sys" crates/*/Cargo.toml      → 仅 backend-win32:9
wc -l crates/y901-backend-win32/src/**/*.rs              → 见 §4 表格
grep "CreateWindowExW|PeekMessageW|D3D11CreateDevice..." → 见 §1.2
grep "pub trait|pub fn" crates/y901-runtime/src/lib.rs   → §2.1
grep "unicode_width|cell_span" crates/y901-core/src/*.rs → §3.1
grep "advance|cell_width" crates/*/backend/font.rs       → §3.2
grep "pen_x|draw_text" crates/*/backend/mod.rs           → §3.3
grep "Clipboard" crates/*/src/*.rs                       → §1.2 剪贴板
grep "dpi|scale_factor" crates/*/src/*.rs                → §1.2 DPI
```

**未验证项汇总**（需要后续实验）：
1. 终端网格的实例生成是否 pen 累加（`atlas.rs` 未读）。
2. `RawEvent` 各变体在 macOS/WASM 的可映射性。
3. Wayland 下 IME 的可用性。
4. macOS 上 winit IME 丢事件的实际频率。
5. WASM 里 `unicode-width` 与浏览器 `measureText` 的一致性。
