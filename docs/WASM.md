# Y901 WASM 后端设计方案

> 版本：2026-11-14 · 状态：设计稿（已批准） · 工作量：13-16 人日

## 1. 方案概述

新增 `y901-backend-web` crate，把现有的 `WindowBackend` 契约实现在浏览器 canvas 上，
让同一套 `UiTree` / 组件 / 运行时在 wasm32 上跑起来。渲染沿用**软后端产出的 RGBA 像素**，
经 `putImageData` 上屏；帧循环由 `requestAnimationFrame` 驱动，而不是阻塞等待消息。

## 2. 核心决策

| # | 决策 | 理由 |
|---|---|---|
| **D1** | 复用 `y901-runtime` 的帧步逻辑，**不另写一套循环** | 事件→命令→动画→重排→渲染的顺序已经在桌面验证过 |
| **D2** | `run()` 在 wasm 上立即返回，帧由 rAF 逐次推进 | 三个示例的 `main` 都是单表达式，`run` 之后没有逻辑，立即返回不丢任何东西 |
| **D3** | 呈现用 `putImageData`，不做增量 dirty 区上屏 | 浏览器侧拿不到可靠的脏区合成；S1 先求正确，性能留 S2 |
| **D4** | DPI 在 wasm 侧**强制 1.0** | 帧哈希要跨后端对拍，`devicePixelRatio` 必须可预测 |
| **D5** | IME 走**隐藏 `<input>` 覆盖层 + 浏览器默认行为**，不做候选窗钉位 | 框架只有 `ime_candidate: Option<Pos>` 提示，候选窗由浏览器绘制，canvas 上不可控 |
| **D6** | **只依赖 `wasm-bindgen`，不依赖 `web-sys`**；canvas / event / rAF 绑定手写在 `src/bindings.rs` | `web-sys` 会让编译时间显著膨胀（见 §7），而我们需要的外部符号不到 20 个；手写绑定约 100-150 行，且有 golden test 兜底 |

## 3. 关键数据通路

### 3.1 事件

canvas 上绑定 `keydown` / `keyup` / `mousedown` / `mouseup` / `mousemove` / `wheel` / `resize`，
转成 `RawEvent` 后交给 `frame_step`。

IME：canvas 无法承载输入法。做法是在 canvas 上叠一个**透明的 `<input>`**，让它接收合成事件与
候选窗，浏览器负责候选窗的定位与绘制；我们只从 `compositionstart/update/end` 与 `input` 事件里取文本。
**不做候选窗钉位** —— 这是浏览器行为，`ime_candidate` 只作为语义提示保留。

### 3.2 呈现

**页面约定**：页面必须提供 `<canvas id="y901-canvas">`。wasm 分支内部通过
`document.get_element_by_id("y901-canvas")` 获取它，**`create_runtime()` 的签名保持不变**
（桌面调用方不受影响）。

软后端渲染到 `Vec<u8>`（RGBA），包成 `ImageData` 后 `putImageData` 到 2D context。
尺寸变化时重建 `ImageData`；每帧一次整幅拷贝，S1 不做局部更新。

### 3.3 DPI

**`devicePixelRatio` 一律按 1.0 处理，不读真实值。** 画布尺寸 = CSS 尺寸 × 1.0。
这样 `Y901_TEST_DPI_SCALE=1.0` 的桌面截图与 wasm 首帧可以逐字节对拍。

### 3.4 节拍

`requestAnimationFrame` 回调里调用一次 `frame_step`，并复用 `run_loop` 里的 Tick 节拍判断
（`tick_period` / `tick_subscribers`）—— 时钟源换成 `performance.now()`。

### 3.5 字体

`fetch` 加载字体文件 → `FontFace` 注册 → 就绪前先出一帧**占位**（用等宽回退字体），
就绪后全量失效重绘。不阻塞首帧。

## 4. 已知限制

| 能力 | 状态 |
|---|---|
| `FileDrop` | **不支持**（浏览器拖放走 DOM `drop` 事件，S1 不接） |
| 剪贴板 | 仅 `navigator.clipboard` 的文本读写，且需用户手势 |
| 多窗口 | **不支持**（一个页面一个 canvas） |
| `Focus` | 单 canvas，焦点由 DOM 元素决定；`EventCtx` 的 focus 路径仍生效，但页面级失焦只上报一次 `Focus(false)` |
| 光标形状 | 仅 `canvas.style.cursor` 的有限映射 |

## 5. 测试与验证策略

1. **golden test**：给定固定 `UiTree`，wasm 与桌面软后端产出的 `frame_hash` 必须相等（dpr=1.0）；
2. **wasm32 check**：`cargo check -p y901-backend-web --target wasm32-unknown-unknown` 必须 exit 0；
3. **绑定 golden test**：`bindings.rs` 的每个 `extern "C"` 声明都要有对应的 JS 侧名字断言
   （防止 `wasm-bindgen` 的 `module` / `js_name` 写错却静默链接失败）；
4. **native 不受影响**：`cargo check --workspace --all-targets` 必须保持 exit 0。

## 6. 任务拆解

| 阶段 | 内容 | 人日 |
|---|---|---|
| **S1** | 骨架 + `present` 管线 + rAF 帧步 + resize/DPI + **手写 JS 绑定（canvas/event/rAF）** | **4-5** |
| **S2** | `input` / `keys` 映射 + DOM 接线 + IME 覆盖层 | **3** |
| **S3** | 字体加载 + Tick 接入 + 帧哈希对拍 | **2-3** |
| **兼容性缓冲** | Safari/Firefox 的 canvas 合成差异、`dpr` 抖动、事件时序、字体加载竞态 | **3-5** |
| | **合计** | **13-16** |

## 7. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| `web-sys` 编译时间膨胀 | 每次全量构建多出数十秒 | **D6：不用 `web-sys`**，手写 100-150 行绑定 |
| 手写绑定写错 JS 名字 | 静默失败，运行时才炸 | §5 的绑定 golden test |
| 整幅 `putImageData` 低端设备掉帧 | 1080p 下每帧数毫秒拷贝 | S1 先保正确；S2 视测量结果再做脏区 / `createImageBitmap` |
| 浏览器 IME 候选窗位置不符合预期 | 输入体验差 | 接受浏览器默认行为（D5），不做钉位 |
| `dpr` 非 1.0 | 帧哈希对拍失败 | **强制 1.0**（D4） |
| `rAF` 在后台标签页暂停 | 动画停住（可接受） | 文档记录；不做补偿 |

## 8. 调查结论摘要（本设计的依据）

| 项 | 结论 | 出处 |
|---|---|---|
| host 参数类型 | **`&mut dyn Host`**（trait object，非泛型） | `crates/y901-runtime/src/runtime.rs:43-47` |
| `Host` trait | `pub trait Host` | `crates/y901-runtime/src/lib.rs:38` |
| `run()` 之后 | **三个示例的 `main` 都是单表达式，`run` 之后无逻辑** | `examples/counter.rs:106`、`switch.rs:60`、`spinner.rs:33` |
| 帧步可复用段 | 事件消化 → 命令/idle → 动画 → 重排 → 主题 → `after_frame` → `render` → `after_present` | `runtime.rs:114-179` |
| 桌面专属段 | `wait_events` 阻塞、profile 计时、Tick 节拍 | `runtime.rs:96`、`181-194`、`196-213` |
| 现有 wasm 代码 | **0 处**（`putImageData`/`web_sys`/`wasm32` 全仓无命中） | grep |
| `frame_hash` 测试 | `soft_backend.rs:134 fn frame_hash_is_deterministic()`，**未固定 DPI** | `soft_backend.rs` |
| `hash-gate` | 显式 `Y901_TEST_DPI_SCALE=1.0` | `scripts/hash-gate.ps1` |

### 8.1 `frame_step` 签名（已实现，含 `dt` 与 `close_requested`）

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

- `dt` 是**上一帧到本帧的时间步**，由调用方提供（桌面用 `Instant` 差值，wasm 用 rAF 时间戳差值）。
  为什么必须显式传：动画推进依赖真实 dt，若在 `frame_step` 内部取 `Instant::now()` 则每帧 dt≈0，**动画静止**。
- `close_requested` 承接 `RawEvent::Close`（桌面关窗）。以前它直接 `return`，现在改成标记，
  由调用方决定是否停止；**wasm 侧忽略该标记**。
- 首帧日志（`first_frame_done` / `started`）与 profile 统计**留在调用方**，不进 `frame_step`。
