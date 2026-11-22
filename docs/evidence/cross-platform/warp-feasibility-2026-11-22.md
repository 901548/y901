# WARP 可行性实验（2026-11-22，Windows 11 本机）

目的：判断能否让 **没有 GPU 的 CI runner** 也真跑 D3D11 用例，
从而结束 `gate.ps1 -SkipGpu` 造成的"渲染路径在 CI 里从未执行"。

## 结论（先说结果）

两件事分开看：

- **`Y901_HEADLESS=1`（跳过 Present、保留渲染与回读）—— 有效，已在本机真机验证**：
  像素哈希仍等于基线 `0xF7455CCE7781A864`，`snapshot_harness` 10/10 通过，
  `tabs_transition_d3d` 1/1、`opacity_parity` 3/3 通过。
  这意味着"**有 GPU 但没有交互桌面**"的机器（自托管 runner 的典型形态）现在可以跑 GPU 用例。
- **`Y901_D3D_WARP=1`（纯软件设备）—— 当前不可行**：WARP 设备能创建成功，
  但**第一次真正使用（绘制/呈现）后设备即被移除**（`0x887A0005` = `DXGI_ERROR_DEVICE_REMOVED`），
  之后任何资源创建都失败。因此**没有把 CI 切到 WARP**，`-SkipGpu` 状态保持不变。

## 新增的三个研究开关（默认关闭，生产路径零影响）

| 环境变量 | 作用 | 位置 |
|---|---|---|
| `Y901_D3D_WARP=1` | 只尝试 `D3D_DRIVER_TYPE_WARP`，不先试硬件 | `crates/y901-backend-win32/src/backend/d3d11.rs`（`force_warp`） |
| `Y901_HEADLESS=1` | 渲染与像素回读照常，**跳过 Present** | 同上（`headless_present_skipped`，在 `render_batched` 末尾） |
| `Y901_SNAPSHOT_DEBUG=1` | 打印快照回读每一步的失败原因 | 同上（`read_back_frame`） |

`headless` 之所以有意义：快照是在 `present` **之前**从后备缓冲回读的
（`d3d11.rs` 里 `// 快照：必须在 present 之前回读`），所以"不上屏"不影响像素断言。

## 实测记录

### 1) 只有 WARP（正常 Present）

```
$env:Y901_D3D_WARP='1'; hash-gate.ps1
[y901] backend ready: engine=self ascent=13.75 atlas=2048
[y901] 渲染失败（上屏失败）：Present 失败：0x887A0005
```

设备建出来了，`Present` 返回 `DEVICE_REMOVED`；`present()` 触发 `device_lost`
→ `rebuild` → 再 `Present` 仍失败 → 整体报错，用例走 `SKIP`。

### 2) WARP + HEADLESS（跳过 Present）

```
$env:Y901_D3D_WARP='1'; $env:Y901_HEADLESS='1'; $env:Y901_SNAPSHOT_DEBUG='1'
[snapshot-debug] staging 纹理创建失败: CreateTexture2D failed: 0x887A0005
SKIP: font_3c_pixels 快照失败（无可用 D3D11 设备）
```

`Present` 已经不参与，但**回读用的 staging 纹理创建仍然报同一个 `DEVICE_REMOVED`**。
说明设备是在**更早**的绘制/拷贝阶段就被移除的，而不是 Present 移除的。

### 3) 定位到的边界

- `ensure_cached()`（自持 RT 创建）**成功**，没有打印失败；
- 之后 `draw_scene` / `blit_cached` 执行；
- 紧接着的 `CreateTexture2D`（staging）失败。

即：**设备在第一次 draw/blit 之后就不可用了**。尚未确认是哪个具体调用
（缺 `GetDeviceRemovedReason` 的读取，见下一步）。

### 4) 真机 + HEADLESS（对照组，证明 HEADLESS 本身没问题）

```
$env:Y901_HEADLESS='1'（不设 Y901_D3D_WARP）
hash-gate.ps1        → hash=0xF7455CCE7781A864 size=400x220，通过
snapshot_harness     → 10 passed / 0 failed（全部 #[ignore] 用例真跑）
tabs_transition_d3d  → 1 passed；opacity_parity → 3 passed
```

同样跳过 Present，换成真实硬件设备就全绿 ⇒ **失败来自 WARP 设备本身，不是 headless 机制**。

## 下一步（未做，建议按序）

1. 读 `ID3D11Device::GetDeviceRemovedReason`，拿到 WARP 的**具体**移除原因
   （当前只有笼统的 `0x887A0005`），据此判断是着色器、格式还是采样器不被 WARP 支持。
2. 试"**完全无交换链**"路线：`D3D11CreateDevice(WARP)` + 离屏 RT，
   快照直接从离屏 RT 回读。这同时能验证"移除是 swap chain 引起的"这一假设。
3. **CI 路线二选一**：
   - **自托管 Windows runner + GPU**（推荐，当前即可用）：`Y901_HEADLESS=1` + 去掉 `-SkipGpu`，
     无需交互桌面；
   - GitHub 托管 runner：只能继续 `-SkipGpu`，直到 WARP 问题查清或找到其它软件光栅路径。

## 为什么要留下这些开关

它们是**可复现的探针**：上面每一步都能用一条环境变量重跑。
默认路径不变（`force_warp()` / `headless_present_skipped()` 都默认 `false`），
所以对产品行为、像素基线、现有门禁**零影响**（本轮 `gate.ps1` 仍为 `GATE OK`）。