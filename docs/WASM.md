# Y901 WASM 后端实现说明

> 本文设计与实现细节最初记录于 HEAD `2b1a0cd`，其历史决策表和实现状态未必反映当前源码。当前平台定位见 [`docs/PLATFORMS.md`](PLATFORMS.md)；无障碍现状以本文后附“无障碍现状”一节、CI 工作流和源码核对结果为准。
>
> **平台定位：Tier 3（演示/触达），不是产品级。** 明确的非目标：整帧 `putImageData`
> 上传、IME 候选窗不可钉位、单 canvas 无多窗口/FileDrop。DOM 无障碍树只在 `a11y` feature 下提供
> （隐藏 ARIA 镜像，已接入帧循环，但**未做浏览器与读屏器验证**）。需要产品级
> Web 能力的项目不应选这个后端。分层说明见 [`docs/PLATFORMS.md`](PLATFORMS.md)。

## 1. 方案概述

`y901-backend-web` crate 把 `WindowBackend` 契约实现在浏览器 canvas 上，让同一套 `UiTree` / 组件 /
运行时在 wasm32 上跑起来。

- **渲染**：`SoftBackend` 产出 **BGRA、自上而下**的像素，`Present` **只换通道（BGRA→RGBA）**，再 `putImageData` 上屏。行序不变。
- **帧循环**：`requestAnimationFrame` 驱动，`run()` 立即返回。
- **输入**：鼠标 / 滚轮 / 键盘 / 中文输入法全部接线完成（见 §3.1）。
- **依赖**：只用 `wasm-bindgen`，**不用 `web-sys` / `js-sys`**；所有 DOM 类型手写在 `src/bindings.rs`。

## 2. 核心决策与实际状态

| # | 决策 | 现状 |
|---|---|---|
| **D1** | 复用 `y901-runtime` 的 `frame_step`，不另写循环 | ✅ 已实现（含 `dt`，见 §8.1） |
| **D2** | `run()` 立即返回，帧由 rAF 推进 | ✅ 已实现 |
| **D3** | 呈现用 `putImageData` | ✅ 已实现；每次呈现都转换并上传整帧（见 §3.2） |
| **D4** | DPI 强制 1.0 | ✅ 已实现（`SoftBackend` 以 `dpi_scale = 1.0` 构造） |
| **D5** | IME 走隐藏 `<input>`，不做候选窗钉位 | ✅ 已实现；候选窗由浏览器绘制，我们不钉位 |
| **D6** | 只依赖 `wasm-bindgen`，绑定手写 | ✅ 已实现（`bindings.rs` 约 400 行）；绑定 golden test 已实现（见 §5） |

## 3. 关键数据通路

### 3.1 事件

**canvas 上**绑定：`mousemove` / `mousedown` / `mouseup` / `wheel` / `keydown`。
**window 上**绑定：`resize`。
**隐藏 `<input>` 上**绑定：`keydown` / `compositionstart` / `compositionupdate` / `compositionend` / `input`。

**没有 `keyup`**：框架的 `RawEvent` **没有 `KeyUp` 变体**（只有 `KeyDown` / `SystemKeyDown` /
`Char` / `Ime`），`translate()` 也没有对应分支。要支持按键抬起必须先给 runtime 加变体并同步三个后端。

#### 文本输入：两条通路，互不重叠

| 输入 | 通路 | 说明 |
|---|---|---|
| **ASCII / 直接字符** | 浏览器写入隐藏 input → **`input` 事件** → `RawEvent::Ime { composition: false }` → `UiEvent::Text` | `keydown` 对可打印键**直接 return**，不产生 `Char` |
| **中文（IME 组字）** | `compositionstart/update/end` | `update` → `Compose`（画拼音）；`end` → `Text`（提交） |

**唯一的交叉点**：部分浏览器在 `compositionend` 之后会把同一串回写进 `input.value` 并再发一次
`input`。用 `last_committed` 记录已提交串，`input` 回调遇到相同串就丢弃（`input.rs` 的输入过滤
是其中的纯判定，带单测）。

#### 坐标

**画面与 DOM 同向**：软缓冲自上而下、`ImageData` 自上而下、UI 树的 y 也自上而下，
所以 `offsetY` 直接就是 UI 的 y（`keys::to_ui_y` 只做越界钳制）：

```rust
// keys::to_ui_y(height, y_dom) = y_dom.clamp(0.0, height - 1.0)
```

> 历史坑：曾经 `present` 对整帧按行翻转（画面上下镜像），输入坐标只能跟着反向映射
> `height - 1 - y_dom` 才能点中。两层错误互相掩盖，页面其实是倒的。现已统一为自上而下。

**高度必须实时**（`WasmState.canvas_h: Rc<Cell<f32>>`，resize 时更新）；用安装时快照会导致
拉窗后点击命中错行。

#### 焦点

`mousedown` 时**先 `preventDefault()`**（阻止 canvas 抢回焦点）再把焦点给隐藏 `input`；
canvas 上设了 `tabindex="0"` 与 `outline: none`。`window` 的 `focus`/`blur` 已接线：`blur` 记下焦点是否还在隐藏 input 上，
`focus` 仅当 `keys::should_refocus(interacted, had_focus)` 为真时才 `input.focus()`。
**不能无条件抢**：页面刚打开或用户正在地址栏时都不应抢焦点。

#### 光标形状

`mousemove` 记录 UI 坐标 → 每帧 `apply_pending` 里 `tree.cursor_at(pos)` → 映射到 CSS
（`pointer` / `text` / `ew-resize` / `ns-resize` / `grab` / `default`），用
`style.setProperty("cursor", css, "important")` 写入。**不能用 `setAttribute("style", …)`**：
那会整条覆盖内联样式，把页面 CSS 的 `width/height` 一并抹掉，resize 就此失效（踩过）。

### 3.2 呈现

**页面约定**：页面必须提供 `<canvas id="y901-canvas">`。wasm 分支通过
`document.get_element_by_id("y901-canvas")` 取它，**`create_runtime()` 签名不变**。
页面 CSS 负责尺寸（`width:100vw; height:100vh`），Rust 只写背存储尺寸与 `cursor`。

**像素通路**：

1. `SoftBackend` 的缓冲是 **BGRA、自上而下**，与 `ImageData` 的行序一致，
   **不翻行**（只换通道 BGRA→RGBA）。
2. 每次呈现均走**整帧路径**：`bgra_to_rgba(全帧)` → `createImageData(w,h)` →
   `data.set(bytes)`（一次批量拷贝）→ `putImageData(0,0)`。没有局部矩形上传实现。

**性能要点**：像素拷贝使用 `TypedArray.prototype.set`（**一次跨界调用**）。
旧实现逐字节写 `data[i] = v`，800×600×4 = **192 万次跨界调用**。

**重绘策略**：每次需要呈现时先清空软缓冲，再令整棵 UI 树失效并完整重绘，最后上传整帧。
这避免了“清屏后只绘制脏区”造成的画面丢失；Web 后端没有脏区裁剪绘制或局部上传路径。

### 3.3 DPI

**`devicePixelRatio` 一律按 1.0 处理，不读真实值。** 画布尺寸 = CSS 尺寸 × 1.0，
这样 `Y901_TEST_DPI_SCALE=1.0` 的桌面帧与 wasm 帧可逐字节对拍（`hash-gate`）。

### 3.4 节拍

`requestAnimationFrame` 回调里推进一帧，**`dt` 来自 rAF 回调的时间戳差值**（钳到 1..100 ms），
不使用 `performance.now()`、也不使用 `Instant::now()`。桌面端用 `Instant` 差值，两条路径都通过
`frame_step(..., dt)` 显式传入。

唯一用到时钟的地方是一行**自限性能探针**：某一帧耗时 > 100 ms 时打印
`[DBG-PERF]` 日志（历史诊断，现由 `Y901_PROFILE` 统一控制）。

### 3.5 字体

**不使用 `FontFace`**。流程是：

1. 页面 `fetch('./fonts/default.ttf?v=…')` 拿字节；
2. `y901_font_loaded(bytes)` → `MyFontEngine::new(bytes, 16.0, cell)`；
3. 用**引擎自己的度量**（`cell_width()` / `cell_height()`）算格子 → `soft.set_font(...)`；
4. **`tree.clear_layout_cache()`** → `tree.layout(...)` → `tree.invalidate_all()`。

第 4 步不能省：`UiTree::layout` 只在视口变化时失效布局缓存，换字体不改变视口，不手动清缓存就会
"布局按旧度量、绘制按新度量"（表现为表头错位、标签与输入框重叠）。

就绪前先出一帧**零字形占位**（`WasmFontEngine`），不阻塞首帧。解析失败会打印
`[y901] 字体解析失败（N 字节）：<reason>`，而不是静默失败。

## 4. 已知限制与债务

| 能力 | 状态 |
|---|---|
| `FileDrop` | **不支持**（不接 DOM `drop`） |
| 剪贴板 | 仅框架内的 `TextInput` 剪贴板抽象，未接 `navigator.clipboard` |
| 多窗口 | 不支持（一个页面一个 canvas / 一个 runtime） |
| `Focus` 事件 | `EventCtx` 的焦点路径生效；`window` 的 `focus`/`blur` 已接线（`should_refocus` 两条件闸门） |
| `keyup` | **无** `RawEvent::KeyUp`，无法表达"按住/释放" |
| 字体格式 | 支持 TTF/OTF；**TTC 容器走 `ttf_parser` 的 face 0**（能读则正常使用），解析失败时会显式提示"导出单个 TTF/OTF" |
| 绑定 golden test | ✅ 已实现（`tests/bindings_golden.rs`，5 条断言锁住 js_name 集合与属性计数） |
| js_sys | 离线环境解析不到，因此 `Object` / `Reflect` 等均未使用，相关能力用手写绑定替代 |

**热重启**：单页单 runtime。要重启必须先调用 `y901_dispose()`（取消 rAF、摘监听器、移除隐藏
input、清全局 `STATE`），否则旧帧循环会与新实例同时向同一 canvas 推像素，并且每次重启多一个隐藏
input。

## 4.5 换字体（页面侧，不需重建）

页面用的字体是**运行时 `fetch` 的静态资源**，不编进 wasm，因此换字体不需要 `wasm-pack build`：

1. 把字体放到 `www/fonts/default.ttf`（文件名固定）；TTC 容器会取 **face 0**，
   若解析失败会打印 `（检测到 ttcf 文件头：这是 TTC 字体集容器）请导出单个 TTF/OTF 字重`；
2. 同步修改 `www/index.html` 里字体 URL 的版本参数：`fetch('./fonts/default.ttf?v=...')`
   —— **不改就会一直读浏览器缓存的旧字体**（这个坑已踩过）；
3. 刷新页面，Console 打印 `[y901] font swapped in`；失败会打印
   `[y901] 字体解析失败（N 字节）：<reason>`。

选字建议：`SimHei`（黑体）字形粗糙、字重单一，只适合验证渲染链路；看效果建议换
**微软雅黑 `msyh.ttc`**（TTC，需先导出单 face）、**思源黑体** 或 **Noto Sans CJK**。

**安全上限**：单个字形位图 **≤ 4096×4096 像素 / ≤ 16M 像素**（`raster::area_is_safe` 与
`font_engine::MAX_GLYPH_DIM`）。超限时拒绝光栅化并打印一行提示——包围盒来自字体文件里的坐标，
损坏或恶意字体可以写出巨大值，不拦会直接 OOM。

## 4.6 web 端未消费的宿主钩子

`WindowBackend` 的默认实现让得无需实现的钩子不会报错，web 端因此**只实现了
`wait_events` / `viewport` / `relayout` / `render` / `set_theme`**。以下钩子在桌面上有真实现，
在浏览器上**不生效**（已知差异，不是漏写）：

| 钩子 | 桌面行为 | web 现状 | 为何 |
|---|---|---|---|
| `set_fullscreen` | 切换真正全屏 | 默认 `Ok(())`（**占位**，不做事） | 浏览器全屏需用户手势 + Fullscreen API，尚未接 |
| `after_frame` 中的 `host.ime_position` | 把 IME 候选窗钉到光标 | **不消费** | 候选窗由浏览器自己绘制，不可钉位（设计决策 D5） |
| `after_frame` 中的 `host.take_font_request` | 动态换字体 | **不消费** | web 的字体路径是 `y901_font_loaded(bytes)`，由页面 fetch 驱动 |
| `after_frame` 中的 `host.take_snapshot_request` / `after_present` | 帧快照回读（验收用） | **不消费** | 浏览器侧直接截屏即可，无需框架中转 |

注意：`host.idle` 与 `host.command` 在 web 端**是消费的**（`MinimalHost` 就在
`wasm_entry.rs` 里处理 `save` / `cancel`），上表只列未消费的部分。

若未来需要在浏览器上支持全屏或动态换字体，正确做法是：在 web 端覆盖
`set_fullscreen` 调 Fullscreen API，并把 `take_font_request` 的请求转成一次页面侧
`fetch` → `y901_font_loaded`，而不是在 Rust 里直接读字体文件。

## 5. 测试与验证策略

1. **`hash-gate`**：桌面软后端的帧哈希必须等于基线 `0xF7455CCE7781A864`（`Y901_TEST_DPI_SCALE=1.0`）。
   它是"桌面零回归"的硬判据。
2. **wasm32 check**：`cargo check -p y901-shell --target wasm32-unknown-unknown` 必须 exit 0。
   （`--workspace` + wasm32 **不能**通过：`y901-backend-win32` 没有 cfg 守卫，这是既有情况。）
   单入口 `y901` 曾经也构建不了 wasm32：它经 `y901-extras → portable-pty → filedescriptor`
   拖进一条在 wasm32 上编译不过的链（`cannot find type RawFileDescriptor`）。
   **现已按 target 门控修好**（`crates/y901-extras/Cargo.toml` 把 `portable-pty` /
   `crossbeam-channel` 放进 `cfg(not(target_arch = "wasm32"))`），
   `cargo check -p y901 --target wasm32-unknown-unknown` 现在 exit 0，CI 已固定校验
   （含 `--features a11y` 与"PTY 仍在宿主机依赖树里"的反向守卫）。
   wasm32 上 `TerminalWidget` 依然可用，只是 `attach_shell` 返回明确错误、`pump` 恒 false、
   `write` 空操作；宿主用 `feed_vt_for_test` 注入 VT 字节。
3. **纯函数单测**：事件映射（`keys.rs`）、坐标映射（`to_ui_y`）、通道交换
   （`present.rs`）与脏区裁剪重绘逻辑（`tree.rs`，用记录型 `RenderCtx`，不需要 GPU）。
4. **native 不受影响**：`cargo check --workspace --all-targets` 与 `cargo test --workspace` 必须绿。
5. **绑定 golden test**：已实现（`crates/y901-backend-web/tests/bindings_golden.rs`）。
   native 下把 `bindings.rs` 当**源码快照**断言：50 个 `js_name` 值（含大小写）、
   `Date.now` 命名空间形态、各属性计数（getter/setter/constructor/indexing_setter/structural）、
   `GLOBAL_WINDOW` 必须是 `thread_local_v2` 静态。改名/删除会直接红。

## 6. 阶段回顾（已完成的实现）

| 阶段 | 内容 | 状态 |
|---|---|---|
| **S1** | 骨架 + `present` 管线 + rAF 帧步 + 首次 layout + 字体换装 + 方向/度量修正 | ✅ |
| **S2** | 事件接线：鼠标 / 滚轮 / 键盘 / IME / 命令 / 光标 / resize | ✅ |
| **性能** | 整树重绘 + 整帧上传 + 批量拷贝 | ✅（实机手感待复测） |

## 7. 风险与对策（仍在生效的部分）

| 风险 | 对策 |
|---|---|
| 手写绑定写错 JS 名字 → 运行时才炸 | 已有 native golden test（源码快照断言）+ 真机逐项验证 |
| `putImageData` 的跨界成本 | 整帧批量 `set()`，不做局部上传或阈值回退 |
| 浏览器 IME 候选窗位置不可控 | 接受浏览器默认行为（D5），不钉位 |
| `dpr` 非 1.0 破坏对拍 | 强制 1.0（D4） |
| rAF 在后台标签页暂停 | 接受（动画停住），不做补偿 |
| 缓存导致测的不是最新产物 | 页面 JS/字体 URL 都带 `?v=`；每次重建必须换值 |

## 无障碍现状

`crates/y901-backend-web/src/aria.rs` 在可选 `a11y` feature 下提供 `AriaMirror` DOM helper；
它把平台无关 ARIA 映射结果写成隐藏 DOM 节点，但截至本文最近核对时没有 runtime 调用点，
因此不会随 UI 帧自动发布。CI 的 `web-aria` job 只执行 wasm32 `cargo check`；没有浏览器 DOM
集成测试，也没有读屏器验收或焦点代理。不要将 helper 的存在描述为已集成的 Web 无障碍树。

## 8. 接口事实摘要

| 项 | 事实 | 出处 |
|---|---|---|
| host 参数类型 | `&mut dyn Host`（trait object） | `crates/y901-runtime/src/runtime.rs` |
| `Host` trait | `pub trait Host` | `crates/y901-runtime/src/lib.rs` |
| `RawEvent` 键盘变体 | `KeyDown` / `SystemKeyDown` / `Char` / `Ime`（**无 `KeyUp`**） | `crates/y901-runtime/src/lib.rs` |
| `translate()` | `KeyDown`,`SystemKeyDown` → `UiEvent::Key`；`Char` → `Text`；`Ime{composition}` → `Compose`/`Text` | 同上 |
| 桌面专属段 | `wait_events` 阻塞、profile 计时、Tick 节拍 | `runtime.rs` |

### 8.1 `frame_step` 签名（已实现）

```rust
pub fn frame_step<W: WindowBackend>(
    window: &mut W,
    tree: &mut UiTree,
    host: &mut dyn Host,
    events: &[RawEvent],
    dt: std::time::Duration,
) -> Result<FrameStats, String>;

pub struct FrameStats {
    pub had_work: bool,
    pub need_layout: bool,
    pub presented: bool,
    pub close_requested: bool,
}
```

- `dt` 是上一帧到本帧的时间步，由调用方提供（桌面 `Instant` 差值，wasm rAF 时间戳差值）。
  必须显式传：动画推进依赖真实 dt，若内部取 `Instant::now()` 则每帧 dt≈0、动画静止。
- `close_requested` 承接 `RawEvent::Close`；**wasm 侧忽略**该标记。
- 首帧日志与 profile 统计留在调用方，不进 `frame_step`。

### 8.2 `wait_events` 语义差异

wasm 侧**永不阻塞**：`wait_ms` 被忽略，返回当前队列（可能为空），节奏完全由 rAF 决定；
桌面侧可以阻塞等待消息。两者都返回 `Option<Vec<RawEvent>>`，`None` 在桌面上表示退出。
