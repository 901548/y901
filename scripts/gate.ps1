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
    [switch]$AllowStaleAudit,
    # CI / 首次构建没有本地 cargo 缓存时用 -Online：去掉所有 --offline。
    [switch]$Online
)

# 本机默认离线（依赖已在缓存里）。CI 上必须 -Online，否则 cargo 解析不到 registry。
# 子脚本（inventory / hash / sync）也读这个环境变量，一处设置全局生效。
if ($Online) { $script:cargoOffline = @() } else { $script:cargoOffline = @('--offline') }
if ($Online) { $env:Y901_ONLINE = '1' }

# cargo writes progress to stderr; 'Stop' would turn that into a fatal NativeCommandError.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
$baselineTests = Join-Path $root 'docs\evidence\test-baseline.txt'
# 审计是否降级到了缓存指南库（影响最终结论的措辞）
$script:auditStale = $false
# 审计降级痕迹：null = 联网审计；数字 = 用了 N 天前的缓存快照。
# 为什么要暴露到最终结论里：旧版在缓存 ≤7 天时直接打印 GATE OK，
# 从结论上看不出"这次没联网"，发布记录会误以为审计是联网的。
$script:auditCacheDays = $null
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

# ---------------------------------------------------------------- 1c. script encoding
# 为什么要有这一步：本仓库的脚本都含中文注释，而 `powershell`（Windows PowerShell 5.1）
# 对**没有 BOM 的 UTF-8** 会按 ANSI/GBK 解码 —— 中文注释会"吃掉"后面的 ASCII 字符，
# 报出与真实原因毫无关系的语法错误（实测：编辑脚本时 BOM 被去掉，gate 直接
# `Missing expression after ','` 崩在第 21 行的 param 块）。这类失败极难自诊，
# 所以在门禁里直接判死，并给出怎么修。
Step "PowerShell script encoding (non-ASCII scripts need a UTF-8 BOM)"
$badEnc = @()
foreach ($f in (Get-ChildItem (Join-Path $PSScriptRoot '*.ps1') -File)) {
    $b = [System.IO.File]::ReadAllBytes($f.FullName)
    $hasBom = ($b.Length -ge 3 -and $b[0] -eq 0xEF -and $b[1] -eq 0xBB -and $b[2] -eq 0xBF)
    if ($hasBom) { continue }
    foreach ($x in $b) { if ($x -gt 0x7F) { $badEnc += $f.Name; break } }
}
if ($badEnc.Count -gt 0) {
    Write-Host ("带非 ASCII 但没有 BOM：" + ($badEnc -join ', '))
    Write-Host "修法：用 UTF-8 with BOM 重新保存（[System.Text.UTF8Encoding]::new(\$true)），不要用无 BOM 的 UTF-8。"
    Fail "$($badEnc.Count) 个脚本缺少 UTF-8 BOM，Windows PowerShell 5.1 会按 ANSI 解码并解析失败"
}
Write-Host "[gate] ok: script encodings safe"

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
    $testArgs = @('test', '--workspace', '--no-fail-fast') + $cargoOffline
    $testOut = & cargo @testArgs 2>&1 | Out-String
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

# ---------------------------------------------------------------- 2b. SKIP 留痕（静态检查）
# 为什么必须有这一步：GPU 用例在没有设备时会"留痕跳过"，约定是**两件事**——
# 打印 `SKIP:` 到 stderr，并写进 `%TEMP%\y901-test-skips.txt`（门禁删文件→跑测试→数行数）。
# 问题在于 `cargo test` 默认**捕获**输出，门禁只能看到失败用例的 stderr，所以"只打印
# 不写文件"的跳过是**完全不可见**的：它被算进 passed，实际一个断言都没执行。
# 实测 5 处跳过点里有 1 处（`window.rs` 的 sync 断言）就漏了文件写入。
# 这里做静态检查：每个 printing SKIP 的位置附近必须出现文件路径。
Step "SKIP bookkeeping (every 'SKIP:' print must also write the marker file)"
$skipBad = @()
$rsFiles = Get-ChildItem (Join-Path $root 'crates') -Recurse -Filter '*.rs' -File
foreach ($f in $rsFiles) {
    $lines = @([System.IO.File]::ReadAllLines($f.FullName))
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -notmatch 'eprintln!\(\s*"SKIP') { continue }
        $from = [Math]::Max(0, $i - 25)
        $to = [Math]::Min($lines.Count - 1, $i + 25)
        $near = ($lines[$from..$to] -join "`n")
        if ($near -notmatch 'y901-test-skips\.txt') {
            $skipBad += ("{0}:{1}" -f $f.FullName.Substring($root.Length + 1), ($i + 1))
        }
    }
}
if ($skipBad.Count -gt 0) {
    Write-Host ("打印 SKIP 但没有写标记文件：" + ($skipBad -join ', '))
    Fail "SKIP 未留痕：这些测试在没有 GPU 等条件下会静默通过（门禁看不见），请改写成同时写 y901-test-skips.txt"
}
Write-Host "[gate] ok: every SKIP print also records the marker file"
# ---------------------------------------------------------------- 3b. 文档相对链接
# 为什么要有：`.md` 里的相对链接是用户第一手入口，断链在 GitHub 上就是 404，
# 而任何测试都不会发现。实测 README.zh-CN.md 的 12 条 "docs/xxx" 链接 href 全写成了
# 裸文件名（少了 `docs/` 前缀）—— 整段文档索引全是死链，长期没人发现。
Step "markdown relative links resolve (no 404s in the docs index)"
$mdBad = @()
$mdFiles = Get-ChildItem (Join-Path $root '*.md'), (Join-Path $root 'docs'), (Join-Path $root 'crates') -Recurse -Filter '*.md' -File -ErrorAction SilentlyContinue
foreach ($f in $mdFiles) {
    if ($f.FullName -match '\\target\\') { continue }
    $txt = [System.IO.File]::ReadAllText($f.FullName)
    foreach ($m in [regex]::Matches($txt, '\]\(([^)#]+?)(#[^)]*)?\)')) {
        $link = $m.Groups[1].Value
        if ($link -match '^(https?:|mailto:|#)') { continue }
        if (-not (Test-Path (Join-Path $f.DirectoryName $link))) {
            $mdBad += ("{0} -> {1}" -f $f.FullName.Substring($root.Length + 1), $link)
        }
    }
}
if ($mdBad.Count -gt 0) {
    ($mdBad | Select-Object -First 15) | ForEach-Object { Write-Host "  broken: $_" }
    Fail "$($mdBad.Count) markdown link(s) point at files that do not exist"
}
Write-Host "[gate] ok: markdown links resolve"
Step "cargo clippy (exit code must be 0 AND warnings must be 0)"
$clippyArgs = @('clippy', '--workspace', '--all-targets', '--message-format=json') + $cargoOffline
$clippyJson = & cargo @clippyArgs 2>&1 | Out-String
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
$hashGateArgs = @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'hash-gate.ps1'))
if ($SkipGpu) { $hashGateArgs += '-AllowNoGpu' }
& powershell @hashGateArgs | Out-Host
$hashCode = $LASTEXITCODE
if ($hashCode -ne 0) {
    Fail "hash gate exit $hashCode (1 = render regression, 2 = client area changed, 3 = baseline missing)"
}
Write-Host "[gate] ok: hash matches committed baseline"

# ---------------------------------------------------------------- 5. docs
Step "missing_docs (must be 0)"
$env:RUSTFLAGS = '-W missing_docs'
$docArgs = @('build', '--workspace') + $cargoOffline
$docOut = & cargo @docArgs 2>&1 | Out-String
$docCode = $LASTEXITCODE
Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue
if ($docCode -ne 0) {
    Write-Host $docOut
    Fail "build under -W missing_docs exited $docCode"
}
$docCount = ([regex]::Matches($docOut, 'missing documentation')).Count
if ($docCount -gt 0) { Fail "$docCount items without docs" }
Write-Host "[gate] ok: 0 missing docs"

# ---------------------------------------------------------------- 5b. rustdoc warnings
# 为什么单独一步：`missing_docs` 只保证"有文档"，不保证"文档里的链接是活的"。
# 0.3.0 发布前手工修掉 8 处 broken intra-doc link 与 1 处未闭合 HTML 标签 ——
# 它们都是 rustdoc 的 warning，而当时的 gate 完全看不见，只能靠人肉扫、且发版前才想起来。
# 用 `-D warnings` 让 rustdoc 直接失败，再用正则兜一层（防止环境吞掉 RUSTDOCFLAGS）。
Step "rustdoc intra-doc links and HTML (warnings must be 0)"
$env:RUSTDOCFLAGS = '-D warnings'
$rdArgs = @('doc', '--workspace', '--no-deps') + $cargoOffline
$rdOut = & cargo @rdArgs 2>&1 | Out-String
$rdCode = $LASTEXITCODE
Remove-Item Env:\RUSTDOCFLAGS -ErrorAction SilentlyContinue
if ($rdCode -ne 0) {
    ($rdOut -split "`n" | Select-String -Pattern 'warning:|error' | Select-Object -First 12) | ForEach-Object { Write-Host $_ }
    Fail "cargo doc exited $rdCode (broken intra-doc link or invalid HTML in doc comments)"
}
$rdWarn = ([regex]::Matches($rdOut, '(?m)^\s*warning:')).Count
if ($rdWarn -gt 0) {
    ($rdOut -split "`n" | Select-String -Pattern 'warning:' | Select-Object -First 12) | ForEach-Object { Write-Host $_ }
    Fail "rustdoc reported $rdWarn warning(s)"
}
Write-Host "[gate] ok: rustdoc clean"

# ---------------------------------------------------------------- version consistency
Step "version consistency (workspace / crates / internal deps / Cargo.lock)"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'check-version-consistency.ps1') | Out-Host
if ($LASTEXITCODE -ne 0) {
    Fail "version inconsistency (release bump touched a wrong manifest or Cargo.lock pin)"
}
Write-Host "[gate] ok: versions consistent"

# ---------------------------------------------------------------- doc numbers (appended)
Step "doc numbers match docs/evidence/numbers.txt"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'sync-doc-numbers.ps1') -Check | Out-Host
if ($LASTEXITCODE -ne 0) {
    Write-Host "[gate] HINT: run 'pwsh -File scripts/sync-doc-numbers.ps1' to fix doc numbers"
    Fail "doc numbers out of sync"
}
Write-Host "[gate] ok: doc numbers in sync"

# ---------------------------------------------------------------- doc-number checker self-test
# 为什么门禁要跑"检查器的检查"：本轮之前 `-Check` 只认加粗写法，README 里非加粗的
# `978 passed ... 149 个测试目标` 一路逃检。检查器本身必须有覆盖证明，
# 否则"doc numbers in sync"只是"检查器没看见"的同义词。
Step "doc-number checker self-test (mutation coverage)"
& powershell -NoProfile -File (Join-Path $PSScriptRoot 'test-doc-numbers-check.ps1') | Out-Host
if ($LASTEXITCODE -ne 0) {
    Fail "doc-number checker does not catch drift (mutation self-test failed)"
}
Write-Host "[gate] ok: doc-number checker catches injected drift"

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
                $script:auditCacheDays = $dbAgeDays
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
    $cacheNote = if ($null -ne $script:auditCacheDays) { (" cached_db_days={0:N1}" -f $script:auditCacheDays) } else { "" }
    $auditContent = "=== audit @ $(Get-Date -Format o) exit=$auditCode stale=$($script:auditStale)$cacheNote ===`n$auditOut"
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
if ($null -ne $script:auditCacheDays) {
    # 缓存快照仍在有效期内，但**结论必须写明"这次没联网"**，否则发布记录会误读。
    Write-Host ("GATE OK (CACHED DB {0:N1}d; offline audit)" -f $script:auditCacheDays)
    exit 0
}
Write-Host "GATE OK"


exit 0
