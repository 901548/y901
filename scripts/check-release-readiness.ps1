# 发布就绪检查（0.x 版本的"发布前 checklist"）。
#
# 为什么需要它：发布前要跑的检查散在 6 个脚本 + 3 个 CI job 里，靠人记一定会漏
# （历史上就漏过 semver、漏过 packaging）。这里把它们串成一条命令，并且**明确区分**
# "能自动检查的"与"只能由人/凭据决定的"——后者必须原样报出来，不能假装通过。
#
# 用法：
#   powershell -NoProfile -File scripts/check-release-readiness.ps1
#   powershell -NoProfile -File scripts/check-release-readiness.ps1 -SkipGate   # 快速跑（跳过全量门禁）
#   powershell -NoProfile -File scripts/check-release-readiness.ps1 -Online     # CI/无本地缓存
#
# 退出码：0 = 自动项全绿（人工项仍会列出）；1 = 有自动项失败。
# **它不会发布任何东西**，也不打 tag、不推送。
param(
    [switch]$SkipGate,
    [switch]$SkipPackaging,
    [switch]$Online,
    [string]$BaselineTag = ''
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

$results = @()
function Record([string]$name, [string]$status, [string]$note = '') {
    $script:results += [pscustomobject]@{ Name = $name; Status = $status; Note = $note }
    $color = switch ($status) {
        'PASS' { 'Green' }
        'FAIL' { 'Red' }
        'BLOCKED' { 'Yellow' }
        'SKIP' { 'DarkGray' }
        default { 'Gray' }
    }
    Write-Host ("[{0}] {1}{2}" -f $status, $name, $(if ($note) { " — $note" } else { '' })) -ForegroundColor $color
}

function Get-WorkspaceVersion {
    $text = [System.IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'))
    $m = [regex]::Match($text, '(?ms)\[workspace\.package\](.*?)(?:^\[|\z)')
    [regex]::Match($m.Groups[1].Value, '(?m)^\s*version\s*=\s*"([^"]+)"').Groups[1].Value
}

$version = Get-WorkspaceVersion
Write-Host "=== Y901 发布就绪检查（workspace $version）==="
Write-Host ""

# ---------------------------------------------------------------- 能自动检查的
$onlineFlag = if ($Online) { @('-Online') } else { @() }

Write-Host '--- 格式与元数据 ---'
& cargo fmt --all -- --check 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) { Record 'cargo fmt --check' 'PASS' } else { Record 'cargo fmt --check' 'FAIL' '先跑 cargo fmt --all' }

& powershell -NoProfile -File (Join-Path $PSScriptRoot 'check-version-consistency.ps1') 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) { Record '版本一致性（workspace/crates/lock）' 'PASS' } else { Record '版本一致性' 'FAIL' '见脚本输出' }

& powershell -NoProfile -File (Join-Path $PSScriptRoot 'check-semver-bump.ps1') 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) { Record 'semver 纪律（! 提交必须升 minor/major）' 'PASS' } else { Record 'semver 纪律' 'FAIL' '见脚本输出' }

Write-Host ''
Write-Host '--- 全量门禁 ---'
if ($SkipGate) {
    Record 'scripts/gate.ps1' 'SKIP' '-SkipGate（发布前必须真跑一次）'
} else {
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'gate.ps1') 2>&1 | Out-Null
    $gateCode = $LASTEXITCODE
    if ($gateCode -eq 0) { Record 'scripts/gate.ps1' 'PASS' }
    elseif ($gateCode -eq 2) { Record 'scripts/gate.ps1' 'BLOCKED' '有 SKIP（无 GPU 等环境限制）：通过但不可发布' }
    else { Record 'scripts/gate.ps1' 'FAIL' "退出码 $gateCode" }
}

Write-Host ''
Write-Host '--- 打包与依赖策略 ---'
if ($SkipPackaging) {
    Record 'scripts/check-packaging.ps1' 'SKIP' '-SkipPackaging'
} else {
    & powershell -NoProfile -File (Join-Path $PSScriptRoot 'check-packaging.ps1') @onlineFlag 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) { Record 'scripts/check-packaging.ps1（12 个 crate）' 'PASS' }
    else { Record 'scripts/check-packaging.ps1' 'FAIL' '见脚本输出' }
}

if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
    & cargo deny check 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) { Record 'cargo deny check' 'PASS' }
    else { Record 'cargo deny check' 'FAIL' '见 cargo deny 输出' }
} else {
    Record 'cargo deny check' 'SKIP' 'cargo-deny 未安装：cargo install cargo-deny --version 0.21.0'
}

if (Get-Command cargo-semver-checks -ErrorAction SilentlyContinue) {
    if (-not $BaselineTag) {
        $BaselineTag = (git describe --tags --abbrev=0 --match 'v[0-9]*' 2>$null)
    }
    if ($BaselineTag) {
        # **排除新 crate**：它在上个 tag 里不存在，无法比较（实测会报 not found in baseline）。
        & cargo semver-checks check-release --workspace --exclude y901-a11y --baseline-rev $BaselineTag 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) { Record "cargo semver-checks（基线 $BaselineTag）" 'PASS' }
        else { Record 'cargo semver-checks' 'FAIL' 'API 与基线不兼容，确认版本号是否已升 minor/major' }
    } else {
        Record 'cargo semver-checks' 'SKIP' '找不到基线 tag'
    }
} else {
    Record 'cargo semver-checks' 'SKIP' '未安装：cargo install cargo-semver-checks --version 0.49.0'
}

# ---------------------------------------------------------------- 只能人工/凭据决定的
Write-Host ''
Write-Host '--- 需要人或凭据决定（脚本无法代替）---'

$credFile = Join-Path $HOME '.cargo\credentials.toml'
if ((Test-Path $credFile) -or $env:CARGO_REGISTRY_TOKEN) {
    Record 'crates.io 凭据' 'PASS' '已找到凭据（脚本仍不会自动发布）'
} else {
    Record 'crates.io 凭据' 'BLOCKED' '无 ~/.cargo/credentials.toml / CARGO_REGISTRY_TOKEN：需先 cargo login'
}

$dirty = git status --porcelain 2>$null
if ($dirty) {
    Record '工作区干净（发布要求）' 'BLOCKED' '有未提交改动；cargo publish 会拒绝'
} elseif ($LASTEXITCODE -eq 128) {
    Record '工作区干净（发布要求）' 'SKIP' '不是 git 仓库'
} else {
    Record '工作区干净（发布要求）' 'PASS'
}

$tag = "v$version"
$tagExists = (git tag --list $tag 2>$null) -eq $tag
if ($tagExists) { Record "tag $tag" 'PASS' '已存在（发布内容必须与 tag 一致）' }
else { Record "tag $tag" 'BLOCKED' "未创建：git tag -a $tag（需用户授权）" }

$published = $null
try {
    $ua = @{ 'User-Agent' = 'y901-release-readiness (github.com/901548/y901)' }
    $resp = Invoke-RestMethod -Uri 'https://crates.io/api/v1/crates/y901' -Headers $ua -TimeoutSec 15
    $published = $resp.crate.max_version
} catch {
    $status = 0
    if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
    if ($status -eq 404) { $published = '<none>' }
}
if ($null -ne $published) {
    if ($published -eq $version) { Record 'crates.io 上是否已有该版本' 'PASS' "$version 已发布" }
    else { Record 'crates.io 上是否已有该版本' 'BLOCKED' "当前已发布：$published；$version 尚未上传（需用户授权）" }
} else {
    Record 'crates.io 上是否已有该版本' 'SKIP' '查不到（无网络或被限流）'
}

Record '真机读屏器验收（讲述人 / Orca / NVDA）' 'BLOCKED' '无障碍桥已能推树，但没有真人验证过读屏效果'
# Linux 验收：Windows 跑不了，但可以查**证据文件是否新鲜**（7 天内 = 可接受）。
$linuxLogs = @(Get-ChildItem (Join-Path $root 'docs\evidence\cross-platform') -Filter '*.log' -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending)
if ($linuxLogs.Count -gt 0) {
    $newest = $linuxLogs[0]
    $ageDays = ((Get-Date) - $newest.LastWriteTime).TotalDays
    if ($ageDays -le 7) {
        Record 'Linux 验收证据（7 天内）' 'PASS' ("{0}（{1:N1} 天前）" -f $newest.Name, $ageDays)
    } else {
        Record 'Linux 验收证据（7 天内）' 'BLOCKED' ("最新是 {0}（{1:N1} 天前）：发布前在 Linux 上重跑 scripts/linux-verify.sh" -f $newest.Name, $ageDays)
    }
} else {
    Record 'Linux 验收证据（7 天内）' 'BLOCKED' '没有 docs/evidence/cross-platform/*.log：必须在 Linux 上跑一次 linux-verify.sh'
}

# ---------------------------------------------------------------- 汇总
Write-Host ''
Write-Host '=== 汇总 ==='
foreach ($group in ($results | Group-Object Status | Sort-Object Name)) {
    $names = ($group.Group | ForEach-Object { $_.Name }) -join "; "
    Write-Host ("{0}: {1}" -f $group.Name, $names)
}
$fails = @($results | Where-Object { $_.Status -eq 'FAIL' })
$blocked = @($results | Where-Object { $_.Status -eq 'BLOCKED' })
Write-Host ''
if ($fails.Count -gt 0) {
    Write-Host "RELEASE READINESS: FAILED（$($fails.Count) 项自动检查未通过）"
    Pop-Location
    exit 1
}
Write-Host ("RELEASE READINESS: 自动项全部通过；仍有 {0} 项需要人工/授权（见上）" -f $blocked.Count)
Pop-Location
exit 0
