# 证据文件索引

> 对应代码：`3cfcd49`（v0.1.1 验收后），基线对照：`f1bda2a`（优化前）。
> 除标注「人工注释」的 4 个文件外，所有 `.log` 均为 `命令 > 文件 2>&1` 的原始输出，未做改写。
> 机器：i7-13650HX / RTX 4060 Laptop / 1920×1080 @144 Hz（同机另挂 GameViewer、ToDesk 两个虚拟适配器）。

## 结论 → 证据

| 论文里的结论 | 主值 | 证据文件 |
|---|---|---|
| 脏区合并 | 135.6 ms → **0.59 ms（−230×）**，终块 10 000 → 1 | `performance/dirty-region-after-3cfcd49.log`、`performance/baseline-before-optimization-f1bda2a.log` |
| 布局 10k | 1 846 → **972 µs（−47%）** | `performance/perf-bench-after-3cfcd49.log` |
| 布局 50k | 12 675 → **6 727 µs（−47%，1.88×）**，7 次中位数 | `performance/layout-50k-after-7runs.log`、`performance/layout-50k-before-7runs.log` |
| 布局去字符串化 | id 4/16/48 字符耗时持平（939–1 031 µs）；优化前 +40% | `performance/id-length-after-3cfcd49.log`、`performance/id-length-before-f1bda2a.log` |
| 字体冷启动 | **16.1 µs/字**（release），500 CJK 共 8.0 ms | `performance/font-cold-start-3cfcd49-release.log` |
| 面积恒等式 | 最大误差 **0.440%**（A 0.395%、o 0.034%） | `correctness/area-identity-3cfcd49.log`（debug=release 同值） |
| 镂空正确性 | 口1 日2 田4 回2 国5 | `correctness/cjk-hollow-preview-3cfcd49.log` |
| 双后端一致性 | **max_diff = 2**，\|d\|≤24 覆盖 100% | `correctness/dual-backend-diff-3cfcd49.log` |
| 验收 | **728** 默认 / **766** 含 ignored，0 失败 | `correctness/final-728-passed.log`、`correctness/final-760-include-ignored.log` |
| 跨平台 | WSL+Xvfb 上 45 passed；Helm MIT-SHM 首帧 65.3 ms | `cross-platform/` 三个文件 |

## 文件清单

| 文件 | 证明什么 | 何时跑 | commit | 能否重跑 |
|---|---|---|---|---|
| `performance/dirty-region-after-3cfcd49.log` | 脏区合并**优化后**：10 000 次流式 add 合计 **631.3 µs**（29 ns/次），终块 **1** | 11-02 20:40 | 3cfcd49 | 是 |
| `performance/perf-bench-after-3cfcd49.log` | 同进程 6 个 ui 基准（单线程）：脏区 589.7 µs、10k flat **972 µs**、10k deep 1 171 µs、id 长度 1 645/1 801/1 850 µs。**其中 50k 的 15 395 µs 受同进程前序用例污染，不得引用** | 11-02 20:57 | 3cfcd49 | 是（50k 请用 7runs 文件） |
| `performance/layout-bench-3cfcd49.log` | 11 111 节点全树重排：avg **1 288.3 µs**、worst 1 613.4 µs。「人工注释」在末尾，说明 50k 不在此文件 | 11-02 21:01 | 3cfcd49 | 是 |
| `performance/layout-10k-50k-3cfcd49.log` | 首轮 10k/50k（**默认多线程**）：10k 1 786 µs、50k 9 765 µs。噪声样本，**不用于引用** | 11-02 20:42 | 3cfcd49 | 是（须加 `--test-threads=1`） |
| `performance/layout-10k-50k-3cfcd49-run2.log` | 第二轮同上：10k 1 068 µs、50k 17 091 µs。两轮 50k 差 75%，用于说明多线程下不可信 | 11-02 20:45 | 3cfcd49 | 是（须加 `--test-threads=1`） |
| `performance/layout-50k-after-7runs.log` | 50k **优化后 7 次独立进程**：中位 **6 727 µs**、p95 8 932、区间 6 372–8 932。逐次原始输出 + 「人工注释」汇总块 | 11-02 21:39 | 3cfcd49 | 是 |
| `performance/layout-50k-before-7runs.log` | 50k **优化前 7 次**（worktree）：中位 **12 675 µs**、p95 19 534、区间 11 629–19 534。加速比 **1.88×** | 11-02 21:39 | f1bda2a | 是（需 worktree） |
| `performance/baseline-before-optimization-f1bda2a.log` | **优化前全量基线**（单线程）：脏区 135.6 ms / 终块 10 000；10k flat 1 672 µs；50k 25 388 µs（同进程偏高）；散点 134.4 ms；id 长度 4 003/3 083/5 309 µs（该轮噪声大） | 11-02 21:00 | f1bda2a | 是（需 worktree） |
| `performance/id-length-after-3cfcd49.log` | id 4/16/48 字符，**优化后**两轮：1 031/950/976 与 961/927/939 µs → 与长度无关，布局路径已无字符串哈希 | 11-02 21:02 | 3cfcd49 | 是 |
| `performance/id-length-before-f1bda2a.log` | 同上**优化前**两轮：1 543/1 861/2 299 与 1 634/1 791/2 286 µs → 随长度上升约 40% | 11-02 21:04 | f1bda2a | 是（需 worktree） |
| `performance/font-cold-start-3cfcd49.log` | 500 CJK 光栅化 **debug**：总计 51 510 µs、**103.0 µs/字**、读字体 6 436 µs | 11-02 20:48 | 3cfcd49 | 是（debug 口径） |
| `performance/font-cold-start-3cfcd49-release.log` | 500 CJK **release（论文口径）**：总计 8 040 µs、**16.1 µs/字**、读字体 6 243 µs | 11-02 20:49 | 3cfcd49 | 是 |
| `correctness/area-identity-3cfcd49.log` | 面积恒等式 **debug**：A 0.395%、o 0.034%、e 0.440%、B 0.188%、8 0.017%，全部 <0.5% | 11-02 20:47 | 3cfcd49 | 是 |
| `correctness/area-identity-3cfcd49-release.log` | 同上 **release**：数值与 debug 逐位相同（纯定点运算） | 11-02 21:05 | 3cfcd49 | 是 |
| `correctness/cjk-hollow-3cfcd49.log` | 镂空断言通过：口/日/田/回/国 内孔数与几何推导一致（无数字输出） | 11-02 20:45 | 3cfcd49 | 是 |
| `correctness/cjk-hollow-preview-3cfcd49.log` | 同组字形的**孔数明细**：口1 日2 田4 回2 国5，附 ink 面积与 ASCII 预览 | 11-02 20:46 | 3cfcd49 | 是 |
| `correctness/cjk-hollow-3cfcd49-release.log` | **release** 下同一组：口1 日2 田4 回2 国5（与 debug 相同） | 11-02 21:05 | 3cfcd49 | 是 |
| `correctness/dual-backend-diff-3cfcd49.log` | 纯 CPU 与 D3D11 逐像素对比：**max_diff=2**、\|d\|≤24 覆盖 100%、完全相等 72.8% | 11-02 20:50 | 3cfcd49 | 是（需 D3D11） |
| `correctness/final-728-passed.log` | **默认套件全量**：123 个 suite、**728 passed / 0 failed** | 11-02 20:06 | 3cfcd49 | 是 |
| `correctness/final-760-include-ignored.log` | **含 `--include-ignored` 全量**：**766 passed / 0 failed**（123 suite）。文件名沿用旧口径 760，「人工注释」在文件头 | 11-02 22:40 | 3cfcd49 | 是 |
| `cross-platform/wsl-x11-tests-2026-11-02.log` | WSL Ubuntu-22.04 + Xvfb `:99` 上 `y901-backend-x11` 全量：**45 passed / 0 failed**（2 单测 + 32 golden + 11 真窗），exit 0 | 11-02 17:46 | 781e5de | 是（`scripts/wsl-x11-count.sh`） |
| `cross-platform/wsl-helm-xvfb-2026-11-02.log` | Helm 在 Xvfb 跑通：**MIT-SHM 已启用 1200×800（3 840 000 B）**、首帧 **65.3 ms**、非背景 98.7%、1039 色；光标 `BadFont` 回退默认箭头 | 11-02 17:42 | 781e5de | 是（`scripts/wsl-helm-check.sh`） |
| `cross-platform/xvfb-server-2026-11-02.log` | Xvfb `:99` 启动时的 unix socket 告警（该环境只有抽象命名空间，不影响 `DISPLAY=:99`），用于交代显示服务器环境 | 11-02 15:42 | 781e5de | 是 |
| `README.md` | 本索引自身（元数据，不是实验数据） | 11-03 07:40 | 3cfcd49 | — |

> 为什么 WSL 三份的 commit 是 `781e5de`：它们跑于 11-02 17:42–17:46，`96f6c5f`（17:52）之前，当时工作区对应的最后一次提交就是 `781e5de`。

## 噪声样本说明（**不可引用**）

`layout-10k-50k-3cfcd49.log` 与 `-run2.log` 是同一 commit 连跑两轮，50k 分别 9 765 / 17 091 µs（差 75%）。
原因是 `cargo test` 默认多线程并行跑用例、共用 CPU。**50k 只能引用 7runs 两份文件**（独立进程、`--test-threads=1`、只跑一个用例）。

同理 `perf-bench-after-3cfcd49.log` 与 `baseline-before-optimization-f1bda2a.log` 里的 50k 数字（15 395 / 25 388 µs）
也是同进程连跑多个 50k 用例的产物，比隔离测量高约 2 倍。

## 口径变化记录

| 指标 | 旧口径 | 新口径 | 原因 |
|---|---|---|---|
| 测试数 | v0.1.1 之前：**722** 默认 / **760** 含 ignored | v0.1.1 之后：**728** / **766** | 新增 6 个测试 |
| 布局 50k | 「6.9~7.8 ms，跑了七次」 | **实测 7 次，中位 6.73 ms**（区间 6.37–8.93） | 旧值实为 11k 线性外推，从未实测 |
| 脏区合并 | 0.66 ms（−220×）/ commit 记 0.64 ms | **0.59 ms（−230×）** | 同会话成对样本；三个值同量级，差异来自跑次 |
| 500 CJK | 18.8 µs/字（未标构建模式） | **16.1 µs/字（release）**；debug 103 µs/字 | 补测并标注 debug/release |
| X11 单帧 | 0.35 ms（无 log） | **删除**，改为「有 `benchmark_shm_vs_putimage_1080p`，待补测」 | 无任何原始输出支撑 |
| 布局 10k | 936 µs | **972 µs**（历史 936 保留括注） | 同口径复测 +3.8%，在跑次波动内 |

## 重跑命令

```powershell
cd F:\Y901
$h = git rev-parse --short HEAD

# 脏区 / 布局 10k / id 长度（同一进程，单线程）
cargo test -p y901-ui --release --test perf_bench -- --ignored --nocapture --test-threads=1

# 50k 隔离跑 7 次（每次独立进程）
1..7 | ForEach-Object { cargo test -p y901-ui --release --test perf_bench -- --ignored --nocapture --test-threads=1 bench_layout_50k_flat }

# 11k 全树重排
cargo test -p y901-ui --release --test layout_bench -- --ignored --nocapture

# 优化前基线（worktree，跑完删除）
git worktree add F:\Y901-baseline f1bda2a
Push-Location F:\Y901-baseline
cargo test -p y901-ui --release --test perf_bench -- --ignored --nocapture --test-threads=1
Pop-Location
git worktree remove F:\Y901-baseline

# 字体：面积恒等式 / 镂空 / 500 CJK（release）
cargo test --release -p y901-backend-win32 --test font_m1     -- --nocapture area_identity
cargo test --release -p y901-backend-win32 --test font_m3_cjk -- --ignored --nocapture debug_print_cjk_frames
cargo test --release -p y901-backend-win32 --test font_m3_cjk -- --ignored --nocapture cold_start_500

# 双后端像素差（需 D3D11）
cargo test -p y901-backend-win32 --test soft_backend -- --ignored --nocapture soft_matches_d3d

# 验收
cargo test --workspace
cargo test --workspace -- --include-ignored

# 跨平台（WSL，需 Xvfb :99）
wsl -d Ubuntu-22.04 -e bash -lc "cd /mnt/f/Y901 && DISPLAY=:99 cargo test -p y901-backend-x11"
wsl -d Ubuntu-22.04 -e bash -lc "cd /mnt/f/Y901 && bash scripts/wsl-helm-check.sh"
```
