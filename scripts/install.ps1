# LiNix bootstrap installer for Windows — the 30-second first run.
#
#   irm https://raw.githubusercontent.com/SYKhayyat/LiNix/HEAD/scripts/install.ps1 | iex
#
# Installs the `linix` binary, runs a health check, and offers to adopt the packages already
# on this machine. Override with env vars:
#   $env:LINIX_REPO      git source        (default: the SYKhayyat/LiNix repo)
#   $env:LINIX_REF       tag or branch     (default: the newest release tag)
#   $env:LINIX_BIN_DIR   install location  (default: cargo's bin dir)
#   $env:LINIX_NO_ADOPT  set to skip the `adopt` prompt
#
# Every name in that list is read below, and the twin's list says the same four. LINIX_REF was
# read by both scripts and documented by neither; LINIX_BIN_DIR was documented by one and read
# by neither - in the two files users pipe from the internet, where the list is the only
# interface anyone sees.
$ErrorActionPreference = 'Stop'

$repo = if ($env:LINIX_REPO) { $env:LINIX_REPO } else { 'https://github.com/SYKhayyat/LiNix' }

function Say($m) { Write-Host "linix " -ForegroundColor Cyan -NoNewline; Write-Host $m }
function Err($m) { Write-Host "linix " -ForegroundColor Red  -NoNewline; Write-Host $m }

Say "bootstrapping - detecting toolchain..."

# Download the published binary for this platform. Returns $false for no asset, no network, or
# a body too small to be a binary - each of which means "build it instead".
#
# **The twin of install.sh's `fetch_binary`, and the reason both exist.** Both headers promise a
# 30-second first run, and the only path either had was a source build: 448 crates under fat LTO
# on a stranger's machine. A published release makes the promise keepable, so the promise runs
# first and the compiler is the fallback. Windows builds one target, so there is no detection to
# do here - which is exactly why the twin's `uname` logic must not be copied in.
function Get-PublishedBinary($destination, $tag) {
    $asset = 'linix-x86_64-pc-windows-msvc.exe'
    $url = if ($tag) { "$repo/releases/download/$tag/$asset" }
           else      { "$repo/releases/latest/download/$asset" }
    # Progress rendering makes Invoke-WebRequest an order of magnitude slower in 5.1, on the one
    # step this whole change exists to make fast.
    $previous = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'
    try {
        Invoke-WebRequest -Uri $url -OutFile $destination -UseBasicParsing -ErrorAction Stop
    } catch {
        return $false
    } finally {
        $ProgressPreference = $previous
    }
    # A 404 page saved to a file is still a file, and running one fails three steps later with a
    # message about something else.
    if (-not (Test-Path $destination)) { return $false }
    if ((Get-Item $destination).Length -lt 1000000) { return $false }
    return $true
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

# The published binary first, into the same place the source path installs to, so a user who set
# LINIX_BIN_DIR gets the same answer whichever path ran.
$installDir =
    if ($env:LINIX_BIN_DIR) { $env:LINIX_BIN_DIR }
    elseif ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' }
    else { Join-Path $HOME '.cargo\bin' }
$downloaded = $false
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
if (Get-PublishedBinary $temp $ref) {
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Move-Item $temp (Join-Path $installDir 'linix.exe') -Force
    Say "installed the published binary to $installDir."
    $downloaded = $true
} else {
    if (Test-Path $temp) { Remove-Item -Force $temp }
    Say "no published binary for this platform - building from source."
}

if (-not $downloaded) {

# Only the source path needs a compiler, and the check belongs where the need is. Demanding Rust
# before knowing whether a binary was available turned "install this program" into "install a
# toolchain first" for every user on a platform that has a published build.
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Err "Rust/cargo not found, and no published binary was available."
    Err "Install Rust from https://rustup.rs and re-run this script."
    exit 1
}

# `--locked`, and no fallback. The retry without it was described as covering an unavailable
# lockfile; `Cargo.lock` is tracked in this repository, so what it actually covered was a
# network blip or a compile error, answered by resolving 448 dependencies fresh. Twin of the
# same three lines in install.sh - change one, change the other.
#
# `--root` when the caller named a directory. cargo installs into "$root\bin", so a
# LINIX_BIN_DIR that already ends in `bin` is that directory's parent; anything else gets a
# staged install and a copy, because cargo cannot be pointed at an arbitrary folder. Computed
# here rather than demanded of the user, who was told this variable names the install location -
# and who, until now, was told that by a script that never read it.
$binDir = $env:LINIX_BIN_DIR
$stage = $null
$cargoArgs = @('install', '--git', $repo, '--locked')
if ($ref) { $cargoArgs += @('--tag', $ref) }
if ($binDir) {
    $trimmed = $binDir.TrimEnd('\', '/')
    if ((Split-Path -Leaf $trimmed) -eq 'bin') {
        $cargoArgs += @('--root', (Split-Path -Parent $trimmed))
    } else {
        $stage = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
        $cargoArgs += @('--root', $stage)
    }
}
if ($ref) {
    Say "building and installing $ref from $repo (this can take a minute)..."
} else {
    Say "building and installing from $repo (this can take a minute)..."
}
& cargo @cargoArgs
# `Err` prints and returns - every other use of it here is a warning the script carries on
# past. A failed build is not one of those, so this exits: continuing would run the health
# check against whatever `linix` was already on PATH and report the old binary as the new one.
if ($LASTEXITCODE -ne 0) {
    Err "the build failed - see the cargo output above."
    exit 1
}

if ($stage) {
    New-Item -ItemType Directory -Force -Path $binDir | Out-Null
    Copy-Item (Join-Path $stage 'bin\linix.exe') (Join-Path $binDir 'linix.exe') -Force
    Remove-Item -Recurse -Force $stage
    Say "installed to $binDir (LINIX_BIN_DIR)"
}

}  # end of the build-from-source path

$cargoBin = $installDir
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
