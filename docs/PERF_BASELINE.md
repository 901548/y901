# Y901 性能基准数据（2026-11-01）

环境：Windows + 虚拟 D3D11 适配器（Present 开销不具代表性，见注）
运行：
```
cargo test -p y901-ui --release --test perf_bench -- --ignored --nocapture
cargo test -p y901-ui            --test perf_bench -- --ignored --nocapture   # debug
cargo test -p y901-backend-win32 --release --test perf_bench -- --ignored --nocapture
```

## 1. 布局（全量重排，交替视口尺寸）

| 场景 | 模式 | nodes | avg | p50 | p99 | 每节点 |
|---|---|---|---|---|---|---|
| 10k flat | release | 10 001 | 1 846us | 1 862us | 2 570us | ~185ns |
| 10k deep | release | 11 111 | 2 097us | 2 148us | 2 783us | ~189ns |
| 50k flat | release | 50 001 | 13 413us | 12 773us | 22 415us | ~268ns |
| 10k flat | debug | 10 001 | 12 384us | 13 095us | 14 329us | ~1 240ns |
| 50k flat | debug | 50 001 | 65 684us | 72 741us | 73 466us | ~1 310ns |

id 长度敏感性（10k，release）：len=4 → 1 925us；len=16 → 1 706us；len=48 → 2 193us。
（噪声 ±15%，但 48 字符 consistently 更慢 → 字符串哈希/分配有可测成本，非主因）

## 2. 脏区合并

| 场景 | 结果 |
|---|---|
| 流式追加 10 000 个小矩形 | 总计 **145ms**，avg 14.5us/次，p99 30.4us，**最终 10 000 块**（未合并） |
| 散点 10 000 个不相交 | 总计 **135ms**，最终 10 000 块 |

结论：`DirtyRegion::add` 是 O(n) 线性扫描 → 10k 次插入 = ~1 亿次矩形比较。
且**相邻但不相交**（边贴边）的矩形不会被合并（`intersect()` 为空）→ 流式场景仍堆到 10k 块
→ 渲染时 `plan_dirty_rects` 超上限 → 每帧退化成整窗重绘。

## 3. 字形图集（Skyline，1024x1024）

| 指标 | 数值 |
|---|---|
| 5 000 字形（8x16 / 16x16 交替） | 总 **324us**，avg 32ns，p99 100ns，max 500ns |
| used_height | 944 / 1024 |
| 碎片率 | **0.7%** |

结论：Skyline 分配不是瓶颈，**无需优化**。

## 4. 实例提交

| 场景 | 结果 |
|---|---|
| 10k `fill_rect` 的纯 CPU 构建 | 32us（p99 47us） |
| 10k 实例 + 真实 GPU 整帧 | 6 889us |
| **空帧**（0 实例，同尺寸） | **6 889us** |

结论：构建与提交对总耗时**几乎无贡献**；整帧耗时全部落在固定开销（Present/blit）。
当前为虚拟适配器，该数字不具代表性，需在真实 GPU 上复测。
