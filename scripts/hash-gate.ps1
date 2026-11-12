# 基线更新记录（2026-11-12）：
#   0xDBB308DD1657EF9C -> 0x87DECF871BB90AFF
#   原因：DPI × scale 接入字号。本机窗口 DPI=120（1.25），基准字号 16 → 20，
#         所有默认字号文字变大 25%。**客户区尺寸不变**（400x220），
#         因此变化完全来自文字尺寸，可归因。
# 基线更新记录（2026-11-12，门禁场景扩展）：
#   0xDBB308DD1657EF9C -> 0x39AE41C1697CBFD0
#   原因：门禁场景新增含框架控件的布局段（Flex + TextLabel size 16/18/13 + align::Center），
#         覆盖布局/字号/align 的像素回归。原有 Scene 绘制逐字保留，客户区 400x220 不变。
# 哈希门禁：同会话前后对比（不硬编码任何哈希常量）
#
# 用法：
#   pwsh -File scripts/hash-gate.ps1            # 对比基线；**基线缺失则失败**（exit 3）
#   pwsh -File scripts/hash-gate.ps1 -Update    # 用当前结果覆盖基线（改动了渲染且确认无误后）
#
# 为什么不再硬编码 `0xF38B9C1AB8A877E2`：
#   `font_3c_pixels` 用 `CreateWindowExW(..., options.width, options.height, WS_OVERLAPPEDWINDOW)` 创建窗口，
#   这两个值是**窗口尺寸**（含边框与标题栏），而渲染用的是 `GetClientRect` 得到的**客户区**尺寸。
#   客户区 = 窗口 - 边框 - 标题栏，随 DPI / 主题 / DWM 状态变化：
#     - 项目早期观测到 382x173
#     - 本轮观测到 378x164
#   尺寸变了，哈希必然变。所以哈希只能作为**同一环境下前后对比**的判据，
#   不能作为跨会话的绝对常量。基线文件里同时记录客户区尺寸，尺寸不同就直接说明原因。

param(
    [switch]$Update
)

# cargo 会向 stderr 写进度信息，`Stop` 会把它当成致命错误 → 这里必须放宽。
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
# 基线**必须入库**（`target/` 被 gitignore），否则换一个 checkout 就“没有基线”而静默通过。
$baseline = Join-Path $root 'docs\evidence\hash-baseline.txt'

Write-Host '[hash-gate] 运行 font_3c_pixels ...'
# **固定 DPI 缩放 = 1.0**：门禁只应报渲染回归，不应因显示器 DPI 不同而变。
# 字号自 75af5b1 起乘 DPI 缩放，而该场景画的就是文字 —— 不固定就不可复现。
$env:Y901_TEST_DPI_SCALE = '1.0'
$out = & cargo test -p y901-backend-win32 --offline --test font_3c_pixels -- --ignored --nocapture 2>&1 | Out-String

$hashLine = ($out -split "`n" | Where-Object { $_ -match 'FNV 哈希 = (0x[0-9A-Fa-f]+)' } | Select-Object -First 1)
$sceneLine = ($out -split "`n" | Where-Object { $_ -match '场景 (\d+)x(\d+)' } | Select-Object -First 1)

if (-not $hashLine -or -not $sceneLine) {
    Write-Host '[hash-gate] 失败：未从输出中解析到哈希/尺寸'
    Write-Host $out
    exit 1
}

$null = $hashLine -match 'FNV 哈希 = (0x[0-9A-Fa-f]+)'
$hash = $Matches[1]
$null = $sceneLine -match '场景 (\d+)x(\d+)'
$size = "$($Matches[1])x$($Matches[2])"
$current = "hash=$hash size=$size"

Write-Host "[hash-gate] 当前：$current"

if ($Update) {
    Set-Content -Path $baseline -Value $current -Encoding utf8
    Write-Host "[hash-gate] 已记录基线：$baseline"
    exit 0
}

if (-not (Test-Path $baseline)) {
    Write-Host "[hash-gate] 失败：基线缺失（$baseline）"
    Write-Host "               请用 -Update 创建，并把文件提交入库。"
    exit 3
}

$stored = (Get-Content -Path $baseline -Raw).Trim()
$storedHash = ($stored -split ' ' | Where-Object { $_ -like 'hash=*' }) -replace '^hash=', ''
$storedSize = ($stored -split ' ' | Where-Object { $_ -like 'size=*' }) -replace '^size=', ''

if ($storedHash -eq $hash) {
    Write-Host "[hash-gate] 通过：哈希与基线一致（$hash，客户区 $size）"
    exit 0
}

if ($storedSize -ne $size) {
    Write-Host "[hash-gate] 不通过但**可解释**：客户区尺寸变了（$storedSize -> $size）。"
    Write-Host '              窗口尺寸受边框/标题栏影响，尺寸变了哈希必然变；这不是渲染回归。'
    Write-Host '              若确认当前画面正确，用 -Update 刷新基线。'
    exit 2
}

Write-Host "[hash-gate] 不通过：客户区尺寸相同（$size），但哈希变了（$storedHash -> $hash）。"
Write-Host '              这是**渲染回归**，请先排查。'
exit 1
