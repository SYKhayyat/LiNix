# LiNix bootstrap installer for Windows — the 30-second first run.
#
#   irm https://raw.githubusercontent.com/SYKhayyat/LiNix/HEAD/scripts/install.ps1 | iex
#
# Installs the `linix` binary, runs a health check, and offers to adopt the packages already
# on this machine. Override with env vars: $env:LINIX_REPO, $env:LINIX_NO_ADOPT.
$ErrorActionPreference = 'Stop'

$repo = if ($env:LINIX_REPO) { $env:LINIX_REPO } else { 'https://github.com/SYKhayyat/LiNix' }

function Say($m) { Write-Host "linix " -ForegroundColor Cyan -NoNewline; Write-Host $m }
function Err($m) { Write-Host "linix " -ForegroundColor Red  -NoNewline; Write-Host $m }

Say "bootstrapping - detecting toolchain..."

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Err "Rust/cargo not found."
    Err "Install Rust from https://rustup.rs and re-run this script."
    exit 1
}

# WHICH LiNix — the twin of install.sh's rule. `HEAD` is whatever was pushed last, which is not
# a thing anyone can ask for twice. The default is the newest release tag; $env:LINIX_REF
# overrides it, and a repo with no tags falls back to the branch and says so.
#
# **`git` is optional here and the twin already knew that.** `cargo install --git` fetches over
# libgit2 and needs no `git.exe`, so a Windows box with Rust and no Git can install LiNix
# perfectly well — but `$ErrorActionPreference = 'Stop'` turns a missing command into a
# terminating `CommandNotFoundException`, and this script died on it with a raw stack trace at
# the one step that is only ever a *preference*. `install.sh` degrades: its `git ls-remote`
# failure is swallowed by the pipeline's exit status and the branch fallback takes over.
# Exactly the twin-that-diverged shape CLAUDE.md is about — the rule is in both files now.
$ref = $env:LINIX_REF
if (-not $ref) {
    if (Get-Command git -ErrorAction SilentlyContinue) {
        # And a `git` that IS present can still fail — no network, a private repo, a proxy. That
        # is the same "we could not ask" as having no git at all, and it gets the same answer.
        try {
            $tags = & git ls-remote --tags --refs --sort=-v:refname $repo 'v*' 2>$null
            if ($tags) { $ref = ($tags | Select-Object -First 1) -replace '.*/', '' }
        } catch {
            $ref = $null
        }
    }
    if (-not $ref) { Say "no release tag published yet - installing from the default branch instead." }
}

# `--locked`, and no fallback. The retry without it was described as covering an unavailable
# lockfile; `Cargo.lock` is tracked in this repository, so what it actually covered was a
# network blip or a compile error, answered by resolving 452 dependencies fresh. Twin of the
# same three lines in install.sh - change one, change the other.
if ($ref) {
    Say "building and installing $ref from $repo (this can take a minute)..."
    cargo install --git $repo --tag $ref --locked
} else {
    Say "building and installing from $repo (this can take a minute)..."
    cargo install --git $repo --locked
}
# `Err` prints and returns - every other use of it here is a warning the script carries on
# past. A failed build is not one of those, so this exits: continuing would run the health
# check against whatever `linix` was already on PATH and report the old binary as the new one.
if ($LASTEXITCODE -ne 0) {
    Err "the build failed - see the cargo output above."
    exit 1
}

$cargoBin = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $HOME '.cargo\bin' }
# The binary just installed, by path, in preference to whatever `linix` resolves to on this
# session's PATH — that could be an older install elsewhere, and the health check below is
# supposed to vouch for the one this script produced.
$fresh = Join-Path $cargoBin 'linix.exe'
$linix = if (Test-Path $fresh) { $fresh } else { 'linix' }

if (-not (Get-Command linix -ErrorAction SilentlyContinue)) {
    Err "Add $cargoBin to your PATH to use 'linix'."
}

Say "running health check..."
& $linix check health

if (-not $env:LINIX_NO_ADOPT) {
    $ans = Read-Host "linix  adopt the packages already installed on this machine into a manifest now? [y/N]"
    if ($ans -match '^(y|yes)$') { & $linix adopt } else { Say "skipped - run 'linix adopt' whenever you're ready." }
}

Say "done. Try 'linix check' or 'linix sync'."
