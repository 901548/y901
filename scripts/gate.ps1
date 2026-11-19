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
    [switch]$SkipGpu,
    [switch]$AllowStaleAudit
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

# ---------------------------------------------------------------- 1b. fmt (fast fail)
# 为何放在测试之前：格式检查约 1s，先跑能尽早失败，
# 不浪费后面几分钟的测试与审计。
Step "cargo fmt --all -- --check"
& cargo fmt --all -- --check 2>&1 | Out-Host
if ($LASTEXITCODE -ne 0) { Fail "格式检查失败：请运行 cargo fmt --all" }
Write-Host "[gate] ok: fmt clean"

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
    # --no-fail-fast：不得因一条失败就停调度后续二进制，否则 passed 数会变成“半截值”
    # （曾出现 passed=335 而实际应为 746），且只能看到第一个失败。
    $testOut = & cargo test --workspace --offline --no-fail-fast 2>&1 | Out-String
    # 保存完整输出：否则失败用例名丢掉，下次只能重跑猜。
    $testOut | Out-File -FilePath (Join-Path $root 'target\gate-test.log') -Encoding utf8
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
Step "test inventory fingerprint (per-binary test names)"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'check-test-inventory.ps1') | Out-Host
if ($LASTEXITCODE -ne 0) { Fail "test inventory mismatch (see [inv] lines above)" }
Write-Host "[gate] ok: test inventory matches"

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

# ---------------------------------------------------------------- doc numbers (appended)
Step "doc numbers match docs/evidence/numbers.txt"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'sync-doc-numbers.ps1') -Check | Out-Host
if ($LASTEXITCODE -ne 0) {
    Write-Host "[gate] HINT: run 'pwsh -File scripts/sync-doc-numbers.ps1' to fix doc numbers"
    Fail "doc numbers out of sync"
}
Write-Host "[gate] ok: doc numbers in sync"

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
            # 拉取失败不等于"指南库很旧"：若本地库是**最近 7 天**更新的快照，仍是有效审计。
            # 为什么需要这条：GitHub 在部分网络下会间歇性拒绝 fetch，而"每次都被判 stale"
            # 会让发布门禁变成靠 -AllowStaleAudit 的橡皮图章（实测发生过）。
            $dbAgeDays = $null
            $dbDir = Join-Path $HOME ".cargo\advisory-db"
            if (Test-Path (Join-Path $dbDir ".git")) {
                $epoch = & git -C $dbDir log -1 --format=%ct 2>$null
                if ($epoch -and ($epoch -as [long])) {
                    $dbAgeDays = ((Get-Date).ToUniversalTime() - [DateTimeOffset]::FromUnixTimeSeconds([long]$epoch).UtcDateTime).TotalDays
                }
            }
            if ($null -ne $dbAgeDays -and $dbAgeDays -le 7) {
                $script:auditStale = $false
                Write-Host ("[gate] ok: audit ran against CACHED advisory DB snapshot {0:N1} day(s) old (fresh enough; offline refresh failed)" -f $dbAgeDays)
            }
            elseif (-not $AllowStaleAudit) {
                Fail "audit requires network or explicit -AllowStaleAudit (cached DB is older than 7 days or unknown)"
            }
            else {
                $script:auditStale = $true
                Write-Host "[gate] ok: audit ran against CACHED advisory DB (may be stale; re-run online for a release)"
            }
        }
    }
    if ($auditCode -ne 0) {
        Fail "audit FAILED: cargo-audit exited $auditCode (no usable advisory database)"
    }
    if ($auditCode -eq 0) {
        Write-Host "[gate] ok: 0 vulnerabilities"
    }
}

# --- audit log persistence (APPENDED: no existing line is deleted or moved) ---
if (-not (Test-Path variable:auditOut))  { $auditOut = $null }
if (-not (Test-Path variable:auditCode)) { $auditCode = -1 }

$auditLogPath = Join-Path (Split-Path $PSScriptRoot -Parent) "target\audit.log"
$auditDir = Split-Path $auditLogPath -Parent
if (-not (Test-Path $auditDir)) { New-Item -ItemType Directory -Force -Path $auditDir | Out-Null }

if ($SkipAudit) {
    $auditContent = "=== audit SKIPPED by -SkipAudit @ $(Get-Date -Format o) ==="
} else {
    $auditContent = "=== audit @ $(Get-Date -Format o) exit=$auditCode stale=$($script:auditStale) ===`n$auditOut"
}
[System.IO.File]::WriteAllText($auditLogPath, $auditContent, [System.Text.UTF8Encoding]::new($false))
if (-not (Test-Path $auditLogPath)) {
    Write-Host "[gate] FAIL: audit log write failed silently: $auditLogPath"
    exit 1
}
Write-Host "[gate] audit log: $auditLogPath"

$notReleasable = @()
if ($SkipTest)  { $notReleasable += "SKIP TEST" }
if ($SkipAudit) { $notReleasable += "SKIP AUDIT" }
if ($script:gpuSkipped -gt 0) { $notReleasable += "SKIP GPU: $($script:gpuSkipped) tests" }

# ---------------------------------------------------------------- platform-change warning
# 为何只 WARN 不硬失败：本 gate 跑在 Windows 上，`#[cfg(unix)]` 代码在这里**根本不编译**
# （x11 crate 整体为空），因此 gate 无法直接验证 Linux；但"不验证"不能被误读为"可发布"。
# 已因此返工三次（RenderOutcome 未导入 / create 签名漏改 / CreateArgs 漏 import）。
$changedFiles = @(& git diff --name-only HEAD 2>$null)
$platformChanged = @($changedFiles | Where-Object { $_ -match '^crates/y901-backend-(x11|win32)/.*\.rs$' })
if ($platformChanged.Count -gt 0) {
    Write-Host ""
    Write-Host "[gate] WARN: 本次改动含平台代码，Windows 绿灯不代表 Linux 能编："
    foreach ($f in $platformChanged) { Write-Host "  - $f" }
    Write-Host "[gate] WARN: 请在 Linux 上跑 scripts/linux-verify.sh（含 x11 check/golden/real_window 等）"
    Write-Host ""
}

if ($notReleasable.Count -gt 0) {
    # 主动跳过：不是发布门禁
    Write-Host "GATE NOT RELEASABLE ($($notReleasable -join '; '))"
    exit 2
}
if ($script:auditStale) {
    # 被动降级（离线，指南库取不到）：代码质量通过，但非发布级
    Write-Host "GATE OK (DEV: STALE AUDIT)"
    exit 0
}
Write-Host "GATE OK"


exit 0
