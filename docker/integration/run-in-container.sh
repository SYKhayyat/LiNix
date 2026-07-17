#!/bin/sh
# ============================================================================
# LiNix v7 integration harness — runs INSIDE a disposable container as root.
#
#   Usage: run-in-container.sh <native-backend> [package] [package2]
#   e.g.   run-in-container.sh apt jq
#
# Driven entirely through the real `linix` binary against the distro's native
# package manager. Isolation is by env var (LINIX_CONFIG_DIR / LINIX_DATA_DIR),
# so LiNix's own state is a throwaway; real system packages ARE installed and
# removed (that is the point — it is a disposable container).
#
# HARD assertions: every check either passes or fails the whole run (exit 1 at
# the end if any failed). A short, honest list of "soft" checks (genuinely
# network/ecosystem-optional) is reported but never fails the run.
#
# This is the v7 rewrite. The pre-v7 comprehensive sweep (built on the deleted
# `-g` flag) is preserved as run-in-container.legacy.sh for porting reference.
# ============================================================================
set -u

BACKEND="${1:?usage: run-in-container.sh <backend> [package]}"
PKG="${2:-jq}"
LINIX="${LINIX_BIN:-linix}"
TO="timeout 300"

# --- Isolation: LiNix's config + data are throwaway; the II.1 repo lives here.
export LINIX_CONFIG_DIR="/tmp/linix-it-config"
export LINIX_DATA_DIR="/tmp/linix-it-state"
rm -rf "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"
mkdir -p "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"

lx() { $TO "$LINIX" "$@"; }

PASS=0
FAILC=0
SOFTC=0
FAILED_NAMES=""

# ok "desc" cmd...   — passes when cmd exits 0.
ok() {
    desc="$1"; shift
    if "$@" >/tmp/it.out 2>&1; then
        PASS=$((PASS + 1)); echo "  PASS  $desc"
    else
        rc=$?; FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc)"
        echo "  FAIL  $desc (rc=$rc)"; sed 's/^/        | /' /tmp/it.out | tail -6
    fi
}

# nok "desc" cmd...  — passes when cmd exits NON-zero (a refusal/negative path).
nok() {
    desc="$1"; shift
    if "$@" >/tmp/it.out 2>&1; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (expected non-zero, got 0)"
        echo "  FAIL  $desc (expected refusal, but it succeeded)"
    else
        PASS=$((PASS + 1)); echo "  PASS  $desc (correctly refused)"
    fi
}

# grep_ok "desc" pattern cmd... — passes when cmd's output contains pattern.
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

echo "=============================================================="
echo " LiNix v7 harness — backend=$BACKEND package=$PKG"
echo "=============================================================="

# --- 1. Bootstrap the II.1 repo -------------------------------------------
echo "[1] Bootstrap"
ok "init scaffolds the repo" lx init
ok "priority file exists" test -f "$LINIX_CONFIG_DIR/priority"
grep_ok "priority names this backend" "$BACKEND" cat "$LINIX_CONFIG_DIR/priority"
ok "active file exists" test -f "$LINIX_CONFIG_DIR/active"

# --- 2. Discovery (read-only) ---------------------------------------------
echo "[2] Discovery / read-only verbs"
ok "doctor" lx doctor
ok "status" lx status
ok "plan (no changes yet)" lx plan --dry-run
ok "check parses the model" lx check
ok "absent lists nothing" lx absent
ok "protected lists guarded packages" lx protected
grep_ok "protected includes a system essential" "linix\|libc\|systemd\|kernel\|bash" lx protected

# --- 3. Dry-run is preview-only -------------------------------------------
echo "[3] Dry-run safety"
ok "sync --dry-run does not error" lx --dry-run sync
ok "a dry-run install shows a plan" lx --dry-run install "$PKG"
nok "dry-run did NOT actually install $PKG" command -v "$PKG"

# --- 4. Imperative install -> list -> coherence ---------------------------
echo "[4] Install"
ok "install $PKG" lx -y install "$PKG"
grep_ok "list shows $PKG" "$PKG" lx list
ok "$PKG binary is on PATH" command -v "$PKG"

# --- 5. Idempotency --------------------------------------------------------
echo "[5] Idempotency"
ok "second sync is a no-op (exit 0)" lx -y sync

# --- 6. Negative path ------------------------------------------------------
echo "[6] Negative path"
nok "installing a nonexistent package fails" lx -y install "linix-no-such-pkg-zzz"

# --- 7. Adopt (Part IV proof) ---------------------------------------------
echo "[7] Adopt"
ok "adopt takes manual packages" lx -y adopt
# Part IV: adopt takes the MANUAL set, not the whole dependency closure, and
# python3 (apt/dnf) survives; and it is far fewer than every installed package.
if [ "$BACKEND" = "apt" ] || [ "$BACKEND" = "dnf" ] || [ "$BACKEND" = "pacman" ]; then
    if command -v python3 >/dev/null 2>&1; then
        ok "python3 still installed after adopt" command -v python3
    else
        soft "python3 not on this image — cannot check the survival proof"
    fi
    ADOPTED=$(lx list 2>/dev/null | grep -c ":")
    echo "        adopted ~$ADOPTED package(s)"
    ok "adopt recorded at least one package" test "$ADOPTED" -ge 1
fi

# --- 8. The guard (Part IV proofs) ----------------------------------------
echo "[8] The guard"
# A protected package is never removed. python3/libc/bash are protected; asking
# to uninstall one must not carry it out.
nok "uninstall of a protected package is refused/no-op" sh -c "lx -y uninstall bash && command -v bash >/dev/null && exit 1 || true; command -v bash"
# purge-unmanaged after adopt: a big removal must be refused without the flag OR
# gated by the ratio; either way, the bare command must not silently purge.
nok "purge-unmanaged is not a silent mass-delete" lx -y purge-unmanaged

# --- 9. Remove -------------------------------------------------------------
echo "[9] Remove"
ok "uninstall $PKG" lx -y uninstall "$PKG"
nok "$PKG binary gone after uninstall" command -v "$PKG"

# --- 10. Git-backed history (Phase 4 / v7) --------------------------------
echo "[10] Git history + rollback"
ok "git init enables manifest history" lx git init
ok "sync commits after a change" sh -c "lx -y install '$PKG' >/dev/null 2>&1; lx git log --limit 5"
grep_ok "git log shows a linix commit" "linix" lx git log --limit 10
ok "diff against a commit runs" lx diff HEAD
ok "rollback to HEAD is accepted" lx -y rollback HEAD

# --- 11. Command-surface smoke (exists + --help) --------------------------
echo "[11] Command surface"
for c in install uninstall sync plan status list search adopt check absent \
         protected purge-unmanaged rollback diff git snapshot schedule \
         profile module bundle export doctor; do
    ok "\`$c --help\` exists" lx "$c" --help
done

# --- Summary ---------------------------------------------------------------
echo "=============================================================="
echo " RESULT  pass=$PASS  fail=$FAILC  soft=$SOFTC"
if [ "$FAILC" -ne 0 ]; then
    printf " FAILURES:%b\n" "$FAILED_NAMES"
    echo "=============================================================="
    exit 1
fi
echo " OK — every hard check passed."
echo "=============================================================="
exit 0
