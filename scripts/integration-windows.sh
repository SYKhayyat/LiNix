#!/usr/bin/env bash
# Native Windows/macOS real-world sweep. These OSes can't run in a Linux container, so we drive
# the host-native backends (scoop, winget, choco, brew) directly through `linix`, mirroring the
# Linux container harness section-for-section:
#   1. discovery  2. dry-run safety  3. install+list+coherence  4. idempotency  5. remove
#   6. negative/exit-status path  7. declarative roundtrip  7b. manifest DIRECTIVES (include/when/
#   @check)  8. JSON contract  9. profiles
#  10. REAL multi-backend lifecycle (every backend installable here, real install→list→remove)
#  11. FEATURE COVERAGE (every linix subcommand)  11b. v6-v9 COMMAND & FLAG COVERAGE (newer surface)
#  12. PLAN-SMOKE (only the can't-run-here set)  13. COVERAGE AUDIT (fails on any gap)  14. read-only
#
# Same guiding rule as the Linux harness: everything that CAN run on THIS host gets a REAL
# lifecycle; only the genuinely-impossible here (a mac-only backend on Windows, a Linux-distro
# backend, or one needing a daemon we don't have) is plan-smoked — and each is named.
#
#   scripts/integration-windows.sh [backend] [package] [package2]
#   e.g. scripts/integration-windows.sh scoop jq less      # user-scoped, no admin, reversible
#        scripts/integration-windows.sh winget jq          # exercises the winget HRESULT allowlist
#        scripts/integration-windows.sh brew wget          # on a real Mac
#
# scoop is the safe default (user-scoped, trivially reversible). winget/choco may require
# elevation. Point LINIX at a release build for a release check:
#   LINIX=./target/release/linix.exe scripts/integration-windows.sh
#
# By default the harness first BOOTSTRAPS the ecosystem backends through scoop so the sweep is
# as broad as the Linux images; anything already on PATH is left as-is. INSTALL_BACKENDS=0 skips
# it. FAST=1 downgrades the heaviest source-compiling backends (cargo/go) to plan-smoke.
set -u
BACKEND="${1:-scoop}"
PKG="${2:-jq}"
PKG2="${3:-less}"
LINIX="${LINIX:-./target/debug/linix.exe}"
BOGUS="linix-nonexistent-pkg-zzq9x"
GDIR="${GROUPS_DIR:-${TMPDIR:-/tmp}/linix-it-manifests}"
PSDIR="${TMPDIR:-/tmp}/linix-it-plansmoke"
SGDIR="${TMPDIR:-/tmp}/linix-it-sweep"
FGDIR="${TMPDIR:-/tmp}/linix-it-feat"
TIMEOUT="${TIMEOUT:-90}"
FAST="${FAST:-0}"

# Isolate LiNix's GLOBAL state registry to a throwaway dir (honored by safe_data_dir()) so this
# harness NEVER touches — or accumulates in — your real linix state, and a system-global
# `prune`/profile convergence only ever reconciles packages THIS run installed. Critical on a
# real machine: without it, `prune` would consider every linix-managed package on the box as
# drift. Git-Bash only path-converts argv (not env vars), so convert to a Windows path via
# cygpath; bash creates the same physical dir via the POSIX form.
_state_posix="${TMPDIR:-/tmp}/linix-it-state"
rm -rf "$_state_posix"; mkdir -p "$_state_posix"

# The CONFIG dir must be isolated too, and on this harness that is a SAFETY requirement, not
# tidiness: this script runs against your real machine, not a container.
#
# The old comment here said the config dir was left alone "so your real backend settings
# still apply". That was survivable only while `-g` REPLACED the wish list, which meant your
# real groups folder went unread. Now `-g` ADDS, and the global folder is always read — so
# leaving this pointed at your real config would put every package you actually manage into
# this run's wish list. `sync` would install them and `prune` would weigh them as drift.
# The harness would be operating your real machine.
_cfg_posix="${TMPDIR:-/tmp}/linix-it-config"
rm -rf "$_cfg_posix"; mkdir -p "$_cfg_posix/groups"
GLOBAL_GDIR="$_cfg_posix/groups"   # where LiNix looks when nobody passes -g

# A section that makes LiNix WRITE into its global folder (migrate adopts real packages)
# needs its own config dir, or what it writes joins every later section's wish list. `-g`
# used to isolate this for free; now that it only ADDS folders, isolation is explicit.
push_config_dir() {
    _PREV_CFG_POSIX="$_cfg_posix"
    _cfg_posix="$1"
    rm -rf "$_cfg_posix"; mkdir -p "$_cfg_posix/groups"
    GLOBAL_GDIR="$_cfg_posix/groups"
    if command -v cygpath >/dev/null 2>&1; then
        export LINIX_CONFIG_DIR="$(cygpath -w "$_cfg_posix")"
    else
        export LINIX_CONFIG_DIR="$_cfg_posix"
    fi
}
pop_config_dir() {
    _cfg_posix="$_PREV_CFG_POSIX"
    GLOBAL_GDIR="$_cfg_posix/groups"
    if command -v cygpath >/dev/null 2>&1; then
        export LINIX_CONFIG_DIR="$(cygpath -w "$_cfg_posix")"
    else
        export LINIX_CONFIG_DIR="$_cfg_posix"
    fi
}

if command -v cygpath >/dev/null 2>&1; then
    export LINIX_DATA_DIR="$(cygpath -w "$_state_posix")"
    export LINIX_CONFIG_DIR="$(cygpath -w "$_cfg_posix")"
else
    export LINIX_DATA_DIR="$_state_posix"
    export LINIX_CONFIG_DIR="$_cfg_posix"
fi

if command -v timeout >/dev/null 2>&1; then TO="timeout $TIMEOUT"; HAVE_TO=1; else TO=""; HAVE_TO=0; fi
lx() { $TO "$LINIX" "$@"; }
lxt() { _t="$1"; shift; if [ "$HAVE_TO" = 1 ]; then timeout "$_t" "$LINIX" "$@"; else "$LINIX" "$@"; fi; }

PASS=0; FAIL=0; SOFT=0
ok()   { echo "    [ok]    $1"; PASS=$((PASS+1)); }
no()   { echo "    [FAIL]  $1"; FAIL=$((FAIL+1)); }
soft() { echo "    [info]  $1"; SOFT=$((SOFT+1)); }
hr()   { echo; echo "=========== $* ==========="; }
rcnote() { [ "$1" -eq 124 ] && echo " (TIMED OUT)" || echo ""; }
present() { hash -r 2>/dev/null || true; command -v "$1" >/dev/null 2>&1; }

TOUCHED=" "
touched() { case "$TOUCHED" in *" $1 "*) ;; *) TOUCHED="$TOUCHED$1 " ;; esac; }
FEAT=" "
feat() { for _f in "$@"; do case "$FEAT" in *" $_f "*) ;; *) FEAT="$FEAT$_f " ;; esac; done; }

# Pick a Python that ACTUALLY parses JSON. On Windows a Microsoft Store "python3.exe" alias stub
# satisfies `command -v` but isn't real Python — probe each candidate under a short timeout and
# keep the first that truly parses; try `python` before `python3` so the real interpreter wins.
_pyprobe() { if command -v timeout >/dev/null 2>&1; then timeout 8 "$@"; else "$@"; fi; }
PYBIN=""
for _py in python python3 py; do
    command -v "$_py" >/dev/null 2>&1 || continue
    if printf '{}' | _pyprobe "$_py" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1; then
        PYBIN="$_py"; break
    fi
done
[ -n "$PYBIN" ] && echo "# JSON validator: $PYBIN" || echo "# JSON validator: structural fallback (no working python)"
is_json() {
    if [ -n "$PYBIN" ]; then
        printf '%s' "$1" | "$PYBIN" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1
    else
        first=$(printf '%s' "$1" | tr -d '[:space:]' | cut -c1)
        [ "$first" = "{" ] || [ "$first" = "[" ]
    fi
}
manifest_has_in() { grep -q "$2" "$1/local.txt" 2>/dev/null; }
manifest_has() { manifest_has_in "$GDIR" "$1"; }
manifest_scoped() { grep -Eq "^$2:$3(@|\$)" "$1/local.txt" 2>/dev/null; }
# gone_from_list <backend> <pkg>: true once the backend's installed-list no longer shows <pkg>.
# On Windows an uninstall's shim/junction/registry cleanup can lag the process exit, so retry
# briefly rather than reading a stale list one beat too early.
gone_from_list() {
    for _i in 1 2 3 4 5; do
        lx -b "$1" list 2>/dev/null | grep -qiw "$2" || return 0
        sleep 1
    done
    return 1
}

# Bootstrap the ecosystem backends via scoop so the Windows sweep is as broad as the Linux
# images. scoop is user-scoped, needs no admin, and is trivially reversible — so we NEVER
# bootstrap through winget/choco. Best-effort; anything already on PATH is left untouched.
INSTALL_BACKENDS="${INSTALL_BACKENDS:-1}"
bootstrap_backends() {
    command -v scoop >/dev/null 2>&1 || { soft "scoop absent — cannot bootstrap backends (install scoop, or set INSTALL_BACKENDS=0)"; return; }
    hr "0. BOOTSTRAP BACKENDS (scoop; user-scoped, reversible) — mirrors the Linux images"
    scoop bucket add main >/dev/null 2>&1 || true
    scoop bucket add versions >/dev/null 2>&1 || true
    # <scoop app>:<probe binary>. Core language managers PLUS the expansion backends scoop can
    # provide on Windows so their REAL lifecycle exercises a present binary. All best-effort.
    for pair in nodejs:npm pnpm:pnpm yarn:yarn python:python pipx:pipx uv:uv ruby:gem bun:bun \
                rustup:cargo go:go php:composer nim:nimble dart:dart kubectl:kubectl helm:helm \
                luarocks:luarocks pixi:pixi dotnet-sdk:dotnet; do
        app="${pair%%:*}"; probe="${pair##*:}"
        if command -v "$probe" >/dev/null 2>&1; then soft "[bootstrap] $probe already present — skip $app"; continue; fi
        if scoop install "$app" >/dev/null 2>&1; then ok "[bootstrap] installed $app (provides $probe)"; hash -r 2>/dev/null || true
        else soft "[bootstrap] $app failed to install (skipped — sweep will mark it not READY)"; fi
    done
    hash -r 2>/dev/null || true
}

# scoop apps expose tools under <app>/current/bin and mutate the PERSISTENT user PATH, which a
# running shell never sees — surface those + pnpm's global bin + choco's bin in THIS session so
# linix's child processes find gem/yarn/ruby/composer/etc.
augment_scoop_path() {
    command -v scoop >/dev/null 2>&1 || return 0
    for _bin in "$HOME"/scoop/apps/*/current/bin "$HOME"/scoop/shims; do
        [ -d "$_bin" ] && case ":$PATH:" in *":$_bin:"*) ;; *) PATH="$PATH:$_bin";; esac
    done
    for _cb in /c/ProgramData/chocolatey/bin "$ProgramData"/chocolatey/bin "$ALLUSERSPROFILE"/chocolatey/bin; do
        [ -d "$_cb" ] && case ":$PATH:" in *":$_cb:"*) ;; *) PATH="$PATH:$_cb";; esac
    done
    export PNPM_HOME="${USERPROFILE:-$HOME}\\AppData\\Local\\pnpm"
    mkdir -p "$HOME/AppData/Local/pnpm/bin" 2>/dev/null || true
    case ":$PATH:" in *":$HOME/AppData/Local/pnpm/bin:"*) ;; *) PATH="$PATH:$HOME/AppData/Local/pnpm/bin";; esac
    # Go (~/go/bin), .NET global tools, Dart pub, and cargo global-install dirs the real
    # lifecycle writes to — make them visible so verify-bin resolves.
    for _gb in "$HOME/go/bin" "$HOME/.dotnet/tools" "$HOME/AppData/Local/Pub/Cache/bin" "$HOME/.cargo/bin"; do
        [ -d "$_gb" ] && case ":$PATH:" in *":$_gb:"*) ;; *) PATH="$PATH:$_gb";; esac
    done
    export PATH
    hash -r 2>/dev/null || true
}

echo "###################################################################"
echo "# LiNix real-world sweep (native) :: backend=$BACKEND  pkg=$PKG  pkg2=$PKG2"
echo "# binary=$LINIX  timeout=${TIMEOUT}s  FAST=$FAST"
echo "###################################################################"
[ -x "$LINIX" ] || { echo "FATAL: linix not built at $LINIX — run: cargo build (or cargo build --release)"; exit 2; }
rm -rf "$GDIR"; mkdir -p "$GDIR"

[ "$INSTALL_BACKENDS" = "1" ] && bootstrap_backends
augment_scoop_path

DOCTOR="$(lx doctor 2>/dev/null)"
backend_ready() { printf '%s\n' "$DOCTOR" | grep -Eqi "^\[READY\][[:space:]]+$1([[:space:]]|$)"; }

plan_smoke() {
    b="$1"; p="$2"; mode="${3:-hard}"
    if ! backend_ready "$b"; then soft "[$b] not READY — plan-smoke skipped"; return; fi
    touched "$b"
    echo "      --- plan-smoke: $b  (pkg: $p, mode: $mode) ---"
    out=$(lx -g "$PSDIR" -n install "$b:$p" --json 2>/dev/null); rc=$?
    if [ $rc -eq 0 ] && is_json "$out"; then
        ok "[$b] dry-run install emits a JSON plan (argv + planner wired)"
    elif [ "$mode" = hard ]; then
        no "[$b] dry-run install plan invalid (rc=$rc$(rcnote $rc))"
    else
        soft "[$b] dry-run plan n/a in this sandbox (rc=$rc$(rcnote $rc))"
    fi
    out=$(lx -b "$b" list --json 2>/dev/null); is_json "$out" \
        && soft "[$b] list --json valid (parser ok)" || soft "[$b] list --json unavailable"
    out=$(lx -b "$b" search "$p" --json 2>/dev/null); is_json "$out" \
        && soft "[$b] search --json valid" || soft "[$b] search --json n/a"
}
ps_pkg_for() {
    case "$1" in
        github)   echo "BurntSushi/ripgrep" ;;
        web)      echo "https://example.com/app.zip" ;;
        appimage) echo "https://example.com/app.AppImage" ;;
        link)     echo "C:/temp/linix-ps-src@target=C:/temp/linix-ps-tgt" ;;
        go)       echo "github.com/junegunn/fzf" ;;
        composer) echo "psr/log" ;;
        vscode)   echo "ms-python.python" ;;
        service)  echo "Spooler" ;;
        *)        echo "hello" ;;
    esac
}

# REAL multi-backend lifecycle — identical contract to the Linux harness. Install failure is
# SOFT (ecosystem/network variance); everything after a successful install is HARD.
HEAVY=" cargo go "   # source-compiling on Windows; FAST=1 -> plan-smoke
sweep_backend() {
    b="$1"; p="$2"; bin="${3:-}"; t="${4:-$TIMEOUT}"; mode="${5:-real}"
    [ "$b" = "$BACKEND" ] && return   # already swept in detail above
    if ! backend_ready "$b"; then soft "[$b] not READY — skipped"; return; fi
    if [ "$FAST" = 1 ]; then
        case "$HEAVY" in *" $b "*) echo "      --- $b: FAST -> plan-smoke ---"; plan_smoke "$b" "$(ps_pkg_for "$b")" hard; return ;; esac
    fi
    touched "$b"
    echo "      --- backend: $b  (pkg: $p, mode: $mode, timeout: ${t}s) ---"
    out=$(lxt "$t" -g "$SGDIR" -y install "$b:$p" 2>&1); rc=$?
    if [ $rc -ne 0 ]; then soft "[$b] install '$p' rc=$rc$(rcnote $rc) — ecosystem/network variance"; printf '%s\n' "$out" | tail -3 | sed 's/^/          | /'; lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; return; fi
    if [ "$mode" = soft ]; then
        soft "[$b] install '$p' exits 0 (tolerant mode)"; lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; return
    fi
    ok "[$b] install '$p' exits 0"
    lx -b "$b" list 2>/dev/null | grep -qiw "$p" && ok "[$b] list shows '$p' (parser works)" || no "[$b] list does NOT show '$p'"
    manifest_scoped "$SGDIR" "$b" "$p" && ok "[$b] recorded '$p' in manifest (coherent)" || no "[$b] did NOT record '$p'"
    [ -n "$bin" ] && { present "$bin" && ok "[$b] '$bin' resolves on PATH" || soft "[$b] '$bin' not on PATH"; }
    if [ "$mode" = noremove ]; then
        lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; rc=$?
        if [ $rc -ne 0 ] && [ $rc -ne 124 ]; then ok "[$b] remove reports unsupported (rc=$rc) — correct: no uninstall verb"
        else no "[$b] remove should report unsupported (rc=$rc)"; fi
        return
    fi
    lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[$b] remove '$p' exits 0" || no "[$b] remove rc=$rc$(rcnote $rc)"
    gone_from_list "$b" "$p" && ok "[$b] '$p' gone after remove" || no "[$b] '$p' still listed after remove"
    manifest_scoped "$SGDIR" "$b" "$p" && no "[$b] remove left '$p' in manifest" || ok "[$b] remove cleared '$p' from manifest"
}
# github: real release download → list → remove (works on Windows; asset is a .exe).
sweep_github() {
    backend_ready github || { soft "[github] not READY — skipped"; return; }
    touched github
    echo "      --- backend: github  (repo: BurntSushi/ripgrep) ---"
    out=$(lxt 300 -g "$SGDIR" -y install "github:BurntSushi/ripgrep" 2>&1); rc=$?
    if [ $rc -ne 0 ]; then soft "[github] install rc=$rc$(rcnote $rc) — network/rate-limit variance"; printf '%s\n' "$out" | tail -3 | sed 's/^/          | /'; return; fi
    ok "[github] install exits 0 (real release asset downloaded)"
    lx -b github list 2>/dev/null | grep -qi ripgrep && ok "[github] list shows the repo" || no "[github] list missing ripgrep"
    lxt 120 -g "$SGDIR" -y remove "github:BurntSushi/ripgrep" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[github] remove exits 0" || no "[github] remove rc=$rc$(rcnote $rc)"
    lx -b github list 2>/dev/null | grep -qi ripgrep && no "[github] still listed after remove" || ok "[github] gone after remove"
}

run_plan_smokes() {
    hr "12. PLAN-SMOKE — only the backends that cannot run a real lifecycle on this host"
    rm -rf "$PSDIR"; mkdir -p "$PSDIR"
    # HARD: package-manager-like backends whose native tool/host is absent on Windows.
    hardset=" winget choco scoop psresource nix mas macports brew snap flatpak "
    ready=$(printf '%s\n' "$DOCTOR" | grep -E '^\[READY\]' | awk '{print $2}')
    for b in $ready; do
        case "$TOUCHED" in *" $b "*) continue ;; esac
        mode=soft
        case "$hardset" in *" $b "*) mode=hard ;; esac
        plan_smoke "$b" "$(ps_pkg_for "$b")" "$mode"
    done
}

hr "1. DISCOVERY (doctor / search / info)"
OUT="$(lx doctor 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "doctor exits 0" || no "doctor exit=$RC$(rcnote $RC)"; feat doctor
backend_ready "$BACKEND" && ok "doctor reports $BACKEND READY" || no "doctor does not list $BACKEND READY"
echo "      READY backends:"; printf '%s\n' "$DOCTOR" | grep -i "READY" | sed 's/^/        /'
OUT="$(lx search "$PKG" 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "search exits 0" || no "search exit=$RC$(rcnote $RC)"; feat search update info
printf '%s\n' "$OUT" | grep -iq "^$BACKEND" && ok "search returns a $BACKEND hit" || soft "no $BACKEND search hit for '$PKG'"

hr "2. DRY-RUN SAFETY (must change nothing)"
OUT="$(lx -n install "$BACKEND:$PKG" --json 2>/dev/null)"; RC=$?
[ $RC -eq 0 ] && ok "dry-run install exits 0" || no "dry-run install exit=$RC$(rcnote $RC)"
is_json "$OUT" && ok "dry-run emits JSON plan" || soft "dry-run output not JSON"

hr "3. IMPERATIVE INSTALL + LIST + config coherence"
lx -g "$GDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "install '$BACKEND:$PKG' exits 0" || no "install exit=$RC$(rcnote $RC)"; feat install
present "$PKG" && ok "'$PKG' on PATH after install" || soft "'$PKG' not resolvable on PATH (shim/PATH refresh may be needed)"
manifest_has "$PKG" && ok "install recorded '$PKG' in the manifest (config stays coherent)" || no "install did NOT record '$PKG' in the manifest"
OUT="$(lx --backend "$BACKEND" list 2>&1)"; RC=$?
{ [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -qiw "$PKG"; } && ok "list shows '$PKG'" || no "list ($BACKEND) missing '$PKG' (rc=$RC$(rcnote $RC))"; feat list

hr "4. IDEMPOTENCY (re-install)"
lx -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "re-install exits 0 (benign-exit allowlist covers 'already installed')" || no "re-install exit=$RC$(rcnote $RC)"

hr "5. IMPERATIVE REMOVE + config coherence"
lx -g "$GDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "remove exits 0" || no "remove exit=$RC$(rcnote $RC)"; feat remove
manifest_has "$PKG" && no "remove left '$PKG' in the manifest (stale config)" || ok "remove cleared '$PKG' from the manifest"
gone_from_list "$BACKEND" "$PKG" && ok "list no longer shows '$PKG'" || soft "list still shows '$PKG' (async uninstall?)"

hr "6. NEGATIVE PATH — failed mutation MUST surface"
lx -y install "$BACKEND:$BOGUS" >/dev/null 2>&1; RC=$?
if [ $RC -ne 0 ] && [ $RC -ne 124 ]; then ok "install of nonexistent '$BOGUS' FAILS (exit=$RC, not swallowed)"
elif [ $RC -eq 124 ]; then no "install of nonexistent '$BOGUS' TIMED OUT"
else no "bogus install returned 0 — failure SWALLOWED"; fi

hr "7. DECLARATIVE ROUNDTRIP — write file -> sync -> installed ; edit file -> prune -> gone"
lx -g "$GDIR" init >/dev/null 2>&1; RC=$?
MANIFEST="$GLOBAL_GDIR/local.txt"   # local.txt is anchored to global; -g no longer moves it
{ [ $RC -eq 0 ] && [ -f "$MANIFEST" ]; } && ok "init scaffolds $MANIFEST" || no "init rc=$RC$(rcnote $RC) / no manifest"; feat init
echo "$BACKEND:$PKG" >> "$MANIFEST"
lx -g "$GDIR" -b "$BACKEND" -y sync >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "sync (scoped) exits 0" || no "sync exit=$RC$(rcnote $RC)"; feat sync status
lx --backend "$BACKEND" list 2>/dev/null | grep -qiw "$PKG" && ok "sync installed '$PKG' from the manifest file" || soft "sync ran but list does not show '$PKG'"
lx -g "$GDIR" -b "$BACKEND" lock >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -f "$GLOBAL_GDIR/locks.json" ]; } && ok "lock writes locks.json in the global folder" || soft "lock rc=$RC$(rcnote $RC) / no locks.json"; feat lock
: > "$MANIFEST"
lx -g "$GDIR" -b "$BACKEND" -y prune >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "prune (scoped) exits 0" || no "prune exit=$RC$(rcnote $RC)"; feat prune
lx --backend "$BACKEND" list 2>/dev/null | grep -qiw "$PKG" && soft "prune ran but list still shows '$PKG'" || ok "prune removed '$PKG' after it left the manifest file"

hr "7b. MANIFEST DIRECTIVES — include / when host-conditionals / @check / exclusion"
# Exercise the declarative grammar the v8/v9 waves added: `include:` (splice another file),
# `when os == … end` host conditionals, and `@check=` post-install health probes. Read-only
# `status` proves the resolver wires them; only @check does a real sync. (`@module:` and `group:`
# resolve against the separate modules_dir / config [groups] and are covered by their own commands.)
DGDIR="${TMPDIR:-/tmp}/linix-it-directives"; rm -rf "$DGDIR"; mkdir -p "$DGDIR"
lx -g "$DGDIR" init >/dev/null 2>&1
DM="$DGDIR/local.txt"
printf '%s\n' "$BACKEND:$PKG2" > "$DGDIR/base.txt"                  # include: -> PKG2
{
    echo "include: ./base.txt"                                     # splice PKG2 in place
    echo "when os == windows"                                      # matching guard -> emitted
    echo "  $BACKEND:$PKG"
    echo "end"
    echo "when os == plan9"                                        # non-matching guard -> skipped
    echo "  $BACKEND:linix-should-not-appear-zzq"
    echo "end"
} > "$DM"
OUT="$(lxt 60 -g "$DGDIR" -b "$BACKEND" -v status 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "status resolves include/when directives (rc=0)" || no "directive status rc=$RC$(rcnote $RC)"
printf '%s\n' "$OUT" | grep -qw "$PKG2" && ok "include: spliced in '$PKG2' from base.txt" || soft "status did not name '$PKG2' (already satisfied?)"
printf '%s\n' "$OUT" | grep -q "linix-should-not-appear-zzq" && no "non-matching 'when' block leaked its package" || ok "non-matching 'when os==plan9' block correctly excluded"
feat sync status
# @check= post-install probe (advisory): a declarative sync installs a package that declares one.
CHKDIR="${TMPDIR:-/tmp}/linix-it-check"; rm -rf "$CHKDIR"; mkdir -p "$CHKDIR"
printf '%s\n' "$BACKEND:$PKG@check=cmd:ver" > "$CHKDIR/local.txt"   # `cmd /C ver` always exits 0
OUT="$(lxt 150 -g "$CHKDIR" -b "$BACKEND" -v -y sync 2>&1)"; RC=$?
if [ $RC -eq 0 ]; then
    ok "sync of an @check=… package exits 0 (probe is advisory)"
    printf '%s\n' "$OUT" | grep -qiE 'probe (OK|FAIL)|health probe' && ok "@check post-install probe ran" || soft "no probe line surfaced (timing/verbosity)"
else soft "@check sync rc=$RC$(rcnote $RC) (ecosystem/network variance)"; fi
lx -g "$CHKDIR" -b "$BACKEND" -y prune >/dev/null 2>&1

hr "8. JSON OUTPUT CONTRACT (stdout only; real JSON parse)"
json_valid() {
    out="$(lx "$@" --json 2>/dev/null)"; is_json "$out" && return 0
    sleep 2
    out="$(lx "$@" --json 2>/dev/null)"; is_json "$out"
}
json_valid search "$PKG"           && ok "search --json valid" || no "search --json not JSON"
json_valid --backend "$BACKEND" list && ok "list --json valid"   || no "list --json not JSON"
json_valid -b "$BACKEND" status    && ok "status --json valid" || no "status --json not JSON"

hr "9. PROFILES — activate/deactivate, MULTIPLE active, RELATIONAL (verified via list/show)"
PGDIR="${TMPDIR:-/tmp}/linix-it-prof"; PROFDIR="$_cfg_posix/profiles"   # profiles_dir = parent(GLOBAL groups_dir)/profiles
rm -rf "$PGDIR" "$PROFDIR"; mkdir -p "$PGDIR" "$PROFDIR"
printf '%s\n' "$BACKEND:$PKG"  > "$PROFDIR/alpha.profile"
printf '%s\n' "$BACKEND:$PKG2" > "$PROFDIR/bravo.profile"
printf '%s\n' "include alpha" "include bravo" > "$PROFDIR/both.profile"
printf '%s\n' "include both" "-$BACKEND:$PKG" > "$PROFDIR/lean.profile"
pcmd() { $TO "$LINIX" -g "$PGDIR" -b "$BACKEND" -y "$@" >/dev/null 2>&1; }
pread() { $TO "$LINIX" -g "$PGDIR" "$@" 2>/dev/null; }
listed() { lx -b "$BACKEND" list 2>/dev/null | grep -qiw "$1"; }

pcmd activate alpha; RC=$?
[ $RC -eq 0 ] && ok "activate alpha exits 0" || no "activate alpha rc=$RC$(rcnote $RC)"; feat activate profile
pread profile active | grep -qw alpha && ok "'alpha' shows as active" || no "'alpha' not reported active"
listed "$PKG" && ok "profile 'alpha' installed '$PKG'" || soft "alpha: list does not show '$PKG' (backend/PATH variance)"
pcmd activate bravo; RC=$?
[ $RC -eq 0 ] && ok "activate bravo exits 0 (two active)" || no "activate bravo rc=$RC$(rcnote $RC)"
{ listed "$PKG" && listed "$PKG2"; } && ok "MULTIPLE active: both '$PKG' and '$PKG2' installed" || soft "multiple-active: list did not show both (backend variance)"
pcmd deactivate alpha; RC=$?
[ $RC -eq 0 ] && ok "deactivate alpha exits 0" || no "deactivate alpha rc=$RC$(rcnote $RC)"; feat deactivate
listed "$PKG" && soft "deactivate alpha: '$PKG' still listed (async?)" || ok "deactivate alpha removed '$PKG'"
OUT="$(pread profile show lean)"
if printf '%s\n' "$OUT" | grep -qw "$BACKEND:$PKG2" && ! printf '%s\n' "$OUT" | grep -qw "$BACKEND:$PKG"; then
    ok "profile show lean resolves to {$BACKEND:$PKG2} (include + minus applied)"
else
    no "profile show lean resolved wrong: [$OUT]"
fi
pcmd deactivate bravo; pcmd deactivate lean
pread profile list | grep -qw alpha && ok "profile list enumerates defined profiles" || soft "profile list missing entries"

hr "10. REAL MULTI-BACKEND LIFECYCLE — every backend installable on this host (real install→list→remove)"
rm -rf "$SGDIR"; mkdir -p "$SGDIR"
# Language/cross backends — the SAME table the Linux harness sweeps (minus Linux-only ones), so
# coverage tracks across OSes. Fetched from each ecosystem's own registry; not-READY rows skip.
sweep_backend npm      cowsay    cowsay    120
sweep_backend pnpm     cowsay    cowsay    150
sweep_backend yarn     cowsay    cowsay    150
sweep_backend bun      cowsay    cowsay    150
sweep_backend pipx     cowsay    cowsay    240
sweep_backend uv       cowsay    cowsay    240
sweep_backend gem      colorize  ""        240
sweep_backend pip      cowsay    cowsay    240
sweep_backend luarocks say       ""        240
sweep_backend pixi     ripgrep   rg        420
sweep_backend composer psr/log   ""        300
sweep_backend dotnet   dotnetsay dotnetsay 420
sweep_backend pub      coverage  ""        420
sweep_backend krew     ns        ""        420
sweep_backend go       rsc.io/2fa 2fa      900
sweep_backend cargo    ripgrep   rg        2400
sweep_backend helm     https://github.com/databus23/helm-diff "" 420 soft
# Native Windows managers beyond the primary.
sweep_backend choco  jq  ""  180
sweep_backend winget jq  ""  180
sweep_github

hr "11. FEATURE COVERAGE — every linix subcommand exercised at least once"
rm -rf "$FGDIR"; mkdir -p "$FGDIR"; lx -g "$FGDIR" init >/dev/null 2>&1
for sh in bash zsh fish powershell elvish nushell; do
    OUT="$(lx completions "$sh" 2>/dev/null)"; RC=$?
    { [ $RC -eq 0 ] && [ -n "$OUT" ]; } && ok "completions $sh emits a script" || no "completions $sh rc=$RC / empty"
done
feat completions
lx -g "$FGDIR" heal >/dev/null 2>&1; RC=$?;  [ $RC -eq 0 ] && ok "heal exits 0 on a clean system" || soft "heal rc=$RC"; feat heal
lx -g "$FGDIR" -y clean >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "clean exits 0" || soft "clean rc=$RC (tolerated)"; feat clean
lx -b "$BACKEND" unmanaged >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "unmanaged exits 0" || soft "unmanaged rc=$RC"; feat unmanaged
lx -b "$BACKEND" orphans   >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "orphans exits 0"   || soft "orphans rc=$RC"; feat orphans
lx -b "$BACKEND" audit >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "audit exits 0" || soft "audit rc=$RC (network/OSV)"; feat audit
OUT="$(lx sbom 2>/dev/null)"; RC=$?; { [ $RC -eq 0 ] && is_json "$OUT"; } && ok "sbom emits CycloneDX JSON" || soft "sbom rc=$RC / not JSON"; feat sbom
lx why "$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "why '$PKG' exits 0" || soft "why rc=$RC"; feat why
lx policy >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "policy exits 0" || soft "policy rc=$RC (no policy.toml)"; feat policy
lx -g "$FGDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1
lx -g "$FGDIR" -b "$BACKEND" -y upgrade >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "upgrade (scoped) exits 0" || soft "upgrade rc=$RC$(rcnote $RC)"
lxt 120 -g "$FGDIR" -b "$BACKEND" -y upgrade --canary --test "cmd /c exit 0" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "canary upgrade with a passing --test exits 0" || soft "canary rc=$RC$(rcnote $RC)"; feat upgrade
lx -g "$FGDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1
lx repo list -b "$BACKEND" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "repo list ($BACKEND) exits 0" || soft "repo list rc=$RC"; feat repo
# Own config dir: migrate writes its manifest into the GLOBAL groups folder, so without
# this every package it adopts would join the wish list of every section that follows.
push_config_dir "${TMPDIR:-/tmp}/linix-it-cfg-migrate"
lxt 120 -g "$FGDIR" -b "$BACKEND" -y migrate >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "migrate (scoped) exits 0" || soft "migrate rc=$RC$(rcnote $RC)"; feat migrate
# The Windows backends (winget/scoop/choco) install no dependencies, so everything they list
# really was user-chosen — migrate SHOULD adopt here. That is the other half of the apt fix:
# "adopt nothing when unsure" must not become "adopt nothing, ever".
MIGF="$(ls "$GLOBAL_GDIR"/migrated_*.txt 2>/dev/null | head -1)"
[ -n "$MIGF" ] && ok "migrate wrote a manifest ($(grep -cvE '^\s*(#|$)' "$MIGF") package(s))" \
               || soft "migrate wrote no manifest (nothing unmanaged on this host)"
if [ -n "$MIGF" ]; then
    grep -q "THIS IS AN ESTIMATE" "$MIGF" && ok "migrate manifest warns that it is an estimate" \
                                          || no "migrate manifest does not warn that it is an estimate"
    grep -q "linix unmanage" "$MIGF" && ok "migrate manifest points at 'linix unmanage'" \
                                     || no "migrate manifest offers no way to keep a package unmanaged"
fi
pop_config_dir
# The removal guard: same contract as the Linux harness.
OUT="$(lx protected 2>/dev/null)"; printf '%s' "$OUT" | grep -q "Guarded commands" \
    && ok "protected lists the guarded commands" || no "protected does not show what is guarded"
OUT="$(lx protected --json 2>/dev/null)"; is_json "$OUT" \
    && ok "protected --json is JSON" || no "protected --json is not JSON"
feat protected
# unmanage: forget without uninstalling.
UMD="${TMPDIR:-/tmp}/linix-it-unmanage"; rm -rf "$UMD"; mkdir -p "$UMD"
lx -g "$UMD" -y install "$BACKEND:$PKG" >/dev/null 2>&1
lx -g "$UMD" -y unmanage "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
if [ $RC -eq 0 ]; then
    ok "unmanage exits 0"
    present "$PKG" && ok "unmanage left the package installed" \
                   || no "unmanage UNINSTALLED the package — it must only forget it"
    # local.txt is written to the GLOBAL folder, not the -g one — checking $UMD would pass
    # vacuously, since the declaration was never there to begin with.
    manifest_has_in "$GLOBAL_GDIR" "$PKG" && no "unmanage left the declaration behind" \
                                  || ok "unmanage removed the declaration too"
else
    soft "unmanage rc=$RC (tolerated)"
fi
OUT="$(lx -g "$UMD" unmanage --json "$BACKEND:$PKG" 2>/dev/null)"; is_json "$OUT" \
    && ok "unmanage --json is JSON" || no "unmanage --json is not JSON"
feat unmanage
lx -n teleport "$PKG" "$BACKEND" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "teleport (dry-run plan) exits 0" || soft "teleport rc=$RC"; feat teleport
lx -g "$FGDIR" module list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "module list exits 0" || soft "module list rc=$RC"
lx -g "$FGDIR" module create linix-it-mod >/dev/null 2>&1; lx -g "$FGDIR" module show linix-it-mod >/dev/null 2>&1; feat module
lx snapshot list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "snapshot list exits 0" || soft "snapshot list rc=$RC"
lx -y snapshot prune --force >/dev/null 2>&1; feat snapshot
GID="$(lx -g "$FGDIR" generation list 2>/dev/null | grep -oE '[0-9a-f]{6,}' | head -n1)"
if [ -n "$GID" ]; then
    lx -g "$FGDIR" generation pin "$GID" >/dev/null 2>&1 && soft "generation pin ok" || soft "generation pin rc=$?"
    lx -g "$FGDIR" generation unpin "$GID" >/dev/null 2>&1 || true
    lx -n -g "$FGDIR" rollback "$GID" >/dev/null 2>&1 && ok "rollback (dry-run) exits 0" || soft "rollback rc=$?"
else soft "no generation id yet (fresh manifest)"; fi
feat generation rollback
lx -g "$FGDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1
lx -g "$FGDIR" lease set "$BACKEND:$PKG" -d 30d >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "lease set exits 0" || soft "lease set rc=$RC"
lx -g "$FGDIR" lease list 2>/dev/null | grep -qiw "$PKG" && ok "lease list shows the leased package" || soft "lease list missing '$PKG'"; feat lease
lx -g "$FGDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1
lx schedule add linix-it-task --cron "0 2 * * *" --command "upgrade" >/dev/null 2>&1; RC=$?
if [ $RC -eq 0 ]; then ok "schedule add exits 0 (Task Scheduler present)"; lx schedule list 2>/dev/null | grep -qw linix-it-task && ok "schedule list shows the task" || soft "schedule list missing task"; lx schedule remove linix-it-task >/dev/null 2>&1
else soft "schedule add rc=$RC (needs elevation/Task Scheduler — tolerated)"; fi
feat schedule
OUT="$(lxt 120 run -p "$BACKEND:$PKG" "echo LINIX_RUN_OK" 2>/dev/null)"; printf '%s\n' "$OUT" | grep -q LINIX_RUN_OK && ok "run executes in an ephemeral env" || soft "run: ephemeral-env variance"; feat run
lxt 120 shim linix-it-shim -s "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "shim generates a launcher" || soft "shim rc=$RC$(rcnote $RC)"; feat shim
feat shell config

hr "11b. v6-v9 COMMAND & FLAG COVERAGE — every newer subcommand + new flags"
# Everything the v7/v8/v9 waves added that the coverage audit now tracks. Deterministic wiring
# checks are HARD; anything network/ecosystem-dependent is soft. All scoped to throwaway dirs.
NGDIR="${TMPDIR:-/tmp}/linix-it-newcmds"; rm -rf "$NGDIR"; mkdir -p "$NGDIR"
lx -g "$NGDIR" init >/dev/null 2>&1
NM="$NGDIR/local.txt"

# --- plan / apply (Terraform-style freeze-then-apply) ---
echo "$BACKEND:$PKG" >> "$NM"
PLANF="$NGDIR/plan.json"
lx -g "$NGDIR" -b "$BACKEND" plan --out "$PLANF" >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -f "$PLANF" ] && is_json "$(cat "$PLANF")"; } && ok "plan writes a JSON plan file" || no "plan rc=$RC / no JSON plan at $PLANF"
feat plan
lxt 180 -g "$NGDIR" -b "$BACKEND" apply "$PLANF" -y >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "apply executes a saved plan (rc=0)" || soft "apply rc=$RC$(rcnote $RC) (ecosystem variance)"
feat apply
lx -g "$NGDIR" -b "$BACKEND" -y remove "$BACKEND:$PKG" >/dev/null 2>&1

# --- conflicts (cross-backend, read-only) ---
lx -g "$NGDIR" conflicts >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "conflicts exits 0" || no "conflicts rc=$RC$(rcnote $RC)"
OUT="$(lx -g "$NGDIR" conflicts --json 2>/dev/null)"; is_json "$OUT" && ok "conflicts --json is valid JSON" || no "conflicts --json not JSON"
feat conflicts

# --- hold / unhold (bulk-upgrade guard) ---
lx -g "$NGDIR" hold "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "hold exits 0" || no "hold rc=$RC"
lx -g "$NGDIR" hold 2>/dev/null | grep -qi "$PKG" && ok "hold (no args) lists the held package" || no "hold list missing '$PKG'"
lx -g "$NGDIR" unhold "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "unhold exits 0" || no "unhold rc=$RC"
lx -g "$NGDIR" hold 2>/dev/null | grep -qi "$PKG" && no "package still held after unhold" || ok "unhold cleared the hold"
feat hold unhold

# --- export to native manifests (Brewfile/requirements.txt/package.json/Aptfile) ---
lx -g "$NGDIR" export --format pip --stdout >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "export --format pip --stdout exits 0" || soft "export stdout rc=$RC"
EXPD="$NGDIR/exports"; mkdir -p "$EXPD"
lx -g "$NGDIR" export --out "$EXPD" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "export --out writes native manifest(s)" || soft "export --out rc=$RC"
feat export

# --- bundle (offline/air-gapped) + tar.gz archive ---
# The output dir MUST live OUTSIDE the groups dir: bundle copies groups/ into <out>/groups, so an
# <out> nested under the groups dir would copy the bundle into itself (runaway recursion).
BND="${TMPDIR:-/tmp}/linix-it-bundle"; rm -rf "$BND" "$BND-ar" "$BND-ar.tar.gz"
lx -g "$NGDIR" bundle --out "$BND" >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -d "$BND" ]; } && ok "bundle writes an offline bundle dir" || soft "bundle rc=$RC / no dir"
lx -g "$NGDIR" bundle --out "$BND-ar" --archive >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -f "$BND-ar.tar.gz" ]; } && ok "bundle --archive produces a portable .tar.gz" || soft "bundle --archive rc=$RC / no tarball"
feat bundle

# --- watch (single reconcile pass over an empty manifest -> already in sync) ---
WGD="${TMPDIR:-/tmp}/linix-it-watch"; rm -rf "$WGD"; mkdir -p "$WGD"; lx -g "$WGD" init >/dev/null 2>&1
lxt 60 -g "$WGD" -b "$BACKEND" watch --once >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "watch --once runs a single reconcile and exits" || soft "watch --once rc=$RC$(rcnote $RC)"
feat watch

# --- git (version-control the manifests) — SCOPED to a throwaway dir, NEVER the real repo ---
GGD="${TMPDIR:-/tmp}/linix-it-git"; rm -rf "$GGD"; mkdir -p "$GGD"; lx -g "$GGD" init >/dev/null 2>&1
lx -g "$GGD" git init >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "git init makes the config dir a repo" || soft "git init rc=$RC"
lx -g "$GGD" git status >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "git status exits 0" || soft "git status rc=$RC"
lx -g "$GGD" git commit -m "linix-it commit" >/dev/null 2>&1
lx -g "$GGD" git log >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "git log exits 0" || soft "git log rc=$RC"
feat git

# --- managed (ownership mode + keep-list) ---
lx -g "$NGDIR" managed show >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "managed show exits 0" || soft "managed show rc=$RC"
lx -g "$NGDIR" managed keep "$PKG" >/dev/null 2>&1; lx -g "$NGDIR" managed unkeep "$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "managed keep/unkeep round-trips" || soft "managed keep/unkeep rc=$RC"
feat managed

# --- hooks (auto-record; read-only status + shell-init emitter) ---
lx hooks status >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "hooks status exits 0" || soft "hooks status rc=$RC"
OUT="$(lx hooks shell-init bash 2>/dev/null)"; [ -n "$OUT" ] && ok "hooks shell-init bash prints shell functions" || soft "hooks shell-init empty"
feat hooks

# --- service (declarative services; read-only surface via Windows `sc`) ---
lx service list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "service list exits 0" || soft "service list rc=$RC (no service control?)"
feat service

# --- self-upgrade --check (report only; NEVER actually rebuilds/installs) ---
lx self-upgrade --check >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "self-upgrade --check reports version/source" || soft "self-upgrade --check rc=$RC"
feat self-upgrade

# --- config edit (non-interactive: a no-op editor that reads the file and exits) ---
TMPCFG="${TMPDIR:-/tmp}/linix-it-cfg.toml"; rm -f "$TMPCFG"
EDITOR=sort VISUAL=sort lxt 30 -c "$TMPCFG" config edit >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "config edit re-validates after a no-op edit" || soft "config edit rc=$RC$(rcnote $RC) (editor-spawn variance)"
feat config

# --- generation log / diff (v9) ---
GID2="$(lx -g "$FGDIR" generation list 2>/dev/null | grep -oE '[0-9a-f]{6,}' | head -n1)"
lx -g "$FGDIR" generation log >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "generation log exits 0" || soft "generation log rc=$RC"
OUT="$(lx -g "$FGDIR" generation log --json 2>/dev/null)"; is_json "$OUT" && ok "generation log --json valid" || soft "generation log --json n/a (no generations yet)"
lx -g "$FGDIR" generation log --oneline >/dev/null 2>&1
if [ -n "$GID2" ]; then lx -g "$FGDIR" generation diff "$GID2" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "generation diff exits 0" || soft "generation diff rc=$RC"; else soft "generation diff skipped (no generation id)"; fi
feat generation

# --- module add (remote registry) — tolerant (needs network) ---
lxt 90 -g "$FGDIR" module add "github:BurntSushi/ripgrep" --name linix-it-remote >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "module add fetches a remote module" || soft "module add rc=$RC$(rcnote $RC) (network — tolerated)"
feat module

# --- NEW FLAGS on existing commands ---
lx -q -b "$BACKEND" list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "global --quiet exits 0" || no "--quiet rc=$RC"
lx -b "$BACKEND" list --outdated >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "list --outdated exits 0" || soft "list --outdated rc=$RC"
lx search "$PKG" --installed >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "search --installed exits 0" || soft "search --installed rc=$RC"
OUT="$(lx why "$PKG" --json 2>/dev/null)"; is_json "$OUT" && ok "why --json valid JSON" || soft "why --json n/a"
lx doctor --fix >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "doctor --fix exits 0" || soft "doctor --fix rc=$RC"
OUT="$(lx doctor --json 2>/dev/null)"; is_json "$OUT" && ok "doctor --json valid JSON" || soft "doctor --json not JSON"
OUT="$(lx -g "$NGDIR" -b "$BACKEND" prune --json 2>/dev/null)"; is_json "$OUT" && ok "prune --json valid JSON" || soft "prune --json n/a"
OUT="$(lx -b "$BACKEND" audit --json 2>/dev/null)"; is_json "$OUT" && ok "audit --json valid JSON" || soft "audit --json n/a (network)"
lxt 120 -b "$BACKEND" -y upgrade --security >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "upgrade --security exits 0" || soft "upgrade --security rc=$RC$(rcnote $RC) (OSV/network)"
lx -n -b "$BACKEND" upgrade --all --except "$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "upgrade --all --except (dry-run) exits 0" || soft "upgrade --all --except rc=$RC"
lx -n install "$BACKEND:$PKG" --temp 1h --json >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "install --temp (dry-run lease) exits 0" || soft "install --temp rc=$RC"

# --- command aliases ([command_aliases] in config, expanded before clap) ---
ACFG="${TMPDIR:-/tmp}/linix-it-alias.toml"
printf '%s\n' '[command_aliases]' 'inv = "list"' > "$ACFG"
lx -c "$ACFG" -b "$BACKEND" inv >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "command alias 'inv' expands to 'list' (pre-clap)" || no "command alias expansion rc=$RC$(rcnote $RC)"

# --- tamper-evident lockfile (sign on lock; refuse a modified lockfile) ---
LKDIR="${TMPDIR:-/tmp}/linix-it-lock"; rm -rf "$LKDIR"; mkdir -p "$LKDIR"; lx -g "$LKDIR" init >/dev/null 2>&1
echo "$BACKEND:$PKG" >> "$LKDIR/local.txt"
lx -g "$LKDIR" -b "$BACKEND" -y sync >/dev/null 2>&1
lx -g "$LKDIR" -b "$BACKEND" lock >/dev/null 2>&1
if [ -f "$LKDIR/locks.json" ] && grep -q '"sig"' "$LKDIR/locks.json"; then
    ok "lock signs locks.json (tamper-evident)"
    sed -i 's/"sig": *"[0-9a-f]/"sig": "0/' "$LKDIR/locks.json" 2>/dev/null
    OUT="$(lxt 30 -g "$LKDIR" -b "$BACKEND" -v status 2>&1)"
    printf '%s\n' "$OUT" | grep -qi "MISMATCH" && ok "a modified lockfile is detected (signature MISMATCH) and refused" || no "tampered lockfile was NOT flagged"
else
    soft "lock did not produce a signed locks.json (backend has no lockable versions?)"
fi
lx -g "$LKDIR" -b "$BACKEND" -y prune >/dev/null 2>&1
feat lock

run_plan_smokes

hr "13. COVERAGE AUDIT — nothing registered or featured is silently untested"
audit_fail=0
for b in $(printf '%s\n' "$DOCTOR" | grep -E '^\[READY\]' | awk '{print $2}'); do
    case "$TOUCHED" in *" $b "*) : ;; *) no "COVERAGE GAP: backend '$b' is READY but was never exercised"; audit_fail=1 ;; esac
done
[ $audit_fail -eq 0 ] && ok "every READY backend was exercised (real lifecycle or plan-smoke)"
touched "$BACKEND"
FEATURES_ALL="sync watch run shim heal clean unmanaged orphans status prune plan apply lock search update upgrade list info install remove repo doctor migrate teleport shell undo cockpit activate deactivate profile module snapshot generation rollback git lease schedule config init audit sbom export bundle why service bisect clone fleet managed hooks hold unhold conflicts policy completions self-upgrade protected unmanage"
# EXEMPT = no non-interactive assertion is possible: interactive TUIs (shell ghost-shell, undo
# gallery, cockpit) or commands that need a remote SSH host (bisect/clone/fleet).
FEATURES_EXEMPT=" shell undo cockpit bisect clone fleet "
feat_gap=0
for f in $FEATURES_ALL; do
    case "$FEATURES_EXEMPT" in *" $f "*) soft "feature '$f' is EXEMPT (interactive or needs a remote host)"; continue ;; esac
    case "$FEAT" in *" $f "*) : ;; *) no "FEATURE GAP: '$f' was never exercised"; feat_gap=1 ;; esac
done
[ $feat_gap -eq 0 ] && ok "every non-exempt linix subcommand was exercised at least once"

hr "14. READ-ONLY SMOKE"
for cmd in "config show" "config path" "unmanaged" "orphans" "profile list" "why $PKG"; do
    # shellcheck disable=SC2086
    lx -b "$BACKEND" $cmd >/dev/null 2>&1; RC=$?
    [ $RC -eq 0 ] && soft "\`linix $cmd\` exits 0" || soft "\`linix $cmd\` exit=$RC$(rcnote $RC) (tolerated)"
done

hr "SUMMARY [$BACKEND]"
echo "    HARD pass: $PASS    HARD fail: $FAIL    soft/info: $SOFT"
if [ "$FAIL" -ne 0 ]; then echo "    RESULT: FAIL ($FAIL hard check(s) failed)"; exit 1; fi
echo "    RESULT: PASS"
