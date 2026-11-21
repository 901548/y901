# 发布到 crates.io（操作手册）

> 面向维护者。首次执行见 §2；后续发布只需 §1 + §3。

## 0. 前置条件

| 条件 | 检查方式 |
|---|---|
| 已登录 crates.io | `cargo login` 一次（token 存在 `~/.cargo/credentials.toml`），或设 `CARGO_REGISTRY_TOKEN` |
| 工作区干净 | `git status --short` 无输出（`cargo publish` 默认要求干净工作树） |
| 版本一致 | `powershell -File scripts/check-version-consistency.ps1` → `ok` |
| 门禁全绿 | `powershell -File scripts/gate.ps1` → `GATE OK` |
| Linux 侧绿 | `bash scripts/linux-verify.sh` → `LINUX VERIFY: OK` |
| 已打 tag | `git tag -a vX.Y.Z`（发布内容必须与 tag 一致） |
| crate 名可用 | 新 crate 先查 `https://crates.io/api/v1/crates/<name>`（404 = 可用；**要带 User-Agent 头**，否则 403 会让人误判为"已存在"） |

> 网络提示：本机直连 `github.com` 会被重置，但 pip 代理端口 `127.0.0.1:7897` 可用时，
> git 要用一次性参数：`git -c http.proxy=http://127.0.0.1:7897 push origin main`。
> 别用 `Test-NetConnection 127.0.0.1:7897` 判代理死活（本机实测它返回 False，但端口其实在听）；
> 用 `netstat -ano | findstr :7897`。

## 1. 发布前自检（每次都做）

**一条命令**（推荐）：[`scripts/check-release-readiness.ps1`](../scripts/check-release-readiness.ps1)
把下面这些检查串起来，并**明确分出"能自动检查的"与"只能由人/凭据决定的"**：

```powershell
powershell -NoProfile -File scripts/check-release-readiness.ps1
# 快速版（跳过全量门禁与打包）：
powershell -NoProfile -File scripts/check-release-readiness.ps1 -SkipGate -SkipPackaging
```

它**不发布任何东西**、不打 tag、不推送；退出码 0 = 自动项全绿，
`BLOCKED` 项会原样列出（凭据 / tag / 工作区 / 真机验收），不会假装通过。

手工等价步骤：

```powershell
cargo fmt --all -- --check
powershell -NoProfile -File scripts/gate.ps1          # 必须 GATE OK
powershell -NoProfile -File scripts/check-version-consistency.ps1
powershell -NoProfile -File scripts/check-semver-bump.ps1   # 破坏性提交必须升 minor
cargo deny check                                      # deny.toml：公告/许可/来源/重复版本
cargo semver-checks check-release --workspace --baseline-rev (git describe --tags --abbrev=0 --match 'v[0-9]*')

# 叶子 crate 的打包冒烟（会被依赖的都要能打包）
cargo package -p y901-core --allow-dirty --offline
```

### 1b. 首发前把**全部 11 个 crate** 都打包验证一遍（不发布）

首次发布时下游 crate 的 `cargo package` 必然报
`no matching package named 'y901-xxx' found` —— 因为 cargo 会去 registry 解析依赖。
但这不代表"打包有问题"。想在不发布的前提下验证全部 crate 都能打包，用
`--config` 临时给上游打 `[patch.crates-io]`：

```powershell
$order = 'y901-core','y901-ui','y901-render','y901-widgets','y901-runtime',
         'y901-extras','y901-backend-win32','y901-backend-x11','y901-backend-web',
         'y901-shell','y901'   # y901 是单一入口，必须最后发
for ($i = 0; $i -lt $order.Count; $i++) {
    $lines = @('[patch.crates-io]')
    for ($j = 0; $j -lt $i; $j++) { $lines += "$($order[$j]) = { path = `"crates/$($order[$j])`" }" }
    $cfg = "target\pkgcfg-$i.toml"
    [System.IO.File]::WriteAllText((Join-Path $PWD $cfg), (($lines -join "`n") + "`n"))
    cargo package -p $order[$i] --allow-dirty --offline --no-verify --config $cfg
}
```

2026-11-20 实测：当时的 **10 个 crate** 全部打包成功（7–78 个文件，18–179 KiB 压缩后）。截至 0.2.6，workspace 已增至 **11 个 crate**；新增 crate 的打包验证记录需另行补充。
这一步抓到过一个真问题：`y901-shell` 的 wasm 依赖 `y901-backend-web` **没写
`version`**，导致 `cargo publish -p y901-shell` 会直接失败：

```
all dependencies must have a version requirement specified when packaging.
dependency `y901-backend-web` does not specify a version
```

> **categories 不是自由文本**：crates.io 只接受固定列表（`GET https://crates.io/api/v1/categories`）。
> 写错会得到 `400 Bad Request: The following category slugs are not currently supported`
> —— 这个错误只在**上传阶段**出现，`cargo package` 完全不拦。实测合法且相关的：
> `gui` / `graphics` / `os` / `wasm` / `command-line-utilities`。
> `keywords` 才是自由的（≤5 个，字母数字与 `-_`）。

> 规则：**任何 path 依赖都必须同时写 `version`**，包括 `target.'cfg(...)'` 段里的。
> 打包时 path 会被剥掉、只留版本约束，所以缺 version 的依赖根本没法发布。
> 已修（v0.2.5）。

> 说明：**下游 crate 的 `cargo package` 在首次发布时必然失败**（报
> `no matching package named 'y901-xxx' found`），因为 cargo 会按"已经发布"去 registry
> 解析 path 依赖。这不是打包错误，按 §2 的顺序发布即可。

## 2. 首次发布顺序（**被依赖的先发**）

> **y901 是单一入口，必须最后发**（它依赖其余全部 crate）。用户侧只需要 `cargo add y901`，
> 其它 10 个 crate 是内部实现；截至 0.2.6 共 11 个 crate，仍可单独依赖（需要裁剪依赖时才这么做）。

一条命令（推荐）：[`scripts/publish-crates.ps1`](../scripts/publish-crates.ps1) 内置了
依赖顺序、已发布版本跳过、索引等待与失败解释：

```powershell
# 先登录一次（token 落到 ~/.cargo/credentials.toml）
cargo login

# 演练（只打包 + 校验，不上传）
powershell -NoProfile -File scripts/publish-crates.ps1 -DryRun

# 真发（每发一个等 45s 让索引生效；-Proxy 用于需要代理的网络）
powershell -NoProfile -File scripts/publish-crates.ps1 -Proxy http://127.0.0.1:7897
```

手工等价流程（脚本挂了时照这个来）：

```powershell
# 顺序固定：core → ui → render → widgets → runtime → extras → 三个后端 → shell
$order = @(
    'y901-core',          # 无内部依赖
    'y901-ui',            # -> core
    'y901-render',        # -> ui, core
    'y901-widgets',       # -> core, ui, render
    'y901-runtime',       # -> ui, widgets
    'y901-extras',        # -> core, ui, render, widgets
    'y901-backend-win32', # -> core, ui, render, runtime, widgets
    'y901-backend-x11',   # -> ui, render, runtime
    'y901-backend-web',   # -> core, ui, render, runtime
    'y901-shell'          # -> 全部
)
foreach ($c in $order) {
    Write-Host "=== $c ==="
    cargo publish -p $c --dry-run   # 首次会失败：上游还没发；先发上游即可
    if ($LASTEXITCODE -ne 0) { Write-Host "  （dry-run 失败正常：上游尚未发布）" }
    # 真发（确认 dry-run 或确认上游已发布后）：
    # cargo publish -p $c
    Start-Sleep -Seconds 45                   # 等索引生效，否则下一个会报 no matching package
}
```

要点：

- **每发完一个等 30–60 秒**。crates.io 索引有传播延迟，急着重试会得到
  `no matching package named ... found`，看起来像"包不存在"。
- 顺序不能乱：`y901-ui` 必须在 `y901-render` 之前（render 依赖 ui）。反过来就是
  §RELEASE_NOTES_v0.2.5 里说的死锁 —— 那正是本版把 ui→render 的 dev-dep 拆掉的原因。
- `y901-shell` 是应用壳。**要支持 `cargo install y901-shell` 就必须发它**；
  不想发就在它的 `Cargo.toml` 里加 `publish = false`（其余 crate 不受影响）。

## 3. 发布后检查

| 检查 | 做法 |
|---|---|
| crate 页出现且 README 正常 | 打开 `https://crates.io/crates/<name>` |
| docs.rs 构建成功 | `https://docs.rs/<name>/<version>/`；`backend-win32` / `backend-web` 走 `default-target`，Linux 默认目标不应再出现空文档 |
| 下游能拉 | `powershell -NoProfile -File scripts/check-consumer.ps1 -Version <新版本>`（从 crates.io 拉包编译，不用 path） |
| 签名级兼容 | CI 的 `semver-checks` job（`cargo-semver-checks --baseline-rev <上一 tag>`）应为绿；发布后新 tag 成为下一轮基线 |
| 版本号已占用 | 发布后**不能重发同一版本**；修复必须升版（0.2.6…） |

## 4. 出错怎么办

| 症状 | 处理 |
|---|---|
| `no matching package named 'y901-xxx' found` | 上游还没发或索引未生效：确认上游 crate 页存在，等 1 分钟重试 |
| `crate version already uploaded` | 这个版本号已用掉，**升版本**；不要试图重发 |
| 发布后发现严重问题 | `cargo yank --version 0.2.5 y901-core`（yank 只是不再被新依赖选中，已依赖的项目不受影响），随后发修复版 |
| docs.rs 构建失败 | 看 `https://docs.rs/crate/<name>/<version>/builds`；平台专属 crate 检查 `[package.metadata.docs.rs] default-target` |

## 5. 与 crates.io 无关、但发货前要确认的事

- Windows 绿色版：用 `-C target-feature=+crt-static` 构建，否则运行时要 VC 运行库
  （见 [`ZERO_C_PROOF.md`](ZERO_C_PROOF.md) §3b）。
- 干净 Windows VM 复核：`powershell -File scripts/win-clean-probe.ps1`（仍建议在
  没有 VS 目录的全新 VM 上跑一次）。
