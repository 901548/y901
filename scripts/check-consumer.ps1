# 下游冒烟：把“已发布到 crates.io 的 y901”当成真实用户来编译。
#
# 为什么需要它：仓库内测试用的是 path 依赖，本地全绿不代表发布版可用。
# 历史上就出现过“workspace 能编、已发布 crate 的 prelude 引用不存在路径”的问题；
# 只有从 registry 拉包才能复现。
#
# 用法：
#   powershell -NoProfile -File scripts/check-consumer.ps1                 # 测 crates.io 最新已发布版
#   powershell -NoProfile -File scripts/check-consumer.ps1 -Version 0.2.5  # 指定版本
#   powershell -NoProfile -File scripts/check-consumer.ps1 -Offline        # 用本地 cargo 缓存
param(
    [string]$Version = '',
    [switch]$Offline,
    [string]$WorkDir = ''
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$ua = 'y901-consumer-smoke (github.com/901548/y901)'

function Fail([string]$msg) {
    Write-Host "[consumer] FAIL: $msg"
    exit 1
}

if (-not $Version) {
    try {
        $resp = Invoke-RestMethod -Uri 'https://crates.io/api/v1/crates/y901' -Headers @{ 'User-Agent' = $ua } -TimeoutSec 20
        $Version = $resp.crate.max_version
    }
    catch {
        Fail "查询 crates.io 最新版本失败（可用 -Version 指定）：$($_.Exception.Message)"
    }
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    Fail "版本号不是稳定 SemVer：$Version"
}
Write-Host "[consumer] 目标版本：y901 = $Version（registry，非 path）"

if (-not $WorkDir) {
    # **必须在仓库之外**：cargo 会把仓库目录内新建的 package 当成 workspace 成员
    # （实测 `cargo init target/consumer-smoke` 会往根 Cargo.toml 的 members 里追加一项，
    # 并让主 Cargo.lock 混入 registry 版本的 y901-*）。用系统临时目录最干净。
    $WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) 'y901-consumer-smoke'
}
if (Test-Path $WorkDir) { Remove-Item $WorkDir -Recurse -Force }
New-Item -ItemType Directory -Path $WorkDir | Out-Null

Push-Location $WorkDir
try {
    # 手写 Cargo.toml/src，避免依赖 cargo init 的模板差异（也避免 cargo init 需要空目录的约束）。
    $cargoToml = @"
[package]
name = "y901_consumer"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
y901 = "=$Version"
"@
    [System.IO.File]::WriteAllText((Join-Path $WorkDir 'Cargo.toml'), $cargoToml, [System.Text.UTF8Encoding]::new($false))
    New-Item -ItemType Directory -Path (Join-Path $WorkDir 'src') -Force | Out-Null
    $mainRs = @"
use y901::prelude::*;

fn main() -> Result<(), String> {
    // 只使用公开入口：不碰子 crate、不碰 #[cfg(target_os)]。
    let _options = Options::default();
    let _size = Size::new(1.0, 1.0);
    let runtime = y901::create_runtime();
    let _ = runtime;
    Ok(())
}
"@
    [System.IO.File]::WriteAllText((Join-Path $WorkDir 'src\main.rs'), $mainRs, [System.Text.UTF8Encoding]::new($false))

    $args = @('check', '--quiet')
    if ($Offline) { $args += '--offline' }
    & cargo @args 2>&1 | Out-Host
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        Fail "已发布版本 y901 $Version 无法被下游工程编译（退出码 $code）。看上面的 cargo 输出。"
    }
    Write-Host "[consumer] ok: 下游工程用 crates.io 上的 y901 $Version 编译通过"
}
finally {
    Pop-Location
}
exit 0