# 版本一致性检查：工作区版本 / 各 crate / 内部 path 依赖约束 / Cargo.lock 必须一致。
#
# 为什么要单独一条：v0.2.3 发版时用"全局字符串替换"升版本，把 Cargo.lock 里
# 第三方 crate（termios / unicode-width）的 0.2.2 也改成了 0.2.3，
# checksum 没变 → `cargo check --offline` 直接解析失败。这类错误应当在门禁里立刻可见，
# 而不是等到构建时才炸（而且报错指向依赖图，很难联想到"发版脚本改错了锁文件"）。
#
# 用法：pwsh -File scripts/check-version-consistency.ps1
# 退出码：0 = 一致；1 = 有不一致（逐条打印）。
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$utf8 = New-Object System.Text.UTF8Encoding($false)
$failures = 0

function Fail($msg) {
    Write-Host "[version] FAIL: $msg"
    $script:failures++
}

# 1) 工作区版本
$workspaceToml = Join-Path $root 'Cargo.toml'
$text = [System.IO.File]::ReadAllText($workspaceToml, $utf8)
$m = [regex]::Match($text, '(?ms)\[workspace\.package\](.*?)(?:^\[|\z)')
if (-not $m.Success) { Fail "Cargo.toml 缺少 [workspace.package]" }
$vm = [regex]::Match($m.Groups[1].Value, '(?m)^\s*version\s*=\s*"([^"]+)"')
if (-not $vm.Success) { Fail "Cargo.toml 的 [workspace.package] 缺少 version" }
$version = if ($vm.Success) { $vm.Groups[1].Value } else { '?' }
Write-Host "[version] workspace version = $version"

# 2) 各 crate manifest：version.workspace = true（或与工作区一致），内部依赖约束一致
Get-ChildItem (Join-Path $root 'crates') -Directory | ForEach-Object {
    $manifest = Join-Path $_.FullName 'Cargo.toml'
    if (-not (Test-Path $manifest)) { return }
    $t = [System.IO.File]::ReadAllText($manifest, $utf8)
    $rel = "crates/$($_.Name)/Cargo.toml"

    $pkg = [regex]::Match($t, '(?ms)^\[package\](.*?)(?:^\[|\z)')
    if ($pkg.Success) {
        $own = [regex]::Match($pkg.Groups[1].Value, '(?m)^\s*version\s*=\s*"([^"]+)"')
        if ($own.Success) {
            if ($own.Groups[1].Value -ne $version) {
                Fail "${rel}: 自身 version=$($own.Groups[1].Value) 与工作区 $version 不一致"
            }
        } elseif ($pkg.Groups[1].Value -notmatch 'version\.workspace\s*=\s*true') {
            Fail "${rel}: 既没有 version，也没有 version.workspace = true"
        }
    } else {
        Fail "${rel}: 缺少 [package]"
    }

    # 内部 path 依赖（y901-*）的 version 约束必须等于工作区版本
    foreach ($dep in [regex]::Matches($t, '(y901-[\w-]+)\s*=\s*\{[^}]*path\s*=\s*"[^"]+"[^}]*\}')) {
        $line = $dep.Value
        $ver = [regex]::Match($line, 'version\s*=\s*"([^"]+)"')
        if ($ver.Success -and $ver.Groups[1].Value -ne $version) {
            Fail "${rel}: 内部依赖 $($dep.Groups[1].Value) 约束为 $($ver.Groups[1].Value)，应为 $version"
        }
    }
}

# 3) Cargo.lock：所有 y901-* 包版本必须等于工作区版本
$lockPath = Join-Path $root 'Cargo.lock'
$lock = [System.IO.File]::ReadAllText($lockPath, $utf8)
$count = 0
foreach ($pkg in [regex]::Matches($lock, '(?ms)\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = "([^"]+)"')) {
    $name = $pkg.Groups[1].Value
    $ver = $pkg.Groups[2].Value
    if ($name -like 'y901-*') {
        $count++
        if ($ver -ne $version) {
            Fail "Cargo.lock: $name 版本 $ver 与工作区 $version 不一致"
        }
    }
}
Write-Host "[version] checked $count y901-* lock entries"

if ($failures -gt 0) {
    Write-Host "[version] $failures 处不一致"
    exit 1
}
Write-Host "[version] ok: workspace/crates/lock 版本一致（$version）"
exit 0
