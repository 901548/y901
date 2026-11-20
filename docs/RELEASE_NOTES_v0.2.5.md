# Y901 v0.2.5 发布说明

> 版本：`0.2.5` · 日期：2026-11-20 · 上一版说明：[`RELEASE_NOTES_v0.2.4.md`](RELEASE_NOTES_v0.2.4.md)

本版是**首次 crates.io 发布**的准备版：不改功能行为，只做"能被 crates.io / docs.rs
正确接收"所需的结构与元数据调整。**无破坏性变更**（patch）。

## 1. 消除 `ui ↔ render` 的 dev-dependency 环（发布硬阻塞）

`y901-render` 正常依赖 `y901-ui`，而 `y901-ui` 曾对 `y901-render` 有 **dev-dependency**
（两条需要真像素回读的用例）。cargo 打包/发布时会**连 dev-dependencies 一起解析**，
于是首次发布形成死锁：发 `ui` 要求 `render@0.2.5` 已在 crates.io，发 `render` 又要求
`ui@0.2.5` 已在 crates.io —— 任何顺序都缺一环。

处理：把这两条用例迁到 `y901-render` 侧（那里两个 crate 都可用），并删掉 `ui` 的 dev-dep。

| 用例 | 原位置 | 现位置 |
|---|---|---|
| Grid 跨格裁剪 / 自动流（4 条） | `crates/y901-ui/tests/grid_span.rs` | `crates/y901-render/tests/ui_grid_span.rs` |
| hover 快照注入 `RenderCtx`（1 条） | `crates/y901-ui/tests/hover.rs` | `crates/y901-render/tests/ui_hover_snapshot.rs` |

用例数不变（Windows 989 passed / 0 failed），只是测试二进制从 135 个变成 137 个。
`y901-ui/Cargo.toml` 里留了注释说明"为什么这里不能有 render 的 dev-dep"，
`FRAMEWORK_GUIDE` 里指向旧路径的引用也已更正。

## 2. crates.io / docs.rs 元数据

| 项 | 处理 | 为什么 |
|---|---|---|
| `rust-version = "1.92"` | `[workspace.package]` 定义，各 crate 继承 | 下游与 docs.rs 需要知道 MSRV；门禁第 1 步本来就卡这个值 |
| `readme = "../../README.md"` | 每个 crate | 否则 docs.rs 页面是空的；`cargo package` 已确认 README 会进 tarball |
| `keywords` / `categories` | 每个 crate 按职责填 | crates.io 搜索与分类 |
| `[package.metadata.docs.rs] default-target` | `backend-win32` → `x86_64-pc-windows-msvc`；`backend-web` → `wasm32-unknown-unknown` | 这两个 crate 是平台专属（`#![cfg(windows)]` / wasm32），docs.rs 默认在 Linux 上构建只会得到**空文档** |

体积核对：每个 crate 打包后 53KB–690KB（crates.io 上限 10MB）；仓库根那个 19MB 的
`www/fonts/default.ttf` 不属于任何 crate，不会被带上。

## 3. 发布顺序（本次实际使用的顺序）

```
y901-core → y901-ui → y901-render → y901-widgets → y901-runtime
          → y901-extras → y901-backend-win32 / y901-backend-x11 / y901-backend-web
          → y901-shell
```

规则：**被依赖的先发**；每发完一个等 30–60 秒让索引生效，再发下一个
（首次发布时下游 crate 的 `cargo publish` 会去 registry 找上游，索引没生效就会报
"no matching package"）。逐条命令见 [`docs/CRATES_IO.md`](CRATES_IO.md)。

## 4. X11：缩放后重建 MIT-SHM 段（补 v0.2.4 的取舍）

v0.2.4 引入运行期跟随缩放时，为了简单把"尺寸变了 → 停用 SHM"作为取舍：画面正确，
但**改一次缩放就永久退化成每帧整窗 `PutImage`**（丢掉共享内存快路径）。

本版补上重建：先发 `ShmDetach` 让服务端放开旧段（只 drop 本地段会让它在服务端仍附着时
"标记删除"，直到连接关闭才回收），再用**轮转的新 XID**（`0x10..0x18`，避开窗口 `0x01`、
GC `0x02`、光标字体 `0x20`）按新尺寸建段；任何一步失败都静默回退 `PutImage`，不影响正确性。

回归断言（`real_window::runtime_scale_change_rebuilds_soft_backend`）：
Xvfb 上 240×160 → 1.5 倍重建为 360×240 后，`shm_active()` 必须仍为 true，且按新尺寸
`present` 成功。实测输出：

```
[y901] MIT-SHM 已启用（240x160，共享内存 153600字节）
[y901] X11 缩放变化：1.00 → 1.50，客户区 → 360x240
[x11] 缩放后 SHM 是否在位：true
```

## 5. 验证矩阵（2026-11-20）

| 检查 | 结果 |
|---|---|
| Windows 门禁 | **989 passed / 0 failed / 61 ignored**（151 个测试目标），clippy 0 警告，像素哈希 `0xF7455CCE7781A864` 不变，`cargo audit` 0 漏洞 |
| Linux（Ubuntu 22.04 + Xvfb） | **906 passed / 0 failed / 27 ignored**，`linux-verify.sh` 八步全 0 |
| 版本一致性 | workspace / 10 个 crate / Cargo.lock 全为 `0.2.5` |
| 打包冒烟 | `cargo package -p y901-core` 成功（7 文件，含 README） |

## 6. 已知限制

沿用 v0.2.4 的清单（每显示器 DPI、各向异性过滤、WASM 单 canvas、多窗口跨窗语义、
macOS、present 设备丢失 / 双窗 IME 未实测），本版未改变。
另外：**API 仍是 0.x 语义** —— 发布后若需破坏性变更，会升 minor（0.3.0）并在
release notes 里给迁移说明。

## 7. 升级注意

1. 无破坏性变更：`0.2.4 → 0.2.5` 直接升级。
2. 下游若自己在 `y901-ui` 的 tests 里依赖 `y901-render`：那是你自己的 dev-dep，
   不受本仓调整影响。
3. 首次发布之后，`0.2.5` 这个版本号即被占用，后续修复请升 `0.2.6`。
