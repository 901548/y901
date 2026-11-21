# 按依赖顺序把全部 workspace crate 打包校验一遍（**不发布**）。
#
# 为什么需要：crates.io 上的打包要求（所有 path 依赖都要写 version、categories 必须是
# 固定 slug、re-export 只能引用对端**已发布**版本里存在的路径）只有 `cargo package`
# 或 `cargo publish` 才检查。CI 里跑这一条，能在"合并前"而不是"发布时"暴露问题。
#
# 原理：下游 crate 的 `cargo package` 会去 registry 解析内部依赖（首次发布时必然失败），
# 所以用 `--config` 临时给上游打 [patch.crates-io] 指向本地 path，逐个校验打包。
#
# 用法：powershell -NoProfile -File scripts/check-packaging.ps1 [-KeepArtifacts] [-Online]
#   -Online ：不用 `--offline`（CI 上必须加；本机默认离线，依赖已在缓存里时更快）
# 退出码：0 = 全部可打包；1 = 有 crate 打包失败（逐条打印原因）。
param(
    [switch]$KeepArtifacts,
    [switch]$Online
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

# 依赖序：与 scripts/publish-crates.ps1 必须一致。改依赖图时两处一起改。
$order = @(
    'y901-core',
    'y901-ui',
    'y901-render',
    'y901-widgets',
    'y901-runtime',
    'y901-extras',
    'y901-backend-win32',
    'y901-backend-x11',
    'y901-backend-web',
    'y901-shell',
    'y901'
)

$tmp = Join-Path $root 'target\pkg-check'
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
# 本机默认离线（依赖都在缓存里）；`cargo package` 仍需要解析索引，
# 所以没网又没缓存时会失败 —— 那时加 -Online 或先 `cargo fetch`。
if ($Online) { $pkgFlags = @() } else { $pkgFlags = @('--offline') }
$failed = @()
try {
    for ($i = 0; $i -lt $order.Count; $i++) {
        $crate = $order[$i]
        # 给"在它之前"的 crate 打 patch（严格说只需它的传递依赖，超集也能用）。
        #
        # **必须写绝对路径**：`--config` 文件里的相对 path 是按**配置文件所在目录**解析的，
        # 而配置文件我们又放在 `target/pkg-check/` 下 —— 写成 `crates/xxx` 会解析成
        # `target/pkg-check/crates/xxx`（不存在），报错是一句很误导人的
        # `failed to load source for dependency 'y901-core'`（看起来像仓库坏了）。
        $repo = $root -replace '\\', '/'
        $lines = @('[patch.crates-io]')
        for ($j = 0; $j -lt $i; $j++) {
            $lines += "$($order[$j]) = { path = `"$repo/crates/$($order[$j])`" }"
        }
        $cfg = Join-Path $tmp "patch-$i.toml"
        [System.IO.File]::WriteAllText($cfg, (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false)))

        # 显式拼参数数组再 splat：PowerShell 5.1 在"混合字面参数 + @变量"时会把
        # `--offline` 拆坏（实测报 `unexpected argument '-'`）。
        $cargoArgs = @('package', '-p', $crate, '--allow-dirty', '--no-verify', '--config', $cfg) + $pkgFlags
        $out = & cargo @cargoArgs 2>&1 | Out-String
        if ($LASTEXITCODE -eq 0) {
            $m = [regex]::Match($out, 'Packaged (\d+) files, ([\d.]+)KiB \(([\d.]+)KiB compressed\)')
            Write-Host ("[pkg] ok   {0,-20} files={1,-4} {2}KiB" -f $crate, $m.Groups[1].Value, $m.Groups[3].Value)
        }
        else {
            Write-Host "[pkg] FAIL $crate"
            ($out -split "`n" | Where-Object { $_ -match 'error|Caused|no matching|categories|version requirement' } |
                Select-Object -First 6) | ForEach-Object { Write-Host "       $($_.Trim())" }
            $failed += $crate
        }
    }
}
finally {
    if (-not $KeepArtifacts) { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
    Pop-Location
}

Write-Host ''
if ($failed.Count -gt 0) {
    Write-Host "[pkg] $($failed.Count)/$($order.Count) 个 crate 打包失败：$($failed -join ', ')"
    exit 1
}
Write-Host "[pkg] 全部 $($order.Count) 个 crate 均可打包"
exit 0
