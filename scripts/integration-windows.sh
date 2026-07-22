#!/usr/bin/env bash
# ============================================================================
# LiNix v7 native Windows/macOS sweep — host-native backends (scoop/winget/
# choco/brew) driven through the real `linix` binary. These OSes can't run in a
# Linux container, so this mirrors the container harness (run-in-container.sh)
# for the host.
#
#   scripts/integration-windows.sh [backend] [package]
#   e.g. scripts/integration-windows.sh scoop jq        # user-scoped, reversible
#        LINIX=./target/release/linix.exe scripts/integration-windows.sh
#
# scoop is the safe default (user-scoped, trivially reversible). LiNix's own
# state is isolated via LINIX_CONFIG_DIR / LINIX_DATA_DIR; real package installs
# do affect the host, so prefer scoop and a throwaway package.
#
# HARD exit-code assertions (ok/nok/grep_ok); the run exits non-zero on any hard
# failure. This is the v7 rewrite — the pre-v7 script (built on the deleted `-g`
# flag, generation/lease, 102 soft assertions) was deleted, NO LEGACY.
# ============================================================================
set -u

BACKEND="${1:-scoop}"
PKG="${2:-jq}"
LINIX="${LINIX:-linix}"

export LINIX_CONFIG_DIR="${TMPDIR:-/tmp}/linix-it-win-config"
export LINIX_DATA_DIR="${TMPDIR:-/tmp}/linix-it-win-state"
rm -rf "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR" 2>/dev/null
mkdir -p "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"

lx() { "$LINIX" "$@"; }

PASS=0; FAILC=0; SOFTC=0; FAILED_NAMES=""

ok() {
    desc="$1"; shift
    if "$@" >/tmp/itw.out 2>&1; then
        PASS=$((PASS + 1)); echo "  PASS  $desc"; return 0
    else
        rc=$?; FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc)"
        echo "  FAIL  $desc (rc=$rc)"; sed 's/^/        | /' /tmp/itw.out | tail -6; return 1
    fi
}
nok() {
    desc="$1"; shift
    if "$@" >/tmp/itw.out 2>&1; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (expected non-zero, got 0)"
        echo "  FAIL  $desc (expected refusal, but it succeeded)"
    else
        PASS=$((PASS + 1)); echo "  PASS  $desc (correctly refused)"
    fi
}
grep_ok() {
    desc="$1"; pat="$2"; shift 2
    if "$@" 2>&1 | grep -q "$pat"; then
        PASS=$((PASS + 1)); echo "  PASS  $desc"
    else
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (missing /$pat/)"
        echo "  FAIL  $desc (output missing /$pat/)"
    fi
}
soft() { SOFTC=$((SOFTC + 1)); echo "  soft  $1"; }

# Is NAME runnable right now? `command -v` alone answers from the shell's hash table
# and keeps naming a path after the file is gone, so a removal check written with it
# cannot fail. A fresh `sh` has an empty cache and has to look.
on_path() { sh -c 'command -v "$1" >/dev/null 2>&1' _ "$1"; }

echo "=============================================================="
echo " LiNix v7 Windows/macOS harness — backend=$BACKEND package=$PKG"
echo " LINIX=$LINIX"
echo "=============================================================="

command -v "$LINIX" >/dev/null 2>&1 || { echo "FATAL: linix not found at '$LINIX' — set LINIX or build it."; exit 2; }

# --- 1. Bootstrap ----------------------------------------------------------
echo "[1] Bootstrap"
ok "init scaffolds the repo" lx init
ok "priority file exists" test -f "$LINIX_CONFIG_DIR/priority"
ok "active file exists" test -f "$LINIX_CONFIG_DIR/active"

# --- 2. Discovery / read-only ---------------------------------------------
echo "[2] Discovery / read-only verbs"
ok "doctor" lx doctor
ok "status" lx status
ok "check" lx check
ok "absent" lx absent
ok "protected" lx protected
ok "plan --dry-run" lx --dry-run plan

# --- 3. Dry-run safety -----------------------------------------------------
echo "[3] Dry-run safety"
ok "sync --dry-run" lx --dry-run sync
ok "install --dry-run shows a plan" lx --dry-run install "$BACKEND:$PKG"

# --- 4. Install -> list -> remove (real, reversible on scoop) --------------
echo "[4] Real lifecycle"
if ok "install $BACKEND:$PKG" lx -y install "$BACKEND:$PKG"; then
    grep_ok "list shows $PKG" "$PKG" lx list
    ok "second sync is a no-op" lx -y sync
    ok "uninstall $BACKEND:$PKG" lx -y uninstall "$BACKEND:$PKG"
else
    soft "install failed (network/ecosystem variance) — skipping the rest of the lifecycle"
fi

# --- 5. Negative path ------------------------------------------------------
echo "[5] Negative path"
nok "installing a nonexistent package fails" lx -y install "$BACKEND:linix-no-such-pkg-zzz"
# The line stays on purpose: a pinned name that a manager could not install is a failed sync,
# not a wrong name, and only a name nothing can resolve is withdrawn. So the harness clears it,
# exactly as the container one does — left in, it is committed and then reinstalled by the
# `rollback` in section 8, which fails there instead of here.
ok "a failed install leaves the model parseable" lx status
sed -i '/linix-no-such-pkg-zzz/d' "$LINIX_CONFIG_DIR/modules/imperative.txt" 2>/dev/null || true

# --- 6. Adopt (II.9: Windows managers install no deps, so adopt is exact) --
echo "[6] Adopt"
ok "adopt runs" lx -y adopt

# --- 7. The guard ----------------------------------------------------------
echo "[7] The guard"
# `lx` is a shell function, so `sh -c "lx …"` ran nothing at all and this asserted
# only that the binary still exists — which it would whatever LiNix did.
"$LINIX" -y uninstall linix >/dev/null 2>&1 || true
ok "linix survives an uninstall attempt" on_path "$LINIX"
nok "purge-unmanaged is not a silent mass-delete" lx -y purge-unmanaged

# --- 8. Git history + rollback --------------------------------------------
echo "[8] Git history + rollback"
if ok "git init" lx git init; then
    # Driven through the binary, not `sh -c "lx …"`: `lx` is a function and a subshell
    # never sees it, so the old form ran nothing and reported whatever came after.
    "$LINIX" -y sync >/dev/null 2>&1 || true
    ok "sync commits" lx git log --limit 5
    grep_ok "git log shows a linix commit" "linix" lx git log --limit 10
    ok "diff HEAD runs" lx diff HEAD
    ok "rollback HEAD accepted" lx -y rollback HEAD
fi

# --- 9. Backend chains, the per-host lock, and unlock (II.7b) -------------
echo "[9] Chains and the per-host lock"
LOCKFILE=$(ls "$LINIX_CONFIG_DIR"/locks/bare.*.toml 2>/dev/null | head -1)
echo "        lock file: ${LOCKFILE:-<none>}"
ok  "a chain is legal"           lx --dry-run install "$BACKEND,cargo:$PKG"
ok  "a chain may end in list"    lx --dry-run install "$BACKEND,list:$PKG"
ok  "list alone is legal"        lx --dry-run install "list:$PKG"
nok "an empty slot is refused"   lx --dry-run install "$BACKEND,,cargo:$PKG"
nok "an unknown link is refused" lx --dry-run install "$BACKEND,nope:$PKG"
nok "list must come last"        lx --dry-run install "list,$BACKEND:$PKG"
nok "a name repeated is refused" lx --dry-run install "$BACKEND,$BACKEND:$PKG"
# A manager no Windows host has: a pin to it must say so rather than no-op.
nok "a pin to a manager this host lacks is not silent" lx -y install "apt:$PKG"
ok  "unlock --list runs"         lx unlock --list
ok  "unlocking an unfrozen name is not an error" lx unlock linix-never-frozen-zzz

# --- 10. Command-surface smoke --------------------------------------------
echo "[10] Command surface"
for c in install uninstall sync plan status list search adopt check absent \
         protected purge-unmanaged rollback diff git snapshot schedule \
         profile module bundle export doctor unlock; do
    ok "\`$c --help\` exists" lx "$c" --help
done

# --- Summary ---------------------------------------------------------------
echo "=============================================================="
echo " RESULT  pass=$PASS  fail=$FAILC  soft=$SOFTC"
if [ "$FAILC" -ne 0 ]; then
    printf " FAILURES:%b\n" "$FAILED_NAMES"
    exit 1
fi
echo " OK — every hard check passed."
exit 0
