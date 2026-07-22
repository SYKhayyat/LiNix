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
LINIX="${LINIX:-linix}"
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

# SMOKE_ONLY: this image's package manager builds from source (Portage), so a real
# install→remove lifecycle costs hours. Everything that does not mutate the machine
# still runs — the grammar, the planner, the guard's refusals, the read verbs — and
# each skipped check is NAMED, because a run that quietly tests less than the others
# and prints the same "OK" is the failure this harness exists to catch.
SMOKE="${SMOKE_ONLY:-}"
skip_smoke() { soft "$1 — SMOKE_ONLY: this run installs and removes nothing"; }

# Is NAME runnable right now? `command -v` alone is not an answer: the shell caches
# where it found a name, and keeps answering from that cache after the file is
# deleted — so a package removed in section 9 still "existed" because section 4 had
# looked it up. A fresh `sh` has an empty cache and has to touch the filesystem.
on_path() { sh -c 'command -v "$1" >/dev/null 2>&1' _ "$1"; }

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
nok "dry-run did NOT actually install $PKG" on_path "$PKG"

# --- 4. Imperative install -> list -> coherence ---------------------------
echo "[4] Install"
if [ -n "$SMOKE" ]; then
    skip_smoke "install $PKG, and the list/PATH checks that read its result"
else
    ok "install $PKG" lx -y install "$PKG"
    grep_ok "list shows $PKG" "$PKG" lx list
    ok "$PKG binary is on PATH" on_path "$PKG"
fi

# --- 5. Idempotency --------------------------------------------------------
echo "[5] Idempotency"
# Runs under SMOKE too: with nothing installed the model is empty, and a sync over an
# empty model must still exit 0 rather than find work that is not there.
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
        ok "python3 still installed after adopt" on_path python3
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
ok "bash survives an uninstall attempt" on_path bash
# purge-unmanaged after adopt: a big removal must be refused without the flag OR
# gated by the ratio; either way, the bare command must not silently purge.
nok "purge-unmanaged is not a silent mass-delete" lx -y purge-unmanaged

# --- 9. Remove -------------------------------------------------------------
echo "[9] Remove"
if [ -n "$SMOKE" ]; then
    skip_smoke "uninstall $PKG (nothing was installed to remove)"
else
    ok "uninstall $PKG" lx -y uninstall "$PKG"
    nok "$PKG binary gone after uninstall" on_path "$PKG"
fi

# --- 10. Git-backed history (Phase 4 / v7) --------------------------------
echo "[10] Git history + rollback"
if ! on_path git; then
    # X.5 keeps git optional, which is not the same as its absence being an empty
    # answer. On an image with no git every history verb must SAY SO — `git log`
    # printing "no commits" here is a machine that can never have any.
    nok "git init refuses when git is not installed" lx git init
    grep_ok "and says git is what is missing" "git is not installed" lx git init
    grep_ok "git log refuses too, not an empty history" "git is not installed" \
        lx git log --limit 10
    soft "the history and rollback checks — this image has no git"
else
ok "git init enables manifest history" lx git init
# `lx` is a function, so `sh -c "lx …"` ran nothing and reported 127 — which the
# next checks then read as "no commit yet". Drive the binary directly.
if [ -n "$SMOKE" ]; then
    # A commit records a change to the machine, and this image cannot make one. The
    # history verbs are still asked to run; only the commit's existence is skipped.
    skip_smoke "the install that would leave a commit behind"
    ok "git log runs on an empty history" lx git log --limit 10
else
ok "an install after git init succeeds" lx -y install "$PKG"
ok "the install left a commit behind" git -C "$LINIX_CONFIG_DIR" rev-parse HEAD
# Subjects are deliberately generic (II.13 puts the detail in the diff), so the
# package name is not in the log — match the subject prefix LiNix actually writes.
grep_ok "git log shows a linix commit" "linix:" lx git log --limit 10
ok "diff against a commit runs" lx diff HEAD
ok "rollback to HEAD is accepted" lx -y rollback HEAD
fi
fi

# --- 11. rebuild asserts, and writes no commit (K14) ----------------------
echo "[11] rebuild"
# Git is asked directly, not `linix git log`: a rebuild that committed by some
# other route would still move HEAD, and only git can say so.
commits() { git -C "$LINIX_CONFIG_DIR" rev-list --count HEAD 2>/dev/null || echo 0; }
# The refusal costs nothing to check anywhere: it happens before any package is touched.
nok "bare rebuild is refused — scope is required" lx -y rebuild
if [ -n "$SMOKE" ]; then
    skip_smoke "the rebuild itself, and K14's no-commit proof (needs an installed package)"
else
BEFORE_COMMITS=$(commits)
# "unchanged" proves nothing if there was no history to change, and nothing if
# the rebuild never ran: both read 0 == 0. Require a commit to exist first.
ok "there is history for a rebuild to leave alone" test "$BEFORE_COMMITS" -ge 1
# Scoped to $PKG, not --all: the machine was adopted in section 7, so `--all`
# would churn every manual package on the image to prove a claim about one.
ok "rebuild $PKG runs" lx -y rebuild "$PKG"
ok "$PKG is reinstalled, not left removed" on_path "$PKG"
AFTER_COMMITS=$(commits)
echo "        commits before=$BEFORE_COMMITS after=$AFTER_COMMITS"
ok "rebuild wrote no git commit (K14)" test "$BEFORE_COMMITS" = "$AFTER_COMMITS"
fi

# --- 12. Backend chains, the per-host lock, and unlock (II.7b) ------------
echo "[12] Chains and the per-host lock"
if [ -n "$SMOKE" ]; then
    # A lock entry is written by a run that changes the machine, so there is nothing
    # here to inspect. The grammar below is checked anyway: it is pure parsing.
    skip_smoke "the per-host lock file, and unlock (no sync recorded an answer)"
else
LOCKFILE=$(ls "$LINIX_CONFIG_DIR"/locks/bare.*.toml 2>/dev/null | head -1)
echo "        lock file: ${LOCKFILE:-<none>}"
# Per-host: the answer is about this machine, so the filename has to be too, or
# two machines sharing a config overwrite each other on every sync.
ok "the lock is named for this host" test -n "$LOCKFILE"
grep_ok "an unpinned name froze to $BACKEND" "\"$BACKEND\"" cat "$LOCKFILE"

# A lock written by another machine is not an answer about this one.
printf '[resolved]\n%s = "linix-no-such-backend"\n' "$PKG" \
    > "$LINIX_CONFIG_DIR/locks/bare.some-other-box.toml"
ok "sync ignores another host's lock file" lx -y sync
ok "and leaves it alone" test -f "$LINIX_CONFIG_DIR/locks/bare.some-other-box.toml"
rm -f "$LINIX_CONFIG_DIR/locks/bare.some-other-box.toml"
fi

# The chain grammar. `list` is the priority file; a comma separates candidates.
ok  "a chain is legal"            lx --dry-run install "$BACKEND,cargo:$PKG"
ok  "a chain may end in list"     lx --dry-run install "$BACKEND,list:$PKG"
ok  "list alone is legal"         lx --dry-run install "list:$PKG"
nok "an empty slot is refused"    lx --dry-run install "$BACKEND,,cargo:$PKG"
nok "an unknown link is refused"  lx --dry-run install "$BACKEND,nope:$PKG"
nok "list must come last"         lx --dry-run install "list,$BACKEND:$PKG"
nok "a name repeated is refused"  lx --dry-run install "$BACKEND,$BACKEND:$PKG"
nok "a pattern cannot span one"   lx --dry-run install "$BACKEND,cargo:re:^$PKG"

# A pin naming a manager this host does not have must fail out loud, not quietly
# decide there is nothing to do — that silence is the bug chains exist to end.
FOREIGN=dnf; [ "$BACKEND" = "dnf" ] && FOREIGN=apt
command -v "$FOREIGN" >/dev/null 2>&1 \
    && soft "$FOREIGN exists on this image — cannot test a pin to a missing manager" \
    || nok "a pin to a manager this host lacks is not silent" lx -y install "$FOREIGN:$PKG"

if [ -z "$SMOKE" ]; then
grep_ok "unlock --list names the frozen package" "$PKG" lx unlock --list
ok "unlock forgets one name" lx unlock "$PKG"
nok "the entry is really gone" grep -q "$PKG" "$LOCKFILE"
fi
ok "unlocking a name that was never frozen is not an error" lx unlock linix-never-frozen-zzz

# --- 12b. A manager that could not answer is not one that said no (V.7c) --
echo "[12b] Silence is not a no"
REAL_CARGO=$(sh -c 'command -v cargo' 2>/dev/null)
if [ -z "$REAL_CARGO" ]; then
    soft "no cargo in this image — cannot stage a manager that fails to answer"
else
    # Shadow only cargo's *search*, so exactly one candidate in the chain goes
    # silent while every other manager on the image is untouched. Breaking the
    # network instead would break the manager under test too.
    mkdir -p /tmp/silent-bin
    cat > /tmp/silent-bin/cargo <<EOSHIM
#!/bin/sh
if [ "\$1" = "search" ]; then
    echo "error: failed to fetch the registry index" >&2
    exit 1
fi
exec "$REAL_CARGO" "\$@"
EOSHIM
    chmod +x /tmp/silent-bin/cargo

    SILENT_CFG=/tmp/linix-it-silent
    rm -rf "$SILENT_CFG"; mkdir -p "$SILENT_CFG/modules" "$SILENT_CFG/profiles"
    printf 'cargo\n%s\n' "$BACKEND" > "$SILENT_CFG/priority"
    printf 'Work\n' > "$SILENT_CFG/active"
    printf 'use base\n' > "$SILENT_CFG/profiles/Work"
    printf '%s\n' "$PKG" > "$SILENT_CFG/modules/base.txt"

    silent_lx() {
        env PATH="/tmp/silent-bin:$PATH" LINIX_CONFIG_DIR="$SILENT_CFG" \
            LINIX_DATA_DIR=/tmp/linix-it-silent-state $TO "$LINIX" "$@"
    }
    if [ -n "$SMOKE" ]; then
        skip_smoke "the sync past a silent manager, and the lock it must not write"
    else
        ok "a sync past a silent manager still resolves" silent_lx -y sync
        # The point of the ruling: it resolved, and wrote nothing down, so the next
        # sync asks again and can still move the package to cargo.
        nok "and freezes nothing" sh -c \
            "cat $SILENT_CFG/locks/bare.*.toml 2>/dev/null | grep -q '$PKG'"
    fi
    # Pure resolution: the plan says which manager went quiet without installing.
    grep_ok "and says which manager could not answer" "could not answer" \
        silent_lx --dry-run plan
    rm -rf /tmp/silent-bin "$SILENT_CFG" /tmp/linix-it-silent-state
fi

# --- 13. Command-surface smoke (exists + --help) --------------------------
echo "[13] Command surface"
for c in install uninstall sync plan status list search adopt check absent \
         protected purge-unmanaged rebuild rollback diff git snapshot schedule \
         profile module bundle export doctor unlock; do
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
if [ -n "$SMOKE" ]; then
    # A smoke run passes with a smaller pass count than the others, so it says which
    # run it was. "OK" over a third of the checks, printed the same way, is how a
    # narrower sweep gets mistaken for a full one.
    echo " OK — every hard check passed (SMOKE_ONLY: no package was installed or"
    echo "      removed; the $SOFTC soft lines above name what was not exercised)."
else
    echo " OK — every hard check passed."
fi
echo "=============================================================="
exit 0
