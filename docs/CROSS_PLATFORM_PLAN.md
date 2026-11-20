# 全平台纯血自研底座 · 架构与落地评估

> 日期：2026-11-01　状态：**评估稿，等老板拍板**
> 前提：全部自研窗口/事件/呈现/IME，**禁止 winit / softbuffer / glfw / cocoa / x11 等第三方 GUI 库**；只允许纯 Rust 的 FFI 声明与标准库。

---

## 0. 一句话结论

> **技术上可行，但要认清一件事：我们不是"重写一个 winit"，而是"重写三个平台的窗口系统接入层"——按当前 Win32 后端的体量（6,588 行）推算，X11 约 12~18 人日、Cocoa 约 15~20 人日、Wayland 25~40 人日且"零 C"会破防（xkbcommon 是 C）。**
>
> **建议：先切 Linux/X11**——它是唯一能在**你这台 Windows 机器上做"协议级"完整验证**的平台
> （X11 协议是字节流，可离线做 golden test；`cargo check --target` 可查类型）。
> 注意：本机 WSL **只装了壳、没有发行版**（实测 `wsl -l -v` 报"没有已安装的分发版"），
> 所以"真窗"验证要么装一个发行版，要么走 CI。

---

## 1. 现状审计：我们离"业务零分叉"还差多远

### 1.1 已经干净的部分（好消息，比预期好）

| 层 | 平台相关代码 | 结论 |
|---|---|---|
| `y901-ui`（5,642 行） | **0** | 完全纯净：无 `HWND`、无 `cfg(target_os)`、无平台 crate |
| `y901-widgets`（4,148 行） | **0** | 同上游 |
| `y901-runtime`（466 行） | **0**（仅文档注释里提到 HWND） | 已定义 `PlatformRuntime` / `RenderBackend` / `PlatformWindowHandle` / `RawEvent` |
| `y901-backend-win32`（6,588 行） | 全部 | D3D11 2.4k + SoftBackend 0.5k + Window 0.9k + 字体 0.9k + 图集/runtime 1.8k |
| `y901-app`（205 行） | **1 行 `use`** | `use y901_backend_win32::runtime::Win32Runtime;` ← **唯一的平台分叉点** |
| 其它业务工程 | 各自独立仓库 | 不在本仓的跨平台范围内 |

**关键结论**：`y901-ui` 和 `y901-widgets` 已经天然平台无关——这轮"全适应"**不需要动业务与组件层**，只需要在 `runtime` 与 `app` 之间加一层。

### 1.2 需要修的三处

| # | 问题 | 现状 | 影响 |
|---|---|---|---|
| 1 | `app` 硬依赖 `y901-backend-win32` | `main.rs:9` | 换平台要改业务代码 |
| 2 | `Window::hwnd() -> HWND` 是 pub | 被 soft 后端与测试使用 | 平台类型泄漏到 crate 边界之外 |
| 3 | `RenderBackend` 有了，但**没有** `Presenter` / `EventLoop` / `WindowHandle` 三个原语 | 只有 `PlatformRuntime::run` 一个大入口 | 每个平台后端各写各的循环，难复用（幂等性、重绘策略、快照协议会各写一遍） |

### 1.3 现有 `RawEvent` 的覆盖度（统一事件集的底子）

已有：`MouseMove` / `MouseDown` / `MouseUp` / `Wheel` / `KeyDown` / `Char` / `Ime` / `Resize` / `SetCursor` / `Close` / `FileDrop`（11 种）。

**缺口**（三个平台都要处理，建议这轮一起加）：

| 缺口 | 为什么需要 |
|---|---|
| `FocusIn` / `FocusOut` | 失焦要收起光标/结束文本选择 |
| `ScaleFactorChanged(f32)` | macOS Retina（2x）、Linux 分数缩放（1.25/1.5） |
| `ModifiersChanged` | 单独按 Shift/Ctrl 不产生 KeyDown 时，UI 要更新状态 |
| `DpiChanged` | Windows 跨屏拖动（已有 WM_DPICHANGED 未转发） |

---

## 2. 目标架构：四个原语 + 一个壳

### 2.1 架构图

```
┌──────────────────────────────────────────────────────────┐
│  业务应用（独立仓库，零 cfg）                              │
└───────────────┬──────────────────────────────────────────┘
                │ 只依赖 y901-shell（唯一的 cfg 阶梯）
┌───────────────▼──────────────────────────────────────────┐
│  y901-shell   ← 一个文件里写 #[cfg(target_os)] 的工厂      │
│    pub fn run(host, build, options) -> Result<(), String>│
└───────────────┬──────────────────────────────────────────┘
                │ 依赖 0..1 个后端（Cargo feature / target_os）
┌───────────────▼──────────────────────────────────────────┐
│  y901-runtime   ← 四个原语（本报告新增设计）               │
│    trait PlatformRuntime { run(...) }                     │
│    struct WindowHandle { raw: PlatformWindowHandle, .. }   │
│    trait EventSource { poll(timeout) -> Vec<RawEvent>; wake() }│
│    trait Presenter   { resize(w,h); present(&[u8], w, h) }│
└───────────────┬──────────────────────────────────────────┘
      ┌─────────┼──────────────┬──────────────────┐
┌─────▼─────┐┌──▼──────────┐┌──▼──────────────┐
│ backend-  ││ backend-x11 ││ backend-cocoa   │
│ win32     ││ (手写协议)   ││ (手写 objc_msgSend)│
│ D3D11+Soft││ Soft + MIT-SHM││ Soft + CGImage │
└───────────┘└─────────────┘└─────────────────┘
```

### 2.2 四个原语的职责（关键设计：**像素呈现与事件循环解耦**）

| 原语 | 职责 | 为什么这样切 |
|---|---|---|
| `WindowHandle` | 不透明包装 `PlatformWindowHandle` + 尺寸 + `scale_factor` | 业务/组件永远不接触 `HWND`/`XID`/`NSWindow*`；**唯一的平台类型容器** |
| `EventSource` | `poll(timeout_ms) -> Vec<RawEvent>` + `wake() -> WakeHandle` | 把"事件来自哪"藏起来；`wake()` 让后台线程能打断阻塞（三个平台机制不同：Win32 `PostMessage`、X11 自管道 + `poll()`、macOS `dispatch_async`） |
| `Presenter` | `present(&[u8] /*BGRA*/, w, h)` + `resize` | **纯 CPU 后端的统一出口**：Win32=GDI `StretchDIBits`、X11=MIT-SHM、macOS=`CGImage`。GPU 后端（D3D11）把 `present` 实现成"上实例"，接口不变 |
| `PlatformRuntime` | 组装三者 + 跑主循环 | 业务只认它（`run(host, build, options)`） |

**这个切法的价值**：我们已有的 `SoftBackend`（0.5k 行，纯 CPU 光栅化）可以**原样复用**在三个平台上；每个新平台只要写
`EventSource + Presenter + 窗口创建` ≈ **2.5~3.5k 行**，而不是重写一个渲染器。**这是本轮最大的杠杆。**

---

## 3. 三平台自研方案与真实工程量

> 工程量按"已有 Win32 后端 6.6k 行为标尺"折算：那 6.6k 里渲染占 ~2.9k、字体 ~0.9k、窗口/事件/IME ~2.8k。
> 新平台**复用渲染（SoftBackend）与字体（自研 TTF 引擎）**，只需写窗口+事件+呈现。

### 3.1 Windows（已完成，只需"搬家"）

| 项 | 现状 | 本轮要做 |
|---|---|---|
| 窗口/事件/IME/呈现 | ✅ 全有（Win32 + D3D11 + Soft + GDI） | 拆成 `WindowHandle/EventSource/Presenter` 三个 impl；`hwnd()` 改 `pub(crate)` |
| 工作量 | — | **1.5 人日**（纯重构，测试兜底） |

### 3.2 Linux / X11（建议第一站）

**手写 X11 协议要做什么**（不引入 `x11` crate，直接连 `/tmp/.X11-unix/X0`）：

| 子任务 | 内容 | 人日 |
|---|---|---|
| 连接 + 认证 | Unix socket 连接、`Setup` 握手、`MIT-MAGIC-COOKIE-1`（解析 `~/.Xauthority` 的二进制格式） | 1.5 |
| 核心请求 | `InternAtom`、`CreateWindow`、`MapWindow`、`ChangeProperty`（`WM_NAME`/`WM_CLASS`/`_NET_WM_PID`）、`SetWMProtocols`（`WM_DELETE_WINDOW`） | 2 |
| 事件解析 | 32 字节事件结构体、`Expose`/`ConfigureNotify`/`ClientMessage`/`KeyPress`/`KeyRelease`/`ButtonPress`/`MotionNotify`/`EnterNotify` | 2.5 |
| 键盘 | `GetKeyboardMapping` → keysym 表；keysym→`Key`/`Char` 映射（含 Shift/CapsLock）；死键暂不支持 | 2 |
| 呈现 | **MIT-SHM**（`shmget/shmat/XShmCreateImage/XShmPutImage`，`libc` FFI 纯声明） | 2.5 |
| 光标 | 内置 `XC_*` 光标字体加载（或 `CreateGlyphCursor`）；`SetCursor` 映射 | 1 |
| 窗口管理互操作 | 装饰/拖动/最小化（靠 WM，握手即可）、多显示器、`_NET_WM_STATE` | 1.5 |
| 事件循环 | 自管道 + `poll()` 实现 `wake()`；`XFlush` 时机 | 1 |
| 测试 | 协议 golden test + Xvfb 集成 | 1.5 |
| **小计** | | **15.5 人日**（首版可用），代码约 **2,800~3,500 行** |

**为什么不用 `PutImage` 传像素**：1920×1080 BGRA = 8.3 MB/帧；60fps = **500 MB/s** 走 Unix socket 必炸。MIT-SHM 是硬需求。

**Wayland（不建议本轮做）**：

| 难点 | 说明 |
|---|---|
| 协议复杂度 | `xdg-shell` + `wl_seat` + `wl_shm` + `wl_output` + 数十个接口，比 X11 大一倍 |
| fd 传递 | 必须用 `sendmsg/recvmsg` + `SCM_RIGHTS` 传 memfd（`libc` FFI） |
| **键盘布局** | 需要 `xkbcommon` —— **那是 C 库**。纯 Rust 实现 xkb 解析 = 数周；替代是指定 "US 布局 only"（中文/欧洲用户直接不可用） |
| 无窗口管理兜底 | 没 `xdg-decoration` 就没有标题栏，得自绘 |
| **小计** | **25~40 人日**，且**"零 C" 会破防** |

> **建议**：Wayland 推迟。第一版 Linux 走 X11；Wayland 用户通过 **XWayland**（兼容层）运行——X11 客户端在 Wayland 桌面上开箱即用。

### 3.3 macOS / Cocoa（不建议第一站）

| 子任务 | 内容 | 人日 |
|---|---|---|
| objc 调用基础 | `extern "C" { fn objc_msgSend(...) }` + 按签名做 `transmute` 到具体函数指针（**这是 unsafe 的核心风险点**）；`sel_registerName`、`objc_getClass` | 2 |
| NSApplication 生命周期 | `sharedApplication`、`setActivationPolicy`、`NSApp.run()`、菜单栏占位 | 2 |
| NSWindow + NSView | `initWithContentRect`、`makeKeyAndOrderFront`、自定义 `NSView` 子类接管绘制 | 2.5 |
| 像素呈现 | `CGDataProviderCreateWithData` + `CGImageCreate` + `CGContextDrawImage`（或 `CALayer.contents`） | 2.5 |
| 事件 | 子类化 `NSView` 实现 `mouseDown/mouseMoved/keyDown/flagsChanged/resize` | 3 |
| Retina / DPI | `backingScaleFactor`、`convertRectToBacking`、2x 渲染 | 1.5 |
| 事件循环唤醒 | `dispatch_async(dispatch_get_main_queue(), ...)` 打断 `nextEventMatchingMask` | 1 |
| 测试 | **无法在 Windows 上跑**，只能在 Mac 上 | 1.5 |
| **小计** | | **15.5 人日**，代码约 **3,000~4,000 行** |

**纯 CPU 像素怎么上屏**（回答报告里的问题）：

```
我们的 Vec<u32> (BGRA)
  → CGDataProviderCreateWithData(ptr, len, release_cb)     // 零拷贝，release_cb 里交给 Rust
  → CGImageCreate(w, h, 8, 32, w*4, colorSpace, kCGImageAlphaPremultipliedFirst, provider, ...)
  → CGContextDrawImage(view_context, rect, image)           // 或 layer.contents = image
```
**关键坑**：`CGImage` 的颜色空间要显式建（`CGColorSpaceCreateDeviceRGB`），alpha 布局要和我们的缓冲区一致（**BGRA + 预乘**）；Retina 下 `rect` 必须是 **backing 坐标**（2x），否则画面糊一半。这些都能在 Mac 上 1~2 天调通，但**没有 Mac 就完全做不了**。

### 3.4 三平台汇总

| 平台 | 代码量 | 人日（首版可用） | 加固（IME/剪贴板/多屏） | 能在 Windows 上验证吗 |
|---|---|---|---|---|
| Windows | 6.6k（已有） | 1.5（重构） | — | ✅ 完整 |
| **Linux/X11** | 2.8~3.5k | **15.5** | +5~8 | ✅ 协议 golden test + `cargo check --target` + WSL/Xvfb |
| Linux/Wayland | 4~6k | 25~40（且零 C 破防） | +10 | ⚠️ 仅协议级 |
| macOS/Cocoa | 3~4k | 15.5 | +6~10 | ❌ **必须 Mac** |

---

## 4. 输入法（IME）：建议 P2，第一版只做 ASCII

| 平台 | 机制 | 自研难度 | 人日 |
|---|---|---|---|
| Windows | `Imm32`（`WM_IME_*` + 候选窗定位） | ✅ 已实现 | 0 |
| Linux/X11 | **XIM**（老协议，ibus/fcitx 都提供兼容） | 中高：需实现 `XIM` 客户端协议 + 焦点/预编辑回调 | 4~6 |
| Linux | **IBus / Fcitx5**（D-Bus） | 高：手写 D-Bus 编解码 + 服务发现 | 15~25 |
| macOS | `NSTextInputClient`（`setMarkedText`/`firstRectForCharacterRange`） | 高：协议方法多、坐标换算易错 | 5~8 |

**建议**：
- 第一版三平台都**只支持 ASCII 键盘输入**（`keysym`/`NSEvent.characters` → `Char`），IME 挂起；
- 终端场景 80% 输入是命令/路径（ASCII），可接受；
- 若要中文输入，Linux 上**优先 XIM**（4~6 人日，比 D-Bus 便宜一个数量级），macOS 用 `NSTextInputClient`。

**但要说清代价**：中文业务应用是中文产品，第一版 Linux/macOS **不能输入中文**，只能显示中文。这是明确的功能降级，需要老板知情。

---

## 5. 验证策略：在只有 Windows 的机器上怎么证明它是对的

### 5.1 X11 的独特优势：**协议是字节流，可以离线 golden test**

```rust
// 不需要任何 X server：把请求编码成字节，和规范算出来的期望字节逐字节比对
assert_eq!(encode_intern_atom(1, "WM_NAME", true), EXPECTED_BYTES);
assert_eq!(encode_create_window(...), EXPECTED_BYTES);
assert_eq!(encode_shm_put_image(...), EXPECTED_BYTES);
```
这能覆盖 **60~70% 的代码路径**（协议编码是纯函数），是本方案里最值钱的验证手段。

### 5.2 类型检查跨平台

```powershell
# 实测：本机只安了 msvc target（`rustup target list --installed`），需先加 Linux/macOS target
rustup target add x86_64-unknown-linux-gnu     # std 组件下载，网络可用
cargo check -p y901-backend-x11 --target x86_64-unknown-linux-gnu    # check 不链接 → 不需要 Linux SDK
# macOS 目标也可 check；但 objc 运行时与 CG 行为只能在 Mac/模拟器上验证
```
`cargo check` 不链接 → **不需要 Linux/macOS SDK**，能在 Windows 上抓出 90% 的编译错误。

### 5.3 真窗验证

| 平台 | 手段 | 可行性 |
|---|---|---|
| Linux | **GitHub Actions `ubuntu-latest` + `xvfb-run`**（推荐）；或本机 WSL2 装一个发行版 + `Xvfb :99` + `xdotool` | ⚠️ 本机 WSL 仅有壳、无发行版（实测）；CI 可行 |
| macOS | GitHub Actions `macos-latest`（免费额度）+ 截图比对 | ✅ 可行但慢（每次 10~20 分钟） |
| macOS 本地 | 必须一台 Mac | ❌ 没有就做不了 |

**结论**：X11 可以做到"在 Windows 上完成 90% 的验证"；Cocoa 只能靠 CI 上的 macOS runner，且调试周期长（一轮 20 分钟）。

---

## 6. 收益 vs 风险

### 收益

| 收益 | 说明 |
|---|---|
| **真·全平台一套业务代码** | 业务零 `cfg`，只需换 Cargo 依赖 |
| **零第三方 GUI 依赖** | 供应链极小；不被 winit/glfw 的 breaking change 绑架 |
| **零 C 编译**（X11/macOS 路线） | 保住上一轮刚立的"零 C"成果（**Wayland 例外**） |
| **复用已有资产** | SoftBackend + 自研 TTF 引擎跨平台直接用，新平台只需窗口/事件/呈现 |
| 极致可控 | 每个像素、每个事件都在自己手里，问题可查 |

### 风险（必须正视）

| 风险 | 等级 | 缓解 |
|---|---|---|
| **手写协议/objc 的 UB 风险** | 高 | 全部 `// SAFETY:` 标注；协议层用 golden test；objc 调用封装成极小的安全 wrapper（< 200 行） |
| **维护成本**：三个后端 ≈ 10k 行平台代码 | 高 | 原语统一 → 平台相关代码只增在"窗口/事件/呈现"三处 |
| **无 GUI 生态兜底**（无障碍、剪贴板、拖放、多屏） | 中 | 逐项排期；第一版明确"不支持"清单 |
| **Wayland 的零 C 破防** | 高 | 推迟；用 XWayland |
| **macOS 无法本地验证** | 高 | 必须有 Mac 或 CI runner；否则只能"写完不敢发布" |

---

## 7. 建议的第一阶段与验收

### 7.1 建议：**先切 Linux/X11**，Windows 只做重构

**为什么不是 macOS**：没有 Mac 就不会有任何"真窗验证"，写完只能靠 `cargo check` 保证能编译——那是赌博。
**为什么不是 Wayland**：会破防"零 C"（xkbcommon），而且工程量翻倍。

### 7.2 分三步走（每步可独立验收）

| 步 | 内容 | 验收 |
|---|---|---|
| **S1** | `runtime` 加四个原语；Win32 后端重构实现之；`app` 改为依赖 `y901-shell` | 现有 **531 测试全绿**；`app` 里 0 处 `cfg`；`grep HWND` 只在 backend-win32 内部 |
| **S2** | `y901-backend-x11`：连接/认证/窗口/事件/键盘/PutImage（先不管性能） | golden test 全过；`cargo check --target x86_64-unknown-linux-gnu` 通过；Xvfb 里能画出窗口并响应键盘 |
| **S3** | MIT-SHM 加速 + 光标 + 多屏 DPI + 与 SoftBackend 打通 | 1080p 下 60fps（帧耗时 < 16ms）；插件化 demo 在 X11 上跑通 |
| S4（可选） | macOS/Cocoa | 需 Mac 或 CI runner |
| P2 | IME（XIM → NSTextInputClient） | 中文可输入 |

### 7.3 需要老板拍板的 5 件事

| # | 问题 | 选项 | 我的建议 |
|---|---|---|---|
| 1 | 第一站平台 | X11 / macOS / Wayland | **X11** |
| 2 | 是否接受"第一版 IME 只支持 ASCII" | 接受 / 必须先做中文 | 接受（XIM 作为紧随其后的 P2） |
| 3 | Wayland 是否本轮做 | 现在做 / 推迟（XWayland 兜底） | **推迟**（零 C 与工期双重原因） |
| 4 | macOS 何时做，机器从哪来 | 现在（需 Mac/CI）/ 等有 Mac 再做 | 等 Mac 或开 macOS CI job |
| 5 | 是否允许"仅 US 键盘布局"（X11 用 `GetKeyboardMapping` 可拿到当前布局映射，理论上够用；但不做死键/组合键） | 允许 / 必须完整布局 | 允许（`GetKeyboardMapping` 已能覆盖绝大多数输入） |

---

## 8. 工作量总表（含我建议的排期）

| 阶段 | 内容 | 人日 | 累计 |
|---|---|---|---|
| S1 | 原语抽象 + Win32 重构 + shell crate | 2 | 2 |
| S2 | X11：连接/窗口/事件/键盘/PutImage | 9 | 11 |
| S3 | X11：MIT-SHM + 光标 + DPI + 联调 | 4.5 | 15.5 |
| P2a | X11：XIM 中文输入 | 5 | 20.5 |
| S4 | macOS：objc + 窗口 + CGImage + 事件 + Retina | 15.5 | 36 |
| P2b | macOS：NSTextInputClient | 6 | 42 |
| （推迟） | Wayland | 25~40 | 67+ |

**最小可用跨平台（Win + Linux/X11，无中文输入）≈ 15.5 人日（约 3 周）。**

---

## 8.5 已知限制：平台后端的行为差异

### 8.5.1 SoftBackend 合成粗体（已实现，有局限）

**现象**：纯 CPU 后端（Linux/X11 与 Windows `--force-soft`）不能像 GPU 那样把粗体交给
DirectWrite，因此需要**合成粗体**。

**实现**：`SoftBackend::draw_text` 在 `style.bold` 时取"当前像素与右侧邻居覆盖度的最大值"，
等价于把字形向右拓宽 1px。

**为什么不是"画两遍"：**两次 `blend` 会把半透明边缘叠加成更黑，
边缘发模糊；取 `max` 只拓宽、不加深，仍保持单次抗锯。

**已知不足**：只向右拓宽，不是真正的轮廓加粗。在 15px 下看上去明显比常规字重，
但整体略偏右（0.5px 不到的视觉偏移）。若要更好，需要轮廓加粗（字形演示层处理）。

**正文一律未验证**：GPU（D3D11）路径把 `ts.bold` 交给 DirectWrite，理论上应该是真粗体，
但**本轮没有做像素对比验证**。

### 8.5.2 光标形状（Xvfb 下回退）

Xvfb 对 `CreateGlyphCursor` 返回 `BadFont`，因此在 Xvfb 下光标恒为默认箭头。
真 Xorg 上待验（字节布局已由 golden test 锁死）。

## 9. 一句话给老板

> **能自研，但请把"三个平台"当成三个项目排期，而不是一个任务。**
> Windows 已经是现成的；Linux/X11 是**唯一能在你现有机器上验证到位**的下一站（15.5 人日，含协议级测试）；
> macOS 没有 Mac 就只能"写而不敢发"；Wayland 会让"零 C"这条刚立的规矩破防。
> 建议：**S1 先做（2 人日，风险最低、立刻让业务零分叉），S2/S3 紧随其后（13.5 人日），macOS 等有机器再立项。**
