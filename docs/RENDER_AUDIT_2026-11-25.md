# Y901 Render Architecture Audit（2026-11-25）

**性质**：只读审查。**未修改任何渲染代码**。所有结论都来自本轮实际读取的源码
（行号对应 `main = cf0a7e1`）；文档与源码不一致处会明确标注。

基线：workspace **1215 passed / 0 failed / 68 ignored**，像素金标
`0xF7455CCE7781A864`（400×220，`font_3c_pixels`，仅在真 GPU 上跑），
`gate.ps1` GATE OK，版本 0.4.0（已发布 12 crates）。

---

## 0. 结论先行（如果只读一段）

1. **Y901 现在没有"帧数据包"这一层。** 绘制是**逐命令直调后端 trait**：
   `UiTree::render(&mut dyn RenderCtx)` → 每个 `Widget::render` 直接调
   `fill_rect`/`draw_text`/`push_clip`…，后端实现里**立刻**做变换/裁剪并把
   `Instance` push 进自己的 `Vec`。不存在 `PaintCommand` / `FramePacket` / `RenderPlan`。
2. **已经存在一套 `Frame`/`RenderBackend` 抽象，但它是死代码。**
   `y901_runtime::gfx::{Frame, GfxInstance, RenderBackend}`
   （[gfx.rs:78-116](../crates/y901-runtime/src/gfx.rs)）只被 `RecordingBackend` 与单测使用；
   真实窗口路径走的是 `D3dBackend` 的**同名但不同签名**的固有方法
   `present(hwnd, w, h, sync)`
   （[mod.rs:303](../crates/y901-backend-win32/src/backend/mod.rs)）。
   这条差异是本阶段最重要的架构事实：**目标架构里要建的东西，仓库里已有一个"空壳"，
   必须先决定是复活它还是删掉它**，否则会做出第三套并行抽象。
3. **三个平台的渲染粒度完全不同**，而且这是当前的**最大性能差距**：

   | 平台 | 脏区策略 | 每帧固定成本（代码可见） |
   |---|---|---|
   | Win32 / D3D11 | **真局部**：`plan_dirty_rects`（≤16 块）→ 逐块 scissor + 局部清屏 | `K` 块脏区 × 全实例重画 + 一次 RT→后缓冲 blit + Present |
   | X11 | **整窗**：`tree.invalidate_all()` + `soft.clear()` + 整树重绘 | 整帧软光栅 + **整帧像素拷贝**（`extend_from_slice`）+ 一次 PutImage/XShm |
   | Web | **整窗**：同上 | 整帧软光栅 + `bgra_to_rgba` **每帧新分配** + `createImageData` 每帧新建 + `putImageData` |

4. **当前测量口径把 Present 混进了"渲染成本"。** 已实测：空帧 6 885 µs、
   10 000 实例满帧 6 887 µs（[PERFORMANCE.md](PERFORMANCE.md) 第 5 节），
   即窗口模式下帧时间由 DWM/vsync 决定，CPU 提交 10k 实例只要 32 µs。
   **不先做 RENDER-04（分层计时）就去做"渲染优化"，测到的几乎全是调度噪声**。
5. **像素正确性设施目前只覆盖一个场景、且只在真 GPU 上跑。** `hash-gate.ps1` 跑
   `font_3c_pixels`（单场景 400×220）；没有一个"Full vs Partial 逐像素相等"的
   跨后端矩阵。RENDER-05 是后续所有 partial rendering 工作的前提。

---

## 1. 真实渲染数据流（当前）

```
run_loop（runtime）
  └─ WindowBackend::render(tree)            ← Win32 window.rs:1458 / X11 window.rs:778 / web lib.rs:259
       ├─[Win32 soft] tree.invalidate_all(); soft.clear(); tree.render(&mut soft); gdi_present(...)
       ├─[Win32 D3D ] dirty = tree.dirty(); rects = plan_dirty_rects(dirty);
       │              backend.set_dirty_bounds(rects); backend.begin();
       │              tree.render(backend as &mut dyn RenderCtx);
       │              backend.present(hwnd, w, h, sync)        ← 固有方法，不是 trait
       ├─[X11       ] tree.invalidate_all(); soft.clear(); tree.render(&mut soft);
       │              frame.extend_from_slice(soft.pixels()); present(&frame, w, h)
       └─[Web       ] tree.invalidate_all(); soft.clear(); tree.render(&mut soft);
                      present.present(soft.pixels(), vw, vh)   ← BGRA→RGBA 新 Vec + ImageData
```

`UiTree::render` 内部（[tree.rs:1987](../crates/y901-ui/src/tree.rs)）：

1. 释放上一帧收集的纹理（`pending_texture_releases`）；
2. 空脏区直接返回；
3. 推悬停快照 `ctx.set_hovered(...)`、动画快照 `AnimSnapshot::new(collect())`（**每帧一次分配**）；
4. `let dirty = self.dirty.clone();`（**每帧一次 Vec 拷贝**）；
5. `paint_all = dirty.is_full()`；
   - `full`：`render_full(ctx)` —— 按 `paint_order` 稠密顺序遍历（前序 = 绘制顺序），
     每节点还原路径、push 祖先累积裁切、`node.render(ctx)`；
   - 局部：`for r in local_rects { ctx.push_clip(r); render_candidates(ctx, r); ctx.pop_clip() }`
     —— **每块脏区都完整扫一遍 `paint_order`**（[tree.rs:2202](../crates/y901-ui/src/tree.rs) 的
     `for idx in 0..count`），筛出与 `rect∪ink` 相交的节点后逐个 `render`；
6. popups 在树之后单独绘制（`push_clip(viewport)` + 模态遮罩 + 逐浮层 `render_pass`）；
7. `self.dirty.clear()`。

**关键点**：`RenderCtx` 的方法调用**同时**是"命令"和"执行"。
没有任何中间表示可以被检查、重排、批处理或计数（除了后端自己 push 的 `Instance`）。

---

## 2. 关键 crate 与模块

| crate | 与渲染的关系 | 关键文件 |
|---|---|---|
| `y901-ui` | 定义 `RenderCtx`（**6 必需 + 21 默认方法**）、`Widget`、`paint_order`、`DirtyRegion`、`UiTree::render` | `widget.rs:539`（trait）、`tree.rs:1987`（render）、`tree.rs:55`（DirtyRegion） |
| `y901-render` | 纯 CPU 光栅器 `SoftBackend`（实现 `RenderCtx`）、字体引擎/图集/mip、shaping | `soft.rs:788`（`impl RenderCtx`）、`atlas.rs`、`font_engine/` |
| `y901-runtime` | `WindowBackend`（事件/渲染循环契约）、`RenderOutcome`、**未使用的** `gfx::{RenderBackend, Frame}` | `window.rs`、`gfx.rs` |
| `y901-backend-win32` | D3D11 执行层（`D3dBackend` 同时是 `RenderCtx` + `RenderBackend`）+ 软渲染回退 + `Window::render_inner` | `backend/mod.rs`、`backend/d3d11.rs`、`window.rs` |
| `y901-backend-x11` | 软光栅 + XShm/XPutImage 上屏，**无脏区** | `window.rs:778`、`shm.rs`、`conn.rs` |
| `y901-backend-web` | 软光栅 + Canvas2D `putImageData`，**无脏区** | `lib.rs:259`、`present.rs` |

依赖方向（`y901-ui` 不依赖任何 backend / render；`y901-render` 依赖 `ui`+`core`；
backend 依赖 `runtime`+`render`+`ui`）是干净的，新架构必须保持这一点。

---

## 3. 现有"绘制命令"与数据结构

### 3.1 `RenderCtx`（唯一的绘制 API）

必需（6）：`size` `fill_rect` `stroke_rect` `draw_text` `push_clip` `pop_clip`
默认（21）：`animation` `set_animations` `push_transform` `pop_transform` `upload_texture`
`draw_image` `retain_texture` `release_texture` `is_hovered` `set_hovered`
`background_color` `draw_char` `fill_round_rect` `fill_gradient_round_rect`
`stroke_round_rect` `fill_round_row` `fill_gradient_rect_dir` `fill_gradient_rect`
`push_opacity` `pop_opacity` `theme`

**注意**：圆角/渐变/渐变圆角/圆角描边在 `y901-ui` 里是**默认实现**，用 `fill_rect`
拆若干块拼出来的（[widget.rs:648-887](../crates/y901-ui/src/widget.rs)）。也就是说
**同一个"圆角矩形"在不同后端会变成不同数量的底层命令**：
soft 有自己的 SDF 实现覆盖了这些方法，D3D 也覆盖了一部分（走 GPU SDF），
但**上层语义没有统一命令**，无法对"一个圆角矩形"计数/合并。

### 3.2 `Instance`（后端内部的"命令"）

D3D 侧：[mod.rs:176](../crates/y901-backend-win32/src/backend/mod.rs) `push_instance` 把
`(pos, size, uv_rect, color, color_end, gradient, params)` 写进 `Vec<Instance>`，
`params[3]` 存当前不透明度。布局与 `y901_runtime::gfx::GfxInstance` **一致**
（`const _: () = assert!(size_of::<GfxInstance>() == 96);`，[gfx.rs:61](../crates/y901-runtime/src/gfx.rs)），
但两份类型并存、只靠一个测试与一个编译期断言保持同步。

### 3.3 脏区 `DirtyRegion`

[tree.rs:55-184](../crates/y901-ui/src/tree.rs)：`full` 标志 + `Vec<Rect>`，上限 16，
`add` 做"包含/被包含/相交合并"三向合并，超限退化成整窗。

### 3.4 批次 `Batch`（D3D 私有）

[mod.rs:865](../crates/y901-backend-win32/src/backend/mod.rs) `build_batches()` 把
`image_marks: Vec<(start, count, texture_id)>` 转成 `Vec<Batch>`，让图片与图集交替 draw。
**这是目前唯一的"计划"**，且只存在于 D3D 后端内部、每帧重新分配。

---

## 4. 三个平台的真实执行路径

### 4.1 Win32 / D3D11（Tier 1）

`Window::render_inner`（[window.rs:1484](../crates/y901-backend-win32/src/window.rs)）：

1. `tree.dirty().is_empty()` → 直接返回（**不 Present**，保留上一帧）；
2. `plan_dirty_rects(dirty)`（`MAX_DIRTY_RECTS = 16`）→ `Option<Vec<(i32,i32,i32,i32)>>`；
   碎片超限 → `tree.invalidate_all()` 升级整窗；
3. `backend.set_dirty_bounds(rects)`；`backend.begin()`（清实例、重置 clip 栈、推客户区基座裁剪）；
4. `tree.render(backend)` —— 逐节点直调，**CPU 侧完成变换 + 裁剪 + 实例 push**；
5. `backend.present(hwnd, w, h, sync)`：
   - `gfx.is_empty()` → `rebuild`；
   - `sync_atlas()`：`text.take_dirty()` → 增量上传图集（**这里是真增量**）；
   - `build_batches()`（**每帧分配 `Vec<Batch>`**）；
   - `render_batched(&instances, &batches, sync)`：
     - `map_discard` 实例缓冲 + 一次 `memcpy(count * INSTANCE_SIZE)` + `unmap`
     - `for rect in rects { draw_scene(..., count, batches, rect, ...) }`
       —— **每块脏区都重新设 RT/视口/管线/纹理/scissor，并把全部实例画一遍**；
       多块脏区 = `K × N` 顶点工作量（像素被 scissor 裁掉，但顶点/批次循环没省）
     - `blit_cached(...)`：自持 RT → 后缓冲
     - `Present(sync_interval)`
   - 设备丢失 → `rebuild` + 全量重传图集 + 重画一次。

**已确认的固定成本**（每帧，与命令数无关）：`map_discard`+`memcpy`、`build_batches` 分配、
`K` 次状态设置、一次全屏 blit、`Present`。**Present 占绝对主导**
（空帧 6.885 ms ≈ 满帧 6.887 ms）。

#### 4.1.1 D3D11 深读补充（独立审计返回，已逐条回源码核对）

1. **图集上传每帧整张 4 MiB 拷贝 + 每脏区一次 patch 分配。**
   `update_atlas` 第一行就是 `self.atlas_data = data.to_vec()`
   （[d3d11.rs:2223](../crates/y901-backend-win32/src/backend/d3d11.rs)），
   图集固定 1024×1024×4 = **4 MiB**；随后每个脏区 `vec![0u8; row_pitch*height]`
   再逐行拷（:2245）。D3D 侧本身已是 `UpdateSubresource` 区域更新（`D3D11_BOX`），
   **浪费全在 CPU 侧的"整张克隆"**。只要这一帧有任何一个字形被光栅化就付一次。
2. **每帧 `env::var("Y901_GAMMA")`，`Y901_DEBUG_BATCHES` 甚至每个脏区读一次。**
   [d3d11.rs:2496](../crates/y901-backend-win32/src/backend/d3d11.rs)（每帧）、
   `:1957`（`draw_scene` 内，**每块脏区一次**）。这与本轮早些时候在软光栅里修掉的
   `Y901_PAINT_TRACE` 是同一类缺陷（诊断开关留在热路径），而且 `env::var` 要拿全局锁 + 分配。
3. **实例上限 10 000，超出部分被静默截断。**
   `let count = instances.len().min(INSTANCE_CAPACITY)`（:2504），
   只在一个**进程级一次**的 `eprintln!` 里提过（:2508-2518）。
   每个字形、每个下划线、每个背景矩形都是一条实例，**100×100 网格带文字就接近上限**；
   一旦越界，画面"后半帧整体消失"，而日志只出现一次。这是**可达的正确性缺陷**，
   不是纯性能问题（建议在 RENDER-04 里把它变成一个可观测计数器 + 超限策略）。
4. **`RenderBackend::present` 硬编码 `sync_interval = 1`**（[mod.rs:805](../crates/y901-backend-win32/src/backend/mod.rs)），
   而它上方 6 行的注释明确写着"sync_interval = 0：不阻塞在 vsync 上…用 1 时窗口被遮挡会让
   Present 永久阻塞"（:334-344）。**注释与代码互相矛盾**，且这是 `Present` 计入帧时间的原因之一。
5. **`y901-backend-win32` 里没有任何 `Instant`** —— CPU 准备 / GPU 提交 / Present 完全没分开。
   唯一的仪器在 `y901-runtime`，而它的 `present_ms` 覆盖"事件分发 → layout → render →
   after_present"整段（[profile.rs:32](../crates/y901-runtime/src/profile.rs) 把它定义成
   "本帧实际耗时"）。**"D3D present 要多少 ms"目前没有任何代码依据。**
6. `draw_scene` 每块脏区都重新绑定全套状态（RT/视口/VS+PS+Cb/VB/拓扑/混合/光栅/图集 SRV+sampler/scissor），
   且每个图片批次前后各切一次 PS（:1990、:1994）。这些在"每块脏区重画全量实例"（B2）之上再乘 K。
7. 图集淘汰会触发 `compact_atlas` → `reset_pending = true` → **下一帧整张重传**；
   而 `compact_atlas` 是在 `draw_text` 内部被触发的（[font.rs:862-873](../crates/y901-backend-win32/src/backend/font.rs)），
   即**渲染中途重排图集 UV**，此前已 push 的实例仍持旧 UV（是否有可见伪影**未验证**，标记 uncertain）。
8. 设备重建路径（`Gfx::rebuild`）会丢 `pipeline/atlas/cached/rtv`，但 `self.images` 里的
   纹理不清理（**未验证**旧设备纹理在下次 `PSSetShaderResources` 的行为）。
### 4.2 X11（Tier 2）

`Window::render_tree`（[window.rs:778](../crates/y901-backend-x11/src/window.rs)）：

```rust
if tree.dirty().is_empty() { … return Ok(()) }
tree.invalidate_all();          // ← 无条件整树重绘
self.soft.clear();
tree.render(&mut self.soft);    // 整帧软光栅
self.frame.clear();
self.frame.extend_from_slice(self.soft.pixels());   // ← 整帧像素拷贝
let frame = std::mem::take(&mut self.frame);
self.present(&frame, w, h);     // XShm / PutImage
```

**没有任何局部渲染**。每帧成本 = 整帧软光栅（1280×800 时 fill/gradient/round 已优化到
0.09–0.10 ms 级，但整棵树遍历 + 逐字形贴图仍在）+ `w*h*4` 拷贝（1080p ≈ 8.3 MB）+ IPC。
`real_window.rs` 里已有 `benchmark_shm_vs_putimage_1080p`（真窗口基准，`#[ignore]`）。

#### 4.2.1 X11 深读补充（独立审计返回，已逐条回源码核对）

1. **X server 只当帧缓冲用，没有任何绘图请求。** 整个 crate 里没有 `POLY_*`/`PolyFillRectangle`/
   `PolyText`/XRender/Xft 调用，opcode 表只有 `CREATE_WINDOW`/`CREATE_GC`/`CHANGE_PROPERTY`/
   `PUT_IMAGE`/`SHM_*`（[lib.rs:74-127](../crates/y901-backend-x11/src/lib.rs)）。
   所以"X11 渲染性能"= "软光栅性能 + 一次上传"，不存在服务端绘制优化空间。
2. **`ConfigureNotify` 不更新缓存的窗口尺寸** —— 见下方 B9，这是本轮新发现的**最高危项**。
3. **SHM 快路径把整帧参数写死。** `ShmPutImageArgs` 本身支持 `src_x/src_y/src_width/src_height`
   + `dst_x/dst_y`（[encode.rs:655-666](../crates/y901-backend-x11/src/encode.rs)），
   但调用点硬编码 `src=0,0,w,h`、`dst=0,0`（[window.rs:1118-1123](../crates/y901-backend-x11/src/window.rs)）。
   **partial present 在协议层完全可行，只被调用方常量挡住。**
4. **SHM 不可用时退化为分块 `PutImage`**：`PUT_IMAGE_MAX_BYTES = 262 116`，
   1080p 一帧 = **32 个请求**、每块重新 `Vec::with_capacity` 分配 ≈262 KiB 并把像素再拷一遍
   （[conn.rs:800-840](../crates/y901-backend-x11/src/conn.rs)）。`stream` 是裸 `UnixStream`
   （无 `BufWriter`），每个 `send` 一次 `write_all`。
5. **不校验视觉格式**：握手只解析 root/visual/depth，不查 `image-byte-order` /
   `bits-per-pixel` / `red-green-blue masks`；正确性依赖"小端 + 24-bit TrueColor root"。
   非 TrueColor / 奇数掩码的服务器会**静默颜色错乱**（无诊断）。
6. **Expose 矩形被丢弃**：`Expose` 携带真实 `(x,y,w,h,count)`
   （[decode.rs:151-157](../crates/y901-backend-x11/src/decode.rs)），但映射成
   `RawEvent::Resize{ 当前窗口尺寸 }`（[window.rs:842-847](../crates/y901-backend-x11/src/window.rs)）。
   于是**两个 damage 源（UI 脏区、服务端 Expose）都被压成"整帧"**。
7. **没有把光栅与 IPC 分开计时**：唯一仪器 `Y901_PROFILE` 的 `present_ms` 覆盖
   "事件分发 → layout → render → after_present"整段（[runtime.rs:227-257](../crates/y901-runtime/src/runtime.rs)），
   无法区分"CPU 光栅慢"与"两次 8 MB 拷贝 + 32 次系统调用"。
8. **dirty 被查了两次**：`window.rs:779`（`render_tree` 内）与 `window.rs:1192`
   （`WindowBackend::render`），而 trait 文档（[window.rs:53-59](../crates/y901-runtime/src/window.rs)）
   明确要求"唯一判定点"。当前行为无害，但违反了自述契约。
### 4.3 Web / WASM（Tier 3）

`WasmWindow::render`（[lib.rs:259](../crates/y901-backend-web/src/lib.rs)）：
`invalidate_all` → `soft.clear()` → `tree.render(&mut soft)` →
`present.present(pixels, w, h)`，而 `present`（[present.rs:37](../crates/y901-backend-web/src/present.rs)，四步在 41–46 行）：

```rust
let rgba = bgra_to_rgba(pixels, w, h);        // ← 每帧新 Vec（w*h*4）
let image_data = ctx.create_image_data(w, h); // ← 每帧新建 ImageData
image_data.data().set_from(&rgba, 0);         // ← 一次跨边界拷贝
ctx.put_image_data(&image_data, 0.0, 0.0);
```

代码注释明确写了这是**刻意的取舍**（"宁可每帧多花几毫秒，也不要那些偶现的局部绘制 bug"），
并且 release 下 800×600 全帧约 15 ms。**这是 RENDER-10 的起点，不是 bug。**

---

#### 4.3.1 Web/WASM 深读补充（独立审计返回，已逐条回源码核对）

**最重要的更正**：Web 后端**没有任何 Canvas 2D 绘制调用**。`bindings.rs` 只绑定了
`putImageData` / `createImageData`（[bindings.rs:132-162](../crates/y901-backend-web/src/bindings.rs)），
全仓 grep `fillRect|strokeRect|fillText|setTransform|globalAlpha|roundRect` **零命中**。
所以 Web 的"渲染器"就是**同一个 CPU 软光栅 + 一次整帧像素上传**，不存在"命令映射"这一层。
之前若有文档暗示 web 走 canvas 图元，那是错的。

已核对的具体成本与缺陷：

1. **`bgra_to_rgba` 逐像素 4 次 `push`**（[present.rs:53-62](../crates/y901-backend-web/src/present.rs)）：
   800×600 = **192 万次 push**，外加一个新 `Vec<u8>`。改成 `chunks_exact(4)` 直接写
   `out[i..i+4]`（或复用缓冲）是一行级优化。
2. **每帧新建 `ImageData`**（`create_image_data`，[present.rs:42](../crates/y901-backend-web/src/present.rs)），
   没有复用/双缓冲 → 每帧约 1.9 MB JS 堆垃圾（800×600 @60fps ≈ 115 MB/s GC 压力）。
3. **每帧新建并丢弃一个 rAF `Closure`**（`lib.rs:974-1079`："先 drop 旧闭包，再 wrap 新闭包"），
   外加每帧一次 `global_window()` 构造 `JsValue`、一次 `drain(..).collect()`。
4. **atlas 记账在软光栅路径上从不推进**（本轮确认）：`text.begin_frame()` / `text.take_dirty()`
   只有 D3D 后端调用（[backend/mod.rs:236,246](../crates/y901-backend-win32/src/backend/mod.rs)），
   `SoftBackend` **从不调用**。后果：`MyFontEngine.frame` 停在 0 →
   `evict_one` 的"热集保护"退化（只能走"全表最冷"分支，
   [engine.rs:738-755](../crates/y901-render/src/font_engine/engine.rs)），
   且 `dirty: Vec<DirtyRect>` 只增不减（每个被光栅化的字形一条，**不随帧增长**，
   但永不释放）。**这条同时影响 Win32 `--force-soft`、X11、Web 三条软渲染路径**，
   是目前唯一"共享渲染代码在不同后端行为分叉"的实例。
5. **DPR 恒为 1.0**：全仓无 `devicePixelRatio` 读取，`canvas.width = client_width`（CSS 像素）；
   `SoftBackend::set_dpi_scale` 在 web 上从不调用。这是 `docs/WASM.md:103` **明确声明的取舍**，
   应记为"已知限制"而非缺陷（HiDPI 下整页被浏览器放大，偏糊）。
6. **`present` 没有任何长度校验**：`ImageData` 用 `canvas` 属性尺寸、
   像素来自 `soft.pixels()`，两者只靠"`apply_pending` 是唯一同时改它们的入口"这一隐式不变式。
   一旦不同步（例如 zoom 未触发 `resize`），`TypedArray.set` 会**留半帧**或抛 `RangeError`
   （wasm trap）。**未复现，标 uncertain**，但缺少防御是事实。
7. **Canvas 上下文丢失无恢复**：`Present::new` 只取一次 `getContext("2d")`，
   `None` 就永远静默跳过上屏（[present.rs:20-26](../crates/y901-backend-web/src/present.rs)），
   没有 `console.error`、没有重试。GPU reset / 标签页丢弃恢复后，应用会继续 60Hz 空转、画面不动。
8. **字体加载前是"空字形引擎"**：`font_engine.rs:48-60` 的 fallback 画不出任何字形，
   `y901_font_loaded` 才换真引擎 + 重排 + 整帧重画（[wasm_entry.rs:135-176](../crates/y901-shell/src/wasm_entry.rs)）。
   首屏到字体 fetch 完成之间**文字不可见**（当前 demo 字体 19.7 MB，首屏窗口很长）。
   这是已知设计，但对"首屏体验"是硬伤。
9. **rAF 无条件自续**：即使没有任何脏区也每 vsync 唤醒一次（`render` 才返回 `Skipped`），
   空闲功耗固定 60 Hz；且 wasm 侧没有任何计时仪器（`y901_runtime::profile` 只服务 `run_loop`），
   `lib.rs:269` 里"release 下 800×600 全帧约 15ms"这句话**在仓库内找不到测量依据**（标 uncertain）。
## 5. 已经完成的性能优化（有实测、可复跑）

| 项 | 前 | 后 | 证据 |
|---|---|---|---|
| 脏区合并（10k 次流式 add） | 135.6 ms | **0.59 ms** | `perf_bench::bench_dirty_region_streaming` |
| 布局 10k / 50k | 1 846 / 12 675 µs | **972 / 6 727 µs** | `layout_bench` |
| 热布局（无变化帧） | 573–735 µs | **0.1 µs** | 复用 `nodes` 索引表做快照 |
| 单次重排分配 | 646 140 B | 88 944 B | `perf_alloc` |
| 冷布局（同机 A/B） | 941 µs | **779 µs** | 世代脏标记 + 墨迹合并 + 去掉每容器 `sizes: Vec` |
| 整帧渲染遍历（11k 节点，空 ctx） | 1 268 µs | **141 µs** | `profile_render_cost_model`（`env::var` 在热路径 → `OnceLock`） |
| 局部重绘（1×1 脏区） | 285–300 µs | **44 µs** | `render_candidates` 稠密索引 |
| 软光栅整屏渐变 | 4.5 ms | **0.09 ms** | 轴对齐快路（`gradient_golden` 金标） |
| 软光栅圆角矩形 | 0.68 ms | **0.09 ms** | 内部区免开方（`round_rect_golden` 金标） |
| 软光栅整屏文字 | 2.9 ms | **1.3 ms** | 覆盖率查表 + 1:1 直贴 |
| Grid 整帧（100×100） | 59–61 ms | **0.21 ms** | `children_clip` 的 O(n²) → 游标查找 |
| D3D 10k 实例 CPU 构建 | — | **32 µs**（p99 47） | `perf_bench::bench_instance_submission_10k` |
| 帧级固定开销 | — | 空帧 **6 885 µs** / 满帧 6 887 µs | `present_bench`，**DWM/vsync 主导** |

**结论**：CPU 侧的"布局 + 命令生成"已经不是瓶颈（10k 实例 32 µs）；
**剩下的都是每帧固定成本、整帧重绘、以及平台上屏路径**。

---

## 6. 当前可确认的性能瓶颈（按证据强度）

| # | 瓶颈 | 证据 | 影响面 |
|---|---|---|---|
| B1 | X11/Web **每帧整树重绘 + 整帧拷贝** | 源码 `invalidate_all()` 无条件调用 + `extend_from_slice` / `bgra_to_rgba` | 大窗口时线性增长；X11 1080p 每帧 8.3 MB 拷贝 |
| B2 | D3D **K 块脏区 × 全实例重画** | `for rect in rects { draw_scene(..., count, batches, ...) }` | 多脏区（如打字 + 悬停 + 滚动）时顶点/批次成本 ×K |
| B3 | `UiTree::render` 局部模式**每块脏区扫一遍 `paint_order`** | `render_candidates` 内 `for idx in 0..count` | K × O(n)；11k 节点 × 4 块 ≈ 44k 次遍历（微秒级，但可合并成一次） |
| B4 | Web 每帧 **2 次 w*h*4 分配**（`bgra_to_rgba` + `ImageData`） | `present.rs:41-46` | 800×600 ≈ 3.8 MB/帧 @60fps ≈ 228 MB/s |
| B5 | `UiTree::render` 每帧 `dirty.clone()` + `AnimSnapshot::new(collect())` | tree.rs:2009-2016 | 小但纯浪费；动画多时 `collect` 分配明显 |
| B6 | D3D `build_batches()` 每帧新建 `Vec` | mod.rs:339/865 | 每帧 1 次分配；图片多时更大 |
| B7 | **Present/合成器节拍**吞掉一切 | 空帧=满帧=6.885 ms | 任何 CPU 优化在窗口模式下都测不出来 → 必须分层计时 |
| B8 | X11 无脏区 → expose/resize 之外也全量重绘 | 源码 | 静止画面若被 invalidate 就整帧重来 |
| B9 | **X11：WM 驱动的窗口 resize 不生效**（最高危，需真机确认） | `ConfigureNotify` → `RawEvent::Resize{w,h}`，但运行时只置 `need_layout = true`、**丢弃 w/h**（[runtime.rs:106-108](../crates/y901-runtime/src/runtime.rs)）；`relayout(tree)` 无尺寸参数、读缓存 `self.size()`（[x11/window.rs:1182-1187](../crates/y901-backend-x11/src/window.rs)）；`self.width/height` 只在 DPI 路径（window.rs:685）与 `Presenter::resize`（**生产代码永不调用**：`frame_step<W: WindowBackend>` 的 bound 不含 `Presenter`）被写；`present` 再 `width.min(self.width)` 钳制（window.rs:1102-1103） | resize 后软缓冲、布局视口、上传区域**全停在旧尺寸**，新区域显示未定义内容。这是 `WindowBackend::relayout` 缺尺寸参数暴露的契约裂缝；Win32 不受影响（自己处理 WM_SIZE） |
| B10 | D3D 图集上传**每帧整张 4 MiB 克隆** + 每脏区 patch 分配 | `self.atlas_data = data.to_vec()`（d3d11.rs:2223）、`vec![0u8; row_pitch*height]`（:2245） | 任何一帧有字形被光栅化就付一次；D3D 侧本来只需要区域更新 |
| B11 | D3D 热路径里的 `env::var`：`Y901_GAMMA` 每帧、`Y901_DEBUG_BATCHES` **每脏区** | d3d11.rs:2496 / :1957 | 全局锁 + 分配；与本轮已修的软光栅 `Y901_PAINT_TRACE` 同类 |
| B12 | **实例上限 10 000，超出静默截断**（进程级只告警一次） | `instances.len().min(INSTANCE_CAPACITY)`（d3d11.rs:2504） | 富 UI（100×100 网格带文字）可达；越界后"后半帧消失"且无后续日志 —— 正确性问题，不只是性能 |
| B13 | Web 每帧：`bgra_to_rgba`（192 万次 `push` + 新 `Vec`）+ 新 `ImageData` + 新 rAF `Closure` + `global_window()` `JsValue` | present.rs:53-62、:42；lib.rs:974-1079 | 800×600 @60fps ≈ 115 MB/s JS 堆垃圾；GC 抖动会直接吃掉帧预算 |
| B14 | **软渲染路径不推进图集记账**：`begin_frame`/`take_dirty` 只有 D3D 调用 | backend/mod.rs:236,246 vs `SoftBackend` 从不调用；engine.rs:738-755 | 影响 Win32 `--force-soft` / X11 / Web：LRU 热集保护退化、`dirty` 列表永不释放 |

**不可确认/需要仪器才能判断**：GPU 侧真实耗时（D3D 没有 GPU timestamp query / disjoint query）；
X11 IPC 占比（只有整帧基准，无分段）；Web 上 `putImageData` vs 光栅各自占比。

---

## 7. 架构缺口（对照目标架构）

| 目标组件 | 现状 | 缺口 |
|---|---|---|
| `PaintCommand` | **不存在**（`RenderCtx` 直调 + 后端私有 `Instance`） | 需要一层"可检查的命令"：`FillRect/StrokeRect/RoundedRect/GlyphRun/Image/Clip/Opacity/Transform` |
| `FramePacket` | **只有死壳** `gfx::Frame{instances, atlas_updates, atlas_reset}` | 缺 `frame_id / viewport / scale_factor / clear / damage / resources`；且 `instances` 是后端布局而非语义命令 |
| `RenderPlanner` | **只有 D3D 的 `build_batches`**（图片/图集交替） | 缺 clip 传播、相邻合并、per-rect 实例列表、统计；且只在后端内、不可跨平台复用 |
| `DamageTracker` | `DirtyRegion` 已在 `y901-ui`，质量不错（合并/上限/full 回退） | 缺"旧∪新"自动追踪（依赖 `mark_moved_map`，已实现但只在布局路径）；缺 damage 面积/计数统计 |
| `ResourceCache` | `TextureId(u64)` + `retain/release` + 图集 `take_dirty` | 缺 **generation**（`TextureId` 是裸 u64，后端 `HashMap<u64, …>`，删后重用 id 会误引用）；图集/渐变无统一句柄 |
| **后端尺寸权威** | Win32 自持（WM_SIZE 自己处理）、X11 **不更新**、Web 由 canvas 决定 | 缺统一契约："谁负责把窗口新尺寸告诉光栅器"。RENDER-02 的 `FramePacket.viewport` 必须由后端每帧提供，而不是让 `relayout` 猜（见 B9） |
| `RenderStats` | `RenderOutcome{Skipped,Presented}` | 缺全部 12 项指标；`present_bench` 只测"一次 present 总时长" |
| 跨后端视觉回归 | `hash-gate`（单场景、仅 D3D）+ 若干软光栅金标 | 缺"同一 FramePacket → 三后端 → 逐像素比较" |

---

## 8. API 兼容风险

| 变更 | 风险 | 说明 |
|---|---|---|
| 在 `RenderCtx` 上加方法 | **低**（有默认实现即可） | 但若把新方法设为必需，所有实现（含测试替身）会编译失败 |
| 把 `RenderCtx` 换成 `FramePacket` 收集器 | **高（破坏性）** | 现有 20+ 处测试 `impl RenderCtx for Recorder`；且 `Widget::render(&mut dyn RenderCtx)` 是公共 API。**必须先并行提供一层适配器**，不能直接替换 |
| 删除 `gfx::{Frame,RenderBackend,GfxInstance}` | **中** | 它们在 `y901-runtime` 公共 API 里；但无生产调用者。删/改都要记 changelog |
| 给 `DirtyRegion` 加统计字段 | 低 | 结构体非 `#[non_exhaustive]` 但字段私有，安全 |
| `TextureId` 加 generation | **中** | `TextureId(u64)` 是 `pub struct`（元组字段是否 pub 需确认）；若变内部布局，自定义后端会受影响 |
| 三后端统一 `present(&FramePacket)` | 高 | X11/Web 目前是"软光栅 + 逐帧上屏"，直接换会让两个平台同时红；必须逐平台迁移并保留旧路径 |

**结论**：新架构应以**"旁挂式"**落地 —— `FramePacket`/`Planner` 先作为**可选路径**与现有
`RenderCtx` 并存（`RenderCtx` 的一个实现负责**收集**命令而不是立即画），
验证通过后再逐个后端切换。这是唯一不破坏 1215 条测试与像素金标的路径。

---

## 9. RENDER-01 … RENDER-05 实施建议

### RENDER-01 Cross-platform render benchmarks

**现状**：基准分散在 4 个 crate、各自打印格式不同；`perf-gate.ps1` 只强制 4 条预算；
X11/Web 的基准要真窗口（`#[ignore]`），CI 里不跑。

**建议**：
1. 统一到一个可执行基准入口：`crates/y901-shell/tests/render_bench.rs`（或
   `examples/render_bench.rs`），用**同一组场景**（`empty / 1k rects / 10k rects / 500 glyphs /
   100 images / mixed`）在三个平台上跑；
2. 输出 **JSON 行**（`{"scene":"10k_rects","stage":"build","p50_us":…,"p95_us":…,"allocs":…}`），
   便于机器比对与入库；
3. 每个场景**分三段计时**：`build`（命令生成）/ `execute`（后端提交）/ `present`（上屏）；
4. `perf-gate.ps1` 增加"同 commit 与上一次基线比较"，超阈值（如 +15%）失败；
5. Web 基准在 `wasm-bindgen-test` 或 headless Chrome 里跑（CI 已有 `web-aria` job 可扩展）。

**完成条件**：三平台都能输出同一 schema 的 JSON；CI 有至少 D3D 的自动比对；
文档记录环境（CPU/GPU/res/DPI/profile）。

### RENDER-02 Immutable FramePacket

**建议设计**（先旁挂，不改 `Widget::render` 签名）：

```rust
// crates/y901-render/src/packet.rs（新）
pub struct FramePacket {
    pub frame_id: u64,
    pub viewport: Size,
    pub scale_factor: f32,
    pub clear: ClearMode,           // Color(Color) | Keep
    pub damage: Vec<Rect>,          // 空 = full
    pub commands: Vec<PaintCommand>,
    pub resources: Vec<ResourceRef>, // 本帧引用的纹理/图集版本
}

pub enum PaintCommand {
    Clip { rect: Rect },  ClipPop,
    FillRect { rect: Rect, color: Color },
    StrokeRect { rect: Rect, color: Color, width: f32 },
    RoundedRect { rect: Rect, color: Color, radius: f32 },      // 语义级，不是拆成 3 个 fill
    GradientRect { rect: Rect, from: Color, to: Color, dir: (f32,f32) },
    GlyphRun { origin: Pos, style: TextStyle, glyphs: Vec<LaidGlyph>, atlas_rev: u64 },
    Image { rect: Rect, texture: TextureRef, style: ImageStyle },
    Opacity { value: f32 }, OpacityPop,
}
```

**收集方式**：新增 `PacketRecorder` 实现 `RenderCtx`，把 27 个方法翻译成命令
（圆角/渐变**保留语义**，不再拆块）。`UiTree::render(&mut recorder)` 即可产出 packet，
**现有 widget 代码零改动**。

**完成条件**：`PacketRecorder` 能记录全部内置控件的绘制；新增测试断言
"同一棵树 → 命令序列稳定"（快照式测试）；D3D/Soft 仍走旧路径，像素不变。

### RENDER-03 RenderPlanner

**第一版只做保守优化**（用户要求：不做未经证明的全局重排）：

1. **clip 传播**：`Clip{rect}` 之后的命令预先与当前 clip 求交（越界命令直接丢弃）；
2. **相邻合并**：连续 ≥2 个 `FillRect` 且**颜色相同、互不重叠、无 clip 变化**时合成一条
   `FillRects`（仅对不透明矩形；透明/渐变一律不合并，保序）；
3. **per-damage 索引**：给每条命令算"影响矩形"，按 damage 分桶 —— 直接消灭 B2/B3 的 `K×`；
4. **统计**：`RenderPlan { batches, stats }`，`stats` 填 `RENDER-04` 的字段。

**禁止**：跨 `Clip`/`Opacity`/`Image` 重排、透明命令合并、全局 z-order 排序。

**完成条件**：`planner` 是纯函数（输入 packet → 输出 plan），单测覆盖 clip 传播、
相邻合并、per-damage 分桶；**用 `RecordingCtx` 对拍"计划前后绘制顺序完全一致"**。

### RENDER-04 RenderStats instrumentation

```rust
pub struct RenderStats {
    pub command_count: u32, pub batch_count: u32,
    pub damage_rect_count: u32, pub damaged_pixels: u64,
    pub build_time: Duration,  pub plan_time: Duration,
    pub execute_time: Duration, pub present_time: Duration,
    pub resource_upload_bytes: u64,
    pub cache_hits: u64, pub cache_misses: u64,
    pub allocation_count: u64, pub allocation_bytes: u64,
}
```

**关键**：`present_time` 必须单独测（`Present` 前后各一次 `Instant`），且**不得**
包含等待 vsync 的时间（现在窗口模式下 Present 会阻塞在 DWM —— 需要区分
"Present 调用耗时"与"帧间隔"）。可先用现有 `present_bench` 的结论（6.885 ms 固定）
在校验里做**上限断言**而不是性能断言。

**完成条件**：`WindowBackend::render` 返回（或旁路暴露）`RenderStats`；

**本轮深读暴露的、必须一起记录的点位**（否则统计不到真正的大头）：

- `atlas_upload_bytes` 要在 `update_atlas` 里按 `reset ? size*size*4 : Σ(patch)` 记，
  而不是按"脏区个数"（见 B10）；
- `instance_count` 要同时记 `pushed` 与 `drawn` —— 后者受 `INSTANCE_CAPACITY` 截断（见 B12），
  两者不等就说明这一帧**丢内容**了；
- `present_time` 在 `swap_chain.present` 前后各取一次 `Instant`，并把"被遮挡"
  （`DXGI_STATUS_OCCLUDED`）单独计一档（它与成功上屏的耗时语义不同）；
- `cache_hits / cache_misses` 直接复用图集的 `take_dirty` 与字形缓存命中计数，不要另建一套。

**顺带建议三个零风险修复**（都不改变渲染结果，可独立提交/独立回滚）：

1. `Y901_GAMMA` / `Y901_DEBUG_BATCHES` / `Y901_GAMMA_AB` / `Y901_SCISSOR_TEST` /
   `Y901_SNAPSHOT` 全部改成 `OnceLock` 缓存（与软光栅里修 `Y901_PAINT_TRACE` 完全同构）；
2. `update_atlas` 去掉 `data.to_vec()`：改成复用缓冲（`clear + extend_from_slice`）；
3. `build_batches()` 与 `self.dirty.clone()` 改成复用 `Vec`（`mem::take` + `clear`）。

`render_bench` 把 12 个字段写进 JSON；文档说明每个字段的测量点。

### RENDER-05 Full-render correctness oracle

**这是 partial rendering 的前置条件**（用户已明确：不能只在空白 buffer 上测）。

建议做成 `crates/y901-render/tests/partial_oracle.rs`：

```
for scene in SCENES {                 // move / delete / hide / show / clip / z-order /
                                       // resize / text / atlas-grow / image-update / expose
    let mut full_ctx = SoftBackend::new_headless();   // 参照：整帧
    tree.invalidate_all(); tree.render(&mut full_ctx);

    let mut partial_ctx = 与 full 相同的初始内容;      // **关键**：先把上一帧画进去
    tree.apply_scene_mutation();
    tree.render(&mut partial_ctx);                    // 局部路径

    assert_bytes_eq(full, partial);
}
```

要点：
1. **partial 起点必须是"上一帧已绘制"的 buffer**（现有 `ui_partial_vs_full.rs` 已这样做，
   但只有 4 个场景）；
2. 扩到 11 个场景，并覆盖 **D3D**（`read_back_pixels` 有现成设施）；
3. 复用现有 `frame_hash` 失败信息（打印首个差异像素）。

**完成条件**：11 场景 × 2 后端（soft + D3D）全绿；作为 `gate.ps1` 的一部分；
任何 partial 优化必须先过这个 oracle。

---

## 10. 第一批最小改动方案（我建议先做这三个，不改任何渲染行为）

| 步骤 | 改动 | 风险 | 为什么先做 |
|---|---|---|---|
| **A** | `RenderStats` 结构 + 在 `Window::render_inner`（Win32）与 X11/Web 的 `render*` 里埋点，只**打印**不改变行为 | 极低 | 没有分层数据，后面所有优化都无法证明有效（B7 会淹没一切） |
| **B** | `render_bench` 统一入口 + JSON 输出，先只接 **soft + D3D**，场景 5 个 | 低 | RENDER-01 的骨架；顺便把"现有 4 条预算"变成结构化基线 |
| **C** | `PacketRecorder` + `PaintCommand`（**只收集、不执行**）+ 一条"命令序列快照"测试 | 低（纯新增文件） | RENDER-02 的最小可用版本；能立刻量化"一个控件画了多少条命令"，为 RENDER-03 立靶 |

**深读后新增的"零风险清单"**（不改变任何像素、不改公开 API，可在批准后并入 A/B/C 任一提交，
每条都能独立回滚；建议**先做这 6 条**，因为它们让后面的性能数字变干净）：

| # | 改动 | 位置 | 预期收益 |
|---|---|---|---|
| Z1 | `Y901_GAMMA`/`Y901_DEBUG_BATCHES`/`Y901_GAMMA_AB`/`Y901_SCISSOR_TEST`/`Y901_SNAPSHOT` → `OnceLock` 缓存 | d3d11.rs:1957,2496,2586,2639,2693 | 去掉每帧/每脏区的 `env::var`（全局锁 + 分配） |
| Z2 | `update_atlas` 去掉 `data.to_vec()`，改复用缓冲 | d3d11.rs:2223 | 每上传帧省 4 MiB 拷贝 |
| Z3 | `bgra_to_rgba` 改就地写/复用缓冲，去掉每像素 4 次 `push` | web/present.rs:53-62 | 800×600 省 192 万次 push + 1.9 MB 分配 |
| Z4 | Web `ImageData` 复用（一次性建，按帧 `set_from`） | web/present.rs:42 | 每帧省一个 1.9 MB JS 对象 |
| Z5 | 软渲染路径补 `text.begin_frame()`（帧首）并周期性 `take_dirty()` 丢弃 | soft.rs 各渲染入口 | 修 B14：LRU 热集保护生效、`dirty` 不再只增不减 |
| Z6 | `build_batches()` 与 `self.dirty.clone()` 改复用 `Vec` | mod.rs:339/868、d3d11.rs:2551 | 每帧省 2 次分配 |

> 这 6 条**不做**任何架构改动，但必须先跑 `hash-gate` 与本批新增的 partial oracle 确认像素不变；
> 其中 Z5 会改变图集淘汰时机，属于"行为可能变化"的一项，必须单独提交、单独验证。
**明确不做**（这一批）：
- 不动 D3D/X11/Web 的执行路径（避免同时红三个平台）；
- 不改 `Widget::render` 签名；
- 不删 `gfx::{Frame,RenderBackend}`（等 RENDER-02 稳定后再决定，见风险表）；
- 不引入任何新 crate 依赖（`RenderStats` 用 `std::time`，JSON 用手写格式化，仓库已有 `Json`）。

---

## 11. 需要用户决策的三件事

1. **`gfx::{Frame, RenderBackend, GfxInstance}` 怎么处理？**
   我的建议：**保留但标记 `#[deprecated]`，RENDER-03 完成后再删**。
   理由：它现在没有生产调用者，但删掉要改 `y901-runtime` 公共 API 与 changelog；
   而它和 `RenderCtx` 的"双抽象"是真实风险 —— 新人很容易往那个空壳上加代码。
2. **`RenderCtx` 是否最终演化成"命令收集器"？**
   我的建议：**不直接改 trait 语义**，而是让 `PacketRecorder` 实现它。
   代价是每次绘制多一层虚调用 + push（实测 10k 实例 32 µs，可接受）；
   收益是零 widget 改动、零测试破坏。
3. **X11/Web 的脏区支持优先级**：
   我的建议：**RENDER-05 通过之前不动它们**。X11 现在"整帧但正确"，
   在 partial 没有 oracle 时改成局部渲染是净风险（而且它当前不是性能瓶颈，
   除非用户报 1080p 卡顿 —— 需要先有真机数据）。

---

## 12. 本轮未做的事（诚实标注）

- 未运行任何 benchmark（本机是 Windows，X11/Web 基准跑不了；D3D 基准需要
  `Y901_PERF_GATE=1` 且会占用 GPU —— 留到 RENDER-01 一并做）；
- 未审计 `backend/font.rs`（1192 行）的图集上传细节，只确认了 `sync_atlas` 是增量的；
- 未审计 `d3d11.rs` 的 COM 引用计数与设备丢失恢复全路径（已有 FINDINGS 在
  `docs/AUDIT_2026-11-25.md` 第 4 条）；
- 三个只读审计子代理（D3D11 / X11 / Web 深读）**均已返回**，且每条都在并入前回到源码核对过
  （§4.1.1、§4.2.1、§4.3.1，以及 B9–B14）。子代理报告里标 `uncertain` 的项我在文中保持
  `uncertain`，没有升级成结论。
- 子代理报告里标 `uncertain` 的三项（图集 `compact_atlas` 在渲染中途改 UV 是否可见、
  设备重建后 `self.images` 里的旧纹理行为、X11 非 TrueColor visual 的颜色错乱范围）
  我**没有**验证，本文也按 `uncertain` 保留，不能当成结论引用。