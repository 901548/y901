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
    # **`$baseline` 也必须从 numbers.txt 读回来**：
    # 之前只写了 `$script:baselineFromFile`，而下面的规则用的是 `$baseline`，
    # 于是 -Check 里它恒为空 —— 文档里只要写了“基线 **N**”，就永远被判 stale，
    # gate 的 doc 数字步会稳定失败（而原因看起来像"文档没同步"）。
    $baseline = if ($numbersFromFile.ContainsKey('baseline')) { $numbersFromFile['baseline'] } else { -1 }
    # 测试**目标数**（cargo 的 "test result:" 行数）：README/FRAMEWORK 写的是"N 个测试目标"，
    # 它和 passed 是两个独立的派生量 —— 只校准 passed 会留下"passed 对了、目标数停在旧值"的漂移。
    $targets  = if ($numbersFromFile.ContainsKey('targets'))  { $numbersFromFile['targets'] }  else { -1 }
    if ($baseline -lt 0) {
        Write-Host "[sync] FAIL: numbers.txt 缺少 baseline= 行"
        exit 1
    }
    # **numbers.txt 的数字必须与 baseline 同源**：baseline 是 `test-baseline.txt` 里
    # 最近一次真实测量的通过数；若 numbers.txt 的 passed 与它不一致，说明有人手改了
    # 文档数字（或只更新了其中一处）。gate 的测试计数步只会挡住"下降"，
    # 上涨/手改只能靠这里挡。
    if ($passed -ne $baseline) {
        Write-Host "[sync] FAIL: numbers.txt 的 passed=$passed 与 baseline=$baseline 不一致（重新跑 sync 生成）"
        exit 1
    }
    if ($ignored -lt 0) {
        Write-Host "[sync] FAIL: numbers.txt 缺少 ignored= 行"
        exit 1
    }
    if ($targets -lt 0) {
        Write-Host "[sync] FAIL: numbers.txt 缺少 targets= 行（重新跑一次 sync 生成）"
        exit 1
    }
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

    # 目标数 = cargo 输出里 "test result:" 的条数（每个 test binary 一条）。
    $targets = ([regex]::Matches($defaultOut, 'test result: \w+\. \d+ passed')).Count
    $baselineLine = (Get-Content (Join-Path $root 'docs\evidence\test-baseline.txt') | Where-Object { $_ -notmatch '^#' -and $_.Trim() -ne '' } | Select-Object -First 1)
    $baseline = [int]$baselineLine.Trim()
    if ($passed -ne $baseline) {
        Write-Host "[sync] FAIL: actual passed=$passed but baseline=$baseline (update test-baseline.txt first)"
        exit 1
    }

}
Write-Host "[sync] measured: passed=$passed failed=$failed ignored=$ignored targets=$targets include_ignored=$includeText baseline=$baseline"

# **行内形式的统一表**：新增一个数字字段，**只需在这里加一行** ——
# `-Check` 的校验规则与完整 sync 的写入都会**自动生成**。
#
# 为何要这样：之前 `passed` / `ignored` / `include_ignored` 是人工枚举的，
# "换个字段名就漏一次"已发生三次（这是第四个盲点的根因）。
# 与之前"$docFiles 移出 if (-not $Check)"是同一思路：**让机制保证正确性，而不是靠人记得**。
#
# 值一律以**字符串**存放：`include_ignored` 可能是 `n/a (full run has failures: …)`，
# 生成时用 `'^\d+$'` 判定是否为数字，非数字则**跳过**（那种情况下文档不应有数字形式）。
$inlineFields = [ordered]@{
    'passed'          = "$passed"
    'ignored'         = "$ignored"
    'include_ignored' = "$includeText"
}

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
  "# targets = number of test binaries ('test result:' lines in the default run)",
  "targets=$targets",
  "# include_ignored = observed passed count from a full --include-ignored run",
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
            # Markdown 表格以及显式包含默认测试统计的行，也纳入检查，避免 ACCEPTANCE.md 漏检。
            @{ name = 'passed table';    pattern = '\|\s*`cargo test --workspace --offline`[^|]*\|\s*\*\*(\d+)\s+passed'; want = $passed },
            @{ name = 'passed summary';  pattern = '默认测试[^\n]{0,50}\*\*(\d+)\s+passed';       want = $passed },
            @{ name = 'include summary'; pattern = '完整运行[^\n]{0,30}\*\*(\d+)\s+passed';       want = $includePassed },
            @{ name = 'include count';   pattern = 'include_ignored\s*=\s*(\d+)';                    want = $includePassed }
            # **行内形式的规则由 `$inlineFields` 自动生成**（见下方 foreach）。
            # 为何限定反引号：散文里也会出现 "Windows 侧 ignored=58、Linux 侧=59"，
            # 那是**平台对比**，不该按本仓全局计数校验；加反引号地匹配能把它排除。

        )
        # 行内规则：从 `$inlineFields` 生成（非数字值跳过）。
        foreach ($k in $inlineFields.Keys) {
            $v = $inlineFields[$k]
            if ("$v" -match '^\d+$') {
                $rules += @{ name = "inline $k"; pattern = ('`' + $k + '=(\d+)`'); want = $v }
            }
        }
        foreach ($rule in $rules) {
            foreach ($m in [regex]::Matches($t, $rule.pattern)) {
                $n = [int]$m.Groups[1].Value
                if ($n -ne [int]$rule.want) {
                    Write-Host "[sync] stale: $f -- '$($rule.name)' is $($rule.want) but the line says $n"
                    $stale++
                }
            }
        }

        # **通用兜底（P1-01 根因）**：上面每条规则都是手写枚举，换个写法（不加粗、
        # 换词序、写进表格）就整条逃检 —— 实测漏掉了 README 的
        # "978 passed ... 149 个测试目标"（真实值已 979/150）。
        #
        # 兜底规则：**含计数语义的行里，所有三位数都必须是当前计数之一**。
        # 用三位数是刻意的：避开年份（4 位）、哈希（0x… 被词边界排除）和小数。
        # 明确不是计数的按后缀排除：百分比 / 尺寸 / 像素 / 颜色数。
        # 平台相关（Linux/Helm）与历史口径的行必须**在文档里写明标记**才豁免
        # （Linux / Helm / 历史 / 旧口径 / 当轮 / `<!-- counts-exempt -->`）——
        # 豁免规则本身也要可审计，不能藏在脚本里。
        $countKeywords = '测试|passed|ignored|基线|项测试|include[-_ ]?ignored|含 ignore'
        $exemptMarkers = 'Linux|Helm|历史|旧口径|当轮|<!--\s*counts-exempt\s*-->'
        $allowed = @($passed, $ignored, $baseline, $targets, $includePassed) | Where-Object { $_ -ge 0 }
        foreach ($line in ($t -split "`n")) {
            if ($line -notmatch $countKeywords) { continue }
            if ($line -match $exemptMarkers) { continue }
            foreach ($m in [regex]::Matches($line, '(?<![\w.])(\d{3})(?![\w.])')) {
                $n = [int]$m.Groups[1].Value
                $suffix = $line.Substring($m.Index + $m.Length)
                $prefix = $line.Substring(0, $m.Index)
                # 不是计数：125% / 400×220 / 12px / 1039 色
                if ($suffix -match '^\s*(%|×|x\b|px|DPI|色|点|个像素)') { continue }
                if ($prefix -match '×\s*$') { continue }
                if ($allowed -notcontains $n) {
                    Write-Host "[sync] stale: $f -- $n 不是当前计数（$($allowed -join '/')）：$($line.Trim())"
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
    # **行内形式的写入同样由 `$inlineFields` 生成**：新增字段不需要再加这里的代码。
    # 非数字值（如 "n/a (...)"）跳过。
    foreach ($k in $inlineFields.Keys) {
        $v = $inlineFields[$k]
        if ("$v" -match '^\d+$') {
            $pat = '`' + $k + '=(\d+)`'
            $rep = [char]96 + "$k=$v" + [char]96
            $t = [regex]::Replace($t, $pat, $rep)
        }
    }

    # 非加粗的 "N passed / M failed / K ignored" 与 "N 个测试目标"（README 的构建块就是这种写法）。
    #
    # **逐行做，并且跳过带平台/历史标记的行**：全局替换会把
    # "Linux 878 passed / 0 failed / 27 ignored" 和 "Helm 230 passed / 0 failed / 3 ignored"
    # 一起改成 Windows 口径 —— 那是**错误的修正**，比不修更糟。
    $exempt = 'Linux|Helm|历史|旧口径|当轮|<!--\s*counts-exempt\s*-->'
    $t = [regex]::Replace($t, '(?m)^.*$', {
        param($line)
        $x = $line.Value
        if ($x -match $exempt) { return $x }
        $x = [regex]::Replace($x, '\b(\d{3})\s*passed\s*/\s*(\d+)\s*failed\s*/\s*(\d+)\s*ignored', { param($m) "$passed passed / $failed failed / $ignored ignored" })
        $x = [regex]::Replace($x, '\b(\d{3})\s*个测试目标', { param($m) "$targets 个测试目标" })
        $x
    })
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