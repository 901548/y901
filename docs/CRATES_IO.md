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

```powershell
cargo fmt --all -- --check
powershell -NoProfile -File scripts/gate.ps1          # 必须 GATE OK
powershell -NoProfile -File scripts/check-version-consistency.ps1

# 叶子 crate 的打包冒烟（会被依赖的都要能打包）
cargo package -p y901-core --allow-dirty --offline
```

> 说明：**下游 crate 的 `cargo package` 在首次发布时必然失败**（报
> `no matching package named 'y901-xxx' found`），因为 cargo 会按"已经发布"去 registry
> 解析 path 依赖。这不是打包错误，按 §2 的顺序发布即可。

## 2. 首次发布顺序（**被依赖的先发**）

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
    cargo publish -p $c --dry-run --offline   # 首次会失败：上游还没发；先发上游即可
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
| docs.rs 构建成功 | `https://docs.rs/<name>/0.2.5/`；`backend-win32` / `backend-web` 走 `default-target`，Linux 默认目标不应再出现空文档 |
| 下游能拉 | 空目录里 `cargo add y901-core && cargo build` |
| 版本号已占用 | 发布后**不能重发同一版本**；修复必须升版（0.2.6…） |

## 4. 出错怎么办

| 症状 | 处理 |
|---|---|
| `no matching package named 'y901-xxx' found` | 上游还没发或索引未生效：确认上游 crate 页存在，等 1 分钟重试 |
| `crate version already uploaded` | 这个版本号已用掉，**升版本**；不要试图重发 |
| 发布后发现严重问题 | `cargo yank --version 0.2.5 y901-core`（yank 只是不再被新依赖选中，已依赖的项目不受影响），随后发修复版 |
| docs.rs 构建失败 | 看 `https://docs.rs/crate/<name>/0.2.5/builds`；平台专属 crate 检查 `[package.metadata.docs.rs] default-target` |

## 5. 与 crates.io 无关、但发货前要确认的事

- Windows 绿色版：用 `-C target-feature=+crt-static` 构建，否则运行时要 VC 运行库
  （见 [`ZERO_C_PROOF.md`](ZERO_C_PROOF.md) §3b）。
- 干净 Windows VM 复核：`powershell -File scripts/win-clean-probe.ps1`（仍建议在
  没有 VS 目录的全新 VM 上跑一次）。
