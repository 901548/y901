# 按依赖顺序把 workspace 里的 y901-* crate 发到 crates.io。
#
# 为什么必须按顺序：cargo 在打包/发布时会去 registry 解析**包括 path 依赖在内**的
# 全部依赖（连 dev-dependencies 也是）。所以"被依赖的先发"是硬约束；发 ui 之前
# core 必须已经在 crates.io 上。乱序的典型报错是：
#   no matching package named `y901-core` found / location searched: crates.io index
#
# 用法：
#   powershell -NoProfile -File scripts/publish-crates.ps1 -DryRun      # 只演练
#   powershell -NoProfile -File scripts/publish-crates.ps1              # 真发
#   powershell -NoProfile -File scripts/publish-crates.ps1 -Only y901-core
#   powershell -NoProfile -File scripts/publish-crates.ps1 -Proxy http://127.0.0.1:7897
#
# 前置：先 `cargo login`（token 写进 ~/.cargo/credentials.toml），或设 CARGO_REGISTRY_TOKEN。
param(
    [switch]$DryRun,
    [switch]$SkipChecks,
    [switch]$SkipSleep,
    [int]$SleepSeconds = 45,
    [string]$Only = '',
    [string]$Proxy = ''
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

# 依赖序：先发被依赖的。改动依赖图时**同步改这里**，否则一定在首次发布时踩坑。
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
    # **单一入口**：依赖上面全部 crate，必须最后发（用户只需要依赖它）。
    'y901'
)

function Get-Version {
    $t = [System.IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'))
    $m = [regex]::Match($t, '(?ms)\[workspace\.package\](.*?)(?:^\[|\z)')
    return [regex]::Match($m.Groups[1].Value, '(?m)^\s*version\s*=\s*"([^"]+)"').Groups[1].Value
}

function Get-PublishedVersion([string]$crate) {
    $ua = 'y901-publish-script (github.com/901548/y901)'
    try {
        $resp = Invoke-RestMethod -Uri "https://crates.io/api/v1/crates/$crate" -Headers @{ 'User-Agent' = $ua } -TimeoutSec 20
        return $resp.crate.max_version
    }
    catch {
        # 404 = 还没发布过（crates.io 对不存在的 crate 返回 404）
        return ''
    }
}

$version = Get-Version
Write-Host "[publish] workspace version = $version；模式 = $(if ($DryRun) { 'DRY-RUN' } else { '真发' })"
if ($Proxy) {
    $env:HTTPS_PROXY = $Proxy
    $env:HTTP_PROXY = $Proxy
    Write-Host "[publish] 代理 = $Proxy"
}

if (-not $SkipChecks -and -not $DryRun) {
    # 发布内容必须与提交一致：脏工作区会被 cargo 自己拒绝，这里先给个更好懂的提示。
    $dirty = git status --porcelain
    if ($dirty) {
        Write-Host "[publish] FAIL: 工作区不干净，先提交（cargo publish 也会拒绝）："
        Write-Host $dirty
        exit 1
    }
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'check-version-consistency.ps1')
    if ($LASTEXITCODE -ne 0) { Write-Host '[publish] FAIL: 版本不一致'; exit 1 }
}

$failed = $false
foreach ($c in $order) {
    if ($Only -and $c -ne $Only) { continue }
    $published = Get-PublishedVersion $c
    if ($published -eq $version) {
        Write-Host "[publish] 跳过 $c：$version 已在 crates.io"
        continue
    }
    Write-Host ""
    Write-Host "=== $c v$version（当前已发布: $(if ($published) { $published } else { '<无>' })）==="
    $args = @('publish', '-p', $c)
    if ($DryRun) { $args += '--dry-run' }
    $out = & cargo @args 2>&1 | Out-String
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        Write-Host $out
        if ($out -match 'no matching package named') {
            Write-Host "[publish] 失败原因：上游 crate 还没发（或索引未生效）。按顺序发上游，再等一会儿重试。"
        }
        if ($out -match 'already uploaded|already exists') {
            Write-Host "[publish] 失败原因：这个版本号已被占用 —— 升版本，不要重试同一版本。"
        }
        $failed = $true
        break
    }
    Write-Host ($out -split "`n" | Where-Object { $_ -match 'Uploaded|Uploading|Packaged|warning' } | Select-Object -Last 4 | Out-String)
    if (-not $DryRun -and -not $SkipSleep -and $c -ne $order[-1]) {
        Write-Host "[publish] 等 $SleepSeconds 秒让索引生效…"
        Start-Sleep -Seconds $SleepSeconds
    }
}

Write-Host ""
if ($failed) { Write-Host 'PUBLISH: FAILED（见上）'; exit 1 }
if ($DryRun) {
    Write-Host 'PUBLISH: DRY-RUN OK（真发请去掉 -DryRun；首次发布时下游 crate 的 dry-run 会因上游未发布而失败，属正常）'
} else {
    Write-Host "PUBLISH: OK（$version）"
}
exit 0
