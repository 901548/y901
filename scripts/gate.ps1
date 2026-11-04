# Unified gate: MSRV + tests(count) + clippy + pixel-hash + docs + audit.
#
# Four ways this gate used to lie, all fixed here:
#   1. audit missing   -> said "skipped" and still printed GATE OK;
#   2. audit DB down   -> reported "found vulnerabilities" (wrong reason) or was silently skipped;
#   3. clippy          -> never checked $LASTEXITCODE, so a compile error could still look clean;
#   4. tests           -> only looked for "test result: ok", so a test that got #[ignore]d just
#                         vanished from the count and the gate stayed green.
#
# Usage:
#   pwsh -File scripts/gate.ps1                 # full gate (use this before a tag)
#   pwsh -File scripts/gate.ps1 -SkipTest       # fast local loop
#   pwsh -File scripts/gate.ps1 -SkipAudit      # offline only; NOT a release gate

# -SkipGpu allows the D3D probes to skip when no GPU exists. NOT the default: "did not run" must
# never quietly count as "passed".
param(
    [switch]$SkipTest,
    [switch]$SkipAudit,
    [switch]$SkipGpu
)

# cargo writes progress to stderr; 'Stop' would turn that into a fatal NativeCommandError.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$baselineTests = Join-Path $root 'docs\evidence\test-baseline.txt'
# 审计是否降级到了缓存指南库（影响最终结论的措辞）
$script:auditStale = $false
# 有多少个 D3D 探针因为没 GPU 而跳过（影响最终结论的措辞）
$script:gpuSkipped = 0

function Step($name) { Write-Host "[gate] $name ..." }
function Fail($msg) { Write-Host "[gate] FAIL: $msg"; exit 1 }

# ---------------------------------------------------------------- 1. MSRV
Step "MSRV (rustc >= 1.92)"
$ver = (& rustc --version) -join ' '
if ($ver -notmatch 'rustc 1\.(9[2-9]|[1-9][0-9]{2})') { Fail $ver }
Write-Host "[gate] ok: $ver"

# ---------------------------------------------------------------- 2. tests (count against baseline)
if (-not $SkipTest) {
    Step "cargo test --workspace (count must not drop below baseline)"
    if (-not (Test-Path $baselineTests)) {
        Fail "test baseline missing: $baselineTests"
    }
    # 基线文件允许写注释（`#` 开头）：取第一个非注释、非空行作为数字
    $baselineLine = (Get-Content $baselineTests | Where-Object { $_.Trim() -ne '' -and -not $_.Trim().StartsWith('#') } | Select-Object -First 1)
    if (-not $baselineLine) { Fail "test baseline has no number: $baselineTests" }
    $expected = [int]$baselineLine.Trim()
    $skipLog = Join-Path ([System.IO.Path]::GetTempPath()) 'y901-test-skips.txt'
    Remove-Item $skipLog -ErrorAction SilentlyContinue
    $testOut = & cargo test --workspace --offline 2>&1 | Out-String
    $skipCount = 0
    if (Test-Path $skipLog) {
        $skipCount = (Get-Content $skipLog | Where-Object { $_.Trim().StartsWith('SKIP:') } | Measure-Object).Count
    }

    if ($testOut -match 'error\[E\d+\]' -or $testOut -match 'could not compile') {
        Write-Host $testOut
        Fail "workspace does not compile"
    }
    $passed = 0
    $failed = 0
    foreach ($m in [regex]::Matches($testOut, 'test result: \w+\. (\d+) passed; (\d+) failed')) {
        $passed += [int]$m.Groups[1].Value
        $failed += [int]$m.Groups[2].Value
    }
    Write-Host "[gate] passed=$passed failed=$failed baseline=$expected"
    if ($failed -gt 0) { Fail "$failed test(s) failed" }
    if ($passed -lt $expected) {
        Fail "test count dropped: expected >= $expected, got $passed (a test was removed or #[ignore]d)"
    }
    if ($skipCount -gt 0) {
        if (-not $SkipGpu) {
            Fail "$skipCount test(s) were SKIPPED (D3D device missing?). Re-run on a GPU machine, or pass -SkipGpu to allow it."
        }
        $script:gpuSkipped = $skipCount
        Write-Host "[gate] WARNING: $skipCount test(s) SKIPPED (-SkipGpu): they asserted nothing this run"
    }
    Write-Host "[gate] ok: $passed tests (>= baseline $expected), 0 failed"
}
else {
    Write-Host "[gate] skipped: cargo test (-SkipTest)"
}

# ---------------------------------------------------------------- 3. clippy (exit code FIRST, then count)
Step "cargo clippy (exit code must be 0 AND warnings must be 0)"
$clippyJson = & cargo clippy --workspace --all-targets --offline --message-format=json 2>&1 | Out-String
$clippyCode = $LASTEXITCODE
if ($clippyCode -ne 0) {
    Write-Host $clippyJson
    Fail "clippy exited $clippyCode (compile error or lint denied)"
}
$warnCount = ([regex]::Matches($clippyJson, '"level":"warning"')).Count
if ($warnCount -gt 0) {
    ($clippyJson -split "`n" | Select-String '"level":"warning"' | Select-Object -First 5) | ForEach-Object { Write-Host $_ }
    Fail "clippy reported $warnCount warnings"
}
Write-Host "[gate] ok: exit 0, 0 warnings"

# ---------------------------------------------------------------- 4. pixel hash (committed baseline)
Step "pixel hash gate (baseline is committed; missing baseline is a failure)"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'hash-gate.ps1') | Out-Host
$hashCode = $LASTEXITCODE
if ($hashCode -ne 0) {
    Fail "hash gate exit $hashCode (1 = render regression, 2 = client area changed, 3 = baseline missing)"
}
Write-Host "[gate] ok: hash matches committed baseline"

# ---------------------------------------------------------------- 5. docs
Step "missing_docs (must be 0)"
$env:RUSTFLAGS = '-W missing_docs'
$docOut = & cargo build --workspace --offline 2>&1 | Out-String
$docCode = $LASTEXITCODE
Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
if ($docCode -ne 0) {
    Write-Host $docOut
    Fail "build under -W missing_docs exited $docCode"
}
$docCount = ([regex]::Matches($docOut, 'missing documentation')).Count
if ($docCount -gt 0) { Fail "$docCount items without docs" }
Write-Host "[gate] ok: 0 missing docs"

# ---------------------------------------------------------------- 6. security audit
Step "cargo audit (must actually run and report 0 vulnerabilities)"
if ($SkipAudit) {
    Write-Host "[gate] WARNING: audit skipped by -SkipAudit -- this run is NOT a release gate"
}
elseif (-not (Get-Command cargo-audit -ErrorAction SilentlyContinue)) {
    Fail "audit SKIPPED: cargo-audit not installed. Install: cargo install cargo-audit"
}
else {
    $auditOut = & cargo audit 2>&1 | Out-String
    $auditCode = $LASTEXITCODE
    Write-Host $auditOut
    if ($auditCode -ne 0 -and ($auditOut -match "couldn't fetch advisory database" -or $auditOut -match "failed to prepare fetch")) {
        # 拉不到最新指南库 ≠ 无法审计：本机已有缓存副本时可以用 --no-fetch 审一遍，
        # 但必须**明说指南库可能过时**——不把"没拉到最新"伪装成"已审计到最新"。
        Write-Host "[gate] WARNING: advisory DB refresh failed; retrying against the CACHED database"
        $auditOut = & cargo audit --no-fetch 2>&1 | Out-String
        $auditCode = $LASTEXITCODE
        Write-Host $auditOut
        if ($auditCode -eq 0) {
            $script:auditStale = $true
            Write-Host "[gate] ok: audit ran against CACHED advisory DB (may be stale; re-run online for a release)"
        }
    }
    if ($auditCode -ne 0) {
        Fail "audit FAILED: cargo-audit exited $auditCode (no usable advisory database)"
    }
    if ($auditCode -eq 0) {
        Write-Host "[gate] ok: 0 vulnerabilities"
    }
}

if ($script:auditStale -and $script:gpuSkipped -gt 0) {
    Write-Host "GATE OK (STALE AUDIT + $($script:gpuSkipped) SKIPPED: not a release gate)"
}
elseif ($script:gpuSkipped -gt 0) {
    Write-Host "GATE OK (SKIPPED: $($script:gpuSkipped) D3D tests)"
}
elseif ($script:auditStale) {
    # 明确区分：这不是"完整门禁通过"，而是"审计基于可能过时的缓存库通过"。
    Write-Host "GATE OK (STALE AUDIT: cached advisory DB, re-run online for a release)"
}
else {
    Write-Host "GATE OK"
}
exit 0
