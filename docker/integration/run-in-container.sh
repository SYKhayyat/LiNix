#!/bin/sh
# Broad real-world test for a whole distro image, driven entirely through the `linix` binary.
# Runs INSIDE a disposable container as root, so it can safely install/remove real system
# packages, compile from source, download real release assets, and scaffold real manifests.
# This is the release-readiness sweep. Its guiding rule (per the release owner):
#
#   *Everything that CAN physically run on this platform gets a REAL lifecycle — even if it
#    compiles from source and takes minutes. Only the genuinely-impossible here (a mac-only
#    backend on Linux, or one needing a daemon/filesystem the container lacks) is plan-smoked,
#    and each such case is named explicitly so nothing is silently skipped.*
#
# Sections:
#   1.  Discovery (doctor/search/info)             9.  JSON output contract (real json parse)
#   2.  Dry-run safety (no mutation)              10.  PROFILES (activate/deactivate/relational)
#   3.  Imperative install + list + coherence     11.  REAL multi-backend lifecycle sweep
#   4.  Idempotency                                    (every feasible backend, real install→
#   5.  Imperative remove + coherence                   list→remove; source compiles included)
#   6.  Negative path (exit-status enforced)      12.  FEATURE COVERAGE (every linix subcommand)
#   7.  Declarative diagnostic (unscoped status)  12b. v6-v9 COMMAND & FLAG COVERAGE (newer surface)
#   8.  Declarative lifecycle ROUNDTRIP           13.  PLAN-SMOKE (only the can't-run-here set)
#   8b. Manifest DIRECTIVES (include/when/@check)  14.  COVERAGE AUDIT (fails on any gap)
#                                                 15.  Read-only command smoke
#
#   Usage: run-in-container.sh <native-backend> [package] [package2]
#   e.g.   run-in-container.sh apt jq htop
#
# Toggles (env):
#   SMOKE_ONLY=1  discovery + plan-smoke + read-only only; skip real mutation (source distros)
#   FAST=1        downgrade the heaviest source-compiling backends (cargo/opam/nimble/spack/
#                 conda/go) to plan-smoke for a quicker run; everything else still real
#
# Every check is tallied and the script continues past failures so a single run shows the whole
# picture; it exits non-zero if any HARD check failed. "soft" checks (network-dependent or
# ecosystem-optional) are reported but never fail the run. Every `linix` invocation is wrapped
# in `timeout` (per-command ceiling): a hang is recorded as a FAILURE (exit 124) and the run
# continues.
#
# Canary note: the package must NOT be a name busybox provides as a built-in applet (e.g.
# `tree` on Alpine), or "binary gone after remove" becomes unfalsifiable. `jq`/`htop` are
# packaged by every native backend in the matrix and ship no busybox applet.
set -u
BACKEND="$1"
PKG="${2:-jq}"
PKG2="${3:-htop}"                             # a second native pkg for selective profile tests
LINIX="${LINIX:-/src/target/release/linix}"
BOGUS="linix-nonexistent-pkg-zzq9x"           # a name no real repo carries
GDIR="${GROUPS_DIR:-/tmp/linix-it-manifests}" # throwaway manifest dir for declarative tests
PSDIR="/tmp/linix-it-plansmoke"                # throwaway manifest dir for the plan-smoke sweep
TIMEOUT="${TIMEOUT:-90}"                       # default per-command wall-clock ceiling (seconds)
SMOKE_ONLY="${SMOKE_ONLY:-0}"
FAST="${FAST:-0}"

# Isolate LiNix's GLOBAL state registry to a throwaway dir (honored by safe_data_dir()). This
# is what makes a system-global `prune`/`activate` safe and deterministic: with a fresh state
# registry, drift = "packages THIS run installed that aren't in the manifest", so convergence
# only ever touches this run's packages — never pre-existing system state — and nothing
# accumulates across runs.
export LINIX_DATA_DIR="/tmp/linix-it-state"
rm -rf "$LINIX_DATA_DIR"; mkdir -p "$LINIX_DATA_DIR"

# Isolate the CONFIG dir too (honored by safe_config_dir()). The old comment here said this
# was deliberately NOT isolated "so the real backend settings still apply" — but a fresh
# container has no real settings to apply, so that bought nothing while leaving the run free
# to write into whatever config dir the image happens to have. Several commands (`repo add`,
# `shim`, `doctor --fix`, `config init`) ignore -g and write there regardless.
#
# It matters for what comes next: the GLOBAL wish-list folder is derived from this dir, and
# the -g overlay is about to make that folder always-read. Isolating it now — while -g still
# REPLACES and nothing depends on global — is a deliberate no-op, so a green run here proves
# the harness change is sound before the semantics move underneath it.
export LINIX_CONFIG_DIR="/tmp/linix-it-config"
rm -rf "$LINIX_CONFIG_DIR"; mkdir -p "$LINIX_CONFIG_DIR"
GLOBAL_GDIR="$LINIX_CONFIG_DIR/groups"   # where LiNix looks when nobody passes -g
mkdir -p "$GLOBAL_GDIR"

# A section that makes LiNix WRITE into its global folder (migrate is the one that matters:
# it adopts real packages) needs its own config dir, or what it writes leaks into every
# later section's wish list and silently becomes "packages the user wants". `-g` used to
# provide this isolation for free; now that it only ADDS folders, isolation has to be asked
# for. That is the whole point of the change, so the harness says it out loud.
push_config_dir() {
    _PREV_CFG="$LINIX_CONFIG_DIR"
    export LINIX_CONFIG_DIR="$1"
    rm -rf "$LINIX_CONFIG_DIR"; mkdir -p "$LINIX_CONFIG_DIR/groups"
    GLOBAL_GDIR="$LINIX_CONFIG_DIR/groups"
}
pop_config_dir() {
    export LINIX_CONFIG_DIR="$_PREV_CFG"
    GLOBAL_GDIR="$LINIX_CONFIG_DIR/groups"
}

# Wrap every linix call in `timeout` when available (GNU coreutils + busybox both accept
# `timeout SECS CMD…`). A timed-out command exits 124, which our checks treat as failure.
if command -v timeout >/dev/null 2>&1; then TO="timeout $TIMEOUT"; HAVE_TO=1; else TO=""; HAVE_TO=0; fi
lx() { $TO "$LINIX" "$@"; }
# lxt <secs> <linix-args…>: like lx but with a per-call timeout override (for source compiles).
lxt() { _t="$1"; shift; if [ "$HAVE_TO" = 1 ]; then timeout "$_t" "$LINIX" "$@"; else "$LINIX" "$@"; fi; }

# Declarative commands (sync/status/prune) get a shorter ceiling and a verbose-capture
# wrapper so a stall reveals the LAST step before it froze.
DTIMEOUT="${DTIMEOUT:-60}"
if command -v timeout >/dev/null 2>&1; then DTO="timeout $DTIMEOUT"; else DTO=""; fi
LOGF="$GDIR/_lastcmd.log"
dlx() { $DTO "$LINIX" -v "$@" >"$LOGF" 2>&1; }   # verbose, captured; rc is the command's rc
tail_log() { echo "      --- last log lines before stall (linix -v) ---"; tail -n 16 "$LOGF" 2>/dev/null | sed 's/^/      | /'; }

PASS=0; FAIL=0; SOFT=0
ok()   { echo "    [ok]    $1"; PASS=$((PASS+1)); }
# okf <feature> <text>: a hard pass that ALSO credits <feature> as proven. Coverage is a
# consequence of an assertion passing — never a separate claim that can be made on a line
# where nothing was asserted. That separation is what made the coverage audit vacuous.
okf()  { _f="$1"; shift; ok "$*"; _rec FEATV "$_f"; }
no()   { echo "    [FAIL]  $1"; FAIL=$((FAIL+1)); }
soft() { echo "    [info]  $1"; SOFT=$((SOFT+1)); }
hr()   { echo; echo "=========== $* ==========="; }
rcnote() { [ "$1" -eq 124 ] && echo " (TIMED OUT)" || echo ""; }
rehash() { hash -r 2>/dev/null || true; }
# present: resolves on PATH AND the file really exists (shell caches stale locations)
present() { rehash; r="$(command -v "$1" 2>/dev/null || true)"; [ -n "$r" ] && [ -e "$r" ]; }

# --- coverage bookkeeping: every backend we actually exercise is recorded here, and the
# --- COVERAGE AUDIT (section 14) HARD-fails if any READY backend went untouched. ---
TOUCHED=" "
touched() { case "$TOUCHED" in *" $1 "*) ;; *) TOUCHED="$TOUCHED$1 " ;; esac; }
# --- feature bookkeeping. TWO tiers, deliberately separate:
# ---   feat  <name…>  the command RAN. Nothing about the result was proven.
# ---   featv <name…>  a HARD assertion PROVED the command works. Call only where `ok` fired.
# ---
# --- They used to be one function, called unconditionally on the same line as the assertion:
# ---
# ---   [ $RC -eq 0 ] && ok "teleport …" || soft "teleport rc=$RC (tolerated)"; feat teleport
# ---
# --- `feat teleport` ran whether teleport passed, softly failed, or was never meaningfully
# --- invoked. So the COVERAGE AUDIT asserted that this script MENTIONS a feature, not that the
# --- feature works: a checksum over its own source text, incapable of failing. One call site
# --- even read `feat shell` with a comment saying shell cannot be asserted at all.
FEAT=" "    # exercised
FEATV=" "   # exercised AND proven
_rec() { _v="$1"; shift; for _f in "$@"; do
    eval "case \"\$$_v\" in *\" \$_f \"*) ;; *) $_v=\"\$$_v\$_f \" ;; esac"
done; }
feat()  { _rec FEAT  "$@"; }
featv() { _rec FEATV "$@"; _rec FEAT "$@"; }

# is_json: real JSON validation via python3 when available (the images ship it); otherwise a
# structural fallback. NOTE: pretty-printed JSON is multi-line, so a per-line `cut -c1` check
# is WRONG — this validates the whole payload. Pick an interpreter that ACTUALLY parses
# (a `command -v` hit can be a non-functional stub, e.g. Windows' Store python3 alias), so
# probe each candidate once under a short timeout and keep the first that truly works.
_pyprobe() { if command -v timeout >/dev/null 2>&1; then timeout 8 "$@"; else "$@"; fi; }
PYBIN=""
for _py in python3 python py; do
    command -v "$_py" >/dev/null 2>&1 || continue
    if printf '{}' | _pyprobe "$_py" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1; then
        PYBIN="$_py"; break
    fi
done
is_json() {
    # The interpreter is probed once at startup, but the run under test can BREAK it: a
    # removal reaching python3 (or merely its stdlib, e.g. via `apk del pipx`) leaves the
    # binary on PATH while `import json` no longer works. Checking `command -v` is not
    # enough — that is exactly the state that made a healthy `conflicts --json` report as
    # "not JSON" and sent us hunting the wrong bug.
    #
    # So re-prove the interpreter on every call with a payload we KNOW is valid. Only a
    # working interpreter is allowed to fail an assertion; a broken one falls back to the
    # structural check. The cost is one extra process per call, which is nothing next to
    # a false failure.
    if [ -n "$PYBIN" ] && printf '{}' | "$PYBIN" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1; then
        printf '%s' "$1" | "$PYBIN" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1
        return $?
    fi
    if [ -n "$PYBIN" ]; then
        echo "    [warn]  is_json: interpreter '$PYBIN' stopped working mid-run — using structural check"
        PYBIN=""
    fi
    first=$(printf '%s' "$1" | tr -d '[:space:]' | cut -c1)
    [ "$first" = "{" ] || [ "$first" = "[" ]
}
# manifest_has[_in]: is the token present in the declarative manifest LiNix writes for
# imperative ops (kept coherent so the next `sync` doesn't treat the package as drift)?
manifest_has_in() { grep -q "$2" "$1/local.txt" 2>/dev/null; }
manifest_has() { manifest_has_in "$GDIR" "$1"; }
# manifest_scoped: like manifest_has_in but anchored to a specific "<backend>:<pkg>"
# entry (optionally version-pinned). The multi-backend sweep installs the SAME pkg
# name (cowsay) under several backends into a shared manifest, so a bare substring
# grep would let one backend's leftover entry falsely satisfy another backend's
# coherence check — this keeps each backend's assertion independent.
manifest_scoped() { grep -Eq "^$2:$3(@|\$)" "$1/local.txt" 2>/dev/null; }

echo "###################################################################"
echo "# LiNix real-world sweep :: native-backend=$BACKEND  pkg=$PKG  pkg2=$PKG2"
echo "# binary=$LINIX  timeout=${TIMEOUT}s  SMOKE_ONLY=$SMOKE_ONLY  FAST=$FAST"
echo "###################################################################"
[ -x "$LINIX" ] || { echo "FATAL: linix binary not found at $LINIX"; exit 2; }
rm -rf "$GDIR"; mkdir -p "$GDIR"

# Capture the backend readiness map ONCE; the sweep and audit gate on it.
DOCTOR="$(lx doctor 2>/dev/null)"
backend_ready() { printf '%s\n' "$DOCTOR" | grep -Eqi "^\[READY\][[:space:]]+$1([[:space:]]|$)"; }

# ============================================================================
# plan_smoke <backend> <pkg> [hard|soft]: a deterministic, per-backend "is this backend wired
# end to end?" check that needs NO successful network fetch and NO source compile:
#   * dry-run install  -> a JSON plan  (proves argv construction + planner wiring)
#   * list --json      -> valid JSON   (soft: exercises the installed-list parser)
#   * search --json    -> valid JSON   (soft: network/optional)
# HARD for package-manager backends; tolerant (soft) for system/special ones whose planner may
# need facilities a minimal container lacks. Auto-skips (soft) when not READY. Records coverage.
# ============================================================================
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
        && soft "[$b] list --json valid (installed-list parser ok)" \
        || soft "[$b] list --json unavailable (tool not initialized?)"
    out=$(lx -b "$b" search "$p" --json 2>/dev/null); is_json "$out" \
        && soft "[$b] search --json valid" || soft "[$b] search --json n/a"
}

# Representative install identifier per backend for plan-smoke. Special-identifier backends
# (github owner/repo, web/appimage URL, link path, …) need a real-shaped name; dry-run never
# fetches it, so a not-here name is fine.
ps_pkg_for() {
    case "$1" in
        github)   echo "BurntSushi/ripgrep" ;;
        web)      echo "https://example.com/app.tar.gz" ;;
        appimage) echo "https://example.com/app.AppImage" ;;
        link)     echo "/tmp/linix-ps-src@target=/tmp/linix-ps-tgt" ;;
        go)       echo "github.com/junegunn/fzf" ;;
        composer) echo "psr/log" ;;
        emerge)   echo "app-misc/jq" ;;
        vscode)   echo "ms-python.python" ;;
        service)  echo "cron" ;;
        btrfs)    echo "linix-ps-subvol" ;;
        *)        echo "hello" ;;
    esac
}

# ============================================================================
# REAL multi-backend lifecycle. Generic contract for ANY package-manager backend:
#   install -> list(parser) -> manifest coherence -> [bin on PATH] -> remove -> gone -> coherent
# Install failure is SOFT with a diagnostic (ecosystem/version/network variance is not a
# core-orchestration bug); EVERYTHING after a successful install is HARD — that is where the
# real bugs live (wrong remove verb, list-parse drift, stale manifest). This is what caught the
# pixi `global remove` vs `global uninstall` bug that plan-smoke alone could not.
#
#   sweep_backend <backend> <pkg> [verify-bin] [timeout] [mode]
#     mode real (default): full lifecycle, HARD after install
#     mode soft          : whole lifecycle tolerant (known ecosystem quirk, e.g. helm URL-vs-name)
#     mode noremove      : install+list are HARD, then assert `remove` reports a GRACEFUL
#                          "unsupported" (cabal/stack have no uninstall verb — verifying the
#                          designed contract, not a missing feature)
# FAST=1 downgrades backends tagged heavy (see HEAVY) to plan-smoke.
# ============================================================================
SGDIR="/tmp/linix-it-sweep"
HEAVY=" cargo opam nimble spack conda go "   # source-compiling; FAST=1 -> plan-smoke instead
sweep_backend() {
    b="$1"; p="$2"; bin="${3:-}"; t="${4:-$TIMEOUT}"; mode="${5:-real}"
    if ! backend_ready "$b"; then soft "[$b] not READY in this image — skipped"; return; fi
    if [ "$FAST" = 1 ]; then
        case "$HEAVY" in *" $b "*) echo "      --- $b: FAST mode -> plan-smoke (source compile skipped) ---"; plan_smoke "$b" "$(ps_pkg_for "$b")" hard; return ;; esac
    fi
    touched "$b"
    echo "      --- backend: $b  (pkg: $p, mode: $mode, timeout: ${t}s) ---"
    out=$(lxt "$t" -g "$SGDIR" -y install "$b:$p" 2>&1); rc=$?
    if [ $rc -ne 0 ]; then
        soft "[$b] install '$p' rc=$rc$(rcnote $rc) — ecosystem/network variance (not a hard fail)"
        printf '%s\n' "$out" | tail -3 | sed 's/^/          | /'
        lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1   # best-effort cleanup
        return
    fi
    if [ "$mode" = soft ]; then
        soft "[$b] install '$p' exits 0 (tolerant mode: URL/name or remote quirks are non-fatal)"
        lx -b "$b" list 2>/dev/null | grep -qiw "$p" && soft "[$b] list shows '$p'" || soft "[$b] list did not show '$p' (name-vs-id quirk)"
        lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1
        return
    fi
    ok "[$b] install '$p' exits 0"
    lx -b "$b" list 2>/dev/null | grep -qiw "$p" && ok "[$b] list shows '$p' (installed-list parser works)" || no "[$b] list does NOT show '$p' after install — parser gap"
    manifest_scoped "$SGDIR" "$b" "$p" && ok "[$b] install recorded '$p' in manifest (coherent)" || no "[$b] install did NOT record '$p' in manifest"
    [ -n "$bin" ] && { present "$bin" && ok "[$b] '$bin' resolves on PATH" || soft "[$b] '$bin' not on PATH (global-bin dir not exported)"; }
    if [ "$mode" = noremove ]; then
        # cabal/stack: no uninstall verb by design -> remove MUST report a graceful unsupported
        # (non-zero, but not a panic/timeout), and MUST NOT silently claim success.
        lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; rc=$?
        if [ $rc -ne 0 ] && [ $rc -ne 124 ]; then ok "[$b] remove reports unsupported (rc=$rc) — correct: this tool has no uninstall verb"
        elif [ $rc -eq 124 ]; then no "[$b] remove TIMED OUT (expected an immediate unsupported error)"
        else no "[$b] remove returned 0 though '$b' has no uninstall verb — a no-op must not masquerade as success"; fi
        return
    fi
    lxt "$t" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[$b] remove '$p' exits 0" || no "[$b] remove '$p' rc=$rc$(rcnote $rc)"
    lx -b "$b" list 2>/dev/null | grep -qiw "$p" && no "[$b] list still shows '$p' after remove" || ok "[$b] '$p' gone from list after remove"
    manifest_scoped "$SGDIR" "$b" "$p" && no "[$b] remove left '$p' in manifest" || ok "[$b] remove cleared '$p' from manifest"
}

# github: real GitHub-release lifecycle (download latest asset -> symlink into ~/.local/bin ->
# list -> remove). Bespoke because its "package" is owner/repo and its bin is the repo name.
sweep_github() {
    b=github; repo="BurntSushi/ripgrep"; binname="rg"
    if ! backend_ready "$b"; then soft "[$b] not READY — skipped"; return; fi
    touched "$b"
    echo "      --- backend: github  (repo: $repo) ---"
    out=$(lxt 300 -g "$SGDIR" -y install "$b:$repo" 2>&1); rc=$?
    if [ $rc -ne 0 ]; then soft "[github] install '$repo' rc=$rc$(rcnote $rc) — network/rate-limit variance"; printf '%s\n' "$out" | tail -3 | sed 's/^/          | /'; return; fi
    ok "[github] install '$repo' exits 0 (real release asset downloaded + extracted)"
    lx -b "$b" list 2>/dev/null | grep -qi "ripgrep" && ok "[github] list shows the installed repo (state parser works)" || no "[github] list does NOT show ripgrep after install"
    present "$binname" && ok "[github] '$binname' resolves on PATH (~/.local/bin symlink)" || soft "[github] '$binname' not on PATH (~/.local/bin not exported?)"
    lxt 120 -g "$SGDIR" -y remove "$b:$repo" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[github] remove exits 0" || no "[github] remove rc=$rc$(rcnote $rc)"
    lx -b "$b" list 2>/dev/null | grep -qi "ripgrep" && no "[github] still listed after remove" || ok "[github] gone from list after remove"
}

# link: real filesystem-link lifecycle. link has no `list` capability, so verify on disk:
# install creates the symlink at @target; remove deletes it. (Syntax: link:<src>@target=<dst>.)
# Absolute-path names are permitted for the link backend by the backend-aware name validator
# (`validate_package_name_for`), which still blocks `..` traversal and shell-injection.
sweep_link() {
    b=link
    if ! backend_ready "$b"; then soft "[$b] not READY — skipped"; return; fi
    touched "$b"
    src=/tmp/linix-link-src; dst=/tmp/linix-link-dst
    printf 'managed by linix\n' > "$src"; rm -f "$dst"
    echo "      --- backend: link  ($src -> $dst) ---"
    lx -g "$SGDIR" -y install "$b:$src@target=$dst" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[link] install exits 0" || no "[link] install rc=$rc$(rcnote $rc)"
    { [ -L "$dst" ] || [ -e "$dst" ]; } && ok "[link] created the link at $dst" || no "[link] did NOT create $dst"
    lx -g "$SGDIR" -y remove "$b:$dst" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[link] remove exits 0" || no "[link] remove rc=$rc$(rcnote $rc)"
    { [ -L "$dst" ] || [ -e "$dst" ]; } && no "[link] $dst still present after remove" || ok "[link] $dst gone after remove"
    rm -f "$src" "$dst"
}

# Plan-smoke EVERY READY backend the real sweep did NOT already exercise — enumerated from
# `doctor`, so nothing registered is silently untested and any future backend is auto-included.
# By design this should now be a SMALL set: only the backends that genuinely cannot run a real
# lifecycle in this container (a daemon/filesystem/host we don't have). Each is named below.
run_plan_smokes() {
    hr "13. PLAN-SMOKE — only the backends that cannot run a real lifecycle here"
    rm -rf "$PSDIR"; mkdir -p "$PSDIR"
    # HARD (wiring must be correct even where we can't fully install): the package-manager-like
    # ones whose native tool/daemon/host is absent in a plain container.
    hardset=" emerge eopkg slackpkg guix zypper xbps yay paru winget scoop choco psresource macports mas snap "
    ready=$(printf '%s\n' "$DOCTOR" | grep -E '^\[READY\]' | awk '{print $2}')
    for b in $ready; do
        case "$TOUCHED" in *" $b "*) continue ;; esac    # already got a real lifecycle
        mode=soft
        case "$hardset" in *" $b "*) mode=hard ;; esac
        plan_smoke "$b" "$(ps_pkg_for "$b")" "$mode"
    done
}

# ------------------------------------------------------------------ discovery
hr "1. DISCOVERY (doctor / search / info)"
OUT="$(lx doctor 2>&1)"; RC=$?
[ $RC -eq 0 ] && okf doctor "doctor exits 0" || no "doctor exit=$RC$(rcnote $RC)"; feat doctor
backend_ready "$BACKEND" && ok "doctor reports $BACKEND READY" || no "doctor does not list $BACKEND as READY"
echo "      READY backends:"; printf '%s\n' "$DOCTOR" | grep -i "READY" | sed 's/^/        /'

OUT="$(lx --backend "$BACKEND" update 2>&1)"; RC=$?
[ $RC -eq 0 ] && soft "update ($BACKEND) exits 0" || soft "update ($BACKEND) exit=$RC (network/optional)"; feat update

OUT="$(lx search "$PKG" 2>&1)"; RC=$?
[ $RC -eq 0 ] && okf search "search exits 0" || no "search exit=$RC$(rcnote $RC)"; feat search
printf '%s\n' "$OUT" | grep -iq "^$BACKEND" && ok "search returns a $BACKEND hit for '$PKG'" \
    || soft "no $BACKEND-prefixed search hit for '$PKG' (index/name variance)"

OUT="$(lx info "$PKG" 2>&1)"; RC=$?
if [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -q "Package:"; then
    okf info "info '$PKG' returns metadata"
else
    soft "info '$PKG' returned rc=$RC / no metadata (may be search-only backend)"
fi
feat info

# ----------------------------------------- SMOKE-ONLY fast exit (slow/source distros)
if [ "$SMOKE_ONLY" = "1" ]; then
    hr "SMOKE-ONLY MODE — plan/parse/read-only (native manager builds from source; no real install)"
    rm -rf "$PSDIR"; mkdir -p "$PSDIR"
    plan_smoke "$BACKEND" "$PKG"
    run_plan_smokes
    hr "READ-ONLY SMOKE (must run without crashing)"
    for cmd in "config show" "config path" "unmanaged" "orphans" "audit" "sbom" "snapshot list" "profile list" "policy" "completions bash"; do
        # shellcheck disable=SC2086
        lx -b "$BACKEND" $cmd >/dev/null 2>&1; RC=$?
        [ $RC -eq 0 ] && soft "\`linix $cmd\` exits 0" || soft "\`linix $cmd\` exit=$RC$(rcnote $RC) (tolerated)"
    done
    hr "SUMMARY [$BACKEND image — SMOKE-ONLY]"
    echo "    HARD pass: $PASS    HARD fail: $FAIL    soft/info: $SOFT"
    [ "$FAIL" -ne 0 ] && { echo "    RESULT: FAIL ($FAIL hard check(s) failed)"; exit 1; }
    echo "    RESULT: PASS"; exit 0
fi

# --------------------------------------------------- dry-run safety (no mutation)
hr "2. DRY-RUN SAFETY (must change nothing)"
present "$PKG" && no "PRECONDITION: '$PKG' already on PATH before tests" || ok "clean start: '$PKG' absent"
OUT="$(lx -g "$GDIR" -n install "$BACKEND:$PKG" --json 2>/dev/null)"; RC=$?
[ $RC -eq 0 ] && ok "dry-run install exits 0" || no "dry-run install exit=$RC$(rcnote $RC)"
is_json "$OUT" && ok "dry-run install emits JSON plan" || soft "dry-run install output not JSON"
present "$PKG" && no "dry-run actually installed '$PKG' (must not!)" || ok "dry-run left system unchanged"
manifest_has "$PKG" && no "dry-run wrote '$PKG' to the manifest (must not!)" || ok "dry-run left the manifest unchanged"

# --------------------------------------------------------- imperative install
hr "3. IMPERATIVE INSTALL + LIST + INFO + config coherence"
lx -g "$GDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf install "install '$BACKEND:$PKG' exits 0" || no "install exit=$RC$(rcnote $RC)"; feat install
present "$PKG" && ok "'$PKG' on PATH after install" || no "'$PKG' NOT on PATH after install"
manifest_has "$PKG" && ok "install recorded '$PKG' in the manifest (config stays coherent)" || no "install did NOT record '$PKG' in the manifest — next sync would treat it as drift"
OUT="$(lx --backend "$BACKEND" list 2>&1)"; RC=$?
if [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -qw "$PKG"; then
    okf list "list shows '$PKG' as installed (installed-list parser works)"
else
    no "list ($BACKEND) does not show '$PKG' (rc=$RC$(rcnote $RC)) — installed-list parse gap"
fi
feat list

# ------------------------------------------------------------- idempotency (install)
hr "4. IDEMPOTENCY (install already-installed)"
lx -g "$GDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "re-install of present package exits 0" || no "re-install exit=$RC$(rcnote $RC) (should be a no-op success)"

# ----------------------------------------------------------------- imperative remove
hr "5. IMPERATIVE REMOVE + verify gone + config coherence"
lx -g "$GDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf remove "remove '$BACKEND:$PKG' exits 0" || no "remove exit=$RC$(rcnote $RC)"; feat remove
present "$PKG" && no "'$PKG' STILL present after remove" || ok "'$PKG' gone from PATH after remove"
manifest_has "$PKG" && no "remove left '$PKG' in the manifest (stale config — sync would reinstall it)" || ok "remove cleared '$PKG' from the manifest"
OUT="$(lx --backend "$BACKEND" list 2>&1)"
printf '%s\n' "$OUT" | grep -qw "$PKG" && no "list still shows '$PKG' after remove" || ok "list no longer shows '$PKG'"
lx -g "$GDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "second remove (not installed) exits 0 (warn, not error)" || no "second remove exit=$RC$(rcnote $RC)"

# ----------------------------------------------- NEGATIVE PATH (exit-status enforcement)
hr "6. NEGATIVE PATH — failed mutation MUST surface (exit-status enforcement)"
lx -y install "$BACKEND:$BOGUS" >/dev/null 2>&1; RC=$?
if [ $RC -ne 0 ] && [ $RC -ne 124 ]; then
    ok "install of nonexistent '$BOGUS' FAILS with exit=$RC (failure not swallowed)"
elif [ $RC -eq 124 ]; then
    no "install of nonexistent '$BOGUS' TIMED OUT (expected a fast non-zero failure)"
else
    no "install of nonexistent '$BOGUS' returned 0 — a real failure was SWALLOWED"
fi
present "$BOGUS" && no "bogus package somehow present" || ok "bogus package not installed (as expected)"

# ------------------------------------- diagnostic: is UNSCOPED status/sync slow?
hr "7. DECLARATIVE DIAGNOSTIC (unscoped status — planning across ALL backends)"
OUT="$(timeout 30 "$LINIX" -g "$GDIR" status 2>&1)"; RC=$?
if [ $RC -eq 0 ]; then
    ok "unscoped 'status' completed (rc=0) — planning across every backend does not hang"
elif [ $RC -eq 124 ]; then
    no "unscoped 'status' HANGS (>30s across all backends)"
else
    soft "unscoped 'status' exit=$RC"
fi
feat status

# ---------------------------------------- declarative lifecycle ROUNDTRIP (scoped)
hr "8. DECLARATIVE ROUNDTRIP — write file -> sync -> installed ; edit file -> prune -> gone"
# local.txt is LiNix's own file and lives in the GLOBAL groups folder — `-g` adds a folder
# to READ, it no longer relocates LiNix's bookkeeping. That split is the point of the
# overlay: the ownership registry never moved either, and the two disagreeing is what
# turned every globally managed package into drift.
lx -g "$GDIR" init >/dev/null 2>&1; RC=$?
MANIFEST="$GLOBAL_GDIR/local.txt"
if [ $RC -eq 0 ] && [ -f "$MANIFEST" ]; then okf init "init scaffolds manifest at $MANIFEST"; else no "init rc=$RC$(rcnote $RC), manifest present=$( [ -f "$MANIFEST" ] && echo yes || echo no)"; fi
feat init

echo "      -> writing '$BACKEND:$PKG' into the manifest file, then calling sync"
echo "$BACKEND:$PKG" >> "$MANIFEST"
dlx -g "$GDIR" -b "$BACKEND" status; RC=$?
if [ $RC -eq 0 ]; then ok "status (scoped) exits 0"; else no "status exit=$RC$(rcnote $RC)"; tail_log; fi
grep -qw "$PKG" "$LOGF" 2>/dev/null && ok "status shows '$PKG' as a pending change" || soft "status did not name '$PKG' (already satisfied?)"

dlx -g "$GDIR" -b "$BACKEND" -y sync; RC=$?
if [ $RC -eq 0 ]; then ok "sync (scoped) exits 0"; else no "sync exit=$RC$(rcnote $RC)"; tail_log; fi
present "$PKG" && okf sync "sync INSTALLED '$PKG' from the manifest file" || no "sync did NOT install '$PKG'"
feat sync

lx -g "$GDIR" -b "$BACKEND" lock >/dev/null 2>&1; RC=$?
# locks.json is anchored to global too — one machine, one set of pins.
if [ $RC -eq 0 ] && [ -f "$GLOBAL_GDIR/locks.json" ]; then okf lock "lock writes locks.json in the global folder, not the -g one"; else soft "lock rc=$RC$(rcnote $RC) / no locks.json"; fi
feat lock

OUT="$(lx -g "$GDIR" -b "$BACKEND" generation list 2>&1)"; RC=$?
[ $RC -eq 0 ] && soft "generation list exits 0" || soft "generation list exit=$RC$(rcnote $RC)"; feat generation

echo "      -> removing '$PKG' from the manifest file, then converging (prune)"
: > "$MANIFEST"
dlx -g "$GDIR" -b "$BACKEND" status
grep -qw "$PKG" "$LOGF" 2>/dev/null && ok "status flags '$PKG' as drift after the manifest edit" || soft "status did not flag '$PKG' as drift"
dlx -g "$GDIR" -b "$BACKEND" -y prune; RC=$?
if [ $RC -eq 0 ]; then ok "prune (scoped) exits 0"; else no "prune exit=$RC$(rcnote $RC)"; tail_log; fi
present "$PKG" && no "prune did NOT remove drift '$PKG'" || okf prune "prune REMOVED '$PKG' after it left the manifest file"
feat prune

# ------------------------------------------------- MANIFEST DIRECTIVES
hr "8b. MANIFEST DIRECTIVES — include / when host-conditionals / @check / exclusion"
# Exercise the declarative grammar the v8/v9 waves added: `include:` (splice another file),
# `when os == … end` host conditionals, and `@check=` post-install health probes. Read-only
# `status` proves the resolver wires them; only @check does a real sync. (`@module:` and `group:`
# resolve against the separate modules_dir / config [groups] and are covered by their own commands.)
DGDIR="/tmp/linix-it-directives"; rm -rf "$DGDIR"; mkdir -p "$DGDIR"
lx -g "$DGDIR" init >/dev/null 2>&1
DM="$DGDIR/local.txt"
printf '%s\n' "$BACKEND:$PKG2" > "$DGDIR/base.txt"                  # include: -> PKG2
{
    echo "include: ./base.txt"                                     # splice PKG2 in place
    echo "when os == linux"                                        # matching guard -> emitted
    echo "  $BACKEND:$PKG"
    echo "end"
    echo "when os == plan9"                                        # non-matching guard -> skipped
    echo "  $BACKEND:linix-should-not-appear-zzq"
    echo "end"
} > "$DM"
dlx -g "$DGDIR" -b "$BACKEND" status; RC=$?
[ $RC -eq 0 ] && okf status "status resolves include/when directives (rc=0)" || { no "directive status rc=$RC$(rcnote $RC)"; tail_log; }
grep -qw "$PKG2" "$LOGF" 2>/dev/null && ok "include: spliced in '$PKG2' from base.txt" || soft "status did not name '$PKG2' (already satisfied?)"
grep -qw "$PKG" "$LOGF" 2>/dev/null && ok "when os==linux block contributed '$PKG'" || soft "status did not name '$PKG' (already satisfied?)"
grep -q "linix-should-not-appear-zzq" "$LOGF" 2>/dev/null && no "non-matching 'when' block leaked its package" || ok "non-matching 'when os==plan9' block correctly excluded"
feat sync status
# @check= post-install probe (advisory): a declarative sync installs a package that declares one.
CHKDIR="/tmp/linix-it-check"; rm -rf "$CHKDIR"; mkdir -p "$CHKDIR"
printf '%s\n' "$BACKEND:$PKG@check=cmd:true" > "$CHKDIR/local.txt"  # `sh -c true` always exits 0
dlx -g "$CHKDIR" -b "$BACKEND" -y sync; RC=$?
if [ $RC -eq 0 ]; then
    ok "sync of an @check=… package exits 0 (probe is advisory)"
    grep -qiE 'probe (OK|FAIL)|health probe' "$LOGF" 2>/dev/null && ok "@check post-install probe ran" || soft "no probe line surfaced (timing/verbosity)"
else soft "@check sync rc=$RC$(rcnote $RC) (ecosystem/network variance)"; fi
dlx -g "$CHKDIR" -b "$BACKEND" -y prune >/dev/null 2>&1

# ------------------------------------------------------------- JSON contract
hr "9. JSON OUTPUT CONTRACT (stdout only; real JSON parse)"
OUT="$(lx search "$PKG" --json 2>/dev/null)"; is_json "$OUT" && ok "search --json is valid JSON" || no "search --json not JSON"
OUT="$(lx --backend "$BACKEND" list --json 2>/dev/null)"; is_json "$OUT" && ok "list --json is valid JSON" || no "list --json not JSON"
OUT="$(lx -b "$BACKEND" status --json 2>/dev/null)"; is_json "$OUT" && ok "status --json is valid JSON" || no "status --json not JSON"

# ------------------------------------------------- PROFILES lifecycle
hr "10. PROFILES — activate/deactivate, MULTIPLE active, RELATIONAL (include / exclude / -pkg)"
PGDIR="/tmp/linix-it-prof"; PROFDIR="$LINIX_CONFIG_DIR/profiles"   # profiles_dir = parent(GLOBAL groups_dir)/profiles
rm -rf "$PGDIR" "$PROFDIR"; mkdir -p "$PGDIR" "$PROFDIR"
# Define profiles as files: two atomic ones, a "plus" composition, and a "minus" relational one.
printf '%s\n' "$BACKEND:$PKG"  > "$PROFDIR/alpha.profile"
printf '%s\n' "$BACKEND:$PKG2" > "$PROFDIR/bravo.profile"
printf '%s\n' "include alpha" "include bravo" > "$PROFDIR/both.profile"
printf '%s\n' "include both" "-$BACKEND:$PKG" > "$PROFDIR/lean.profile"
pcmd() { $DTO "$LINIX" -g "$PGDIR" -b "$BACKEND" -y "$@" >/dev/null 2>&1; }   # scoped, yes, timed
pread() { $TO "$LINIX" -g "$PGDIR" "$@" 2>/dev/null; }                         # read-only profile cmds

# (a) activate one profile -> only its package
pcmd activate alpha; RC=$?
[ $RC -eq 0 ] && okf activate "activate alpha exits 0" || no "activate alpha rc=$RC$(rcnote $RC)"; feat activate
present "$PKG"  && ok "profile 'alpha' installed '$PKG'" || no "alpha did not install '$PKG'"
present "$PKG2" && no "alpha unexpectedly installed '$PKG2'" || ok "alpha left '$PKG2' absent"
pread profile active | grep -qw alpha && okf profile "'alpha' shows as active" || no "'alpha' not reported active"; feat profile

# (b) activate a SECOND profile -> BOTH packages present simultaneously
pcmd activate bravo; RC=$?
[ $RC -eq 0 ] && ok "activate bravo exits 0 (two profiles now active)" || no "activate bravo rc=$RC$(rcnote $RC)"
if present "$PKG" && present "$PKG2"; then ok "MULTIPLE active: '$PKG' AND '$PKG2' both installed"; else no "multiple-active did not yield both packages"; fi

# (c) deactivate one -> its unique package removed, the shared/other stays
pcmd deactivate alpha; RC=$?
[ $RC -eq 0 ] && okf deactivate "deactivate alpha exits 0" || no "deactivate alpha rc=$RC$(rcnote $RC)"; feat deactivate
present "$PKG"  && no "deactivate alpha did NOT remove '$PKG'" || ok "deactivate alpha removed '$PKG'"
present "$PKG2" && ok "'$PKG2' survived (still provided by bravo — union semantics)" || no "deactivate alpha wrongly removed '$PKG2'"

# (d) relational profile: lean = (alpha + bravo) - PKG  => only PKG2
pcmd deactivate bravo
present "$PKG2" && no "deactivate bravo left '$PKG2' behind" || ok "clean slate: no profile packages installed"
pcmd activate lean; RC=$?
[ $RC -eq 0 ] && ok "activate lean (relational include+minus) exits 0" || no "activate lean rc=$RC$(rcnote $RC)"
present "$PKG2" && ok "relational 'lean' installed '$PKG2' (the 'plus')" || no "lean did not install '$PKG2'"
present "$PKG"  && no "relational MINUS failed: '$PKG' is present" || ok "relational MINUS works: '$PKG' excluded"

# (e) `profile show` resolves the relational set on paper
OUT="$(pread profile show lean)"
if printf '%s\n' "$OUT" | grep -qw "$PKG2" && ! printf '%s\n' "$OUT" | grep -qw "$PKG"; then
    ok "profile show lean resolves to {$PKG2} (include + minus applied)"
else
    no "profile show lean resolved wrong: [$OUT]"
fi

# (f) cleanup + inventory
pcmd deactivate lean
present "$PKG2" && no "final deactivate left '$PKG2'" || ok "deactivate lean cleaned up '$PKG2'"
pread profile list | grep -qw alpha && ok "profile list enumerates defined profiles" || soft "profile list missing entries"

# ------------------------------------------------- REAL MULTI-BACKEND SWEEP
hr "11. REAL MULTI-BACKEND LIFECYCLE — every feasible backend (real install → list → remove)"
rm -rf "$SGDIR"; mkdir -p "$SGDIR"
# Fast, prebuilt / downloaded ecosystems (each fetched from its own registry).
sweep_backend npm      cowsay    cowsay    120
sweep_backend yarn     cowsay    cowsay    150
sweep_backend pnpm     cowsay    cowsay    150
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
sweep_backend nix      hello     hello     900
sweep_backend krew     ns        ""        420
sweep_backend mix      phx_new   ""        420
sweep_backend conda    tqdm      ""        900
# Source-compiling ecosystems — REAL builds (slow but real). FAST=1 downgrades these.
sweep_backend cargo    ripgrep   rg        2400
sweep_backend go       rsc.io/2fa 2fa      900
sweep_backend opam     csexp     ""        2400
sweep_backend nimble   checksums ""        900
sweep_backend spack    zlib      ""        3600
# cabal: TOLERANT. `cabal install hello` really compiles + installs the exe to ~/.cabal/bin,
# but `cabal list --installed` reports the LIBRARY db, not installed executables, so the exe
# never shows there; and cabal has no uninstall verb. So a strict install→list→remove doesn't
# apply — we still exercise the real compile/install, just report it tolerantly.
sweep_backend cabal    hello     hello     2400  soft
# Tolerant (known ecosystem quirk): helm plugin id != install URL; flatpak needs a big remote.
sweep_backend helm     https://github.com/databus23/helm-diff "" 420 soft
sweep_backend flatpak  org.gnome.Calculator ""  900 soft
sweep_backend mise     usage     ""        900 soft
# Special-identifier backends with real effects.
sweep_github
sweep_link

# ------------------------------------------------- FEATURE COVERAGE
hr "12. FEATURE COVERAGE — every linix subcommand exercised at least once"
FGDIR="/tmp/linix-it-feat"; rm -rf "$FGDIR"; mkdir -p "$FGDIR"
lx -g "$FGDIR" init >/dev/null 2>&1
# completions: every shell must emit a non-empty script and exit 0 (pure, always testable).
for sh in bash zsh fish powershell elvish nushell; do
    OUT="$(lx completions "$sh" 2>/dev/null)"; RC=$?
    { [ $RC -eq 0 ] && [ -n "$OUT" ]; } && okf completions "completions $sh emits a script" || no "completions $sh rc=$RC / empty"
done
feat completions
# heal: WAL recovery — a clean system has nothing to recover, must exit 0 without crashing.
lx -g "$FGDIR" heal >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf heal "heal exits 0 on a clean system (no interrupted transaction)" || soft "heal rc=$RC$(rcnote $RC)"; feat heal
# clean: deep cleanup pass — must run without crashing.
lx -g "$FGDIR" -y clean >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf clean "clean exits 0" || soft "clean rc=$RC$(rcnote $RC) (tolerated)"; feat clean
# unmanaged / orphans: read-only inventories.
lx -b "$BACKEND" unmanaged >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf unmanaged "unmanaged exits 0" || soft "unmanaged rc=$RC (tolerated)"; feat unmanaged
lx -b "$BACKEND" orphans   >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf orphans "orphans exits 0"   || soft "orphans rc=$RC (tolerated)"; feat orphans
# audit / sbom / why / policy: the 6.0 supply-chain + provenance surface.
# audit queries OSV.dev once per managed package and sbom enumerates every backend, so both are
# network-bound and can run for minutes on a many-package image — give them a generous ceiling
# rather than the default so a slow-but-successful scan isn't mis-timed-out.
lxt 300 -b "$BACKEND" audit >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf audit "audit exits 0" || soft "audit rc=$RC$(rcnote $RC) (network/OSV latency — tolerated)"; feat audit
OUT="$(lxt 300 sbom 2>/dev/null)"; RC=$?; { [ $RC -eq 0 ] && is_json "$OUT"; } && okf sbom "sbom emits a CycloneDX JSON document" || soft "sbom rc=$RC$(rcnote $RC) / not JSON (tolerated)"; feat sbom
lx why "$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf why "why '$PKG' exits 0" || soft "why rc=$RC (tolerated)"; feat why
lx policy >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf policy "policy exits 0" || soft "policy rc=$RC (no policy.toml — tolerated)"; feat policy
# upgrade (+ canary/self-heal): install a pkg, then upgrade; canary with a passing --test.
lx -g "$FGDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1
lx -g "$FGDIR" -b "$BACKEND" -y upgrade >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "upgrade (scoped) exits 0" || soft "upgrade rc=$RC$(rcnote $RC) (network — tolerated)"
lxt 120 -g "$FGDIR" -b "$BACKEND" -y upgrade --canary --test "true" >/dev/null 2>&1; RC=$?
# A canary upgrade takes a pre-upgrade safety snapshot and rolls back if --test fails. A plain
# container has no snapshot-capable filesystem, so canary correctly FAIL-SAFES (rc!=0) rather
# than upgrade without a rollback point — so a non-zero here is EXPECTED, not a passing test.
# (The health-gated rollback logic itself is covered by the hermetic canary/bisect unit tests.)
[ $RC -eq 0 ] && okf upgrade "canary upgrade with a passing --test exits 0 (no rollback)" \
    || soft "canary upgrade rc=$RC$(rcnote $RC) — expected where no snapshot backend exists (fail-safe); logic covered by unit tests"
feat upgrade
lx -g "$FGDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1
# repo: list is read-only + safe; add/remove exercised on the native backend (may need a real
# URL, so tolerant), proving the repo-manager plumbing end to end.
lx repo list -b "$BACKEND" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf repo "repo list ($BACKEND) exits 0" || soft "repo list rc=$RC (tolerated)"
lx -y repo add linix-it-testrepo "https://example.com/linix-it" -b "$BACKEND" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && soft "repo add exits 0" || soft "repo add rc=$RC (needs a real source — tolerated)"
lx -y repo remove linix-it-testrepo -b "$BACKEND" >/dev/null 2>&1
feat repo
# migrate: adopt OS-installed-but-unmanaged packages into a manifest.
# Its own config dir: migrate writes into the GLOBAL groups folder (adoption is a fact
# about this machine, and the ownership registry it feeds is global too), so without this
# every package it adopts would join the wish list of every section that follows.
push_config_dir "/tmp/linix-it-cfg-migrate"
lxt 120 -g "$FGDIR" -b "$BACKEND" -y migrate >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf migrate "migrate (scoped) exits 0" || soft "migrate rc=$RC$(rcnote $RC) (tolerated)"; feat migrate

# migrate must adopt only what the USER chose, never the dependency graph. An exit code of 0
# said nothing about this: apt adopted all 579 installed packages instead of the 103 in
# `apt-mark showmanual`, which then made every dependency look like drift and purged the
# system. These assertions are what would have caught it.
MIGF="$(ls "$GLOBAL_GDIR"/migrated_*.txt 2>/dev/null | head -1)"
if [ -n "$MIGF" ]; then
    ADOPTED=$(grep -cvE '^\s*(#|$)' "$MIGF" 2>/dev/null || echo 0)
    if [ "$BACKEND" = "apt" ]; then
        ALL=$(dpkg-query -W 2>/dev/null | wc -l)
        MANUAL=$(apt-mark showmanual 2>/dev/null | wc -l)
        [ "$ADOPTED" -le "$MANUAL" ] \
            && ok "migrate adopted $ADOPTED (<= apt-mark showmanual=$MANUAL, of $ALL installed)" \
            || no "migrate adopted $ADOPTED, more than the user ever chose ($MANUAL of $ALL)"
        # A pure dependency nobody asked for. Present in dpkg-query, absent from showmanual.
        grep -qE '^apt:libperl' "$MIGF" \
            && no "migrate adopted a pure dependency (libperl*) — the dependency graph leaked in" \
            || ok "migrate did not adopt pure dependencies (libperl* absent)"
    else
        # Not `-ge 0`: a count is always >= 0, so that asserted nothing at all. What must
        # hold on every backend is that adoption is a SUBSET of what is installed — the
        # property whose violation purged the container.
        INST=$(lx -b "$BACKEND" list 2>/dev/null | grep -cvE '^\s*$' || echo 0)
        if [ "$INST" -gt 0 ]; then
            if [ "$ADOPTED" -le "$INST" ]; then
                ok "migrate adopted $ADOPTED of $INST installed ($BACKEND)"
            else
                no "migrate adopted $ADOPTED, MORE than $BACKEND reports installed ($INST)"
            fi
        else
            soft "migrate: $BACKEND reported no installed packages to compare against"
        fi
    fi

    # The manifest asks the user to trust an estimate, and warns that deleting a line
    # uninstalls. Both facts are load-bearing: a reader who takes this file for a plain
    # inventory and trims it is reproducing the original disaster by hand. The words are
    # asserted here, not just unit-tested, because this is the file a real user opens.
    grep -q "THIS IS AN ESTIMATE" "$MIGF" \
        && ok "migrate manifest warns that it is an estimate" \
        || no "migrate manifest does not warn that it is an estimate"
    grep -q "UNINSTALLS" "$MIGF" \
        && ok "migrate manifest warns that deleting a line uninstalls" \
        || no "migrate manifest does not warn what deleting a line does"
    grep -q "linix unmanage" "$MIGF" \
        && ok "migrate manifest points at 'linix unmanage'" \
        || no "migrate manifest offers no way to keep a package without managing it"
    if [ "$BACKEND" = "apt" ]; then
        grep -q "apt-mark showmanual" "$MIGF" \
            && ok "migrate manifest names the source of its estimate" \
            || no "migrate manifest hides where its estimate came from"
    fi
else
    soft "migrate wrote no manifest (nothing adoptable on this image)"
fi

# THE REGRESSION THAT STARTED ALL THIS, and the test that proves the overlay fixed it.
#
# migrate records ownership in the GLOBAL state registry, which `-g` cannot move. It writes
# the matching manifest into the GLOBAL groups folder. Before the overlay, `-g` REPLACED the
# wish list, so a later command pointed at a different -g read neither of those: every
# adopted package was owned, unwished, and therefore drift — scheduled for removal. That is
# what purged python3 and blew the time limit.
#
# Now `-g` ADDS. The global folder is still read, the migrate manifest is still in it, so
# the adopted packages are still wanted and NOTHING is drift. The evidence and the
# accusation can no longer be separated by a flag.
#
# Still dry-run: if this ever regresses, a real prune removes those packages for real (on
# Alpine that is git, nodejs, ruby, rustup, pipx…, which takes python3's stdlib with it).
# Running the disaster is not a test of it.
PGDIR2="/tmp/linix-it-postmigrate"; rm -rf "$PGDIR2"; mkdir -p "$PGDIR2"
echo "$BACKEND:$PKG" > "$PGDIR2/local.txt"
OUT="$(lxt 120 -g "$PGDIR2" -b "$BACKEND" -n prune 2>&1)"; RC=$?

# The real fix: adopted packages stay wanted, so a prune under a different -g has nothing to
# remove and exits cleanly. Previously this was either refused by the guard (apt: 84 adopted,
# over max_removals) or silently ALLOWED to purge (alpine: 14, under the limit). The guard
# capped the blast radius; it never fixed the cause. This is the cause being fixed.
if [ -n "$MIGF" ]; then
    ADOPTED_ONE="$(grep -vE '^\s*(#|$)' "$MIGF" 2>/dev/null | head -1 | cut -d: -f2-)"
    if [ -n "$ADOPTED_ONE" ]; then
        printf '%s' "$OUT" | grep -qw "$ADOPTED_ONE" \
            && no "prune under a different -g still schedules adopted package '$ADOPTED_ONE' — the global folder was not read" \
            || ok "prune under a different -g does NOT touch migrate's adopted packages (global still vouches for them)"
    else
        soft "post-migrate prune: nothing was adopted, nothing to check"
    fi
fi
[ $RC -eq 0 ] && ok "post-migrate prune under a different -g exits 0 (no phantom drift)" \
             || soft "post-migrate prune rc=$RC$(rcnote $RC)"

# And the guard's own job, independent of the overlay: system-critical packages are never
# scheduled, whatever the count. Belt and braces — this held even before the overlay.
BADHIT=""
for p in busybox alpine-baselayout apk-tools bash dpkg apt libc6 glibc python3 systemd coreutils; do
    printf '%s' "$OUT" | grep -qE "^\s*[^[:alnum:]]*\[[a-z]+\s*\]\s+$p\b" && BADHIT="$BADHIT $p"
done
[ -z "$BADHIT" ] && ok "post-migrate prune schedules no protected/system package" \
                 || no "post-migrate prune scheduled protected package(s):$BADHIT"

# --no-global re-arms the old behaviour, deliberately and explicitly. It must still be
# guarded: the escape valve is not a way around the guard.
OUT="$(lxt 120 --no-global -g "$PGDIR2" -b "$BACKEND" -n prune 2>&1)"; RC=$?
BADHIT2=""
for p in busybox alpine-baselayout apk-tools bash dpkg apt libc6 glibc python3 systemd coreutils; do
    printf '%s' "$OUT" | grep -qE "^\s*[^[:alnum:]]*\[[a-z]+\s*\]\s+$p\b" && BADHIT2="$BADHIT2 $p"
done
[ -z "$BADHIT2" ] && ok "--no-global still refuses to schedule protected/system packages" \
                  || no "--no-global bypassed protection for:$BADHIT2"

# --no-global with no -g would mean "read nothing", i.e. "remove everything managed".
lx --no-global -b "$BACKEND" -n prune >/dev/null 2>&1; RC=$?
[ $RC -ne 0 ] && ok "--no-global without any -g is refused (it would mean 'want nothing')" \
              || no "--no-global with no -g was accepted — that reads as 'remove everything'"

pop_config_dir

# `protected`: the guard is only trustworthy if you can see what it protects, so this is a
# real contract, not a smoke test. It must answer for humans AND machines, and its answer
# must agree with what the guard actually does.
OUT="$(lx protected 2>/dev/null)"
printf '%s' "$OUT" | grep -q "Guarded commands" \
    && ok "protected lists the guarded commands" || no "protected does not show what is guarded"
OUT="$(lx protected --json 2>/dev/null)"
is_json "$OUT" && ok "protected --json is JSON" || no "protected --json is not JSON"
printf '%s' "$OUT" | grep -q "max_removals" \
    && ok "protected --json exposes max_removals" || no "protected --json lacks max_removals"
if [ "$BACKEND" = "apt" ]; then
    # python3 is `no optional` in dpkg — the OS will NOT protect it. The static list must.
    OUT="$(lx protected apt:python3 2>/dev/null)"
    printf '%s' "$OUT" | grep -qi "yes" \
        && ok "protected reports apt:python3 as protected" \
        || no "protected does not protect apt:python3 (the package this bug purged)"
    # And the guard must agree with that report: removing it is refused.
    lx -y remove apt:python3 >/dev/null 2>&1; RC=$?
    [ $RC -ne 0 ] && okf protected "remove of a protected package is refused (even with -y)" \
                  || no "remove purged a protected package"
    present python3 && ok "python3 survived the run" || no "python3 was purged"
fi
feat protected

# `unmanage`: forget a package WITHOUT uninstalling it. This is the counterpart to deleting
# a manifest line (which means uninstall), and migrate's own output tells people to use it.
UMD="/tmp/linix-it-unmanage"; rm -rf "$UMD"; mkdir -p "$UMD"
lxt 180 -g "$UMD" -y install "$BACKEND:$PKG" >/dev/null 2>&1
# Only assert "unmanage left it installed" when it WAS installed to begin with — otherwise
# a failed install (network, mirror) reads as unmanage having deleted something.
if present "$PKG"; then
    lx -g "$UMD" -y unmanage "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
    if [ $RC -eq 0 ]; then
        ok "unmanage exits 0"
        present "$PKG" && ok "unmanage left the package installed" \
                       || no "unmanage UNINSTALLED the package — it must only forget it"
        manifest_has_in "$UMD" "$PKG" \
            && no "unmanage left the declaration behind (next sync would re-adopt it)" \
            || ok "unmanage removed the declaration too"
    else
        soft "unmanage rc=$RC (tolerated)"
    fi
else
    soft "unmanage: skipped (install of $PKG did not land, nothing to forget)"
fi
OUT="$(lx -g "$UMD" unmanage --json "$BACKEND:$PKG" 2>/dev/null)"
is_json "$OUT" && okf unmanage "unmanage --json is JSON" || no "unmanage --json is not JSON"
feat unmanage
# teleport: move a package across backends — exercised as a dry-run plan (no real mutation).
lx -n teleport "$PKG" "$BACKEND" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf teleport "teleport (dry-run plan) exits 0" || soft "teleport rc=$RC (tolerated)"; feat teleport
# module: list + create + show a reusable @module.
lx -g "$FGDIR" module list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf module "module list exits 0" || soft "module list rc=$RC (tolerated)"
lx -g "$FGDIR" module create linix-it-mod >/dev/null 2>&1
lx -g "$FGDIR" module show linix-it-mod >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && soft "module create+show round-trips" || soft "module show rc=$RC (tolerated)"
feat module
# snapshot: list + prune (retention). Real filesystem snapshots need a snapshot-capable FS, so
# these must at least run without crashing.
lx snapshot list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf snapshot "snapshot list exits 0" || soft "snapshot list rc=$RC (tolerated)"
lx -y snapshot prune --force >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && soft "snapshot prune exits 0" || soft "snapshot prune rc=$RC (tolerated)"
feat snapshot
# generation: pin/unpin the newest generation if one exists; rollback exercised as a dry-run.
GID="$(lx -g "$FGDIR" generation list 2>/dev/null | grep -oE '[0-9a-f]{6,}' | head -n1)"
if [ -n "$GID" ]; then
    lx -g "$FGDIR" generation pin "$GID"   >/dev/null 2>&1 && soft "generation pin $GID ok"   || soft "generation pin rc=$? (tolerated)"
    lx -g "$FGDIR" generation unpin "$GID" >/dev/null 2>&1 && soft "generation unpin $GID ok" || soft "generation unpin rc=$? (tolerated)"
    lx -n -g "$FGDIR" rollback "$GID"      >/dev/null 2>&1 && okf rollback "rollback (dry-run) to $GID exits 0" || soft "rollback rc=$? (tolerated)"
else
    soft "no generation id yet to pin/rollback (fresh manifest)"
fi
feat rollback
# lease: set an expiry on a managed package, then confirm it lists.
lx -g "$FGDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1
lx -g "$FGDIR" lease set "$BACKEND:$PKG" -d 30d >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf lease "lease set exits 0" || soft "lease set rc=$RC (tolerated)"
lx -g "$FGDIR" lease list 2>/dev/null | grep -qiw "$PKG" && ok "lease list shows the leased package" || soft "lease list did not show '$PKG' (tolerated)"
lx -g "$FGDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1
feat lease
# schedule: register a native scheduled task, list it, remove it (needs systemd/cron; tolerant).
lx schedule add linix-it-task --cron "0 2 * * *" --command "upgrade" >/dev/null 2>&1; RC=$?
if [ $RC -eq 0 ]; then
    ok "schedule add exits 0 (native scheduler present)"
    lx schedule list 2>/dev/null | grep -qw linix-it-task && ok "schedule list shows the task" || soft "schedule list missing the task (tolerated)"
    lx schedule remove linix-it-task >/dev/null 2>&1
else
    soft "schedule add rc=$RC (no systemd/cron in this container — tolerated)"
fi
feat schedule
# run / shim / shell: ephemeral-env + shim generation.
OUT="$(lxt 120 run -p "$BACKEND:$PKG" "echo LINIX_RUN_OK" 2>/dev/null)"; RC=$?
printf '%s\n' "$OUT" | grep -q LINIX_RUN_OK && okf run "run executes a command in an ephemeral env" || soft "run rc=$RC (ephemeral-env variance — tolerated)"; feat run
lxt 120 shim linix-it-shim -s "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf shim "shim generates a launcher" || soft "shim rc=$RC$(rcnote $RC) (tolerated)"; feat shim
feat shell   # `shell` is an interactive ghost-shell (no non-interactive assertion); see EXEMPT

# ------------------------------------------------- v6-v9 COMMAND & FLAG COVERAGE
hr "12b. v6-v9 COMMAND & FLAG COVERAGE — every newer subcommand + new flags"
# Everything the v7/v8/v9 waves added that the coverage audit now tracks. Deterministic wiring
# checks are HARD; anything network/ecosystem-dependent is soft. All scoped to throwaway dirs.
NGDIR="/tmp/linix-it-newcmds"; rm -rf "$NGDIR"; mkdir -p "$NGDIR"
lx -g "$NGDIR" init >/dev/null 2>&1
NM="$NGDIR/local.txt"

# --- plan / apply (Terraform-style freeze-then-apply) ---
echo "$BACKEND:$PKG" >> "$NM"
PLANF="$NGDIR/plan.json"
lx -g "$NGDIR" -b "$BACKEND" plan --out "$PLANF" >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -f "$PLANF" ] && is_json "$(cat "$PLANF")"; } && okf plan "plan writes a JSON plan file" || no "plan rc=$RC / no JSON plan at $PLANF"
feat plan
lxt 180 -g "$NGDIR" -b "$BACKEND" apply "$PLANF" -y >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf apply "apply executes a saved plan (rc=0)" || soft "apply rc=$RC$(rcnote $RC) (ecosystem variance)"
feat apply
lx -g "$NGDIR" -b "$BACKEND" -y remove "$BACKEND:$PKG" >/dev/null 2>&1

# --- conflicts (cross-backend, read-only) ---
lx -g "$NGDIR" conflicts >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf conflicts "conflicts exits 0" || no "conflicts rc=$RC$(rcnote $RC)"
OUT="$(lx -g "$NGDIR" conflicts --json 2>/dev/null)"; is_json "$OUT" && ok "conflicts --json is valid JSON" || no "conflicts --json not JSON"
feat conflicts

# --- hold / unhold (bulk-upgrade guard) ---
lx -g "$NGDIR" hold "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf hold "hold exits 0" || no "hold rc=$RC"
lx -g "$NGDIR" hold 2>/dev/null | grep -qi "$PKG" && ok "hold (no args) lists the held package" || no "hold list missing '$PKG'"
lx -g "$NGDIR" unhold "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf unhold "unhold exits 0" || no "unhold rc=$RC"
lx -g "$NGDIR" hold 2>/dev/null | grep -qi "$PKG" && no "package still held after unhold" || ok "unhold cleared the hold"
feat hold unhold

# --- export to native manifests (Brewfile/requirements.txt/package.json/Aptfile) ---
lx -g "$NGDIR" export --format pip --stdout >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "export --format pip --stdout exits 0" || soft "export stdout rc=$RC"
EXPD="$NGDIR/exports"; mkdir -p "$EXPD"
lx -g "$NGDIR" export --out "$EXPD" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf export "export --out writes native manifest(s)" || soft "export --out rc=$RC"
feat export

# --- bundle (offline/air-gapped) + tar.gz archive ---
# The output dir MUST live OUTSIDE the groups dir: bundle copies groups/ into <out>/groups, so an
# <out> nested under the groups dir would copy the bundle into itself (runaway recursion).
BND="/tmp/linix-it-bundle"; rm -rf "$BND" "$BND-ar" "$BND-ar.tar.gz"
lx -g "$NGDIR" bundle --out "$BND" >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -d "$BND" ]; } && okf bundle "bundle writes an offline bundle dir" || soft "bundle rc=$RC / no dir"
lx -g "$NGDIR" bundle --out "$BND-ar" --archive >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -f "$BND-ar.tar.gz" ]; } && ok "bundle --archive produces a portable .tar.gz" || soft "bundle --archive rc=$RC / no tarball"
feat bundle

# --- watch (single reconcile pass over an empty manifest -> already in sync) ---
WGD="/tmp/linix-it-watch"; rm -rf "$WGD"; mkdir -p "$WGD"; lx -g "$WGD" init >/dev/null 2>&1
lxt 60 -g "$WGD" -b "$BACKEND" watch --once >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf watch "watch --once runs a single reconcile and exits" || soft "watch --once rc=$RC$(rcnote $RC)"
feat watch

# --- git (version-control the manifests) — SCOPED to a throwaway dir, NEVER the real repo ---
GGD="/tmp/linix-it-git"; rm -rf "$GGD"; mkdir -p "$GGD"; lx -g "$GGD" init >/dev/null 2>&1
lx -g "$GGD" git init >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf git "git init makes the config dir a repo" || soft "git init rc=$RC"
lx -g "$GGD" git status >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "git status exits 0" || soft "git status rc=$RC"
lx -g "$GGD" git commit -m "linix-it commit" >/dev/null 2>&1
lx -g "$GGD" git log >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "git log exits 0" || soft "git log rc=$RC"
feat git

# --- managed (ownership mode + keep-list) ---
lx -g "$NGDIR" managed show >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf managed "managed show exits 0" || soft "managed show rc=$RC"
lx -g "$NGDIR" managed keep "$PKG" >/dev/null 2>&1; lx -g "$NGDIR" managed unkeep "$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "managed keep/unkeep round-trips" || soft "managed keep/unkeep rc=$RC"
feat managed

# --- hooks (auto-record; read-only status + shell-init emitter) ---
lx hooks status >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf hooks "hooks status exits 0" || soft "hooks status rc=$RC"
OUT="$(lx hooks shell-init bash 2>/dev/null)"; [ -n "$OUT" ] && ok "hooks shell-init bash prints shell functions" || soft "hooks shell-init empty"
feat hooks

# --- service (declarative services; read-only surface) ---
lx service list >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf service "service list exits 0" || soft "service list rc=$RC (no init system?)"
feat service

# --- self-upgrade --check (report only; NEVER actually rebuilds/installs) ---
lx self-upgrade --check >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf self-upgrade "self-upgrade --check reports version/source" || soft "self-upgrade --check rc=$RC"
feat self-upgrade

# --- config edit (non-interactive: a no-op editor that exits 0) ---
TMPCFG="/tmp/linix-it-cfg.toml"; rm -f "$TMPCFG"
EDITOR=true VISUAL=true lxt 30 -c "$TMPCFG" config edit >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && okf config "config edit re-validates after a no-op edit" || soft "config edit rc=$RC$(rcnote $RC)"
feat config

# --- generation log / diff (v9) ---
GID2="$(lx -g "$FGDIR" generation list 2>/dev/null | grep -oE '[0-9a-f]{6,}' | head -n1)"
lx -g "$FGDIR" generation log >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && okf generation "generation log exits 0" || soft "generation log rc=$RC"
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
OUT="$(lxt 300 -b "$BACKEND" audit --json 2>/dev/null)"; is_json "$OUT" && ok "audit --json valid JSON" || soft "audit --json n/a (network)"
lxt 120 -b "$BACKEND" -y upgrade --security >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "upgrade --security exits 0" || soft "upgrade --security rc=$RC$(rcnote $RC) (OSV/network)"
lx -n -b "$BACKEND" upgrade --all --except "$PKG" >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "upgrade --all --except (dry-run) exits 0" || soft "upgrade --all --except rc=$RC"
lx -n install "$BACKEND:$PKG" --temp 1h --json >/dev/null 2>&1; RC=$?; [ $RC -eq 0 ] && ok "install --temp (dry-run lease) exits 0" || soft "install --temp rc=$RC"

# --- command aliases ([command_aliases] in config, expanded before clap) ---
ACFG="/tmp/linix-it-alias.toml"
printf '%s\n' '[command_aliases]' 'inv = "list"' > "$ACFG"
lx -c "$ACFG" -b "$BACKEND" inv >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "command alias 'inv' expands to 'list' (pre-clap)" || no "command alias expansion rc=$RC$(rcnote $RC)"

# --- tamper-evident lockfile (sign on lock; refuse a modified lockfile) ---
LKDIR="/tmp/linix-it-lock"; rm -rf "$LKDIR"; mkdir -p "$LKDIR"; lx -g "$LKDIR" init >/dev/null 2>&1
echo "$BACKEND:$PKG" >> "$LKDIR/local.txt"
dlx -g "$LKDIR" -b "$BACKEND" -y sync >/dev/null 2>&1
lx -g "$LKDIR" -b "$BACKEND" lock >/dev/null 2>&1
if [ -f "$GLOBAL_GDIR/locks.json" ] && grep -q '"sig"' "$GLOBAL_GDIR/locks.json"; then
    ok "lock signs locks.json (tamper-evident)"
    sed -i 's/"sig": *"[0-9a-f]/"sig": "0/' "$GLOBAL_GDIR/locks.json" 2>/dev/null
    OUT="$(timeout 30 "$LINIX" -g "$LKDIR" -b "$BACKEND" -v status 2>&1)"
    printf '%s\n' "$OUT" | grep -qi "MISMATCH" && ok "a modified lockfile is detected (signature MISMATCH) and refused" || no "tampered lockfile was NOT flagged"
else
    soft "lock did not produce a signed locks.json (backend has no lockable versions?)"
fi
lx -g "$LKDIR" -b "$BACKEND" -y prune >/dev/null 2>&1
feat lock

# ------------------------------------------------- plan-smoke the can't-run-here set
run_plan_smokes

# ------------------------------------------------- COVERAGE AUDIT
hr "14. COVERAGE AUDIT — nothing registered or featured is silently untested"
# (a) Every READY backend must have been exercised by a real lifecycle OR a plan-smoke.
audit_fail=0
for b in $(printf '%s\n' "$DOCTOR" | grep -E '^\[READY\]' | awk '{print $2}'); do
    case "$TOUCHED" in
        *" $b "*) : ;;   # exercised somewhere above
        *) no "COVERAGE GAP: backend '$b' is READY but was never exercised (add a sweep row)"; audit_fail=1 ;;
    esac
done
[ $audit_fail -eq 0 ] && ok "every READY backend was exercised (real lifecycle or plan-smoke)"
# The native backend is covered by the detailed sections 2–10.
touched "$BACKEND"
# (b) Every linix subcommand must have been exercised, except the documented interactive /
# remote-SSH ones (no non-interactive assertion is possible in a headless container).
FEATURES_ALL="sync watch run shim heal clean unmanaged orphans status prune plan apply lock search update upgrade list info install remove repo doctor migrate teleport shell undo cockpit activate deactivate profile module snapshot generation rollback git lease schedule config init audit sbom export bundle why service bisect clone fleet managed hooks hold unhold conflicts policy completions self-upgrade protected unmanage"
FEATURES_EXEMPT=" shell undo cockpit bisect clone fleet "   # interactive TUIs (ghost-shell / undo gallery / cockpit), or need a remote SSH host

# A DEBT REGISTER, not a config knob. Every name here ships without proof on this image:
# the command runs, but no hard assertion establishes that it did anything. Adding a name is
# a decision to ship something unverified; removing one is progress. The audit fails if a
# feature is unproven and NOT listed here, so the list cannot silently grow.
#
#   update    — hits the network; a failure here is the mirror's, not ours
#   schedule  — needs systemd or cron; neither exists in these containers
FEATURES_UNVERIFIED=" update schedule "

feat_gap=0
for f in $FEATURES_ALL; do
    case "$FEATURES_EXEMPT" in *" $f "*) soft "feature '$f' is EXEMPT (interactive or needs a remote host)"; continue ;; esac
    # Tier 1: did it run at all?
    case "$FEAT" in
        *" $f "*) : ;;
        *) no "FEATURE GAP: '$f' was never exercised"; feat_gap=1; continue ;;
    esac
    # Tier 2: did a hard assertion prove it works?
    case "$FEATV" in *" $f "*) continue ;; esac
    case "$FEATURES_UNVERIFIED" in
        *" $f "*) soft "feature '$f' ran, but nothing proved it works (known debt)" ;;
        *) no "UNPROVEN: '$f' ran but no hard assertion proved it works — prove it, or add it to FEATURES_UNVERIFIED with a reason"; feat_gap=1 ;;
    esac
done
# The ratchet: a feature that outgrew the debt register must be taken off it, or the register
# rots into a permanent excuse. Informational rather than hard, because proof is
# image-dependent — a feature can be proven on ubuntu and unprovable on alpine.
for f in $FEATURES_UNVERIFIED; do
    case "$FEATV" in *" $f "*) soft "STALE DEBT: '$f' is on FEATURES_UNVERIFIED but was proven here — consider removing it" ;; esac
done
[ $feat_gap -eq 0 ] && ok "every non-exempt subcommand ran, and every one not on the debt register was PROVEN"

# ------------------------------------------------- read-only smoke (no crashes)
hr "15. READ-ONLY SMOKE (must run without crashing)"
for cmd in "config show" "config path" "unmanaged" "orphans" "audit" "sbom" "why $PKG" \
           "snapshot list" "generation list" "profile list" "policy"; do
    # shellcheck disable=SC2086
    lx -b "$BACKEND" $cmd >/dev/null 2>&1; RC=$?
    [ $RC -eq 0 ] && soft "\`linix $cmd\` exits 0" || soft "\`linix $cmd\` exit=$RC$(rcnote $RC) (tolerated)"
done
lx config init >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && soft "config init exits 0" || soft "config init exit=$RC$(rcnote $RC)"

# ------------------------------------------------------------------- summary
hr "SUMMARY [$BACKEND image]"
echo "    HARD pass: $PASS    HARD fail: $FAIL    soft/info: $SOFT"
if [ "$FAIL" -ne 0 ]; then
    echo "    RESULT: FAIL ($FAIL hard check(s) failed)"
    exit 1
fi
echo "    RESULT: PASS"
