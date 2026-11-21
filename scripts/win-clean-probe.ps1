# 探针：Windows 上"只装 Rust 能不能构建"到底依赖什么。
#
# 背景：文档长期写"本机装了 VS 2022，所以证明不了" —— 这句话本身没被测量过。
# 本脚本把它拆成四个可测的量，逐项打印证据：
#   1. 机器上**实际存在**什么：VS 安装目录 / Windows SDK / vswhere / link.exe / cl / VC 库
#   2. 干净环境（清空 LIB/INCLUDE/VC*/WindowsSdkDir，PATH 只留 rustup + System32）里，
#      用工具链自带的 rust-lld 构建整个 workspace（独立 target 目录，强制重新链接）
#   3. 产物的 PE 导入表：运行时到底要不要 VC 运行库（VCRUNTIME140.dll / api-ms-win-crt-*）
#   4. 同一份代码用 `+crt-static` 再构建一次，看导入表是否变成"零 CRT 依赖"
#
# 判定口径（写死在输出里，避免"各人各读"）：
#   BUILD=RUST-ONLY  ⇔ 干净环境构建成功
#   RUNTIME=NEEDS-VCREDIST ⇔ 默认产物导入 VCRUNTIME140.dll
#   RUNTIME=SELF-CONTAINED ⇔ +crt-static 产物没有 CRT 导入
#
# 用法：powershell -NoProfile -File scripts/win-clean-probe.ps1 [-SkipStatic]
# 退出码：0 = 干净环境构建成功；1 = 失败（或探针无法判定）。
param(
    [string]$Target = 'target\clean-probe',
    [switch]$SkipStatic
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
# 离线/在线：CI 上必须联网（`$env:Y901_ONLINE=1` 由 scripts/gate.ps1 -Online 设置），
# 本机默认离线（依赖已在缓存里，避免每次跑都摸网络）。
if ($env:Y901_ONLINE -eq '1') { $script:cargoOffline = @() } else { $script:cargoOffline = @('--offline') }
Push-Location $root
$failed = $false
try {
    Write-Host "=== 1. 机器上实际有什么 ==="
    rustc -vV | ForEach-Object { Write-Host "    $_" }
    $sysroot = (rustc --print sysroot).Trim()
    $lld = Join-Path $sysroot 'lib\rustlib\x86_64-pc-windows-msvc\bin\rust-lld.exe'
    Write-Host "    rust-lld: $(if (Test-Path $lld) { $lld } else { '<缺失>' })"
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    $vsPath = if (Test-Path $vswhere) { (& $vswhere -latest -property installationPath) } else { '' }
    Write-Host "    vswhere 报告的 VS 安装路径: $(if ($vsPath) { $vsPath } else { '<空：没有完整的 VS 实例>' })"
    foreach ($k in 'VSINSTALLDIR', 'VCINSTALLDIR', 'WindowsSdkDir', 'VCToolsInstallDir') {
        Write-Host "    $k = [$([Environment]::GetEnvironmentVariable($k))]"
    }
    foreach ($p in 'C:\Program Files\Microsoft Visual Studio', 'C:\Program Files (x86)\Windows Kits\10') {
        Write-Host "    $p 存在: $(Test-Path $p)"
    }
    $sdkVer = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\Lib' -Directory -ErrorAction SilentlyContinue |
        Select-Object -First 1 -ExpandProperty Name
    Write-Host "    Windows 10 SDK 版本目录: $(if ($sdkVer) { $sdkVer } else { '<无：没有安装 Win10/11 SDK>' })"
    foreach ($t in 'link.exe', 'cl.exe') {
        $c = Get-Command $t -ErrorAction SilentlyContinue
        Write-Host "    $t 在 PATH: $(if ($c) { $c.Source } else { '<不在>' })"
    }

    Write-Host ""
    Write-Host "=== 2. 干净环境构建（rust-lld，独立 target 目录）==="
    # 只保留 rustup + System32：即使 VS 装有 link.exe，也不让它在 PATH 上。
    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:SystemRoot\System32;$env:SystemRoot"
    $env:LIB = ''; $env:INCLUDE = ''
    $env:VCINSTALLDIR = ''; $env:VCToolsInstallDir = ''; $env:WindowsSdkDir = ''; $env:VSINSTALLDIR = ''
    $env:RUSTFLAGS = '-C linker=rust-lld'
    $buildArgs = @('build', '--release', '--workspace', '--examples', '--target-dir', $Target) + $cargoOffline
$log = & cargo @buildArgs 2>&1 | Out-String
    $code = $LASTEXITCODE
    Write-Host "    build exit=$code"
    if ($code -ne 0) {
        Write-Host $log
        $failed = $true
    }

    function Get-PeImports([string]$path) {
        $b = [System.IO.File]::ReadAllBytes($path)
        $peOff = [BitConverter]::ToInt32($b, 0x3C)
        if ([System.Text.Encoding]::ASCII.GetString($b, $peOff, 4) -ne "PE`0`0") { return @() }
        $optOff = $peOff + 24
        $magic = [BitConverter]::ToUInt16($b, $optOff)
        $ddOff = if ($magic -eq 0x20B) { $optOff + 112 } else { $optOff + 96 }
        $importRva = [BitConverter]::ToInt32($b, $ddOff + 8)
        $nSec = [BitConverter]::ToUInt16($b, $peOff + 6)
        $optSize = [BitConverter]::ToUInt16($b, $peOff + 20)
        $secOff = $optOff + $optSize
        $sections = @()
        for ($i = 0; $i -lt $nSec; $i++) {
            $o = $secOff + $i * 40
            $sections += [pscustomobject]@{
                Va    = [BitConverter]::ToUInt32($b, $o + 12)
                VSize = [BitConverter]::ToUInt32($b, $o + 8)
                Raw   = [BitConverter]::ToUInt32($b, $o + 20)
            }
        }
        function RvaToOff([int]$rva) {
            foreach ($s in $sections) {
                if ($rva -ge $s.Va -and $rva -lt ($s.Va + $s.VSize)) {
                    return [int]($s.Raw + ($rva - $s.Va))
                }
            }
            return -1
        }
        $names = @()
        $desc = RvaToOff $importRva
        if ($desc -lt 0) { return @() }
        while ($true) {
            $nameRva = [BitConverter]::ToInt32($b, $desc + 12)
            if ($nameRva -eq 0) { break }
            $no = RvaToOff $nameRva
            if ($no -lt 0) { break }
            $end = $no
            while ($b[$end] -ne 0) { $end++ }
            $names += [System.Text.Encoding]::ASCII.GetString($b, $no, $end - $no)
            $desc += 20
        }
        return $names | Sort-Object -Unique
    }

    function Show-Crt([string]$label, [string]$exe) {
        if (-not (Test-Path $exe)) { Write-Host "    ${label}: <没有产物 $exe>"; return $null }
        $imports = Get-PeImports $exe
        $crt = @($imports | Where-Object { $_ -match '(?i)VCRUNTIME|MSVCP|ucrtbase|api-ms-win-crt' })
        Write-Host "    ${label}：$(Split-Path $exe -Leaf)（$((Get-Item $exe).Length) 字节）"
        Write-Host "        导入 DLL 数 $($imports.Count)；CRT 相关：$(if ($crt.Count -gt 0) { $crt -join ', ' } else { '<无>' })"
        return $crt
    }

    Write-Host ""
    Write-Host "=== 3. 默认产物的运行时依赖 ==="
    $exeDefault = Join-Path $root "$Target\release\examples\hello.exe"
    $crtDefault = Show-Crt '默认（动态 CRT）' $exeDefault

    if (-not $SkipStatic) {
        Write-Host ""
        Write-Host "=== 4. +crt-static 对照 ==="
        $env:RUSTFLAGS = '-C linker=rust-lld -C target-feature=+crt-static'
        $staticTarget = "$Target-static"
        $staticArgs = @('build', '-p', 'y901-shell', '--example', 'hello', '--release', '--target-dir', $staticTarget) + $cargoOffline
$slog = & cargo @staticArgs 2>&1 | Out-String
        $scode = $LASTEXITCODE
        Write-Host "    静态 CRT build exit=$scode"
        if ($scode -ne 0) {
            Write-Host $slog
            $failed = $true
        }
        else {
            $crtStatic = Show-Crt '静态 CRT' (Join-Path $root "$staticTarget\release\examples\hello.exe")
        }
    }

    Write-Host ""
    Write-Host "=== 结论 ==="
    if ($failed) {
        Write-Host "BUILD=NEEDS-EXTERNAL-TOOLCHAIN（干净环境构建失败，详见上面的 stderr）"
        exit 1
    }
    Write-Host "BUILD=RUST-ONLY（干净环境 + rust-lld 构建成功；本机实测，干净 VM 仍需外部执行一次）"
    if ($null -eq $crtDefault) {
        Write-Host "RUNTIME=UNKNOWN（没有找到默认产物，探针失效，不能据此下结论）"
        exit 1
    }
    if ($crtDefault.Count -gt 0) {
        Write-Host "RUNTIME=NEEDS-VCREDIST（默认产物导入 $($crtDefault -join ', ')）"
        Write-Host "        若要发布成「解压即用」，请用 -C target-feature=+crt-static 构建（见第 4 步对照）"
    }
    else {
        Write-Host "RUNTIME=SELF-CONTAINED（默认产物没有 CRT 导入）"
    }
    exit 0
}
finally {
    Pop-Location
}
