# Unified gate: MSRV + tests + clippy + pixel-hash + docs completeness.
#
# Why this script exists: until now the "four-condition gate" was three commands typed by hand
# every time, and the clippy one used a regex that silently missed warnings whose text starts with
# a backtick. A gate you retype is a gate you eventually mistype.
#
# Usage:
#   pwsh -File scripts/gate.ps1              # full gate
#   pwsh -File scripts/gate.ps1 -SkipTest    # fast local loop (skips cargo test)

param(
    [switch]$SkipTest
)

# cargo writes progress to stderr; 'Stop' would turn that into a fatal NativeCommandError.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$failed = @()

function Step($name) { Write-Host "[gate] $name ..." }

# ---------------------------------------------------------------- 1. MSRV
Step "MSRV (rustc >= 1.92)"
$ver = (& rustc --version) -join ' '
if ($ver -notmatch 'rustc 1\.(9[2-9]|[1-9][0-9]{2})') {
    Write-Host "[gate] FAIL: $ver"
    exit 1
}
Write-Host "[gate] ok: $ver"

# ---------------------------------------------------------------- 2. tests
if (-not $SkipTest) {
    Step "cargo test --workspace"
    $testOut = & cargo test --workspace --offline 2>&1 | Out-String
    if ($testOut -notmatch 'test result: ok') {
        Write-Host "[gate] FAIL: no 'test result: ok' in output"
        Write-Host $testOut
        exit 1
    }
    $failedTests = [regex]::Matches($testOut, '(\d+) failed')
    foreach ($m in $failedTests) {
        if ([int]$m.Groups[1].Value -gt 0) {
            Write-Host "[gate] FAIL: $($m.Value)"
            exit 1
        }
    }
    Write-Host "[gate] ok: all test binaries reported 0 failed"
}
else {
    Write-Host "[gate] skipped: cargo test (-SkipTest)"
}

# ---------------------------------------------------------------- 3. clippy (JSON, no text regex)
Step "cargo clippy (warnings must be 0)"
$clippyJson = & cargo clippy --workspace --all-targets --offline --message-format=json 2>&1 | Out-String
# Count only real diagnostic objects, not the human-readable lines around them.
$warnCount = ([regex]::Matches($clippyJson, '"level":"warning"')).Count
if ($warnCount -gt 0) {
    Write-Host "[gate] FAIL: clippy reported $warnCount warnings"
    # Show the first few for convenience.
    ($clippyJson -split "`n" | Select-String '"level":"warning"' | Select-Object -First 5) | ForEach-Object { Write-Host $_ }
    exit 1
}
Write-Host "[gate] ok: 0 warnings"

# ---------------------------------------------------------------- 4. pixel hash
Step "pixel hash gate (same-session baseline)"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'hash-gate.ps1') | Out-Host
$hashCode = $LASTEXITCODE
if ($hashCode -ne 0) {
    Write-Host "[gate] FAIL: hash gate exit $hashCode (2 = client area changed, 1 = render regression)"
    exit 1
}
Write-Host "[gate] ok: hash matches baseline"

# ---------------------------------------------------------------- 5. docs
Step "missing_docs (must be 0)"
$env:RUSTFLAGS = '-W missing_docs'
$docOut = & cargo build --workspace --offline 2>&1 | Out-String
Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
$docCount = ([regex]::Matches($docOut, 'missing documentation')).Count
if ($docCount -gt 0) {
    Write-Host "[gate] FAIL: $docCount items without docs"
    exit 1
}
Write-Host "[gate] ok: 0 missing docs"

# ---------------------------------------------------------------- 6. security audit
Step "cargo audit (no known vulnerabilities)"
if (Get-Command cargo-audit -ErrorAction SilentlyContinue) {
    $auditOut = & cargo audit 2>&1 | Out-String
    $auditCode = $LASTEXITCODE
    Write-Host $auditOut
    if ($auditCode -ne 0) {
        if ($auditOut -match "couldn't fetch advisory database" -or $auditOut -match "failed to prepare fetch" -or $auditOut -match "network") {
            # 指南库拉不到时**并不代表没有漏洞**，但也不能因为断网就把本地门禁卡死。
            # 这里明确标注“未完成审计”，并要求在联网环境/CI 重跑。
            Write-Host "[gate] audit UNAVAILABLE: advisory database could not be fetched (offline?)"
            Write-Host "[gate]   -> this is NOT a pass; re-run in a networked environment or CI"
        }
        else {
            Write-Host "[gate] FAIL: cargo audit reported a problem (exit $auditCode)"
            exit 1
        }
    }
    else {
        Write-Host "[gate] ok: no vulnerabilities (warnings such as 'unmaintained' are reported above)"
    }
}
else {
    Write-Host "[gate] skipped: cargo-audit not installed (cargo install cargo-audit)"
}

Write-Host "GATE OK"
exit 0
