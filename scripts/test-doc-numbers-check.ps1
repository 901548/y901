# 数字检查器的**变异测试**：证明 `sync-doc-numbers.ps1 -Check` 真的能抓到漂移。
#
# 为什么需要它：审查反复指出"检查通过不代表路径被覆盖" —— 上一版 `-Check` 只认加粗写法，
# 于是 README 里非加粗的 `978 passed ... 149 个测试目标` 一路逃检（真实值已是 979/150）。
# 这条脚本把"应该被抓到"的场景固化成可重跑的测试，而不是靠人记得手动试一次。
#
# 做法：临时改一处文档 → 跑 `-Check` → **必须**报 stale 且退出码非 0 → 恢复原文。
# 任何一步不满足就失败；结束时再跑一次干净的 `-Check`（豁免行不得误报）。
#
# 用法：pwsh -File scripts/test-doc-numbers-check.ps1
# 注意：会短暂改写工作区里的 md 文件，但每条都在 finally 里恢复；不要并发运行。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$sync = Join-Path $PSScriptRoot 'sync-doc-numbers.ps1'
$numbersPath = Join-Path $root 'docs\evidence\numbers.txt'

$numbers = @{}
foreach ($line in Get-Content $numbersPath) {
    if ($line -match '^\s*([a-z_]+)\s*=\s*(-?\d+)\s*$') { $numbers[$matches[1]] = [int]$matches[2] }
}
foreach ($k in @('passed', 'ignored', 'targets')) {
    if (-not $numbers.ContainsKey($k)) {
        Write-Host "[mutate] FAIL: numbers.txt 缺少 $k（先跑一次 sync）"
        exit 1
    }
}
$passed = $numbers['passed']
$ignored = $numbers['ignored']
$targets = $numbers['targets']

function Invoke-Check {
    $out = & powershell -NoProfile -File $sync -Check 2>&1 | Out-String
    return @{ Code = $LASTEXITCODE; Out = $out }
}

$utf8 = New-Object System.Text.UTF8Encoding($false)

# 每条用例：(文件, 原文, 改成, 说明)
$cases = @(
    @{ File = 'README.md'; Old = "$passed passed / 0 failed / $ignored ignored"; New = "$($passed - 1) passed / 0 failed / $ignored ignored"; Label = '非加粗的 passed 漂移（README 构建块）' },
    @{ File = 'README.md'; Old = "（$targets 个测试目标）"; New = "（$($targets - 1) 个测试目标）"; Label = '测试目标数漂移' },
    @{ File = 'docs\ACCEPTANCE.md'; Old = "| **$passed passed / 0 failed / $ignored ignored** |"; New = "| **$($passed - 1) passed / 0 failed / $ignored ignored** |"; Label = '表格内的 passed 漂移' },
    @{ File = 'docs\FRAMEWORK.md'; Old = "$passed 项通过、$ignored 项忽略（$targets 个测试目标）"; New = "$passed 项通过、$ignored 项忽略（$($targets - 1) 个测试目标）"; Label = 'FRAMEWORK 目标数漂移' },
    # 反自伤：这两类"脚本改坏文档"的指纹必须被抓到（A-2）。
    @{ File = 'README.md'; Old = '基线 0xF7455CCE7781A864'; New = "基线 **$passed**xF7455CCE7781A864"; Label = '十六进制基线被当成计数（0x 被吃）' },
    @{ File = 'docs\STATUS.md'; Old = 'vs win32 默认族'; New = "vs win**$passed** 默认族"; Label = '词内数字被当成计数（win32 → win**N**）' }
)

$failures = 0
foreach ($c in $cases) {
    $path = Join-Path $root $c.File
    $orig = [System.IO.File]::ReadAllText($path)
    if (([regex]::Matches($orig, [regex]::Escape($c.Old))).Count -ne 1) {
        Write-Host "[mutate] FAIL: 锚点失效（$($c.File) 里找不到唯一匹配）—— 文档改写过就更新本脚本"
        $failures++
        continue
    }
    try {
        [System.IO.File]::WriteAllText($path, $orig.Replace($c.Old, $c.New), $utf8)
        $r = Invoke-Check
        $caught = ($r.Code -ne 0) -and ($r.Out -match 'stale')
        if ($caught) {
            Write-Host "[mutate] ok: 抓到 $($c.Label)"
        } else {
            Write-Host "[mutate] FAIL: 漏检 $($c.Label)（exit=$($r.Code)）"
            Write-Host ($r.Out -split "`n" | Select-Object -First 8 | Out-String)
            $failures++
        }
    }
    finally {
        [System.IO.File]::WriteAllText($path, $orig, $utf8)
    }
}

# 额外一条：整行新增一个无标记的假计数（不是"改已有数字"，而是"多写一个"）。
$readme = Join-Path $root 'README.md'
$origReadme = [System.IO.File]::ReadAllText($readme)
try {
    [System.IO.File]::WriteAllText($readme, $origReadme.TrimEnd() + "`n`n本轮 $($passed - 2) 项测试全绿。`n", $utf8)
    $r = Invoke-Check
    if (($r.Code -ne 0) -and ($r.Out -match 'stale')) {
        Write-Host "[mutate] ok: 抓到 新增的无标记假计数"
    } else {
        Write-Host "[mutate] FAIL: 漏检 新增的无标记假计数（exit=$($r.Code)）"
        $failures++
    }
}
finally {
    [System.IO.File]::WriteAllText($readme, $origReadme, $utf8)
}

# 反向对照：恢复后必须是绿的，否则说明豁免规则过严（会天天误报）。
$final = Invoke-Check
if ($final.Code -ne 0) {
    Write-Host "[mutate] FAIL: 恢复后 -Check 仍然失败（豁免规则误报）"
    Write-Host ($final.Out -split "`n" | Select-Object -First 12 | Out-String)
    $failures++
}

if ($failures -gt 0) {
    Write-Host "[mutate] $failures 条用例失败"
    exit 1
}
Write-Host "[mutate] ok: $($cases.Count + 1) 条变异全部被抓到，且干净状态下不误报"
exit 0