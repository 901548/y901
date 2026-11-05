# 把文档里的关键数字从“手写”改为“生成”。
#
# 为何存在：审查 P1-01/P2-03 的根因是数字手写进散文，
# 代码变了文档不变（README 818 vs 实测 824，ignore 45 vs 52）。
param([switch]$Check)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$numbersPath = Join-Path $root 'docs\evidence\numbers.txt'

function Get-Count([string]$pattern, [string]$text) {
    $m = [regex]::Match($text, $pattern)
    if ($m.Success) { return [int]$m.Groups[1].Value }
    return -1
}

$defaultOut = & cargo test --workspace --offline 2>&1 | Out-String
$passed = 0; $failed = 0; $ignored = 0
foreach ($m in [regex]::Matches($defaultOut, 'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored')) {
    $passed += [int]$m.Groups[1].Value; $failed += [int]$m.Groups[2].Value; $ignored += [int]$m.Groups[3].Value
}
# include-ignored 跑的是**全部**用例（含并行基准），而部分 `#[ignore]` 基准依赖**进程内全局计数器**，
# 并行时会把别的用例的分配记到自己头上（实测：每帧应 0 却测到 4）。
# 所以这个口径**只在全部通过时**才记录，否则写 n/a —— 不拿一个失真的数字去充数。
$includeOut = & cargo test --workspace --offline -- --include-ignored 2>&1 | Out-String
$includeCode = $LASTEXITCODE
$includePassed = 0
foreach ($m in [regex]::Matches($includeOut, 'test result: \w+\. (\d+) passed')) { $includePassed += [int]$m.Groups[1].Value }
$includeText = if ($includeCode -eq 0) { "$includePassed" } else { "n/a (full run has failures: parallel fixtures break allocation-counter tests)" }

$baselineLine = (Get-Content (Join-Path $root 'docs\evidence\test-baseline.txt') | Where-Object { $_ -notmatch '^#' -and $_.Trim() -ne '' } | Select-Object -First 1)
$baseline = [int]$baselineLine.Trim()
if ($passed -ne $baseline) {
    Write-Host "[sync] FAIL: actual passed=$passed but baseline=$baseline (update test-baseline.txt first)"
    exit 1
}
Write-Host "[sync] measured: passed=$passed failed=$failed ignored=$ignored include_ignored=$includeText baseline=$baseline"

$stamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
# 时间戳不进 numbers.txt：那会让“纯数据文件”每次 sync 都变脏，
# 把工具输出和运行日志混在一起。时间戳改写到 target/（已被 gitignore）。
$logPath = Join-Path $root 'target\doc-numbers-last-sync.txt'
if (-not (Test-Path (Split-Path $logPath -Parent))) {
    New-Item -ItemType Directory -Force -Path (Split-Path $logPath -Parent) | Out-Null
}
[System.IO.File]::WriteAllText($logPath, "last sync: $stamp`n", (New-Object System.Text.UTF8Encoding($false)))

$numbers = @(
  "# DO NOT EDIT - regenerate with: pwsh -File scripts/sync-doc-numbers.ps1",
  "passed=$passed",
  "ignored=$ignored",
  "# include_ignored = passed + ignored (full --include-ignored run, not a separate suite)",
  "include_ignored=$includeText",
  "baseline=$baseline"
)
[System.IO.File]::WriteAllText($numbersPath, (($numbers -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false)))

$docFiles = @('README.md','docs\STATUS.md','docs\FRAMEWORK.md','docs\PERFORMANCE.md','docs\ACCEPTANCE.md')
$changes = 0
$stale = 0
foreach ($f in $docFiles) {
    $p = Join-Path $root $f
    if (-not (Test-Path $p)) { continue }
    $orig = [System.IO.File]::ReadAllText($p)
    $t = $orig
    # 只替换“数字 + 明确语义”的组合，不做全文数字替换
    if ($Check) {
        # 校验比同步更严：**任何**与“测试 / passed”同行的 3 位数，
        # 只要不等于当前实测值，就算漂移。
        # 为何要这么宽：上一版只匹配 `**N** 项测试` 这类固定形式，
        # 改 README 表格里的 `**824**（cargo test` 就检不出来。
        foreach ($line in ($t -split "`n")) {
            if ($line -notmatch '测试|passed|ignored') { continue }
            # 只看**加粗**的 3 位数：历史记录里的 760/801 是普通文本，不应报。
            foreach ($m in [regex]::Matches($line, '\*\*(\d{3})\*\*')) {
                $n = [int]$m.Groups[1].Value
                if ($n -eq $passed -or $n -eq $ignored -or $n -eq $includePassed -or $n -eq $baseline) { continue }
                if ($n -lt 100) { continue }
                Write-Host "[sync] OUT OF SYNC: $f has stale number $n (expected $passed)"
                $stale++
                break
            }
        }
        continue
    }
    $t = [regex]::Replace($t, '(\d+)\s*passed', { param($m) "$passed passed" })
    $t = [regex]::Replace($t, '(\d+)\s*failed', { param($m) "$failed failed" })
    $t = [regex]::Replace($t, '(\d+)\s*ignored', { param($m) "$ignored ignored" })
    $t = [regex]::Replace($t, '--include-ignored\s*\*\*(\d+)\*\*', { param($m) "--include-ignored **$includePassed**" })
    $t = [regex]::Replace($t, '包含\s*\*\*(\d+)\*\*', { param($m) "包含 **$includePassed**" })
    $t = [regex]::Replace($t, '\*\*(\d{3})\*\*\s*项测试', { param($m) "**$passed** 项测试" })
    $t = [regex]::Replace($t, '(\d{3})\s*项测试', { param($m) "$passed 项测试" })
    $t = [regex]::Replace($t, '测试[^\n]{0,6}\*\*(\d{3})\*\*', { param($m) "测试 **$passed**" })
    $t = [regex]::Replace($t, '\*\*(\d{3})\s*测试全绿\*\*', { param($m) "**$passed 测试全绿**" })
    if ($t -ne $orig) {
        if ($Check) { Write-Host "[sync] OUT OF SYNC: $f"; $stale++ }
        else {
            [System.IO.File]::WriteAllText($p, $t, (New-Object System.Text.UTF8Encoding($false)))
            Write-Host "[sync] updated: $f"; $changes++
        }
    }
}
if ($Check) {
    if ($stale -gt 0) { Write-Host "[sync] $stale file(s) out of sync"; exit 1 }
    Write-Host "[sync] ok: doc numbers match numbers.txt (passed=$passed ignored=$ignored)"
    exit 0
}
Write-Host "[sync] wrote numbers.txt; updated $changes file(s)"
exit 0