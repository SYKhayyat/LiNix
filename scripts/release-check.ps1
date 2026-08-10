# release-check.ps1 - the single "am I ready to ship?" gate for Windows.
#
# Runs the hermetic gates then the native Windows integration sweep and prints one go/no-go:
#   1. cargo clippy -D warnings   (HARD)
#      cargo test --release --no-fail-fast   (HARD)
#      cargo build --release       (HARD)
#      cargo fmt -- --check        (HARD - CI fails the build on it, so this must too)
#   2. scripts/integration-windows.sh - real install/list/remove for every backend installable
#      on this host (scoop + any bootstrapped ecosystem managers + winget/choco if present),
#      full feature coverage, and the self-checking coverage audit.
#
# Usage (from the repo root, in PowerShell):
#   ./scripts/release-check.ps1
#   ./scripts/release-check.ps1 -Backend scoop -Package jq        # choose the primary backend
#   ./scripts/release-check.ps1 -SkipIntegration                  # hermetic gates only
#
# The integration step needs Git-Bash (bash), which ships with Git for Windows. Run elevated to
# exercise choco/winget mutation; scoop alone needs no admin.
#
# NOTE: a bare `bash` on PATH is often scoop's *busybox* shim, whose bash cannot run this POSIX
# script (it fails with "Could not create process"). We therefore locate the real Git-for-Windows
# bash explicitly and only fall back to a PATH `bash` that is NOT busybox.
param(
    [string]$Backend = "scoop",
    [string]$Package = "jq",
    [string]$Package2 = "less",
    [switch]$SkipIntegration
)
$ErrorActionPreference = "Continue"
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

$results = @()
$hardFail = $false
function Pass($m) { Write-Host "[PASS] $m" -ForegroundColor Green; $script:results += "  PASS  $m" }
function Fail($m) { Write-Host "[FAIL] $m" -ForegroundColor Red;   $script:results += "  FAIL  $m"; $script:hardFail = $true }
function Info($m) { Write-Host "[INFO] $m" -ForegroundColor Yellow; $script:results += "  INFO  $m" }
function Step($m) { Write-Host "`n############### $m ###############" }

# Find a REAL bash (Git for Windows), never scoop's busybox shim.
function Find-Bash {
    $candidates = @(
        "$env:ProgramFiles\Git\bin\bash.exe",
        "${env:ProgramFiles(x86)}\Git\bin\bash.exe",
        "$env:LOCALAPPDATA\Programs\Git\bin\bash.exe",
        "$HOME\scoop\apps\git\current\bin\bash.exe",
        "$HOME\scoop\apps\git\current\usr\bin\bash.exe"
    )
    foreach ($c in $candidates) { if (Test-Path $c) { return $c } }
    # Derive it from git.exe's location (…\Git\cmd\git.exe -> …\Git\bin\bash.exe).
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($git) {
        $gitRoot = Split-Path (Split-Path $git.Source -Parent) -Parent
        $b = Join-Path $gitRoot "bin\bash.exe"
        if (Test-Path $b) { return $b }
    }
    # Last resort: a PATH bash that is NOT the busybox shim.
    $g = Get-Command bash -ErrorAction SilentlyContinue
    if ($g -and $g.Source -notmatch 'busybox') { return $g.Source }
    return $null
}

# ------------------------------------------------------------------ 1. hermetic
Step "1. HERMETIC GATES (cargo clippy / test / build / fmt)"

Write-Host "-> cargo fmt -- --check"
cargo fmt -- --check *> $null
if ($LASTEXITCODE -eq 0) { Pass "cargo fmt -- --check (formatting clean)" } else { Fail "cargo fmt -- --check reports diffs - run ``cargo fmt``" }

Write-Host "-> cargo clippy --all-targets --all-features -- -D warnings"
cargo clippy --all-targets --all-features -- -D warnings
if ($LASTEXITCODE -eq 0) { Pass "clippy: no warnings" } else { Fail "clippy reported warnings/errors" }

Write-Host "-> cargo test --release --no-fail-fast"
# --no-fail-fast because CI does: without it cargo stops at the first failing test TARGET and
# the rest of the suite goes unmeasured (G-4).
cargo test --release --no-fail-fast
if ($LASTEXITCODE -eq 0) { Pass "cargo test: all tests pass" } else { Fail "cargo test: failures" }

Write-Host "-> cargo build --release"
cargo build --release
if ($LASTEXITCODE -eq 0) { Pass "release build succeeds" } else { Fail "release build FAILED" }

# The `supply-chain` and `msrv` CI jobs, run locally. A CI job nothing local drives is a gate a
# developer finds out about from a red push, which is what grade6_gate_parity asserts against.
# Both are soft here and hard in CI: cargo-deny and a pinned toolchain are installs a contributor
# may not have, and a release script that refuses to run without them stops being run.
# Twin of the same block in release-check.sh - change one, change the other.
Write-Host "-> cargo deny check (advisories, bans, licences, sources)"
if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
    cargo deny check advisories bans licenses sources
    if ($LASTEXITCODE -eq 0) { Pass "cargo deny: clean" } else { Fail "cargo deny: findings - see above" }
} else {
    Info "cargo-deny not installed (cargo install cargo-deny --locked); CI runs it regardless"
}

# The `shell` CI job, run locally where the tool exists. Twin of the same block in
# release-check.sh - change one, change the other.
Write-Host "-> shellcheck (scripts/*.sh, docker/**/*.sh)"
if (Get-Command shellcheck -ErrorAction SilentlyContinue) {
    $shellScripts = @(Get-ChildItem -Path 'scripts' -Filter '*.sh' -File) +
                    @(Get-ChildItem -Path 'docker' -Filter '*.sh' -File -Recurse)
    shellcheck -S warning @($shellScripts.FullName)
    if ($LASTEXITCODE -eq 0) { Pass "shellcheck: clean" } else { Fail "shellcheck: findings - see above" }
} else {
    Info "shellcheck not installed (scoop install shellcheck); CI runs it regardless"
}

Write-Host "-> cargo check on the declared MSRV"
$msrvLine = Select-String -Path "Cargo.toml" -Pattern '^rust-version' | Select-Object -First 1
$msrv = if ($null -ne $msrvLine) { ($msrvLine.Line -split '"')[1] } else { $null }
if ($null -eq $msrv) {
    Fail "Cargo.toml declares no rust-version - the MSRV job has nothing to pin to"
} elseif ((rustup toolchain list) -match "^$msrv") {
    cargo "+$msrv" check --all-targets --locked
    if ($LASTEXITCODE -eq 0) { Pass "builds on the declared MSRV ($msrv)" }
    else { Fail "does NOT build on rust-version = $msrv - raise it deliberately or fix the use" }
} else {
    Info "toolchain $msrv not installed (rustup toolchain install $msrv); CI runs it regardless"
}

# CI runs this on every push and this gate did not run it at all, so the one check that asks
# whether the harnesses' own predicates work could fail in CI after a local GO.
Write-Host "-> scripts/harness-logic-test.sh"
$bashForPredicates = Find-Bash
if ($null -eq $bashForPredicates) {
    Fail "no real bash found (install Git for Windows): cannot run the harness predicates"
} else {
    $env:LINIX_BIN = "$RepoRoot/target/release/linix.exe"
    & $bashForPredicates "scripts/harness-logic-test.sh"
    if ($LASTEXITCODE -eq 0) { Pass "harness predicates" } else { Fail "harness predicates FAILED" }
}

# The same asymmetry one gate over: CI runs the mutation gate on every push and neither release
# script ran it. A harness is trustworthy because its checks can go red, not because they are
# green, and this is the only thing that measures that.
Write-Host "-> scripts/harness-mutation-test.sh --check"
$bashForMutation = Find-Bash
if ($null -eq $bashForMutation) {
    Fail "no real bash found (install Git for Windows): cannot run the mutation gate"
} else {
    & $bashForMutation "scripts/harness-mutation-test.sh" "--check"
    if ($LASTEXITCODE -eq 0) { Pass "harness mutation budget" } else { Fail "harness mutation budget EXCEEDED - checks that examine nothing" }

    # And the OTHER harness (G-4). CI mutation-tests both; this script tested one, and the
    # parity gate reported ok because it compared basenames. The four-distro
    # harness runs on every push against 136 checks and was measured in exactly one place.
    # Needs no Docker: the harness is shell, and the point is to run it against a stub.
    Write-Host "-> scripts/harness-mutation-test.sh docker/integration/run-in-container.sh --check"
    & $bashForMutation "scripts/harness-mutation-test.sh" "docker/integration/run-in-container.sh" "--check" "apt" "jq"
    if ($LASTEXITCODE -eq 0) { Pass "container harness mutation budget" } else { Fail "container harness mutation budget EXCEEDED - checks that examine nothing" }
}

# ------------------------------------------------------------------ 2. integration
if ($SkipIntegration) {
    Info "-SkipIntegration: skipped the native Windows sweep (hermetic gates only)"
} else {
    Step "2. NATIVE WINDOWS INTEGRATION SWEEP (real backends via linix)"
    $bashExe = Find-Bash
    if ($null -eq $bashExe) {
        Fail "no real bash found (install Git for Windows): cannot run the integration sweep"
    } else {
        Write-Host "Using bash: $bashExe"
        $env:LINIX = "$RepoRoot/target/release/linix.exe"
        & $bashExe "scripts/integration-windows.sh" $Backend $Package $Package2
        if ($LASTEXITCODE -eq 0) { Pass "native Windows integration sweep PASS" } else { Fail "native Windows integration sweep FAILED" }
    }
}

# ------------------------------------------------------------------ verdict
Step "RELEASE VERDICT"
$results | ForEach-Object { Write-Host $_ }
Write-Host ""
if (-not $hardFail) {
    Write-Host "=====> GO: every hard gate passed. Ready to release." -ForegroundColor Green
    exit 0
} else {
    Write-Host "=====> NO-GO: at least one hard gate failed (see above)." -ForegroundColor Red
    exit 1
}
