# 一条命令复跑性能基准（本地与 CI 用同一套）。
#
# 为什么需要它：性能数字散在几个墙钟断言与 `--nocapture` 打印里，
# 而 `docs/PERFORMANCE.md` 的数字是**手工复测**留下的 —— 换台机器、隔几周，
# 就没人能快速回答"现在还是这个数吗"。这个脚本把"当时怎么测的"固化成可执行文件，
# 输出里带机器信息，便于把结果贴进文档。
#
# 用法：
#   powershell -NoProfile -File scripts/perf-gate.ps1              # 全部 4 项
#   powershell -NoProfile -File scripts/perf-gate.ps1 -Only grid   # 只跑一项
#   powershell -NoProfile -File scripts/perf-gate.ps1 -NoEnforce   # 只测不断言（看数字）
#
# 退出码：0 = 全部通过（或只测不断言）；1 = 有基准失败。
#
# **默认强制阈值**：`grid_perf` 的墙钟断言只有在 `Y901_PERF_GATE=1` 时生效，
# 而共享 CI runner 的墙钟不可靠 —— 所以本脚本**默认设 Y901_PERF_GATE=1**
# （这是"想当门禁"的语义），要纯粹看数字就加 `-NoEnforce`。
#
# 每个基准的"是否 #[ignore]"不同（grid_perf 是普通测试，其余有的是 ignore），
# 所以一律用 `--include-ignored` 而不是 `--ignored`：后者会把非 ignore 的用例过滤掉，
# 结果是"跑了 0 个用例却 PASS"——那正是本仓库最忌讳的假绿。
param(
    [ValidateSet('all', 'grid', 'layout', 'dirty', 'soft')]
    [string]$Only = 'all',
    [switch]$NoEnforce
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

if (-not $NoEnforce) { $env:Y901_PERF_GATE = '1' }

$machine = try { (Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object -First 1).Name } catch { 'unknown-cpu' }
Write-Host "=== Y901 perf gate ==="
Write-Host "cpu      : $machine"
Write-Host "os       : $([System.Environment]::OSVersion.VersionString)"
Write-Host "rustc    : $((& rustc --version) -join ' ')"
Write-Host "enforce  : $(if ($NoEnforce) { 'no（只测数字）' } else { 'yes（Y901_PERF_GATE=1）' })"
Write-Host ""

$results = @()
function Invoke-Bench([string]$name, [string]$package, [string]$test, [string]$filter = '') {
    Write-Host "=== $name ==="
    $cargoArgs = @('test', '-p', $package, '--test', $test, '--release', '--offline', '--', '--include-ignored', '--nocapture')
    if ($filter) { $cargoArgs += $filter }
    $out = & cargo @cargoArgs 2>&1 | Out-String
    $code = $LASTEXITCODE
    # 只打印带含语义数字的行：这些基准都把结果 eprintln! 出来。
    # 同时滤掉 PowerShell 把 cargo 的 stderr 当异常回显时插入的那一堆噪声行
    # （`At F:\…char:12`、`CategoryInfo`、`+ CategoryInfo` …），它们不带信息只有干扰。
    $noise = '^\s*(Compiling|Finished|Running|warning|Downloaded|Downloading|Blocking)|CategoryInfo|FullyQualifiedErrorId|^\s*At [A-Z]:|^\s*\+ |^\s*所在位置|^\s*\+ CategoryInfo'
    $lines = ($out -split "`n") | Where-Object { $_ -match '\d' -and $_ -notmatch $noise }
    foreach ($l in $lines) { Write-Host "  $($l.Trim())" }
    if ($code -ne 0) {
        ($out -split "`n" | Select-Object -Last 12) | ForEach-Object { Write-Host "  $_" }
    }
    $script:results += [pscustomobject]@{ Name = $name; Code = $code }
    Write-Host ""
}

if ($Only -in @('all', 'grid'))   { Invoke-Bench 'grid_perf（10k 网格布局；release 预算 6ms）' 'y901-ui' 'grid_perf' }
if ($Only -in @('all', 'layout')) { Invoke-Bench 'layout_bench（10k/50k 布局）' 'y901-ui' 'layout_bench' }
if ($Only -in @('all', 'dirty'))  { Invoke-Bench 'dirty_strategy_bench（脏区合并）' 'y901-render' 'dirty_strategy_bench' }
if ($Only -in @('all', 'soft'))   { Invoke-Bench 'soft_fill_rate_1280x800（软光栅填充率）' 'y901-backend-win32' 'soft_backend' 'soft_fill_rate' }

Write-Host "=== 汇总 ==="
$failed = @($results | Where-Object { $_.Code -ne 0 })
foreach ($r in $results) { Write-Host ("{0,-6} {1}" -f $(if ($r.Code -eq 0) { 'PASS' } else { 'FAIL' }), $r.Name) }
Pop-Location
if ($failed.Count -gt 0) {
    Write-Host "PERF GATE: FAILED（$($failed.Count) 项）"
    exit 1
}
Write-Host "PERF GATE: OK"
exit 0
