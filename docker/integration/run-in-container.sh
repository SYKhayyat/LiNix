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

# LiNix commits as you (II.13) and injects no identity of its own, so git needs
# to know who that is. A bare container has no identity and every commit fails.
git config --global user.name "LiNix Integration" >/dev/null 2>&1 || true
git config --global user.email "integration@linix.invalid" >/dev/null 2>&1 || true

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

# A missing binary is not a failing run, it is an unrun one — and it does not
# look like one. `nok` reads "command not found" as the refusal it was hoping
# for, and `grep_ok` for /linix/ matches the words "failed to run command
# 'linix'", so an image with no binary reported nine passes. Stop here instead.
if ! $LINIX --version >/dev/null 2>&1; then
    echo "FATAL: '$LINIX' is not runnable in this image — nothing below was tested."
    echo "       The image must put the built binary on PATH (see the Dockerfiles)."
    exit 1
fi

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
# The failure must not be left in the manifest. Every later command parses the
# model, so one unresolvable line wedges the config until someone hand-edits it.
ok "a failed install leaves the model parseable" lx status
# Whatever the verdict above, the rest of the run needs a model it can read.
sed -i '/linix-no-such-pkg-zzz/d' "$LINIX_CONFIG_DIR/modules/imperative.txt" 2>/dev/null || true

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
# A protected package is never removed. Only survival is asserted: whether the
# verb refuses or no-ops depends on whether bash was declared, and the earlier
# form asserted an exit code so convoluted that a correct refusal failed it.
lx -y uninstall bash >/dev/null 2>&1 || true
ok "bash survives an uninstall attempt" command -v bash
# purge-unmanaged after adopt: a big removal must be refused without the flag OR
# gated by the ratio; either way, the bare command must not silently purge.
nok "purge-unmanaged is not a silent mass-delete" lx -y purge-unmanaged

# --- 9. Remove -------------------------------------------------------------
echo "[9] Remove"
ok "uninstall $PKG" lx -y uninstall "$PKG"
echo "        DIAG path=[$(command -v "$PKG" 2>&1)] ls=[$(ls -l /usr/bin/"$PKG" 2>&1 | head -1)]"
echo "        DIAG pkgstate=[$(dpkg -l "$PKG" 2>/dev/null | tail -1)]"
echo "        DIAG declared=[$(grep -rl "$PKG" "$LINIX_CONFIG_DIR"/modules/ 2>/dev/null | tr '\n' ' ')]"
nok "$PKG binary gone after uninstall" command -v "$PKG"

# --- 10. Git-backed history (Phase 4 / v7) --------------------------------
echo "[10] Git history + rollback"
ok "git init enables manifest history" lx git init
# `lx` is a function, so `sh -c "lx …"` ran nothing and reported 127 — which the
# next checks then read as "no commit yet". Drive the binary directly.
ok "an install after git init succeeds" lx -y install "$PKG"
ok "the install left a commit behind" git -C "$LINIX_CONFIG_DIR" rev-parse HEAD
# Subjects are deliberately generic (II.13 puts the detail in the diff), so the
# package name is not in the log — match the subject prefix LiNix actually writes.
grep_ok "git log shows a linix commit" "linix:" lx git log --limit 10
ok "diff against a commit runs" lx diff HEAD
ok "rollback to HEAD is accepted" lx -y rollback HEAD

# --- 11. rebuild asserts, and writes no commit (K14) ----------------------
echo "[11] rebuild"
# Git is asked directly, not `linix git log`: a rebuild that committed by some
# other route would still move HEAD, and only git can say so.
commits() { git -C "$LINIX_CONFIG_DIR" rev-list --count HEAD 2>/dev/null || echo 0; }
nok "bare rebuild is refused — scope is required" lx -y rebuild
BEFORE_COMMITS=$(commits)
# "unchanged" proves nothing if there was no history to change, and nothing if
# the rebuild never ran: both read 0 == 0. Require a commit to exist first.
ok "there is history for a rebuild to leave alone" test "$BEFORE_COMMITS" -ge 1
# Scoped to $PKG, not --all: the machine was adopted in section 7, so `--all`
# would churn every manual package on the image to prove a claim about one.
ok "rebuild $PKG runs" lx -y rebuild "$PKG"
ok "$PKG is reinstalled, not left removed" command -v "$PKG"
AFTER_COMMITS=$(commits)
echo "        commits before=$BEFORE_COMMITS after=$AFTER_COMMITS"
ok "rebuild wrote no git commit (K14)" test "$BEFORE_COMMITS" = "$AFTER_COMMITS"

# --- 12. Command-surface smoke (exists + --help) --------------------------
echo "[12] Command surface"
for c in install uninstall sync plan status list search adopt check absent \
         protected purge-unmanaged rebuild rollback diff git snapshot schedule \
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
