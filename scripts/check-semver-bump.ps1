# semver 纪律守卫（离线，不联网）。
#
# 为什么需要它：仓库的约定是“0.x 下破坏性变更升 minor、patch 只修 bug”，
# 但这条约定只写在 README 里，没有机器检查。历史上已经出现过带 `!` 的提交
# 落进 patch 版本（v0.2.1），而该版发布说明写着“无破坏性变更”。
#
# 规则（自最近 tag 到 HEAD）：
#   1. 出现 `type(scope)!:` 形式的提交时，workspace 版本必须相对最近 tag
#      升 minor 或 major（patch 或未升级都算失败）；
#   2. 没有 `!` 提交时不强制升版本（版本升级发生在发布准备的提交里，
#      允许日常 PR 保持版本号不变）。
#
# 为什么规则 2 不要求“必须升版本”：版本号是发布时统一 bump 的，若每次 PR 都要求
# 已经 bump，CI 会在发布后的所有 PR 上误报。真正要挡的是“破坏性变更落进 patch 版”。
#
# 用法：
#   pwsh -NoProfile -File scripts/check-semver-bump.ps1
#   pwsh -NoProfile -File scripts/check-semver-bump.ps1 -Tag v0.2.5
param(
    [string]$Tag = ''
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

function Fail([string]$msg) {
    Write-Host "[semver] FAIL: $msg"
    exit 1
}

function Get-WorkspaceVersion {
    $text = [System.IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'))
    $m = [regex]::Match($text, '(?ms)\[workspace\.package\](.*?)(?:^\[|\z)')
    if (-not $m.Success) { return '' }
    return [regex]::Match($m.Groups[1].Value, '(?m)^\s*version\s*=\s*"([^"]+)"').Groups[1].Value
}

function Parse-Semver([string]$v) {
    # 只支持 X.Y.Z（发布必须稳定版；预发布标签只用于 tag 比较时跳过）。
    $m = [regex]::Match($v, '^(\d+)\.(\d+)\.(\d+)$')
    if (-not $m.Success) { return $null }
    return [pscustomobject]@{
        Major = [int]$m.Groups[1].Value
        Minor = [int]$m.Groups[2].Value
        Patch = [int]$m.Groups[3].Value
    }
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Write-Host '[semver] 跳过：没有 git（发布包/CI 解压目录下不适用）'
    exit 0
}

if (-not $Tag) {
    $Tag = (git describe --tags --abbrev=0 --match 'v[0-9]*' 2>$null)
}
if (-not $Tag) {
    Write-Host '[semver] 跳过：找不到任何 v* tag（首次发布前）'
    exit 0
}

$current = Get-WorkspaceVersion
if (-not $current) { Fail '读不到 workspace 版本号' }
$currentSem = Parse-Semver $current
if (-not $currentSem) { Fail "workspace 版本不是稳定 SemVer：$current" }

$tagVersion = $Tag.TrimStart('v')
$tagSem = Parse-Semver $tagVersion
if (-not $tagSem) {
    Write-Host "[semver] 跳过：最近 tag $Tag 不是稳定的 vX.Y.Z（预发布/临时 tag 不参与比较）"
    exit 0
}

$bump = 'none'
if ($currentSem.Major -gt $tagSem.Major) { $bump = 'major' }
elseif ($currentSem.Major -eq $tagSem.Major -and $currentSem.Minor -gt $tagSem.Minor) { $bump = 'minor' }
elseif ($currentSem.Major -eq $tagSem.Major -and $currentSem.Minor -eq $tagSem.Minor -and $currentSem.Patch -gt $tagSem.Patch) { $bump = 'patch' }
elseif ($currentSem.Major -lt $tagSem.Major -or ($currentSem.Major -eq $tagSem.Major -and $currentSem.Minor -lt $tagSem.Minor)) {
    Fail "workspace 版本 $current 比最近 tag $Tag 还旧（版本号不能回退）"
}

$subjects = @(git log "$Tag..HEAD" --format='%s' 2>$null)
if ($LASTEXITCODE -ne 0) { Fail "git log $Tag..HEAD 失败" }

# conventional commit 的破坏性标记：type(scope)!: / type!:
$breaking = @($subjects | Where-Object { $_ -match '^[a-zA-Z]+(\([^)]*\))?!:' })

if ($breaking.Count -eq 0) {
    Write-Host "[semver] ok: $Tag -> $current（$bump，无破坏性提交；不强制日常 PR 升版本）"
    exit 0
}

if ($bump -eq 'major' -or $bump -eq 'minor') {
    Write-Host "[semver] ok: $Tag -> $current（$bump，含 $($breaking.Count) 个已声明的破坏性提交）"
    exit 0
}

Write-Host '[semver] 找到带 ! 的提交：'
$breaking | ForEach-Object { Write-Host "  - $_" }
Fail "有 $($breaking.Count) 个破坏性提交，但版本只从 $tagVersion 到 $current（$bump）——应升 minor，或在提交里去掉 ! 并确认不是破坏性变更"