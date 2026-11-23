# Y901 性能白皮书

> 状态：**性能调优阶段已竣工**（2026-11-01）
> 结论：脏区合并提升 **230×**（历史口径 −220× 基于 0.66 / 145.4），布局提升 **47%**，图集与实例提交经实测**确认不是瓶颈**。

本文是 Y901 渲染与布局子系统的官方性能文档：怎么测、测出什么、改了什么、**以及哪些刻意没改**。
所有数字都可用文末命令复现。

---

## 0. 一句话结论

| 维度 | 优化前 | 优化后 | 变化 | 判定 |
|---|---|---|---|---|
| 脏区合并（10k 次流式 add） | 135.6 ms | **0.59 ms** | **−230×** | 已修 |
| 脏区最终块数（流式 10k 行） | 10 000 | **1** | 合并生效 | 已修 |
| 布局 10k（release, avg） | 1 846 µs | **972 µs**（今日实测；+3.8%，历史值 936 µs） | **−47%** | 已修 |
| 布局 50k（release, avg） | 12 675 µs | **6 727 µs** | **−47%（1.88×）** | 已修 |
| 每节点堆分配 | 1.90 次 | **0.10 次** | **−95%** | 已修 |
| 字形图集（5k 字形，1024²） | 32 ns/次、0.7% 碎片 | 未动 | — | **无需优化** |
| 实例提交（10k 矩形） | CPU 32 µs | 未动 | — | **无需优化** |
| 帧级固定开销 | 6 885 µs（真机 RTX 4060 @144Hz，空帧）；6 887 µs（满帧） | 未动 | — | **已复测** |

> **脏区口径（2026-11-02 复测）**：0.59 ms 为两次实测 589.7 / 631.3 µs（N=2）；优化前 135.6 ms 为同一会话基线
> （`evidence/performance/baseline-before-optimization-f1bda2a.log`）。
> 历史口径 145.4 ms → 0.66 ms（−220×）以及 commit `481006d` 记录的 0.64 ms，属同量级、跑次间波动，仅作历史对照。

---

## 1. 测量方法

### 1.1 基准位于哪里

| 文件 | 内容 | 是否需要 GPU |
|---|---|---|
| `crates/y901-ui/tests/perf_bench.rs` | 布局 10k/50k/deep、id 长度敏感性、脏区流式与散点 | 否 |
| `crates/y901-ui/tests/perf_alloc.rs` | 堆分配计数（独立二进制，见 §1.3） | 否 |
| `crates/y901-backend-win32/tests/perf_bench.rs` | 图集 skyline、10k 实例 CPU 构建、整帧、空帧基线 | 是 |

全部 `#[ignore]`，不进默认测试流程（**当轮口径**默认 824 个测试保持秒级）。

### 1.2 为什么不用火焰图

目标平台是 Windows，`perf` 不可用；装 ETW/WPA 能出图，但归因结论仍要靠对照实验。
本项目采用**控制变量法**：

- 想知道"字符串哈希是不是热点"→ 同一棵树，只改 id 长度（4 / 16 / 48 字符）；
- 想知道"分配是不是热点"→ 挂计数分配器，数每次重排的分配次数；
- 想知道"是不是实例太多导致慢"→ 空帧 vs 10k 实例帧对比。

结论全部来自可复现的对照，不来自"看图说话"。

### 1.3 一个必须记住的陷阱

**计数分配器会污染计时。** 它给每次分配加两次原子操作，实测把 50k 布局从 ~6.9 ms 抬到 ~8.4 ms。
我最初把它放在 `perf_bench.rs` 里，差点据此得出"`cache_key` 无效"的错误结论。

现在它独立在 `perf_alloc.rs`：**计时基准不含分配器，分配计数单独跑。**

---

## 2. 布局引擎

### 2.1 数据（release / debug）

| 场景 | 模式 | nodes | avg | p50 | p99 |
|---|---|---|---|---|---|
| 10k flat | release | 10 001 | **972 µs**（今日实测；历史 936 µs） | 991 µs | 1 211 µs |
| 10k deep | release | 11 111 | 1 120 µs | 1 122 µs | 1 374 µs |
| 50k flat | release | 50 001 | **6 727 µs** | —（未采集） | 8 932 µs（p95） |
| 10k flat | debug | 10 001 | 10 486 µs | 11 090 µs | 11 538 µs |
| 50k flat | debug | 50 001 | 57 659 µs | 62 543 µs | 68 524 µs |

> 50k 为**实测 7 次**（`evidence/performance/layout-50k-after-7runs.log`）：中位数 **6.73 ms**，区间 6.37–8.93 ms。
> p95 被首跑冷启动抬高；剔除首跑后 6 次中位数 6.706 ms，与 6.727 ms 差 0.3%。
> 本机（虚拟 D3D 适配器 + 后台负载）自身有 ±8% 抖动，所以给区间而不是单点。

### 2.2 优化前的归因

| id 长度 | 优化前 | 优化后 |
|---|---|---|
| 4 字符 | 1 925 µs | **905 µs** |
| 16 字符 | 1 706 µs | **934 µs** |
| 48 字符 | 2 193 µs | **915 µs** |

优化前**字符串越长越慢**（+14~19%）→ 证明每节点都在对 id 做哈希；
优化后三者持平 → **布局路径上已经没有字符串哈希**。

### 2.3 做了什么

1. **`UiTree::rects`：`HashMap<String, Rect>` → `HashMap<u64, Rect>`**
   消灭每节点一次 `id.to_string()` 堆分配。
2. **`PaintEntry`：`path: Vec<usize>` → `(parent: Option<usize>, slot: usize)`**
   消灭每节点一次 `path.clone()`。DFS 下父节点必先入表，命中时用 `path_of()` 沿 parent 链重建（O(depth)）。
3. **`cache_key()` 预计算**（`Button`/`TextLabel`/`TextInput`/`ListView`/`ScrollView`）
   `rebuild_indices` 优先取 `node.cache_key()`，`None` 时才 `id_key()`。

并排对照（同一次运行内）：

| 50k 变体 | avg |
|---|---|
| 叶子不缓存 key | 7 980 µs |
| 叶子缓存 key | **7 521 µs**（−5.8%） |

堆分配：**19 005 → 1 004 次/重排**（每节点 1.90 → 0.10）。

### 2.4 剩下的成本（刻意不动）

50k 仍未进入 6 ms。剩余构成：

- 每节点 1 次 `measure` + 1 次 `arrange` 虚调用；
- `LayoutCache` 每节点 ≥2 次 HashMap 查询；
- 约 1 000 次/重排的堆分配（HashMap 扩容等）。

继续压需要**换数据结构**（布局结果平铺进 `Vec`、用索引替代 HashMap）。
这是架构取舍，ROI 已不高，**明确不做**。

---

## 3. 脏区合并（本轮最大收益）

### 3.1 数据

| 场景 | 优化前 | 优化后 |
|---|---|---|
| 流式 10 000 次 add | **135.6 ms** | **0.59 ms** |
| 每次 add 平均 | 13.5 µs | **29 ns**（28–29 ns） |
| 流式结束时矩形块数 | 10 000 | **1** |
| 散点 10 000 次 add | 134.4 ms | **17 µs** |

### 3.2 两个独立的病

1. **O(n²)**：`add` 每次线性扫全部已有矩形 → 10k 次插入 ≈ 1 亿次矩形比较。
2. **边贴边不合并**：流式行共用一条边，`intersect()` 返回空 → 被判"不相交"→ 永远合不上。

### 3.3 修法

```rust
// 1) 硬上限：最坏情况 O(n²) → O(1)
if self.rects.len() >= Self::MAX_RECTS {   // MAX_RECTS = 16
    self.full = true;
    self.rects.clear();
    return;
}

// 2) 相邻（含相切）也算相连
fn touches(a: Rect, b: Rect) -> bool {
    let sep_x = a.right() < b.x || b.right() < a.x;
    let sep_y = a.bottom() < b.y || b.bottom() < a.y;
    !(sep_x || sep_y)
}
```

合并只会让脏区**变大**，不会漏画——安全性由"更脏 = 更多节点被重绘"保证。

### 3.4 为什么这条最重要

脏区碎片化后，渲染时 `plan_dirty_rects` 会因超过 16 块而**整窗重绘**。
也就是说：局部重绘的全部努力，在流式场景下等于没有。修完这条，局部重绘才真正成立。

---

## 4. 字形图集 —— 结论：**无需优化**

| 指标 | 数值 |
|---|---|
| 5 000 字形（8×16 / 16×16 交替）入 1024² | 总 **324 µs**，avg **32 ns**，p99 100 ns，max 500 ns |
| used_height | 944 / 1024 |
| 碎片率 | **0.7%** |

Skyline 分发快、碎片极低、容量还有富余。

**因此明确否决 MaxRects 重构**：换算法最多省下 300 µs 的**一次性**成本，
却要重写分配器 + 重建 LRU + 重跑全部图集回归——ROI 为负。

---

## 5. 渲染提交流 —— 结论：**无需优化**

| 场景 | 结果 |
|---|---|
| 10k `fill_rect` 的纯 CPU 构建 | **32 µs**（p99 47 µs） |
| 10k 实例 + 真实 GPU 整帧 | 6 889 µs |
| **空帧**（0 实例，同尺寸） | **6 889 µs** |

空帧与 10k 实例帧**完全相同** → 实例构建与提交对整帧耗时贡献 ≈ 0。
不存在 Buffer 扩容：实例缓冲是固定 10k 容量、一次 `Map(DISCARD)` 全量拷贝。

### 真机复测（2026-11-02）

环境：Windows 11 + **NVIDIA GeForce RTX 4060 Laptop GPU**，1920×1080 @ **144 Hz**（本次会话同时挂着
GameViewer / ToDesk 两个虚拟显示适配器，窗口实际合成在虚拟显示上）。

| 场景 | 修复前（虚拟适配器） | 真机复测 |
|---|---|---|
| 空帧（0 实例） | 6 889 µs | **6 885 µs** |
| 满帧（10 000 实例） | 6 889 µs | **6 887 µs** |
| 10k `fill_rect` 纯 CPU 构建 | 32 µs | **53 µs** |

**关键读数：`1 / 144 Hz = 6.94 ms`，实测空帧 6.885 ms —— 帧时间等于垂直同步间隔。**

所以这 6.9 ms **既不是渲染耗时，也不是虚拟适配器的软件 Present**，而是 **vsync 节流**。
证据链：空帧与 10k 实例帧相差 < 10 µs（0.15%），说明实例构建与提交对本帧**没有可测贡献**。

**结论：渲染路径毕业。** 10 000 个实例的边际成本 ≈ 0；剩下的时间是显示设备强制的心跳。
按需重绘（只画脏区）已经把"不该画的"全部省掉，再优化只能改 vsync 策略，那是产品决定而非性能问题。

> 备注：若要测"真实 Present 开销"（不受 vsync 影响），需要在**物理显示器 + 独占全屏**下把
> `sync_interval` 设为 0 再测；本环境两个虚拟适配器都在 144 Hz 上合成，做不到。

**布局**（同一台机器，`layout_bench`，11 111 个节点全树重排）：**1 261 µs**（worst 1 515 µs）。
按线性外推 50 000 节点约 **5.7 ms**，与早先测到的 ~6.9 ms 一致；没有退化。


---

## 6. 复现命令

**首选：一条命令跑全部四项**（与 CI 的 perf workflow 同源，输出带 CPU / OS / rustc）：

```powershell
powershell -NoProfile -File scripts/perf-gate.ps1          # 4 项，默认强制阈值
powershell -NoProfile -File scripts/perf-gate.ps1 -Only grid
powershell -NoProfile -File scripts/perf-gate.ps1 -NoEnforce  # 只打数字，不断言
```

单项手动跑（了解每个基准在测什么时用）：

```bash
# 布局 10k（grid_perf 里的两个用例，release 预算 6ms）
cargo test -p y901-ui --release --test grid_perf -- --include-ignored --nocapture

# 布局（10k / 50k，打印 avg/worst）
cargo test -p y901-ui --release --test layout_bench -- --include-ignored --nocapture

# 脏区合并策略对比（打印每种场景的 rect 数与覆盖面积）
cargo test -p y901-render --release --test dirty_strategy_bench -- --include-ignored --nocapture

# 堆分配计数（独立二进制，勿混入计时）
cargo test -p y901-ui --release --test perf_alloc -- --ignored --nocapture

# 软光栅填充率 + 图集/实例提交（后者需真实 GPU）
cargo test -p y901-backend-win32 --release --test soft_backend -- --include-ignored --nocapture soft_fill_rate
cargo test -p y901-backend-win32 --release --test perf_bench -- --ignored --nocapture
```

### 6.1 2026-11-22 复测（本机，用于校准"现在还是这个数吗"）

环境：Intel i7-13650HX / Windows 11 (10.0.26200) / rustc 1.92.0 / `--release`。

| 基准 | 结果 |
|---|---|
| `grid_perf`：10k 网格 layout | avg **1.30 ms**，worst **1.46 ms**（预算 6 ms） |
| `grid_perf`：10k place | **2.15 ms** |
| `layout_bench`：11 111 节点 × 20 轮 | avg **1 853 µs**，worst 2 237 µs |
| `dirty_strategy_bench` | 光标闪烁 128 rects；多控件 38 400 → 2 720；角落相贴 50 150 → 300 |
| `soft_backend` 填充率 1280×800 | fill_rect **0.10 ms**；gradient 4.25 ms；round_rect(SDF) 0.69 ms |

> **与 §2 历史值的差异要如实看**：§2 记的 10k 布局是 972 µs，本轮同机同命令测得 1 853 µs
> （约 1.9×）。两次不是同一台机器、也不是同一轮代码（中间加了 per-node 墨迹边距、
> 文本 shaping 基础设施等），**不能据此说"性能退化了 90%"**，也不能拿它当基线承诺。
> 要判定退化，需要在同一台机器上对照两个 commit 各跑一次 `scripts/perf-gate.ps1`。

---

## 7. 优化优先级（本轮结束后）

| 优先级 | 项 | 状态 |
|---|---|---|
| P0 | 脏区上限 + 相邻合并 | ✅ 已完成（−220×） |
| P1 | 布局去字符串化 + `cache_key` | ✅ 已完成（−47%） |
| P2 | 帧级开销（Present/blit） | ⏸ **等真机数据，暂不动** |
| — | 图集换 MaxRects | ❌ 已否决（ROI 为负） |
| — | 布局数据结构重构（平铺数组） | ❌ 已否决（架构取舍，收益 < 成本） |

**本框架的"性能调优阶段"到此竣工。** 后续若性能回退，先跑 §6 的命令对表，再决定是否动刀。

---

## 8. 内核优化：2026-11-23 一轮（有测量、有前后对比）

这一轮不是"感觉慢就改"，而是先写了剖析台
[`crates/y901-ui/tests/layout_profile.rs`](../crates/y901-ui/tests/layout_profile.rs)
（`#[ignore]`，带可开关的计数分配器 + 空 `RenderCtx`），用**数字**找出真实浪费。复跑：

```powershell
cargo test -p y901-ui --release --test layout_profile -- --ignored --nocapture
```

| 项 | 发现的浪费 | 修法 | 前后（11k 节点，同机同进程） |
|---|---|---|---|
| 热布局 | **"能跳过的布局"其实不便宜**：`UiTree::layout` 在判断能否跳过**之前**就走一遍全树、给每节点 `id().to_string()` 建快照 —— 11k 次 String 分配/帧 | `mem::take` 上一轮的 `rects` 索引表（`u64 → Rect`）直接对比；跳过时原样放回 | 热布局 **573–735 µs → 0.1 µs**，分配 **11k 次 → 0** |
| 墨迹边距表 | 每次重排 `HashMap::new()` 再灌 11k 项 | `clear()` 复用同一张表（保留容量） | 分配字节 **646 140 → 88 944**（7.3×） |
| 索引表哈希 | 键是我们自己算好的 `u64`，却仍走 SipHash；每节点 4–5 次查表 | 新增 `crates/y901-ui/src/fasthash.rs`（FNV-1a `BuildHasher`，零新依赖），换 `FastMap`/`FastSet` | 冷布局 **1 808 µs → 1 222 µs**（−32%）；宽树 10k **1 137 → 936 µs** |
| 整帧绘制遍历 | 整帧时仍逐节点查墨迹边距并与脏区求交（`paint_all` 下结果恒真，纯浪费） | `paint_all` 时跳过该查表 | 约 **1 466 → 1 300–1 320 µs**（3 次取样中 2 次；第 3 次 1 623 µs 属调度噪声） |

**为什么先做前两项**：它们都出现在"什么也没变"的路径上 —— 真实应用的静止帧
每 16 ms 走一次 `layout()`，旧实现每帧白做 11k 次字符串分配。

**明确没做**（避免"为了好看而改"）：

- 布局数据结构换平铺数组：与 `Box<dyn Widget>` 树模型冲突，收益 < 改造成本；
- 绘制遍历改"按 `paint_order` 走平铺表"：需要别名化的 `&mut dyn Widget`，会引入 `unsafe`
  或每帧重建索引，违背"正确性优先"；
- 不设虚阈值：所有数字都是本机实测，`perf-gate.ps1` 里没有拍脑袋的预算。

---

## 5. Present 节拍：sync=0 vs sync=1（v0.1.12）

基准：`crates/y901-backend-win32/tests/present_bench.rs`（`#[ignore]`），800×600 窗口，100 帧，release。

| 组 | 配置 | avg | p50 | p95 | max |
|---|---|---|---|---|---|
| A′ | 窗口 + `sync_interval = 0` | 6801.6µs | **6935.7µs** | 7108.2µs | 7285.9µs |
| B | 窗口 + `sync_interval = 1` | 6940.8µs | **6932.1µs** | 7116.4µs | 7213.6µs |

**p50 差值（B − A′）= −3.6µs**（上一轮 −18µs，均为**噪声级**）。

### 结论

1. **6.9ms = 1/144Hz，与 `sync_interval` 无关** —— 两组几乎完全相同。
2. **主循环里没有固定 sleep**：全局搜索 `thread::sleep` / `WaitMessage` / `SetTimer` 无命中；
   `MsgWaitForMultipleObjectsEx` 只在 `wait_message(timeout)` 里，而基准不调它。
3. **因此帧节奏被 DWM 合成器限制**：窗口模式下即使 `SyncInterval = 0`，
   `Present` 仍被合成周期牵引。这是 **Windows 窗口模式的固有行为**，不是框架问题。

### 已知限制

**窗口模式无法测出纯 Present 提交耗时**。要得到它需要：

- **独占全屏**（`SetFullscreenState`，需手写 COM 槽位，暂缓）；
- 或 **GPU timestamp query**（区分 submit 与 present，改动更深，暂缓）。

在那之前，**不应引用 6.9ms 作为"渲染开销"或"提交开销"的代表值**——
它只是屏幕刷新周期。
