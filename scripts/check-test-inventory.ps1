# 每个二进制的测试名指纹校验（逐 exe 独立 --list + 超时保护 + 守恒检查）。
#
# 为什么逐 exe：cargo 的进度行在 2>&1 下与测试名交错，按 Running 行分组会静默错分。
# 为什么超时：某些 exe（D3D 相关）在无 GPU 的机器上 `--list` 会**挂起等设备**，
# 整个脚本会无限等待——这是"工具撒谎"的又一种形态：看起来像没输出。
param([switch]$Update)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$inv = Join-Path $root 'docs\evidence\test-inventory.txt'

function Invoke-ListWithTimeout {
    param([string]$exePath, [int]$timeoutSec = 10)
    # 用**临时文件**重定向，而不用管道：
    # 管道有 4KB 缓冲区，输出超过就会子进程写满阻塞、父进程等不到退出。
    # 写文件不存在这个问题。
    $tmpOut = [System.IO.Path]::GetTempFileName()
    $tmpErr = [System.IO.Path]::GetTempFileName()
    try {
        $proc = Start-Process -FilePath $exePath -ArgumentList '--list' `
            -RedirectStandardOutput $tmpOut -RedirectStandardError $tmpErr `
            -NoNewWindow -PassThru
        if (-not $proc.WaitForExit($timeoutSec * 1000)) {
            try { $proc.Kill() } catch {}
            Write-Host "[inv] TIMEOUT: $(Split-Path $exePath -Leaf) did not respond within ${timeoutSec}s"
            return $null
        }
        return Get-Content $tmpOut -Raw
    } finally {
        Remove-Item $tmpOut, $tmpErr -ErrorAction SilentlyContinue
    }
}

# 只取**测试**二进制：`cargo test --no-run` 会把 example 也编出来（hello.exe / y901.exe …），
# 它们不认识 `--list`，会直接运行应用并卡住——这正是上一版“挂起”的真正原因。
# 每条 JSON 一行，只取 `"test":true` 的那些。
$raw = & cargo test --workspace --offline --no-run --message-format json 2>&1 | Out-String
$exes = @()
foreach ($line in ($raw -split "`n")) {
    if ($line -notmatch '"executable":"([^"]+)"') { continue }
    $exe = $matches[1]
    # 同一条 JSON 里必须有 test=true 且不是 example/bench 目标
    if ($line -notmatch '"test":true') { continue }
    if ($line -match '"kind":\["example"\]') { continue }
    # bin 目标也会被编成测试 harness，但它启动后会直接运行应用（并卡住）
    if ($line -match '"kind":\["bin"\]') { continue }
    $exes += $exe
}
$exes = $exes | Sort-Object -Unique

$groups = @{}
$total = 0
$timeouts = @()
foreach ($exe in $exes) {
    if (-not (Test-Path $exe)) { continue }
    $out = Invoke-ListWithTimeout $exe
    if ($null -eq $out) { $timeouts += (Split-Path $exe -Leaf); continue }
    $stem = [System.IO.Path]::GetFileNameWithoutExtension($exe)
    $idx = $stem.LastIndexOf('-')
    $bin = if ($idx -gt 0) { $stem.Substring(0, $idx) } else { $stem }
    $names = @()
    foreach ($line in ($out -split "`n")) {
        $t = $line.Trim()
        if ($t -match '^(.+): test$') { $names += $matches[1] }
    }
    if ($names.Count -eq 0) { continue }
    if ($groups.ContainsKey($bin)) { $groups[$bin] += $names } else { $groups[$bin] = $names }
    $total += $names.Count
}

function Hash-Names($names) {
    $joined = (($names | Sort-Object) -join "`n")
    $sha = [System.Security.Cryptography.SHA256]::Create()
    (($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($joined)) | ForEach-Object { $_.ToString('x2') }) -join '')
}

if ($timeouts.Count -gt 0) {
    Write-Host "[inv] PARSE ERROR: $($timeouts.Count) exe(s) timed out and cannot be fingerprinted:"
    $timeouts | ForEach-Object { Write-Host "  - $_" }
    Write-Host "[inv] (超时不能当成 “0 个测试”，否则会误报“内容变了”)"
    exit 1
}

if ($Update) {
    $lines = @('# Per-binary test fingerprint (Windows platform).',
               '# format: <binary> <count> <sha256 of sorted test names>',
               '# Regenerate: pwsh -File scripts/check-test-inventory.ps1 -Update',
               '')
    foreach ($b in ($groups.Keys | Sort-Object)) {
        $lines += ("{0} {1} {2}" -f $b, $groups[$b].Count, (Hash-Names $groups[$b]))
    }
    [System.IO.File]::WriteAllText($inv, (($lines -join "`n") + "`n"), (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "[inv] wrote $($groups.Count) binaries / $total tests to test-inventory.txt"
    exit 0
}

if (-not (Test-Path $inv)) { Write-Host "[inv] FAIL: test-inventory.txt missing"; exit 1 }
$bad = 0
foreach ($line in (Get-Content $inv)) {
    if ($line.StartsWith('#') -or $line.Trim() -eq '') { continue }
    $p = $line -split ' '
    $bin = $p[0]; $want = [int]$p[1]; $wantHash = $p[2]
    if (-not $groups.ContainsKey($bin)) { Write-Host "[inv] FAIL: binary $bin missing"; $bad++; continue }
    $names = $groups[$bin]
    if ($names.Count -ne $want) { Write-Host "[inv] FAIL: ${bin}: expected $want tests, got $($names.Count)"; $bad++; continue }
    if ((Hash-Names $names) -ne $wantHash) { Write-Host "[inv] FAIL: ${bin}: test names changed"; $bad++ }
}
$sum = 0
foreach ($k in $groups.Keys) { $sum += $groups[$k].Count }
if ($sum -ne $total) { Write-Host "[inv] PARSE ERROR: grouped $sum but listed $total"; exit 1 }
if ($bad -gt 0) { Write-Host "[inv] $bad mismatch(es) (parse check ok: sum=$sum == listed=$total)"; exit 1 }
Write-Host "[inv] ok: $($groups.Count) binaries match test-inventory.txt (parse check: sum=$sum == listed=$total)"
exit 0
