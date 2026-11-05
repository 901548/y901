# 脏区调度：后端标定结果与不对称性

> 本文是 `docs/design/dirty-scissor-scheduling.md` 的**测量附录**。
> 结论先行：**CPU 与 D3D 后端的 `c_switch/c_pixel` 相差约四个数量级，最优策略相反。**

---

## 1. CPU（SoftBackend）标定

方法见 `crates/y901-render/tests/dirty_strategy_bench.rs`：

| 量 | 方法 | 实测 |
|---|---|---|
| `c_pixel` | 1920×1080 满屏 `fill_rect` × 200 次取均值 | **0.174 ns/px**（361.0 µs/帧） |
| `c_switch` | `push_clip`/`pop_clip` × 100,000 次取均值 | **1.8 ns/次**（0.0018 µs） |

**比值 `c_switch / c_pixel ≈ 10 像素`。**

含义：一次 scissor 切换 = 填 10 个像素。
**任何面积超过 10 px 的脏区，合并都不划算。** CPU 后端应当**逐块绘制**。

---

## 2. D3D 侧：无法直接微基准，但有更强的结构证据

### 2.1 为什么微基准不可行

1. **DWM 节流**：`perf_bench` 的空帧基线本身就是 **6211 µs**（Present 只做一次 swap，
   仍被合成器限制在 ~1/144Hz）。任何"多切 1000 次 scissor"的差值都会被这个地板吞掉。
2. **API 边界**：`set_scissor` 是 `d3d11.rs` 里的 **私有 `unsafe fn`**，且带 `#[allow(dead_code)]`，
   只在 `Y901_SCISSOR_TEST` 探针路径使用 —— 测试无法直接调用，除非改公开面（本轮不允许）。

### 2.2 结构证据（比微基准更硬）

`d3d11.rs:2392-2416`：

```rust
let rects: Vec<RectI32> = /* 脏矩形列表，或 [full] */;
let full_frame = ...;
for rect in rects {
    draw_scene(context, pipeline, atlas, cached, instances, count, batches,
               &self.images, rect, is_full, w, h);   // ← 每个脏矩形一次完整实例化绘制
}
```

**每一个脏矩形都会重新提交整张实例表（`count` 个实例）**，只是换了 scissor。

所以 D3D 侧的"一次切换"**不是一次轻量状态设置**，而是：

```
c_rect = c_scissor + c_draw(N_instances)
```

`c_draw` 随实例数线性增长，**与脏区面积无关**。

### 2.3 从现有基准反推的量级

`perf_bench`（release, 10k 实例）：

| 场景 | 实测 |
|---|---|
| 空帧（仅 Present） | 6211 µs |
| 10k 实例整帧 | 7825 µs |
| 10k 实例 CPU 构建 | 42 µs |

差值 `7825 − 6211 ≈ 1614 µs` 是"构建 + Map + Draw"的可观测部分（**下界**，因为两者都撞到 DWM 地板）。

若一个脏区一份实例表，那么 `k` 个脏区的 D3D 代价 ≈ `k × O(1614 µs)`：

| k | 估算 |
|---|---|
| 1（合并成一块） | ~1.6 ms |
| 8 | **~13 ms** |
| 16 | **~26 ms** |

> 这解释了为什么 `MAX_DIRTY_RECTS = 16` 之前的表现并不差：
> **它其实一直在保护 D3D 后端**，因为超过几块脏区，整帧就会慢到 10ms 以上。

---

## 3. 不对称性：两个后端的最优策略相反

| 后端 | `c_switch / c_pixel` | 最优策略 | 现状 |
|---|---|---|---|
| CPU（Soft） | **≈ 10 px** | **逐块**（永不合并） | `touches` 合并在浪费 167× 像素（角落病态） |
| D3D | **≫ 10⁴ px**（每块 = 一次全量 draw） | **激进合并**（越少越好） | `touches` 方向正确 |

**这解释了为什么"当前策略在普通负载上没出问题"** ——
它服务的后端（D3D）确实需要合并；而 CPU 后端（快照/测试路径）才是被它拖累的那个。

---

## 4. 对设计文档的修正

原代价模型

```
cost = k·c_switch + c_pixel·area(∪s)
```

**在 D3D 上是错的**，应为：

```
cost = k·(c_scissor + c_draw(N)) + c_pixel·area(∪s)
```

其中 `c_draw(N)` 与实例数成正比、与面积无关。这带来两个直接推论：

1. **D3D 的最优 `k` 极小**（通常 1~2）：因为 `c_draw` 太大，任何额外脏区几乎都不值；
2. **合并判据应从"几何"改为"实例代价"**：合并两个脏区省下一整次 `c_draw`，
   代价只是多覆盖一些像素 —— 在 D3D 上几乎总是划算；在 CPU 上完全相反。

> 因此 `plan_dirty_rects` 不应是**后端无关**的策略。
> 它需要后端提供 `c_rect`（一次脏区绘制的代价）与 `c_pixel` 两个数，
> 由后端自己回答"合并是否划算"。

---

## 5. 仍未测量的

| 项 | 状态 | 下一步 |
|---|---|---|
| D3D 真实 `c_scissor`（纯状态设置） | **未测** | 需 GPU timestamp query 或 `SetScissorRect` 10 万次的纯 CPU 侧计时（不含 Present） |
| D3D 的 `c_pixel`（填充率） | **未测** | 满屏 `ClearView` 单帧计时，需绕开 Present（用 `read_back_pixels` 强制同步） |
| `c_draw(N)` 的斜率 | **只有两点估计** | 用 N = 1k / 5k / 10k 三点拟合 |
| CPU 侧 167× 病态的真实帧影响 | **未测帧级** | 目前只有代价模型，未在 `render()` 里验证 |

**在补齐这三项之前，"改 `plan_dirty_rects`"仍然不该做** ——
但**"修 `touches` 的零重叠合并"是安全的**：零重叠时 `area(A∪B) − area(A) − area(B)` 恒 ≥ 0，
任何后端上都不会因此变好（除了省一次 `c_switch`，而它只在 D3D 上有意义，
且那时也应该由 `k` 决策统一处理，而不是靠几何相邻性）。
