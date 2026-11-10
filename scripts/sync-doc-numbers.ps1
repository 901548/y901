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

# **-Check 不跑测试**。
#
# 原因：- Check 原本也会实跑 `--include-ignored`，而那一轮包含依赖**进程内全局计数器**
# 的基准用例，并行时互相污染，实测会得到 560 这种半截值（应为 804）。
# gate 每次都调 `-Check`，于是门禁随机报 "doc numbers out of sync"。
#
# 现在分工明确：
#   - `-Check`：**只读** `docs/evidence/numbers.txt`，比对文档与它是否一致（确定、快、无并行噪声）。
#   - 无 `-Check`：实跑测试重新测量并重写 numbers.txt（这是唯一会改数字的路径）。
$numbersFromFile = @{}
if (Test-Path $numbersPath) {
    foreach ($line in Get-Content $numbersPath) {
        if ($line -match '^\s*([a-z_]+)\s*=\s*(-?\d+)\s*$') {
            $numbersFromFile[$matches[1]] = [int]$matches[2]
        }
    }
}

if ($Check) {
    # 只读真值，不跑测试
    $passed   = if ($numbersFromFile.ContainsKey('passed')) { $numbersFromFile['passed'] } else { -1 }
    $ignored  = if ($numbersFromFile.ContainsKey('ignored')) { $numbersFromFile['ignored'] } else { -1 }
    $failed   = 0
    $includePassed = if ($numbersFromFile.ContainsKey('include_ignored')) { $numbersFromFile['include_ignored'] } else { -1 }
    $includeText = "$includePassed"
    if ($numbersFromFile.ContainsKey('baseline')) { $script:baselineFromFile = $numbersFromFile['baseline'] }
} else {
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

}
Write-Host "[sync] measured: passed=$passed failed=$failed ignored=$ignored include_ignored=$includeText baseline=$baseline"

$stamp = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss')
# 时间戳不进 numbers.txt：那会让“纯数据文件”每次 sync 都变脏，
# 把工具输出和运行日志混在一起。时间戳改写到 target/（已被 gitignore）。
# **必须在 `if (-not $Check)` 之外**：否则 -Check 模式下 $docFiles 为 $null，
# 文档校验循环根本不执行 —— -Check 会恒返回 ok（已发生）。
$docFiles = @('README.md','docs\STATUS.md','docs\FRAMEWORK.md','docs\PERFORMANCE.md','docs\ACCEPTANCE.md')

if (-not $Check) {
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

$changes = 0
}

$stale = 0
foreach ($f in $docFiles) {
    $p = Join-Path $root $f
    if (-not (Test-Path $p)) { continue }
    $orig = [System.IO.File]::ReadAllText($p)
    $t = $orig
    # 只替换“数字 + 明确语义”的组合，不做全文数字替换
    if ($Check) {
        # **\u6309\u884c\u8bed\u4e49**\u6821\u9a8c\uff1a\u6bcf\u4e2a\u6570\u5b57\u53ea\u5728\u5b83\u7684\u8bed\u8a00\u73af\u5883\u91cc\u88ab\u6bd4\u5bf9\u3002
        # \u4e3a\u4f55\u4e0d\u80fd\u201c\u6574\u884c\u6240\u6709\u6570\u5b57\u201d\uff1a\u4e00\u53e5\u8bdd\u91cc\u53ef\u80fd\u540c\u65f6\u6709\u57fa\u7ebf/\u9ed8\u8ba4/ignore \u4e09\u4e2a\u6570\uff0c
        # \u4e5f\u53ef\u80fd\u6709\u65e0\u5173\u6570\u5b57\uff08\u5982\u5de5\u5177\u7248\u672c 13\uff09\u3002\u56e0\u6b64\u7528**\u7d27\u8d34\u5173\u952e\u8bcd\u7684\u6a21\u5f0f**\u3002
        $rules = @(
            @{ name = 'ignored';         pattern = '`#\[ignore\]`\s*\*\*(\d+)\*\*'; want = $ignored },
            @{ name = 'baseline';        pattern = '\u57fa\u7ebf\s*\*\*(\d+)\*\*';            want = $baseline },
            @{ name = 'include_ignored'; pattern = 'include-ignored`?\s*[|:]\s*\*\*(\d+)\*\*'; want = $includePassed },
            @{ name = 'include_ignored'; pattern = '\u542b ignore[^\n]{0,8}\*\*(\d+)\*\*';       want = $includePassed },
            @{ name = 'passed';          pattern = '\*\*(\d+)\*\*\s*passed';                 want = $passed  },
            # **行内派生形式**（STATUS 第 5 行：passed=746 等）。
            # 为何限定反引号：散文里也会出现 "Windows 侧 ignored=58、Linux 侧=59"，
            # 那是**平台对比**，不该按本仓全局计数校验；加反引号地匹配能把它排除。
            @{ name = 'inline passed';  pattern = '`passed=(\d+)`'; want = $passed },
            @{ name = 'inline ignored'; pattern = '`ignored=(\d+)`'; want = $ignored },
            # **include_ignored 的行内形式**（第 4 个盲点）：之前它既不被 -Check 校验、
            # 也不被完整 sync 写入 —— 于是只能靠手工对齐（已发生一次）。
            @{ name = 'inline include_ignored'; pattern = '`include_ignored=(\d+)`'; want = $includePassed }

        )
        foreach ($rule in $rules) {
            foreach ($m in [regex]::Matches($t, $rule.pattern)) {
                $n = [int]$m.Groups[1].Value
                if ($n -ne [int]$rule.want) {
                    Write-Host "[sync] stale: $f -- '$($rule.name)' is $($rule.want) but the line says $n"
                    $stale++
                }
            }
        }
        continue
    }
    # **不做全文 "数字 + passed/failed/ignored" 替换**：
    # 那会把散文里与本仓全局计数无关的数字一并改掉。
    # 实例（已发生）："Helm `cargo test` 230 passed / 0 failed / 3 ignored"
    # 被改成 746 / 0 / 58；"real_window 从 11 passed 变 6 failed" 被改成 746 / 0。
    # 如今只改**加粗形式**（`**N** passed`）与带明确锚词的行；
    # 散文里裸写的 "230 passed" 不再被碰。
    $anchor = '测试全绿|`#\[ignore\]`|基线|include-ignored|项测试|默认|含 ignore'
    $t = [regex]::Replace($t, "(?m)^.*(?:$anchor).*$", {
        param($line)
        $x = $line.Value
        $x = [regex]::Replace($x, '\*\*(\d+)\*\*(\s*)passed', { param($m) "**$passed**$($m.Groups[2].Value)passed" })
        $x = [regex]::Replace($x, '\*\*(\d+)\*\*(\s*)failed', { param($m) "**$failed**$($m.Groups[2].Value)failed" })
        $x = [regex]::Replace($x, '\*\*(\d+)\*\*(\s*)ignored', { param($m) "**$ignored**$($m.Groups[2].Value)ignored" })
        $x
    })
    # **派生表述**也要能自动改写，否则每次计数变化都要手工对齐。
    # 前瞻 (?=…) 保留后缀；后顾 (?<=…) 保留前缀。
    $t = [regex]::Replace($t, '(?<!含 ignore\s{0,10})\*{0,2}(\d{1,4})\*{0,2}(?=\s*默认)', { param($m) "**$passed**" })
    $t = [regex]::Replace($t, '(含 ignore[^\n]{0,12}?)\*\*(\d{1,4})\*\*', { param($m) $m.Groups[1].Value + "**$includePassed**" })
    $t = [regex]::Replace($t, '(基线\s*)\*{0,2}(\d{1,4})\*{0,2}', { param($m) $m.Groups[1].Value + "**$baseline**" })
    $t = [regex]::Replace($t, '--include-ignored\s*\*\*(\d+)\*\*', { param($m) "--include-ignored **$includePassed**" })
    $t = [regex]::Replace($t, '包含\s*\*\*(\d+)\*\*', { param($m) "包含 **$includePassed**" })
    # 行内派生形式（反引号包住的 passed=/ignored=/include_ignored=）同样要维护，
    # 否则它会长期漂移而 -Check 也查不到（上一轮就是这个漏洞）。
    $t = [regex]::Replace($t, '`passed=(\d+)`', { param($m) ([char]96 + "passed=$passed" + [char]96) })
    $t = [regex]::Replace($t, '`ignored=(\d+)`', { param($m) ([char]96 + "ignored=$ignored" + [char]96) })
    # include_ignored 同样要维护（否则它与 numbers.txt 漂离而 -Check 也查不到）。
    # 注意类型：`$includePassed` 是字符串（可能为 "n/a (...)"），非数字时不替换。
    if ($includePassed -match '^\d+$') {
        $t = [regex]::Replace($t, '`include_ignored=(\d+)`', { param($m) ([char]96 + "include_ignored=$includePassed" + [char]96) })
    }

    $t = [regex]::Replace($t, '\*\*(\d{3})\*\*\s*项测试', { param($m) "**$passed** 项测试" })
    $t = [regex]::Replace($t, '(\d{3})\s*项测试', { param($m) "$passed 项测试" })
    # 测试行的加粗数字：只替换数字本身，不动冒号与其他字符
    $t = [regex]::Replace($t, '(?<=测试[^\n]{0,4})\*\*(\d{3})\*\*', { param($m) "**$passed**" })
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