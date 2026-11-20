# Y901 v0.2.4 发布说明

> 版本：`0.2.4` · 日期：2026-11-20 · 上一版说明：[`RELEASE_NOTES_v0.2.3.md`](RELEASE_NOTES_v0.2.3.md)

本版**无破坏性变更**（patch）。主题是**把两个长期"记录为限制"的项按实测收口**：
X11 的 HiDPI 来源与运行期缩放，以及"只装 Rust 能不能构建"这个问题本身。

## 1. X11 HiDPI：接上 XSettings，并支持运行期跟随缩放

此前 X11 只看根窗口 `Xft.dpi`（`xrdb` 载入的静态快照）与屏幕毫米估算；GNOME/KDE 改缩放后
应用要重启才生效。本版补齐两件事：

- **XSettings**（桌面设置服务）：`GetSelectionOwner(_XSETTINGS_S0)` → 读该窗口的
  `_XSETTINGS_SETTINGS` 二进制块 → 解析 `Xft/DPI` 与 `Gdk/WindowScalingFactor`。
  字符串条目按 4 字节补齐、**未知类型整块作废**（否则后面所有键静默错位）。
  缩放取 `max(dpi/96, factor)`：桌面只更新其中一个值时也不会把 2 倍读成 1 倍。
  零新依赖，只加了 `GetSelectionOwner`（opcode 23）的编码与解码。
- **运行期跟随缩放**：桌面改缩放后，窗口在 ≤1 秒（`Y901_X11_DPI_POLL_MS`，可设 0）内
  重建字形引擎与软后端、`ConfigureWindow` 到新物理尺寸，并合成一次重排。
  重建失败时保留旧缩放并停用跟随，不会每秒重试刷日志。

**优先级（高 → 低）**：`Y901_X11_SCALE` → `Y901_DPI` → XSettings → `Xft.dpi` → 屏幕毫米（吸附 0.25 档）→ 1.0。
XSettings 与 `Xft.dpi` 都不吸附（它们是用户意图），只有毫米估算吸附。

**为什么用轮询而不是 `PropertyNotify`**：XSettings 的键值在别人的窗口属性上，
订阅变化要给它选 `PropertyChangeMask` 并新增事件解码；漏一个事件就会永久停在旧缩放。
轮询每秒两个小请求，代价远低于这条链路的维护成本。这是个**明示的取舍**，已写进代码注释与 STATUS。

**修掉的一个坑**：缩放后事件队列里可能还排着**旧尺寸** `ConfigureNotify`，照它重排会把
刚建好的新缓冲 resize 回去（字号新、缓冲旧 → 错位）。现在会吞掉一次匹配的旧事件。

## 2. xvfb-run 下连不上 X（Xauthority 主机名）

`xvfb-run -a cargo test` 时测试进程里**没有 `HOSTNAME` 环境变量**，旧的匹配逻辑传空主机名，
于是 Xauthority 的 `FamilyLocal` 条目全部匹配失败 —— 服务端回
`Authorization required, but no authorization protocol specified`（现象像"没发认证"，
实际是"有 cookie 没匹配上"）。现在 `xauth::local_hostname()` 按
`$HOSTNAME` → `/etc/hostname` → 空串 兜底。

## 3. 各向异性图片缩放：补质量测试，不改策略

布局本来就允许 X/Y 独立缩放，但过滤器只有一个各向同性 mip：

- 新增 4 条**解析真值**用例：X 轴强缩小 / Y 轴强缩小各一条（闭式解恒 127.5，实测 127..127）、
  比值 > 2 的有界性、近各向同性对照。
- **变异验证**：把 `draw_scale` 的 `min` 改成 `max`，两条强缩小轴用例立刻红（实测 R 0..255），
  改回即绿。第一版用例其实**抓不住**这个变异（采样点恰好落在明暗交界，走样与正确都给 127.5），
  已改成"6 列目标 + 周期 4 图案"修正。
- 行为未改：`min` 是"牺牲另一轴模糊换强缩小轴不走样"，改 `max` 会让强缩小轴走样；
  没有质量证明前不动。D3D 侧仍未启用专用 anisotropic sampler。

## 4. 零 C 依赖：把"能不能只装 Rust"实测清楚

- 守卫（`crates/y901-shell/tests/no_c_deps.rs`）改为**按 crate 名 + 精确版本**定位源码：
  `version.workspace = true` 也能解析，源码缺失**失败关闭**而不是静默跳过。
- 新增 [`scripts/win-clean-probe.ps1`](../scripts/win-clean-probe.ps1)：一条命令打印
  机器清单 → 干净环境构建 → 产物 PE 导入表 → `+crt-static` 对照，并给出 `BUILD=`/`RUNTIME=` 结论。

**实测结论（与旧文档相反，旧文档已按实测改写）**：

| 平台 | 构建 | 运行 |
|---|---|---|
| Windows | **不需要 VS/SDK**：本机无 Win10/11 SDK、`link.exe`/`cl.exe` 不在 PATH、`vswhere` 空，clean-env + 工具链自带 `rust-lld` 构建 **exit 0**；链接输入里**没有任何导入库**（`raw-dylib`） | 默认产物导入 `VCRUNTIME140.dll` → 需要 VC 运行库；`+crt-static` 产物**零 CRT 导入** |
| GNU/Linux | **必须有系统 `cc`**：去掉后 rust-lld 找不到 `-lc/-lm/-lgcc_s`，补显式 `-L` 也只产出无 `PT_INTERP` 的二进制（运行 `SIGSEGV`） | 需要 libc/CRT；全量验收还要 Xvfb |

对外表述请用"**不编译 C/C++ 源码**"：Windows 可以说"构建不需要 VS/SDK"，但别说"零系统依赖"。

## 5. 验证矩阵（2026-11-20）

### Windows（`scripts/gate.ps1`）

| 检查 | 结果 |
|---|---|
| 默认测试 | **989 passed / 0 failed / 61 ignored**（150 个测试目标） |
| `--include-ignored` | **1050 passed / 0 failed** |
| clippy / fmt / `missing_docs` | 0 warning / clean / 0 |
| 像素哈希 | `0xF7455CCE7781A864` / 400×220 |
| `cargo audit` | 0 漏洞 |
| doc 数字 / test-inventory | 同步（含 8 条变异自检） |

### Linux（Ubuntu 22.04 / WSL2 + Xvfb；scripts/linux-verify.sh）

| 检查 | 结果 |
|---|---|
| 默认测试（`xvfb-run cargo test --workspace`） | **906 passed / 0 failed / 27 ignored**（150 个测试目标） |
| x11 `check --all-targets` / `golden` | exit 0 / 33 passed |
| `xvfb-run real_window` | 16 passed / 0 failed / 2 ignored |
| x11 `clippy --all-targets -D warnings` | exit 0（补上 Windows 门禁看不到的 `#[cfg(unix)]` 代码） |
| stroke / sgr_reverse / ui+widgets / full workspace | 全部 exit 0（脚本汇总 8/8） |

证据：[`linux-full-2026-11-20.log`](evidence/cross-platform/linux-full-2026-11-20.log)、
[`linux-verify-2026-11-20.log`](evidence/cross-platform/linux-verify-2026-11-20.log)、
[`linux-numbers.txt`](evidence/linux-numbers.txt)、[`numbers.txt`](evidence/numbers.txt)。

## 6. 已知限制（未变）

| # | 项 | 说明 |
|---|---|---|
| 1 | **每显示器 DPI** | X11 协议层没有可靠 API；多屏不同缩放只能靠 `Y901_DPI` / `Y901_X11_SCALE` 显式覆盖 |
| 2 | **干净 Windows VM 的正式复核** | 本机已无 SDK 且构建通过，但"连 VS 目录都没有"的全新 VM 复核仍建议做一次（脚本已就绪） |
| 3 | **ClearType gamma / GDI 锐度 / 两后端字形引擎差异** | 设计取舍（v0.2.0 起记录） |
| 4 | **各向异性过滤 / D3D anisotropic sampler** | 已有 4 条解析用例锁住现状；真正做各向异性采样需独立验证 |
| 5 | **多窗口 / macOS** | 桌面独立窗口已实现（`run_many` + 示例），不承诺跨窗共享状态/焦点/标签；WASM 单 page/canvas；macOS 无计划 |

## 7. 升级注意

1. **无破坏性变更**：`0.2.3 → 0.2.4` 可直接升级。
2. X11 上若之前用 `Y901_DPI` / `Y901_X11_SCALE` 强制缩放，行为不变（环境变量优先级最高）。
   没设环境变量的桌面现在会跟随 XSettings —— 如果发现缩放与以前不同，**这是修正**，
   想固定回旧行为请显式设 `Y901_X11_SCALE`。
3. 需要发布"解压即用"的 Windows 包时，请用 `-C target-feature=+crt-static` 构建。
