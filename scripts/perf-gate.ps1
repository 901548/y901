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
#   powershell -NoProfile -File scripts/perf-gate.ps1 -Only stats  # 打印分阶段统计 JSON
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
    [ValidateSet('all', 'grid', 'layout', 'dirty', 'soft', 'stats')]
    [string]$Only = 'all',
    [switch]$NoEnforce,
    # 强制只用本地缓存（不联网）。
    #
    # 默认**不**强制：本脚本会先试 `--offline`（本地最快），一旦发现是
    # "依赖没下载"就自动联网重试。CI runner 首次运行时 registry 缓存是冷的，
    # 写死 `--offline` 会让 4 项基准一起红，看起来像性能退化 ——
    # 真实发生过一次（Perf gate run 45174360573，全部报 `failed to download`）。
    [switch]$Offline
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

# "这次失败到底是不是性能问题"——必须区分开：
# 依赖下载失败/工具链问题都**不是**性能退化，但退出码一样是 1，
# 混在一起报就会误导人（上一次 Perf gate 红就是这么来的）。
$script:dependencyErrors = 0
$networkPattern = 'failed to download|Unable to update|no matching package named|offline mode|spurious network|Couldn''t resolve hostname|failed to fetch|Connection refused|timed out|network failure|registry index'

function Invoke-Bench([string]$name, [string]$package, [string]$test, [string]$filter = '') {
    Write-Host "=== $name ==="
    $base = @('test', '-p', $package, '--test', $test, '--release')
    $tail = @('--', '--include-ignored', '--nocapture')
    if ($filter) { $tail += $filter }

    # 先离线（本地命中缓存最快、断网也不会卡），不在 `-Offline` 时再允许联网重试。
    $attempts = @(, @('--offline'))
    if (-not $Offline) { $attempts += , @() }

    $out = ''
    $code = 1
    for ($i = 0; $i -lt $attempts.Count; $i++) {
        $cargoArgs = $base + $attempts[$i] + $tail
        $out = & cargo @cargoArgs 2>&1 | Out-String
        $code = $LASTEXITCODE
        if ($code -eq 0) { break }
        if ($out -notmatch $networkPattern) { break }
        # 只有"后面还有一次联网尝试"时才说这句（`-Offline` 下不该出现）
        if ($i + 1 -lt $attempts.Count) { Write-Host "  （依赖下载失败，改为联网重试）" }
    }
    if ($code -ne 0 -and $out -match $networkPattern) {
        Write-Host "  !! 这次失败是**依赖/网络**问题，不是性能退化。"
        $script:dependencyErrors += 1
    }
    # 只打印带含语义数字的行：这些基准都把结果 eprintln! 出来。
    # 同时滤掉 PowerShell 把 cargo 的 stderr 当异常回显时插入的那一堆噪声行
    # （`At F:\…char:12`、`CategoryInfo`、`+ CategoryInfo` …），它们不带信息只有干扰。
    $noise = '^\s*(Compiling|Finished|Running|warning|Downloaded|Downloading|Blocking)|CategoryInfo|FullyQualifiedErrorId|^\s*At [A-Z]:|^\s*\+ |^\s*所在位置|^\s*\+ CategoryInfo'
    $lines = ($out -split "`n") | Where-Object { $_ -match '\d' -and $_ -notmatch $noise }
    foreach ($l in $lines) { Write-Host "  $($l.Trim())" }
    if ($code -ne 0) {
        ($out -split "`n" | Select-Object -Last 12) | ForEach-Object { Write-Host "  $_" }
    }
    $script:results += [pscustomobject]@{ Name = $name; Code = $code; Dependency = ($code -ne 0 -and $out -match $networkPattern) }
    Write-Host ""
}

# 分阶段统计（RenderStats）：**不是阈值基准**，而是把"一帧的各阶段耗时"打成 JSON 行，
# 便于贴进 docs/PERFORMANCE.md 或跨机器比较。因此：
# - 用 `--nocapture` 抓 eprintln 出的 JSON 行；
# - **不做通过/失败判定**（阈值会随机器漂移，硬编码只会制造假红）；
# - 仍然记录退出码：连测试都跑不起来（无 D3D11 设备时用例会留痕跳过）要能看出来。
function Invoke-Stats([string]$name, [string]$package, [string]$test, [string]$envName, [string]$envValue) {
    Write-Host "=== $name ==="
    $prev = [Environment]::GetEnvironmentVariable($envName)
    [Environment]::SetEnvironmentVariable($envName, $envValue)
    try {
        $out = & cargo test -p $package --test $test --release --offline -- --nocapture --test-threads=1 2>&1 | Out-String
        $code = $LASTEXITCODE
    } finally {
        [Environment]::SetEnvironmentVariable($envName, $prev)
    }
    # JSON 行以 `{` 开头、`"frame"` 打头（render_stats::write_json 的固定形状）。
    $json = ($out -split "`n") | Where-Object { $_ -match '^\s*\{\s*"frame"' }
    if ($json.Count -eq 0) {
        Write-Host "  （没有 JSON 行：本机可能没有可用设备，或用例按约定留痕跳过）"
        $skip = ($out -split "`n") | Where-Object { $_ -match 'SKIP:' }
        foreach ($l in $skip) { Write-Host "  $($l.Trim())" }
    } else {
        foreach ($l in $json) { Write-Host "  $($l.Trim())" }
    }
    $script:results += [pscustomobject]@{ Name = $name; Code = $code; Dependency = ($code -ne 0 -and $out -match $networkPattern) }
    Write-Host ""
}

if ($Only -in @('all', 'stats')) {
    # D3D11 与 `--force-soft` 各跑一次：两条路径的可测阶段不同（能力表不同）。
    Invoke-Stats 'render_stats（D3D11 分阶段统计 JSON）' 'y901-backend-win32' 'render_stats_enabled' 'Y901_RENDER_STATS' '1'
    Invoke-Stats 'render_stats（--force-soft 分阶段统计 JSON）' 'y901-backend-win32' 'render_stats_soft' 'Y901_RENDER_STATS' '1'
}

if ($Only -in @('all', 'grid'))   { Invoke-Bench 'grid_perf（10k 网格布局；release 预算 6ms）' 'y901-ui' 'grid_perf' }
if ($Only -in @('all', 'layout')) { Invoke-Bench 'layout_bench（10k/50k 布局）' 'y901-ui' 'layout_bench' }
if ($Only -in @('all', 'dirty'))  { Invoke-Bench 'dirty_strategy_bench（脏区合并）' 'y901-render' 'dirty_strategy_bench' }
if ($Only -in @('all', 'soft'))   { Invoke-Bench 'soft_fill_rate_1280x800（软光栅填充率）' 'y901-backend-win32' 'soft_backend' 'soft_fill_rate' }

Write-Host "=== 汇总 ==="
$failed = @($results | Where-Object { $_.Code -ne 0 })
foreach ($r in $results) {
    # 依赖/网络导致的失败标成 ERROR，不冒充"性能退化"
    $label = if ($r.Code -eq 0) { 'PASS' } elseif ($r.Dependency) { 'ERROR' } else { 'FAIL' }
    Write-Host ("{0,-6} {1}" -f $label, $r.Name)
}
Pop-Location
if ($failed.Count -gt 0) {
    if ($dependencyErrors -gt 0) {
        Write-Host "PERF GATE: FAILED —— 其中 $dependencyErrors 项是**依赖下载/网络**问题，不是性能退化。"
        Write-Host "           先跑一次 `cargo fetch`（或联网）再重试；本脚本默认会自动联网重试。"
    } else {
        Write-Host "PERF GATE: FAILED（$($failed.Count) 项）"
    }
    exit 1
}
Write-Host "PERF GATE: OK"
exit 0
