# 逐 crate 统计测试数量（唯一可信口径）
#
# 为什么需要脚本：`cargo test --workspace 2>&1 | Select-String ...` 在
# PowerShell 下会因为 stdout/stderr 合流而丢行，曾把 183 误报成 126。
# 必须先把完整输出落盘，再解析。

# cargo 的 warning 走 stderr，Stop 会把它当致命错误中断解析`n$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$log = Join-Path $root "target\test-workspace.log"

Push-Location $root
try {
    & cargo test --workspace --no-fail-fast 2>&1 | Out-File -FilePath $log -Encoding utf8
    $code = $LASTEXITCODE

    $cur = ""
    $sum = 0
    $fail = 0
    $rows = @()
    foreach ($l in Get-Content $log) {
        if ($l -match 'Running (?:unittests )?.*?deps\\([a-z0-9_]+)-[0-9a-f]+\.exe') {
            $cur = $Matches[1]
        }
        elseif ($l -match 'Running (tests)\\(.+?)\.rs') {
            $cur = $Matches[2]
        }
        if ($l -match '^test result: (\w+)\. (\d+) passed; (\d+) failed') {
            $n = [int]$Matches[2]
            $f = [int]$Matches[3]
            $rows += [pscustomobject]@{ Target = $cur; Passed = $n; Failed = $f }
            $sum += $n
            $fail += $f
        }
    }

    $rows | Group-Object Target |
        Sort-Object Name |
        ForEach-Object {
            $p = ($_.Group | Measure-Object Passed -Sum).Sum
            $f = ($_.Group | Measure-Object Failed -Sum).Sum
            "{0,-24} {1,3} passed  {2} failed" -f $_.Name, $p, $f
        }

    "----------------------------------------"
    "TOTAL {0} passed, {1} failed (exit {2})" -f $sum, $fail, $code
    exit $code
}
finally {
    Pop-Location
}
