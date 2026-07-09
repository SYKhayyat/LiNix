#!/bin/sh
# Broad real-world test for a whole distro image, driven entirely through the `linix` binary.
# Runs INSIDE a disposable container as root, so it can safely install/remove real system
# packages and scaffold real manifests. This is the release-readiness sweep. It exercises:
#
#   1. Discovery (doctor/search/info)              7. Declarative diagnostic (unscoped status)
#   2. Dry-run safety (no mutation)                8. Declarative lifecycle ROUNDTRIP
#   3. Imperative install + list + coherence          (write file -> sync -> installed;
#   4. Idempotency                                     edit file  -> prune -> gone)
#   5. Imperative remove + coherence               9. JSON output contract (real json parse)
#   6. Negative path (exit-status enforced)       10. PROFILES (activate/deactivate/relational)
#                                                 11. MULTI-BACKEND sweep (every READY backend)
#                                                 12. Read-only command smoke
#
#   Usage: run-in-container.sh <native-backend> [package] [package2]
#   e.g.   run-in-container.sh apt jq htop
#
# Every check is tallied and the script continues past failures so a single run shows the
# whole picture; it exits non-zero if any HARD check failed. "soft" checks (network-dependent
# or backend/ecosystem-optional) are reported but never fail the run.
#
# Every `linix` invocation is wrapped in `timeout` (TIMEOUT secs, default 90): a hang is
# recorded as a FAILURE (exit 124) and the run continues.
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
TIMEOUT="${TIMEOUT:-90}"                       # per-command wall-clock ceiling (seconds)

# Wrap every linix call in `timeout` when available (GNU coreutils + busybox both accept
# `timeout SECS CMD…`). A timed-out command exits 124, which our checks treat as failure.
if command -v timeout >/dev/null 2>&1; then TO="timeout $TIMEOUT"; else TO=""; fi
lx() { $TO "$LINIX" "$@"; }

# Declarative commands (sync/status/prune) get a shorter ceiling and a verbose-capture
# wrapper so a stall reveals the LAST step before it froze.
DTIMEOUT="${DTIMEOUT:-60}"
if command -v timeout >/dev/null 2>&1; then DTO="timeout $DTIMEOUT"; else DTO=""; fi
LOGF="$GDIR/_lastcmd.log"
dlx() { $DTO "$LINIX" -v "$@" >"$LOGF" 2>&1; }   # verbose, captured; rc is the command's rc
tail_log() { echo "      --- last log lines before stall (linix -v) ---"; tail -n 16 "$LOGF" 2>/dev/null | sed 's/^/      | /'; }

PASS=0; FAIL=0; SOFT=0
ok()   { echo "    [ok]    $1"; PASS=$((PASS+1)); }
no()   { echo "    [FAIL]  $1"; FAIL=$((FAIL+1)); }
soft() { echo "    [info]  $1"; SOFT=$((SOFT+1)); }
hr()   { echo; echo "=========== $* ==========="; }
rcnote() { [ "$1" -eq 124 ] && echo " (TIMED OUT after ${TIMEOUT}s)" || echo ""; }
rehash() { hash -r 2>/dev/null || true; }
# present: resolves on PATH AND the file really exists (shell caches stale locations)
present() { rehash; r="$(command -v "$1" 2>/dev/null || true)"; [ -n "$r" ] && [ -e "$r" ]; }
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
    if [ -n "$PYBIN" ]; then
        printf '%s' "$1" | "$PYBIN" -c 'import sys,json; json.load(sys.stdin)' >/dev/null 2>&1
    else
        first=$(printf '%s' "$1" | tr -d '[:space:]' | cut -c1)
        [ "$first" = "{" ] || [ "$first" = "[" ]
    fi
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
echo "# binary=$LINIX  timeout=${TIMEOUT}s"
echo "###################################################################"
[ -x "$LINIX" ] || { echo "FATAL: linix binary not found at $LINIX"; exit 2; }
rm -rf "$GDIR"; mkdir -p "$GDIR"

# Capture the backend readiness map ONCE; the multi-backend sweep gates on it.
DOCTOR="$(lx doctor 2>/dev/null)"
backend_ready() { printf '%s\n' "$DOCTOR" | grep -Eqi "^\[READY\][[:space:]]+$1([[:space:]]|$)"; }

# ------------------------------------------------------------------ discovery
hr "1. DISCOVERY (doctor / search / info)"
OUT="$(lx doctor 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "doctor exits 0" || no "doctor exit=$RC$(rcnote $RC)"
backend_ready "$BACKEND" && ok "doctor reports $BACKEND READY" || no "doctor does not list $BACKEND as READY"
echo "      READY backends:"; printf '%s\n' "$DOCTOR" | grep -i "READY" | sed 's/^/        /'

OUT="$(lx --backend "$BACKEND" update 2>&1)"; RC=$?
[ $RC -eq 0 ] && soft "update ($BACKEND) exits 0" || soft "update ($BACKEND) exit=$RC (network/optional)"

OUT="$(lx search "$PKG" 2>&1)"; RC=$?
[ $RC -eq 0 ] && ok "search exits 0" || no "search exit=$RC$(rcnote $RC)"
printf '%s\n' "$OUT" | grep -iq "^$BACKEND" && ok "search returns a $BACKEND hit for '$PKG'" \
    || soft "no $BACKEND-prefixed search hit for '$PKG' (index/name variance)"

OUT="$(lx info "$PKG" 2>&1)"; RC=$?
if [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -q "Package:"; then
    ok "info '$PKG' returns metadata"
else
    soft "info '$PKG' returned rc=$RC / no metadata (may be search-only backend)"
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
[ $RC -eq 0 ] && ok "install '$BACKEND:$PKG' exits 0" || no "install exit=$RC$(rcnote $RC)"
present "$PKG" && ok "'$PKG' on PATH after install" || no "'$PKG' NOT on PATH after install"
manifest_has "$PKG" && ok "install recorded '$PKG' in the manifest (config stays coherent)" || no "install did NOT record '$PKG' in the manifest — next sync would treat it as drift"
OUT="$(lx --backend "$BACKEND" list 2>&1)"; RC=$?
if [ $RC -eq 0 ] && printf '%s\n' "$OUT" | grep -qw "$PKG"; then
    ok "list shows '$PKG' as installed (installed-list parser works)"
else
    no "list ($BACKEND) does not show '$PKG' (rc=$RC$(rcnote $RC)) — installed-list parse gap"
fi

# ------------------------------------------------------------- idempotency (install)
hr "4. IDEMPOTENCY (install already-installed)"
lx -g "$GDIR" -y install "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "re-install of present package exits 0" || no "re-install exit=$RC$(rcnote $RC) (should be a no-op success)"

# ----------------------------------------------------------------- imperative remove
hr "5. IMPERATIVE REMOVE + verify gone + config coherence"
lx -g "$GDIR" -y remove "$BACKEND:$PKG" >/dev/null 2>&1; RC=$?
[ $RC -eq 0 ] && ok "remove '$BACKEND:$PKG' exits 0" || no "remove exit=$RC$(rcnote $RC)"
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

# ---------------------------------------- declarative lifecycle ROUNDTRIP (scoped)
hr "8. DECLARATIVE ROUNDTRIP — write file -> sync -> installed ; edit file -> prune -> gone"
lx -g "$GDIR" init >/dev/null 2>&1; RC=$?
MANIFEST="$GDIR/local.txt"
if [ $RC -eq 0 ] && [ -f "$MANIFEST" ]; then ok "init scaffolds manifest at $MANIFEST"; else no "init rc=$RC$(rcnote $RC), manifest present=$( [ -f "$MANIFEST" ] && echo yes || echo no)"; fi

echo "      -> writing '$BACKEND:$PKG' into the manifest file, then calling sync"
echo "$BACKEND:$PKG" >> "$MANIFEST"
dlx -g "$GDIR" -b "$BACKEND" status; RC=$?
if [ $RC -eq 0 ]; then ok "status (scoped) exits 0"; else no "status exit=$RC$(rcnote $RC)"; tail_log; fi
grep -qw "$PKG" "$LOGF" 2>/dev/null && ok "status shows '$PKG' as a pending change" || soft "status did not name '$PKG' (already satisfied?)"

dlx -g "$GDIR" -b "$BACKEND" -y sync; RC=$?
if [ $RC -eq 0 ]; then ok "sync (scoped) exits 0"; else no "sync exit=$RC$(rcnote $RC)"; tail_log; fi
present "$PKG" && ok "sync INSTALLED '$PKG' from the manifest file" || no "sync did NOT install '$PKG'"

lx -g "$GDIR" -b "$BACKEND" lock >/dev/null 2>&1; RC=$?
if [ $RC -eq 0 ] && [ -f "$GDIR/locks.json" ]; then ok "lock writes locks.json"; else soft "lock rc=$RC$(rcnote $RC) / no locks.json"; fi

OUT="$(lx -g "$GDIR" -b "$BACKEND" generation list 2>&1)"; RC=$?
[ $RC -eq 0 ] && soft "generation list exits 0" || soft "generation list exit=$RC$(rcnote $RC)"

echo "      -> removing '$PKG' from the manifest file, then converging (prune)"
: > "$MANIFEST"
dlx -g "$GDIR" -b "$BACKEND" status
grep -qw "$PKG" "$LOGF" 2>/dev/null && ok "status flags '$PKG' as drift after the manifest edit" || soft "status did not flag '$PKG' as drift"
dlx -g "$GDIR" -b "$BACKEND" -y prune; RC=$?
if [ $RC -eq 0 ]; then ok "prune (scoped) exits 0"; else no "prune exit=$RC$(rcnote $RC)"; tail_log; fi
present "$PKG" && no "prune did NOT remove drift '$PKG'" || ok "prune REMOVED '$PKG' after it left the manifest file"

# ------------------------------------------------------------- JSON contract
hr "9. JSON OUTPUT CONTRACT (stdout only; real JSON parse)"
OUT="$(lx search "$PKG" --json 2>/dev/null)"; is_json "$OUT" && ok "search --json is valid JSON" || no "search --json not JSON"
OUT="$(lx --backend "$BACKEND" list --json 2>/dev/null)"; is_json "$OUT" && ok "list --json is valid JSON" || no "list --json not JSON"
OUT="$(lx -b "$BACKEND" status --json 2>/dev/null)"; is_json "$OUT" && ok "status --json is valid JSON" || no "status --json not JSON"

# ------------------------------------------------- PROFILES lifecycle
hr "10. PROFILES — activate/deactivate, MULTIPLE active, RELATIONAL (include / exclude / -pkg)"
PGDIR="/tmp/linix-it-prof"; PROFDIR="/tmp/profiles"   # profiles_dir = parent(groups_dir)/profiles
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
[ $RC -eq 0 ] && ok "activate alpha exits 0" || no "activate alpha rc=$RC$(rcnote $RC)"
present "$PKG"  && ok "profile 'alpha' installed '$PKG'" || no "alpha did not install '$PKG'"
present "$PKG2" && no "alpha unexpectedly installed '$PKG2'" || ok "alpha left '$PKG2' absent"
pread profile active | grep -qw alpha && ok "'alpha' shows as active" || no "'alpha' not reported active"

# (b) activate a SECOND profile -> BOTH packages present simultaneously
pcmd activate bravo; RC=$?
[ $RC -eq 0 ] && ok "activate bravo exits 0 (two profiles now active)" || no "activate bravo rc=$RC$(rcnote $RC)"
if present "$PKG" && present "$PKG2"; then ok "MULTIPLE active: '$PKG' AND '$PKG2' both installed"; else no "multiple-active did not yield both packages"; fi

# (c) deactivate one -> its unique package removed, the shared/other stays
pcmd deactivate alpha; RC=$?
[ $RC -eq 0 ] && ok "deactivate alpha exits 0" || no "deactivate alpha rc=$RC$(rcnote $RC)"
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

# ------------------------------------------------- MULTI-BACKEND sweep
hr "11. MULTI-BACKEND SWEEP — every READY language/cross backend"
SGDIR="/tmp/linix-it-sweep"; rm -rf "$SGDIR"; mkdir -p "$SGDIR"
# Generic lifecycle for ANY backend: install -> list(parser) -> coherence -> remove. Install
# failure is SOFT with a diagnostic (ecosystem/version variance — e.g. corepack yarn v4, uv
# venv rules — is not a core-orchestration bug); everything AFTER a successful install is HARD.
sweep_backend() {
    b="$1"; p="$2"; bin="${3:-}"
    if ! backend_ready "$b"; then soft "[$b] not READY in this image — skipped"; return; fi
    echo "      --- backend: $b  (pkg: $p) ---"
    out=$($TO "$LINIX" -g "$SGDIR" -y install "$b:$p" 2>&1); rc=$?
    if [ $rc -ne 0 ]; then
        soft "[$b] install '$p' rc=$rc$(rcnote $rc) — ecosystem/network variance (not a hard fail)"
        printf '%s\n' "$out" | tail -3 | sed 's/^/          | /'
        $TO "$LINIX" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1   # best-effort cleanup
        return
    fi
    ok "[$b] install '$p' exits 0"
    lx -b "$b" list 2>/dev/null | grep -qiw "$p" && ok "[$b] list shows '$p' (installed-list parser works)" || no "[$b] list does NOT show '$p' after install — parser gap"
    manifest_scoped "$SGDIR" "$b" "$p" && ok "[$b] install recorded '$p' in manifest (coherent)" || no "[$b] install did NOT record '$p' in manifest"
    [ -n "$bin" ] && { present "$bin" && soft "[$b] '$bin' resolves on PATH" || soft "[$b] '$bin' not on PATH (global-bin dir not exported)"; }
    $TO "$LINIX" -g "$SGDIR" -y remove "$b:$p" >/dev/null 2>&1; rc=$?
    [ $rc -eq 0 ] && ok "[$b] remove '$p' exits 0" || no "[$b] remove '$p' rc=$rc$(rcnote $rc)"
    lx -b "$b" list 2>/dev/null | grep -qiw "$p" && no "[$b] list still shows '$p' after remove" || ok "[$b] '$p' gone from list after remove"
    manifest_scoped "$SGDIR" "$b" "$p" && no "[$b] remove left '$p' in manifest" || ok "[$b] remove cleared '$p' from manifest"
}
# Table: <backend> <test-package> [verify-bin]. The same package name is fetched from each
# ecosystem's own registry, proving that backend end-to-end. Not-READY rows auto-skip.
sweep_backend npm  cowsay   cowsay
sweep_backend yarn cowsay   cowsay
sweep_backend pnpm cowsay   cowsay
sweep_backend bun  cowsay   cowsay
sweep_backend pipx cowsay   cowsay
sweep_backend uv   cowsay   cowsay
sweep_backend gem  colorize ""
# cargo is READY (rustup) but `cargo install` COMPILES from source (minutes) — verify only
# that it plans a build rather than paying the compile in every run.
if backend_ready cargo; then
    OUT="$(lx -g "$SGDIR" -n install "cargo:ripgrep" --json 2>/dev/null)"
    is_json "$OUT" && ok "[cargo] dry-run install produces a JSON plan (compile skipped)" || soft "[cargo] dry-run plan not JSON"
else
    soft "[cargo] not READY — skipped"
fi

# ------------------------------------------------- read-only smoke (no crashes)
hr "12. READ-ONLY SMOKE (must run without crashing)"
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
