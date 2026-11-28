# 把 a11y_demo 的真实 UIA 树 dump 出来（读屏器读的就是这棵树）。
#
# 为什么要有这个脚本：**"读屏器会念什么"是可自动核对的** —— UIA 是公开 API，
# 与讲述人消费的是同一套数据。人只需要负责最后一步"语音引擎念得对不对"。
# 2026-11-28 靠它抓到 7 个真缺陷（角色报成按钮、名字念内部 id、列表/告警不在树里…）。
#
# 用法：
#   powershell -NoProfile -File scripts/dump-a11y-uia.ps1                 # dump 到屏幕
#   powershell -NoProfile -File scripts/dump-a11y-uia.ps1 -OutFile docs/evidence/xxx.log
#
# 注意：`a11y_demo.exe` 是所有 `cargo test/clippy --all-targets` 共用的产物，
# **不带 `--features a11y` 的构建会把它覆盖成"没有无障碍桥"的版本**
# （现象：窗口能开，但 UIA 里 DESCENDANTS=0）。所以本脚本每次都会先带 feature 重建。
param(
    [string]$OutFile = '',
    [int]$WaitSeconds = 6
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

Write-Host '[a11y] 带 a11y feature 重建示例…'
# cargo 把进度打到 stderr，`$ErrorActionPreference='Stop'` 会把它当成异常 —— 这里临时放开。
$prevEap = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$build = & cargo build -p y901-shell --features a11y --example a11y_demo --offline 2>&1 | Out-String
$buildCode = $LASTEXITCODE
$ErrorActionPreference = $prevEap
if ($buildCode -ne 0) { Write-Host $build; throw '[a11y] 构建失败' }

$exe = Join-Path $root 'target\debug\examples\a11y_demo.exe'
if (-not (Test-Path $exe)) { throw "[a11y] 找不到 $exe" }

$proc = Start-Process -FilePath $exe -PassThru
try {
    Start-Sleep -Seconds $WaitSeconds
    $dump = & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root 'target\uia-dump4.ps1') $proc.MainWindowHandle 2>&1 | Out-String
} finally {
    Get-Process -Id $proc.Id -ErrorAction SilentlyContinue | Stop-Process -Force
}

if ($dump -notmatch 'DESCENDANTS=(?!0)\d+') {
    Write-Host $dump
    throw '[a11y] UIA 树是空的：确认用的是带 a11y feature 的二进制、窗口标题存在'
}

$header = @(
    '# Windows UIA 无障碍树取证（scripts/dump-a11y-uia.ps1）',
    '# 工具：PowerShell + UIAutomationClient（与讲述人消费同一套 API）',
    ('# 时间：' + (Get-Date -Format 'yyyy-MM-dd HH:mm:ss')),
    ''
) -join "`n"
$text = $header + "`n" + $dump

if ($OutFile) {
    $path = if ([System.IO.Path]::IsPathRooted($OutFile)) { $OutFile } else { Join-Path $root $OutFile }
    [System.IO.File]::WriteAllText($path, $text, (New-Object System.Text.UTF8Encoding($false)))
    Write-Host ("[a11y] 已写入 " + $path)
} else {
    Write-Host $text
}