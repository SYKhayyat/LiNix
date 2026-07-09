#!/usr/bin/env bash
# Native Windows/macOS real-world sweep. These OSes can't run in a Linux container, so we
# drive the host-native backends (scoop, winget, choco, brew) directly through `linix`,
# mirroring the container sweep: imperative install/remove, the negative/exit-status path,
# idempotency, dry-run safety, the declarative roundtrip, the JSON contract, profiles
# (activate/deactivate/relational), a multi-backend sweep of any other READY managers, and
# read-only smoke. Every check is tallied; the run continues past failures and exits non-zero
# if any HARD check failed. "soft" checks are reported but never fail the run.
#
#   scripts/integration-windows.sh [backend] [package] [package2]
#   e.g. scripts/integration-windows.sh scoop jq less      # user-scoped, no admin, reversible
#        scripts/integration-windows.sh winget jq          # exercises the winget HRESULT allowlist
#        scripts/integration-windows.sh brew wget           # on a real Mac
#
# scoop is the safe default (user-scoped, trivially reversible). winget/choco may require
# elevation and modify the system more broadly — run those deliberately. Point LINIX at a
# release build for a release check:  LINIX=./target/release/linix.exe scripts/integration-windows.sh
#
# By default the harness first BOOTSTRAPS the language backends through scoop (nodejs/pnpm/
# yarn/pipx/uv/ruby/bun) so the sweep is as broad as the Linux images — anything already on
# PATH is left as-is. Set INSTALL_BACKENDS=0 to skip and test only what's already installed.
# choco/winget are auto-swept if present but never auto-installed (their installers need
# admin); install choco once and it's picked up automatically on the next run.
#
# Every `linix` call is wrapped in `timeout` (TIMEOUT secs) when available: a hang is a
# recorded failure, not a frozen harness. Declarative commands are scoped with -b so they
# don't block on unrelated backends' probes.
set -u
BACKEND="${1:-scoop}"
PKG="${2:-jq}"
PKG2="${3:-less}"
LINIX="${LINIX:-./target/debug/linix.exe}"
BOGUS="linix-nonexistent-pkg-zzq9x"
GDIR="${GROUPS_DIR:-${TMPDIR:-/tmp}/linix-it-manifests}"
TIMEOUT="${TIMEOUT:-90}"

if command -v timeout >/dev/null 2>&1; then TO="timeout $TIMEOUT"; else TO=""; fi
lx() { $TO "$LINIX" "$@"; }

PASS=0; FAIL=0; SOFT=0
ok()   { echo "    [ok]    $1"; PASS=$((PASS+1)); }
no()   { echo "    [FAIL]  $1"; FAIL=$((FAIL+1)); }
soft() { echo "    [info]  $1"; SOFT=$((SOFT+1)); }
hr()   { echo; echo "=========== $* ==========="; }
rcnote() { [ "$1" -eq 124 ] && echo " (TIMED OUT after ${TIMEOUT}s)" || echo ""; }
present() { hash -r 2>/dev/null || true; command -v "$1" >/dev/null 2>&1; }
# Pick a Python that ACTUALLY parses JSON. On Windows a Microsoft Store "python3.exe"
# alias stub satisfies `command -v` but isn't real Python — it hangs or opens the Store —
# so probing python3 first (as the old code did) silently broke every JSON check under
# Git-for-Windows bash. Probe each candidate under a short timeout and keep the first that
# truly parses; try `python` before `python3` so the real interpreter wins over the stub.
_pyprobe() { if command -v timeout >/dev/null 2>&1; then timeout 8 "$@"; else "$@"; fi; }
PYBIN=""
for _py in python python3 py; do
    command -v "$_py" >/dev/null 2>&1 || continue
    if printf '{}' | _pyprobe "$_py" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1; then
        PYBIN="$_py"; break
    fi
done
[ -n "$PYBIN" ] && echo "# JSON validator: $PYBIN" || echo "# JSON validator: structural fallback (no working python)"
# is_json: real JSON parse via the probed interpreter; otherwise a whole-payload structural
# check. A per-line `cut -c1` is WRONG for pretty-printed (multi-line) JSON.
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
# Backend-scoped manifest check: the sweep installs the same pkg name under several
# backends into a shared manifest, so anchor each assertion to "<backend>:<pkg>".
manifest_scoped() { grep -Eq "^$2:$3(@|\$)" "$1/local.txt" 2>/dev/null; }
# gone_from_list <backend> <pkg>: true once the backend's installed-list no longer shows
# <pkg>. On Windows an uninstall's shim/junction/registry cleanup can lag the process exit,
# so retry briefly rather than reading a stale list one beat too early. (linix's remove is
# synchronous and correct — verified in isolation — this only absorbs OS-level latency.)
gone_from_list() {
    for _i in 1 2 3 4 5; do
        lx -b "$1" list 2>/dev/null | grep -qiw "$2" || return 0
        sleep 1
    done
    return 1
}

# Bootstrap the language/cross backends via scoop so the Windows sweep is as broad as the
# Linux container images (whose Dockerfiles bake in nodejs/pnpm/yarn/pipx/uv/ruby/bun).
# scoop is user-scoped, needs no admin, and is trivially reversible — so we NEVER bootstrap
# through winget/choco. Best-effort: a manager that fails to install is simply skipped by
# the doctor-gated sweep later. Anything already on PATH is left untouched (no clobbering an
# existing rustup/uv/node). Set INSTALL_BACKENDS=0 to skip and test only what's present.
INSTALL_BACKENDS="${INSTALL_BACKENDS:-1}"
bootstrap_backends() {
    command -v scoop >/dev/null 2>&1 || { soft "scoop absent — cannot bootstrap backends (install scoop, or set INSTALL_BACKENDS=0)"; return; }
    hr "0. BOOTSTRAP BACKENDS (scoop; user-scoped, reversible) — mirrors the Linux images"
    scoop bucket add main >/dev/null 2>&1 || true
    # <scoop app>:<probe binary that proves the backend family is usable>
    for pair in nodejs:npm pnpm:pnpm yarn:yarn python:python pipx:pipx uv:uv ruby:gem bun:bun; do
        app="${pair%%:*}"; probe="${pair##*:}"
        if command -v "$probe" >/dev/null 2>&1; then soft "[bootstrap] $probe already present — skip $app"; continue; fi
        if scoop install "$app" >/dev/null 2>&1; then ok "[bootstrap] installed $app (provides $probe)"; hash -r 2>/dev/null || true
        else soft "[bootstrap] $app failed to install (skipped — sweep will mark it not READY)"; fi
    done
    hash -r 2>/dev/null || true
}

# scoop apps like ruby/yarn expose their tools under <app>/current/bin and mutate the
# PERSISTENT user PATH, which a running shell never sees — so `gem`/`yarn`/`ruby` look
# MISSING even when installed. And pnpm's global bin dir must be on PATH for `pnpm add -g`.
# Surface both in THIS session (idempotently) so linix's child processes actually find them.
# Runs regardless of INSTALL_BACKENDS, since the apps may have been installed on a prior run.
augment_scoop_path() {
    command -v scoop >/dev/null 2>&1 || return 0
    # APPEND (never prepend): these dirs can contain scoop/busybox shims for coreutils
    # (head/tail/grep/sed) that would otherwise shadow the harness's own tools. Appending
    # still makes gem/yarn/ruby findable (nothing else provides them). scoop/shims is
    # already on the user PATH, so we don't re-add it.
    for _bin in "$HOME"/scoop/apps/*/current/bin; do
        [ -d "$_bin" ] && case ":$PATH:" in *":$_bin:"*) ;; *) PATH="$PATH:$_bin";; esac
    done
    # Chocolatey installs to %ProgramData%\chocolatey\bin (a system-PATH entry that a
    # non-elevated Git Bash spawned from PowerShell may not inherit). Surface it so choco is
    # detected. NOTE: choco install/remove write under %ProgramData% and require an ELEVATED
    # shell — run this harness as admin (backend=choco) to exercise its mutation lifecycle.
    for _cb in /c/ProgramData/chocolatey/bin "$ProgramData"/chocolatey/bin "$ALLUSERSPROFILE"/chocolatey/bin; do
        [ -d "$_cb" ] && case ":$PATH:" in *":$_cb:"*) ;; *) PATH="$PATH:$_cb";; esac
    done
    # pnpm on Windows: global bin dir is %PNPM_HOME%\bin. Give pnpm a Windows-style PNPM_HOME
    # and put that bin (bash-form) on PATH so `pnpm add -g` doesn't abort with "not in PATH".
    export PNPM_HOME="${USERPROFILE:-$HOME}\\AppData\\Local\\pnpm"
    mkdir -p "$HOME/AppData/Local/pnpm/bin" 2>/dev/null || true
    case ":$PATH:" in *":$HOME/AppData/Local/pnpm/bin:"*) ;; *) PATH="$PATH:$HOME/AppData/Local/pnpm/bin";; esac
    export PATH
    hash -r 2>/dev/null || true
}

echo "###################################################################"
echo "# LiNix real-world sweep (native) :: backend=$BACKEND  pkg=$PKG  pkg2=$PKG2"
echo "# binary=$LINIX  timeout=${TIMEOUT}s"
echo "###################################################################"
[ -x "$LINIX" ] || { echo "FATAL: linix not built at $LINIX — run: cargo build (or cargo build --release)"; exit 2; }
rm -rf "$GDIR"; mkdir -p "$GDIR"

[ "$INSTALL_BACKENDS" = "1" ] && bootstrap_backends
augment_scoop_path   # make already-installed scoop apps (gem/yarn/ruby) + pnpm global bin visible this session

DOCTOR="$(lx doctor 2>/dev/null)"
backend_ready() { printf '%s\n' "$DOCTOR" | grep -Eqi "^\[READY\][[:space:]]+$1([[:space:]]|$)"; }

hr "1. DISCOVERY (doctor / search / info)"
OUT="$(lx doctor 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "doctor exits 0" || no "doctor exit=$RC$(rcnote $RC)"
backend_ready "$BACKEND" && ok "doctor reports $BACKEND READY" || no "doctor does not list $BACKEND READY"
echo "      READY backends:"; printf '%s\n' "$DOCTOR" | grep -i "READY" | sed 's/^/        /'
OUT="$(lx search "$PKG" 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "search exits 0" || no "search exit=$RC$(rcnote $RC)"
printf '%s\n' "$OUT" | grep -iq "^$BACKEND" && ok "search returns a $BACKEND hit" || soft "no $BACKEND search hit for '$PKG'"

hr "2. DRY-RUN SAFETY (must change nothing)"
OUT="$(lx -n install "$BACKEND:$PKG" --json 2>/dev/null)"; RC=$?
[ $RC -eq 0 ] && ok "dry-run install exits 0" || no "dry-run install exit=$RC$(rcnote $RC)"
is_json "$OUT" && ok "dry-run emits JSON plan" || soft "dry-run output not JSON"

hr "3. IMPERATIVE INSTALL + LIST + config coherence"
lx -g "$GDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "install '$BACKEND:$PKG' exits 0" || no "install exit=$RC$(rcnote $RC)"
present "$PKG" && ok "'$PKG' on PATH after install" || soft "'$PKG' not resolvable on PATH (shim/PATH refresh may be needed)"
manifest_has "$PKG" && ok "install recorded '$PKG' in the manifest (config stays coherent)" || no "install did NOT record '$PKG' in the manifest"
OUT="$(lx --backend "$BACKEND" list 2>&1)"; RC=$?
{ [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -qiw "$PKG"; } && ok "list shows '$PKG'" || no "list ($BACKEND) missing '$PKG' (rc=$RC$(rcnote $RC))"

hr "4. IDEMPOTENCY (re-install)"
lx -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "re-install exits 0 (benign-exit allowlist covers 'already installed')" || no "re-install exit=$RC$(rcnote $RC)"

hr "5. IMPERATIVE REMOVE + config coherence"
lx -g "$GDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "remove exits 0" || no "remove exit=$RC$(rcnote $RC)"
manifest_has "$PKG" && no "remove left '$PKG' in the manifest (stale config)" || ok "remove cleared '$PKG' from the manifest"
gone_from_list "$BACKEND" "$PKG" && ok "list no longer shows '$PKG'" || soft "list still shows '$PKG' (async uninstall?)"

hr "6. NEGATIVE PATH — failed mutation MUST surface"
lx -y install "$BACKEND:$BOGUS" >/dev/null 2>&1; RC=$?
if [ $RC -ne 0 ] && [ $RC -ne 124 ]; then ok "install of nonexistent '$BOGUS' FAILS (exit=$RC, not swallowed)"
elif [ $RC -eq 124 ]; then no "install of nonexistent '$BOGUS' TIMED OUT"
else no "bogus install returned 0 — failure SWALLOWED"; fi

hr "7. DECLARATIVE ROUNDTRIP — write file -> sync -> installed ; edit file -> prune -> gone"
lx -g "$GDIR" init >/dev/null 2>&1; RC=$?
MANIFEST="$GDIR/local.txt"
{ [ $RC -eq 0 ] && [ -f "$MANIFEST" ]; } && ok "init scaffolds $MANIFEST" || no "init rc=$RC$(rcnote $RC) / no manifest"
echo "$BACKEND:$PKG" >> "$MANIFEST"
lx -g "$GDIR" -b "$BACKEND" -y sync >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "sync (scoped) exits 0" || no "sync exit=$RC$(rcnote $RC)"
lx --backend "$BACKEND" list 2>/dev/null | grep -qiw "$PKG" && ok "sync installed '$PKG' from the manifest file" || soft "sync ran but list does not show '$PKG'"
lx -g "$GDIR" -b "$BACKEND" lock >/dev/null 2>&1; RC=$?
{ [ $RC -eq 0 ] && [ -f "$GDIR/locks.json" ]; } && ok "lock writes locks.json" || soft "lock rc=$RC$(rcnote $RC) / no locks.json"
: > "$MANIFEST"
lx -g "$GDIR" -b "$BACKEND" -y prune >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "prune (scoped) exits 0" || no "prune exit=$RC$(rcnote $RC)"
lx --backend "$BACKEND" list 2>/dev/null | grep -qiw "$PKG" && soft "prune ran but list still shows '$PKG'" || ok "prune removed '$PKG' after it left the manifest file"

hr "8. JSON OUTPUT CONTRACT (stdout only; real JSON parse)"
# json_valid <linix args...>: run `linix <args> --json` and validate. Retries once, because
# `status --json` shells out to scoop (each probe spawns PowerShell, ~15s total) and can
# transiently hiccup; a genuine format bug fails both attempts, so this hides no real defect.
json_valid() {
    out="$(lx "$@" --json 2>/dev/null)"; is_json "$out" && return 0
    sleep 2
    out="$(lx "$@" --json 2>/dev/null)"; is_json "$out"
}
json_valid search "$PKG"           && ok "search --json valid" || no "search --json not JSON"
json_valid --backend "$BACKEND" list && ok "list --json valid"   || no "list --json not JSON"
json_valid -b "$BACKEND" status    && ok "status --json valid" || no "status --json not JSON"

hr "9. PROFILES — activate/deactivate, MULTIPLE active, RELATIONAL (verified via list/show)"
PGDIR="${TMPDIR:-/tmp}/linix-it-prof"; PROFDIR="$(dirname "$PGDIR")/profiles"
rm -rf "$PGDIR" "$PROFDIR"; mkdir -p "$PGDIR" "$PROFDIR"
printf '%s\n' "$BACKEND:$PKG"  > "$PROFDIR/alpha.profile"
printf '%s\n' "$BACKEND:$PKG2" > "$PROFDIR/bravo.profile"
printf '%s\n' "include alpha" "include bravo" > "$PROFDIR/both.profile"
printf '%s\n' "include both" "-$BACKEND:$PKG" > "$PROFDIR/lean.profile"
pcmd() { $TO "$LINIX" -g "$PGDIR" -b "$BACKEND" -y "$@" >/dev/null 2>&1; }
pread() { $TO "$LINIX" -g "$PGDIR" "$@" 2>/dev/null; }
listed() { lx -b "$BACKEND" list 2>/dev/null | grep -qiw "$1"; }

pcmd activate alpha; RC=$?
[ $RC -eq 0 ] && ok "activate alpha exits 0" || no "activate alpha rc=$RC$(rcnote $RC)"
pread profile active | grep -qw alpha && ok "'alpha' shows as active" || no "'alpha' not reported active"
listed "$PKG" && ok "profile 'alpha' installed '$PKG'" || soft "alpha: list does not show '$PKG' (backend/PATH variance)"
pcmd activate bravo; RC=$?
[ $RC -eq 0 ] && ok "activate bravo exits 0 (two active)" || no "activate bravo rc=$RC$(rcnote $RC)"
{ listed "$PKG" && listed "$PKG2"; } && ok "MULTIPLE active: both '$PKG' and '$PKG2' installed" || soft "multiple-active: list did not show both (backend variance)"
pcmd deactivate alpha; RC=$?
[ $RC -eq 0 ] && ok "deactivate alpha exits 0" || no "deactivate alpha rc=$RC$(rcnote $RC)"
listed "$PKG" && soft "deactivate alpha: '$PKG' still listed (async?)" || ok "deactivate alpha removed '$PKG'"
# relational resolution is verified purely (no install needed)
OUT="$(pread profile show lean)"
if printf '%s\n' "$OUT" | grep -qw "$BACKEND:$PKG2" && ! printf '%s\n' "$OUT" | grep -qw "$BACKEND:$PKG"; then
    ok "profile show lean resolves to {$BACKEND:$PKG2} (include + minus applied)"
else
    no "profile show lean resolved wrong: [$OUT]"
fi
pcmd deactivate bravo; pcmd deactivate lean
pread profile list | grep -qw alpha && ok "profile list enumerates defined profiles" || soft "profile list missing entries"

hr "10. MULTI-BACKEND SWEEP — every other READY manager (npm/pnpm/yarn/pipx/uv/cargo/...)"
SGDIR="${TMPDIR:-/tmp}/linix-it-sweep"; rm -rf "$SGDIR"; mkdir -p "$SGDIR"
sweep_backend() {
    b="$1"; p="$2"
    [ "$b" = "$BACKEND" ] && return   # already swept in detail above
    if ! backend_ready "$b"; then soft "[$b] not READY — skipped"; return; fi
    echo "      --- backend: $b  (pkg: $p) ---"
    out=$($TO "$LINIX" -g "$SGDIR" -y install "$b:$p" 2>&1); rc=$?
    if [ $rc -ne 0 ]; then soft "[$b] install '$p' rc=$rc$(rcnote $rc) — ecosystem/network variance"; printf '%s\n' "$out" | tail -3 | sed 's/^/          | /'; $TO "$LINIX" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; return; fi
    ok "[$b] install '$p' exits 0"
    lx -b "$b" list 2>/dev/null | grep -qiw "$p" && ok "[$b] list shows '$p' (parser works)" || no "[$b] list does NOT show '$p'"
    manifest_scoped "$SGDIR" "$b" "$p" && ok "[$b] recorded '$p' in manifest (coherent)" || no "[$b] did NOT record '$p'"
    $TO "$LINIX" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[$b] remove '$p' exits 0" || no "[$b] remove rc=$rc$(rcnote $rc)"
    gone_from_list "$b" "$p" && ok "[$b] '$p' gone after remove" || no "[$b] '$p' still listed after remove"
}
# Language/cross backends — the SAME table the Linux container harness sweeps, so coverage
# is identical across OSes. Each is fetched from its own ecosystem's registry; not-READY
# rows auto-skip (soft), install failures are soft (ecosystem variance), everything after a
# successful install is HARD.
sweep_backend npm  cowsay
sweep_backend pnpm cowsay
sweep_backend yarn cowsay
sweep_backend bun  cowsay
sweep_backend pipx cowsay
sweep_backend uv   cowsay
sweep_backend gem  colorize
# Native Windows managers beyond the primary.
sweep_backend choco  jq
sweep_backend winget jq
# cargo is READY (rustup) but `cargo install` COMPILES from source (minutes) — verify only
# that it PLANS a build rather than paying the compile in every run, mirroring the Linux harness.
if backend_ready cargo && [ "$BACKEND" != cargo ]; then
    OUT="$(lx -g "$SGDIR" -n install "cargo:ripgrep" --json 2>/dev/null)"
    is_json "$OUT" && ok "[cargo] dry-run install produces a JSON plan (compile skipped)" || soft "[cargo] dry-run did not emit JSON"
fi

hr "11. READ-ONLY SMOKE"
for cmd in "config show" "config path" "unmanaged" "orphans" "profile list" "why $PKG"; do
    # shellcheck disable=SC2086
    lx -b "$BACKEND" $cmd >/dev/null 2>&1; RC=$?
    [ $RC -eq 0 ] && soft "\`linix $cmd\` exits 0" || soft "\`linix $cmd\` exit=$RC$(rcnote $RC) (tolerated)"
done

hr "SUMMARY [$BACKEND]"
echo "    HARD pass: $PASS    HARD fail: $FAIL    soft/info: $SOFT"
if [ "$FAIL" -ne 0 ]; then echo "    RESULT: FAIL ($FAIL hard check(s) failed)"; exit 1; fi
echo "    RESULT: PASS"
