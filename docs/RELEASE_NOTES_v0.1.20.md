# Y901 v0.1.20 发布说明

> 版本：`0.1.20` · 日期：2026-11-16

## 新增：WASM 后端（`y901-backend-web`）

浏览器端从"能编译"走到"能交互"：

- **S1 渲染**：手写 DOM 绑定（不依赖 `web-sys` / `js-sys`）、`WindowBackend` 实现、
  与桌面共用的 `frame_step`（新增 `dt` 参数）、rAF 帧循环、首次 layout + 首帧 Present。
- **S2 交互**：鼠标（move/down/up）、滚轮、键盘 `keydown`、命令（`save` / `cancel`）、
  光标形状（`tree.cursor_at` → `cursor: … !important`）、窗口 resize。
- **中文输入（IME）**：隐藏 `<input>` 承接组字；**ASCII 走 `input` 事件、中文走 `compositionend`**，
  两条通路互不重叠，交叉处用 `last_committed` 去重。
- **局部重绘**：按脏区裁剪的整树重绘（方案 D）+ 局部 `putImageData`，
  脏区块数 > 8 或面积 > 视口 50% 时回退整帧。

## 修复

| 类别 | 内容 |
|---|---|
| **帧节拍** | `frame_step` 改为显式接收 `dt`（桌面 `Instant` 差值、wasm rAF 时间戳）；此前内部取"当前时间"导致 dt≈0、动画静止 |
| **`DirtyRegion` 语义** | 渲染改为"每个脏矩形各画一遍整树并裁剪"，脏区外像素不再被清屏色覆盖（历史黑屏的根因）；弹层打开与动画尾帧强制整窗重绘 |
| **字体度量** | `bearing_y` 确认是**位图底边**偏移，字形顶部 = 基线 − `bearing_y` − 高度；同时修正 `ascent` 被 `cell_h / cell.h` 错误缩放的问题 |
| **上屏性能** | 像素上传从逐字节 `data[i] = v`（800×600 每帧 192 万次跨界调用）改为 `TypedArray.set` 一次批量拷贝 |
| **光栅化** | `fill_rect` 不透明快路径（`sa == 255` 时整行 `fill()`，与 `blend()` 逐位等价） |
| **稳定性** | rAF 闭包与 `Listener` 的强引用环改 `Weak`，新增 `dispose()` / `y901_dispose()`；字体位图尺寸上限（4096 / 16M 像素）；Win32 小键盘映射 |

## 已知债务

| # | 项 | 说明 |
|---|---|---|
| 1 | **无 `RawEvent::KeyUp`** | 只有 `KeyDown` / `SystemKeyDown` / `Char` / `Ime`；按键抬起、持续按住无法表达。需给 runtime 加变体并同步三个后端 |
| 2 | **TTC 只取 face 0** | `ttf_parser` 能读 TTC 的第 0 个 face；多 face 选择未支持，解析失败时会提示"导出单个 TTF/OTF" |
| 3 | **高频 MouseMove 未合并** | 每次移动都入队一条事件；合并在 DOM 回调层做，尚未实现 |
| 4 | **web 端未消费的宿主钩子** | `set_fullscreen` 为占位；`ime_position` / `take_font_request` / `take_snapshot_request` 在浏览器上不生效（见 `docs/WASM.md` §4.6） |
| 5 | **性能专项** | 全量重绘 + 整帧上传的路径仍在（阈值回退时会走），真机手感待复测 |

## 验证

- `cargo test --workspace`：**932 用例**全绿
- `cargo clippy --workspace --all-targets -- -D warnings`：exit 0
- `hash-gate`：`0x39AE41C1697CBFD0`（桌面帧哈希，与 v0.1.19 一致）
- `cargo check -p y901-shell --target wasm32-unknown-unknown`：exit 0，**0 警告**

## 升级注意

- 页面必须提供 `<canvas id="y901-canvas">`，CSS 负责尺寸（`width:100vw; height:100vh`）。
- 字体是运行时资源：放 `www/fonts/default.ttf` 并**同步修改页面里的 `?v=` 参数**，否则浏览器会继续用缓存。
- 单页单 runtime；要热重启请先调 `y901_dispose()`。
