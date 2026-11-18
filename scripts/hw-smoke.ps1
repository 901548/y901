# 硬件/窗口场景的**证据落盘**脚本。
#
# 为什么单独一个脚本：`cargo test --workspace` 默认**不跑**那 60 条 `#[ignore]`
# （GPU 回读、真实窗口、DPI 探针）——"没跑"和"通过"必须分开记录，
# 否则门禁的绿灯只覆盖一半。这个脚本把那一批**真的跑一遍**，把结果写进
# `docs/evidence/hw-smoke-<日期>.log`，并在结尾打印可复制的摘要。
#
# 用法：
#   pwsh -File scripts/hw-smoke.ps1              # 跑并落盘
#   pwsh -File scripts/hw-smoke.ps1 -NoRecord    # 只跑不写文件（本地快速验证）
#
# 退出码：0 = 全部通过（允许 SKIP）；1 = 有失败；2 = 用了 -NoRecord。
param(
    [switch]$NoRecord
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$evidenceDir = Join-Path $root 'docs\evidence'
$stamp = Get-Date -Format 'yyyy-MM-dd'
$logPath = Join-Path $evidenceDir "hw-smoke-$stamp.log"

function Get-DisplayScale {
    # 显示缩放：LogPixels 96 = 100%，120 = 125% …（读不到就记 unknown，不猜）
    try {
        $lp = (Get-ItemProperty 'HKCU:\Control Panel\Desktop' -Name LogPixels -ErrorAction Stop).LogPixels
        if ($lp) { return [string]::Format('{0}% (LogPixels={1})', [int]($lp / 96 * 100), $lp) }
    } catch {}
    return 'unknown'
}

function Get-GpuName {
    try {
        return (Get-CimInstance Win32_VideoController -ErrorAction Stop |
            Select-Object -First 1 -ExpandProperty Name)
    } catch {
        return 'unknown'
    }
}

Write-Host '[hw-smoke] 采集环境 ...'
$envLines = @(
    "# 硬件冒烟证据 — $stamp",
    '',
    '## 环境',
    "- HEAD: $(git -C $root rev-parse --short HEAD 2>$null)",
    "- rustc: $((& rustc --version) -join ' ')",
    "- OS: $([System.Environment]::OSVersion.VersionString)",
    "- GPU: $(Get-GpuName)",
    "- 显示缩放: $(Get-DisplayScale)",
    '',
    '## 命令',
    '`cargo test --workspace --offline --no-fail-fast -- --ignored`',
    ''
)

# 门禁与部分用例会用这个文件记录"因为没 GPU 而跳过"，先清掉，避免读到上一轮的陈旧记录
$skipLog = Join-Path ([System.IO.Path]::GetTempPath()) 'y901-test-skips.txt'
Remove-Item $skipLog -ErrorAction SilentlyContinue

Write-Host '[hw-smoke] 运行 #[ignore] 用例（需要真实 GPU / 窗口）...'
$t0 = Get-Date
$out = & cargo test --workspace --offline --no-fail-fast -- --ignored 2>&1 | Out-String
$code = $LASTEXITCODE
$elapsed = [int]((Get-Date) - $t0).TotalSeconds

$passed = 0
$failed = 0
foreach ($m in [regex]::Matches($out, 'test result: \w+\. (\d+) passed; (\d+) failed')) {
    $passed += [int]$m.Groups[1].Value
    $failed += [int]$m.Groups[2].Value
}
$skips = 0
if (Test-Path $skipLog) {
    $skips = (Get-Content $skipLog | Where-Object { $_.Trim().StartsWith('SKIP:') } | Measure-Object).Count
}

$status = if ($failed -gt 0) { 'FAIL' } elseif ($passed -eq 0) { 'NO-OP (一条都没跑到?)' } else { 'PASS' }
$summary = @(
    '## 结果',
    "| 项 | 值 |",
    "|---|---|",
    "| 结论 | **$status** |",
    "| passed | $passed |",
    "| failed | $failed |",
    "| SKIP（无设备等） | $skips |",
    "| 耗时 | ${elapsed}s |",
    '',
    '> SKIP 表示用例**主动**判定当前环境不具备条件（例如没有可用 D3D 设备），',
    '> 它会写 `SKIP:` 到临时日志。SKIP 不等于通过，但也不等于失败——',
    '> 这条区分正是这个脚本存在的理由。',
    ''
)

$body = $envLines + $summary + @('## 原始输出', '```', $out.TrimEnd(), '```', '')

if ($NoRecord) {
    Write-Host ($summary -join "`n")
    Write-Host "[hw-smoke] -NoRecord：未写文件（结论 $status）"
    exit 2
}

if (-not (Test-Path $evidenceDir)) {
    New-Item -ItemType Directory -Force -Path $evidenceDir | Out-Null
}
[System.IO.File]::WriteAllText($logPath, ($body -join "`n"), (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[hw-smoke] 证据已写入：$logPath"
Write-Host ($summary -join "`n")

if ($failed -gt 0) { exit 1 }
# **NO-OP 不是通过**：一条用例都没跑到（或全被过滤掉）说明脚本没测到东西，
# 不能给 0。这与脚本自己"没跑 ≠ 通过"的原则一致。
if ($passed -eq 0) {
    Write-Host '[hw-smoke] 失败：一条 #[ignore] 用例都没跑到（检查过滤条件/编译是否成功）'
    exit 3
}
exit 0
