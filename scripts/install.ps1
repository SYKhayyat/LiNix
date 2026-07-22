# LiNix bootstrap installer for Windows — the 30-second first run.
#
#   irm https://raw.githubusercontent.com/OWNER/linix/HEAD/scripts/install.ps1 | iex
#
# Installs the `linix` binary, runs a health check, and offers to adopt the packages already
# on this machine. Override with env vars: $env:LINIX_REPO, $env:LINIX_NO_ADOPT.
$ErrorActionPreference = 'Stop'

$repo = if ($env:LINIX_REPO) { $env:LINIX_REPO } else { 'https://github.com/OWNER/linix' }

function Say($m) { Write-Host "linix " -ForegroundColor Cyan -NoNewline; Write-Host $m }
function Err($m) { Write-Host "linix " -ForegroundColor Red  -NoNewline; Write-Host $m }

Say "bootstrapping - detecting toolchain..."

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Err "Rust/cargo not found."
    Err "Install Rust from https://rustup.rs and re-run this script."
    exit 1
}

Say "building and installing from $repo (this can take a minute)..."
# Prefer the reproducible, lockfile-pinned build; fall back if the lock is unavailable.
cargo install --git $repo --locked
if ($LASTEXITCODE -ne 0) { cargo install --git $repo }

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
& $linix doctor

if (-not $env:LINIX_NO_ADOPT) {
    $ans = Read-Host "linix  adopt the packages already installed on this machine into a manifest now? [y/N]"
    if ($ans -match '^(y|yes)$') { & $linix migrate } else { Say "skipped - run 'linix migrate' whenever you're ready." }
}

Say "done. Try 'linix status' or 'linix doctor'."
