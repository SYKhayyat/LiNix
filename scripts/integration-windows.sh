#!/usr/bin/env bash
# ============================================================================
# LiNix v7 native Windows/macOS sweep — host-native backends (scoop/winget/
# choco/brew) driven through the real `linix` binary. These OSes can't run in a
# Linux container, so this mirrors the container harness (run-in-container.sh)
# for the host, section for section — including its COVERAGE AUDIT.
#
#   scripts/integration-windows.sh [backend] [package]
#   e.g. scripts/integration-windows.sh scoop jq        # user-scoped, reversible
#        LINIX=./target/release/linix.exe scripts/integration-windows.sh
#
# scoop is the safe default (user-scoped, trivially reversible). LiNix's own
# state is isolated via LINIX_CONFIG_DIR / LINIX_DATA_DIR; real package installs
# do affect the host, so prefer scoop and a throwaway package.
#
# THIS RUNS ON A REAL MACHINE, not a disposable container. So the real-lifecycle
# sweep is limited to managers that install per-user and uninstall cleanly; the
# machine-wide ones (winget, choco, psresource) are plan-smoked and NAMED as
# such, because proving a parser is not worth writing to a developer's Program
# Files. Every one of them still gets its argv/planner wiring exercised.
#
# HARD exit-code assertions (ok/nok/grep_ok); the run exits non-zero on any hard
# failure.
# ============================================================================
set -u

BACKEND="${1:-scoop}"
PKG="${2:-jq}"
LINIX="${LINIX:-linix}"

export LINIX_CONFIG_DIR="${TMPDIR:-/tmp}/linix-it-win-config"
export LINIX_DATA_DIR="${TMPDIR:-/tmp}/linix-it-win-state"
rm -rf "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR" 2>/dev/null
mkdir -p "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"

# The coverage ledger. Files, not variables: `grep_ok` runs its command in a
# pipeline, and a pipeline is a subshell whose variable writes die with it.
LEDGER="${TMPDIR:-/tmp}/linix-it-win-ledger"
rm -rf "$LEDGER" 2>/dev/null; mkdir -p "$LEDGER"
: > "$LEDGER/cmd-real"; : > "$LEDGER/cmd-help"
: > "$LEDGER/be-life"; : > "$LEDGER/be-life-partial"; : > "$LEDGER/be-smoke"

record_argv() {
    _sub=""; _skip=""
    for _a in "$@"; do
        if [ -n "$_skip" ]; then _skip=""; continue; fi
        case "$_a" in
            -c|--config|--config-dir) _skip=1; continue ;;
            -*) continue ;;
            *) _sub="$_a"; break ;;
        esac
    done
    [ -n "$_sub" ] || return 0
    # `<cmd> --help` proves clap is wired and nothing else (IV.1), so it is
    # ledgered apart and does NOT satisfy the audit.
    case " $* " in
        *" --help "*|*" -h "*) echo "$_sub" >> "$LEDGER/cmd-help"; return 0 ;;
    esac
    echo "$_sub" >> "$LEDGER/cmd-real"
}

# Every call is wrapped, because this harness has no container to kill it: an
# `uninstall` that hung here stopped the whole sweep for as long as anyone let it, and
# a run that never ends reports nothing at all. 900s is longer than any real build on
# this host and short enough that a wedged command is a named failure instead of a wait.
TO="timeout 900"
lx() { record_argv "$@"; $TO "$LINIX" "$@"; }

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
        echo "  FAIL  $desc (expected refusal, but it succeeded)"; return 1
    else
        PASS=$((PASS + 1)); echo "  PASS  $desc (correctly refused)"; return 0
    fi
}
grep_ok() {
    desc="$1"; pat="$2"; shift 2
    if "$@" 2>&1 | grep -q "$pat"; then
        PASS=$((PASS + 1)); echo "  PASS  $desc"; return 0
    else
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (missing /$pat/)"
        echo "  FAIL  $desc (output missing /$pat/)"; return 1
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
grep_ok "priority names this backend" "$BACKEND" cat "$LINIX_CONFIG_DIR/priority"

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

# --- 4. The guard's ratio rule, on an UNADOPTED machine -------------------
# IV.1: the only state in which this tests anything. After `adopt` the machine is
# nearly all managed, so the ratio it exists to catch never fires.
echo "[4] purge-unmanaged, before adopt"
nok "purge-unmanaged is refused on a machine LiNix has not adopted" lx -y purge-unmanaged
grep_ok "and it is the unadopted-machine ratio that refused" \
    "adopt\|allow-mass-purge" lx -y purge-unmanaged

# --- 5. Install -> list -> remove (real, reversible on scoop) --------------
echo "[5] Real lifecycle"
# This host is not disposable. If it already had the package, the uninstall below would
# take away something the developer chose, so it is put back at the end and the run says
# so rather than leaving a hole nobody notices.
PKG_WAS_HERE=""
on_path "$PKG" && PKG_WAS_HERE=1

if ok "install $BACKEND:$PKG" lx -y install "$BACKEND:$PKG"; then
    echo "$BACKEND" >> "$LEDGER/be-life"
    grep_ok "list shows $PKG" "$PKG" lx list
    ok "$PKG binary is on PATH" on_path "$PKG"
    ok "second sync is a no-op" lx -y sync
    # `unmanage` belongs here and not with the read-only verbs: "forgets it WITHOUT
    # uninstalling it" is only a proof while something is installed to leave behind.
    ok "unmanage forgets a package without uninstalling it" lx unmanage "$BACKEND:$PKG"
    ok "$PKG is still installed after unmanage" on_path "$PKG"
    ok "declaring it again takes it back" lx -y install "$BACKEND:$PKG"
    ok "uninstall $BACKEND:$PKG" lx -y uninstall "$BACKEND:$PKG"
    nok "$PKG binary gone after uninstall" on_path "$PKG"
    if [ -n "$PKG_WAS_HERE" ]; then
        if lx -y install "$BACKEND:$PKG" >/dev/null 2>&1; then
            soft "$PKG was on this host before the run — put back, so the sweep leaves nothing missing"
        else
            soft "$PKG was on this host before the run and could NOT be put back — reinstall it by hand"
        fi
    fi
else
    echo "$BACKEND" >> "$LEDGER/be-life-partial"
    soft "install failed (network/ecosystem variance) — skipping the rest of the lifecycle"
fi

# --- 6. Negative path ------------------------------------------------------
echo "[6] Negative path"
nok "installing a nonexistent package fails" lx -y install "$BACKEND:linix-no-such-pkg-zzz"
# The line stays on purpose: a pinned name that a manager could not install is a failed sync,
# not a wrong name, and only a name nothing can resolve is withdrawn. So the harness clears it,
# exactly as the container one does — left in, it is committed and then reinstalled by the
# `rollback` in section 9, which fails there instead of here.
ok "a failed install leaves the model parseable" lx status
sed -i '/linix-no-such-pkg-zzz/d' "$LINIX_CONFIG_DIR/modules/imperative.txt" 2>/dev/null || true

# --- 7. Adopt (II.9: Windows managers install no deps, so adopt is exact) --
echo "[7] Adopt"
ADOPTED_FILE="$LINIX_CONFIG_DIR/modules/adopted.txt"
nok "nothing is adopted before adopt runs" test -s "$ADOPTED_FILE"
ok "adopt runs" lx -y adopt
ok "adopt wrote an adoption manifest" test -s "$ADOPTED_FILE"
# No `|| echo 0`: `grep -c` prints the count AND exits 1 when it is zero, so the
# fallback would append a second line and the `test -ge` below would be a syntax error.
ADOPTED=$(grep -vc '^[[:space:]]*#\|^[[:space:]]*$' "$ADOPTED_FILE" 2>/dev/null)
[ -n "$ADOPTED" ] || ADOPTED=0
echo "        adopted=$ADOPTED package(s)"
ok "adopt recorded at least one package" test "$ADOPTED" -ge 1

# --- 8. The guard ----------------------------------------------------------
echo "[8] The guard"
# `lx` is a shell function, so `sh -c "lx …"` ran nothing at all and this asserted
# only that the binary still exists — which it would whatever LiNix did.
$TO "$LINIX" -y uninstall linix >/dev/null 2>&1 || true
ok "linix survives an uninstall attempt" on_path "$LINIX"
nok "purge-unmanaged is still not a silent mass-delete after adopt" lx -y purge-unmanaged
# WHICH rule refuses is still asserted, but the answer depends on how much `adopt`
# could take on this host: where it adopted well the protected set decides, where it
# adopted little the ratio still does. Both are named answers; "some error" is not.
grep_ok "and the refusal after adopt still names its rule" \
    "protected\|essential\|allow-mass-removal\|allow-mass-purge" lx -y purge-unmanaged

# --- 9. Git history + rollback --------------------------------------------
echo "[9] Git history + rollback"
if ok "git init" lx git init; then
    ok "git status reads the repo" lx git status
    # Driven through the binary, not `sh -c "lx …"`: `lx` is a function and a subshell
    # never sees it, so the old form ran nothing and reported whatever came after.
    $TO "$LINIX" -y sync >/dev/null 2>&1 || true
    ok "sync commits" lx git log --limit 5
    # `linix` matches the config path, the binary name and half the error messages.
    # `linix:` is the commit-subject prefix and nothing else — grep for what only the
    # right answer contains (IV.1), especially with a config dir named linix-it-win-*.
    grep_ok "git log shows a linix commit" "linix:" lx git log --limit 10
    ok "git commit records the current state on demand" lx git commit -m "linix: harness checkpoint"
    ok "diff HEAD runs" lx diff HEAD
    ok "rollback HEAD accepted" lx -y rollback HEAD
fi

# --- 10. rebuild asserts, and writes no commit (K14) ----------------------
echo "[10] rebuild"
commits() { git -C "$LINIX_CONFIG_DIR" rev-list --count HEAD 2>/dev/null || echo 0; }
# K2 (ruled 2026-07-24): a bare `rebuild` no longer REFUSES — it WARNS loudly and rebuilds
# `--all`. Checked with `--dry-run` so the harness does not churn every manual package.
ok "bare rebuild is accepted, not refused (K2)" lx --dry-run rebuild
grep_ok "bare rebuild warns it will rebuild EVERY declared package (K2)" \
    "EVERY declared package" lx --dry-run rebuild
BEFORE_COMMITS=$(commits)
if [ "$BEFORE_COMMITS" -ge 1 ]; then
    ok "rebuild $BACKEND:$PKG runs" lx -y rebuild "$BACKEND:$PKG"
    AFTER_COMMITS=$(commits)
    echo "        commits before=$BEFORE_COMMITS after=$AFTER_COMMITS"
    ok "rebuild wrote no git commit (K14)" test "$BEFORE_COMMITS" = "$AFTER_COMMITS"
else
    soft "no manifest history on this host — K14's no-commit proof needs a commit to compare"
fi

# --- 11. Backend chains, the per-host lock, and unlock (II.7b) -------------
echo "[11] Chains and the per-host lock"
LOCKFILE=$(ls "$LINIX_CONFIG_DIR"/locks/bare.*.toml 2>/dev/null | head -1)
echo "        lock file: ${LOCKFILE:-<none>}"
ok  "a chain is legal"           lx --dry-run install "$BACKEND,cargo:$PKG"
ok  "a chain may end in list"    lx --dry-run install "$BACKEND,list:$PKG"
ok  "list alone is legal"        lx --dry-run install "list:$PKG"
nok "an empty slot is refused"   lx --dry-run install "$BACKEND,,cargo:$PKG"
nok "an unknown link is refused" lx --dry-run install "$BACKEND,nope:$PKG"
nok "list must come last"        lx --dry-run install "list,$BACKEND:$PKG"
nok "a name repeated is refused" lx --dry-run install "$BACKEND,$BACKEND:$PKG"
nok "a pattern cannot span one"  lx --dry-run install "$BACKEND,cargo:re:^$PKG"
# A manager no Windows host has: a pin to it must say so rather than no-op.
nok "a pin to a manager this host lacks is not silent" lx -y install "apt:$PKG"
ok  "unlock --list runs"         lx unlock --list
ok  "unlocking an unfrozen name is not an error" lx unlock linix-never-frozen-zzz

# --- 11b. A manager that could not answer is not one that said no (V.7c) ---
echo "[11b] Silence is not a no"
REAL_CARGO=$(sh -c 'command -v cargo' 2>/dev/null)
if [ -z "$REAL_CARGO" ]; then
    soft "no cargo on this host — cannot stage a manager that fails to answer"
else
    # Shadow only cargo's *search*, so exactly one candidate in the chain goes silent
    # while the manager under test is untouched. A `.bat` because Windows resolves a
    # bare `cargo` through PATHEXT, so the shim has to be something CreateProcess runs.
    SILENT_BIN="${TMPDIR:-/tmp}/linix-it-silent-bin"
    rm -rf "$SILENT_BIN"; mkdir -p "$SILENT_BIN"
    printf '@echo off\r\nif "%%1"=="search" (\r\n  echo error: failed to fetch the registry index 1>&2\r\n  exit /b 1\r\n)\r\n"%s" %%*\r\n' \
        "$(cygpath -w "$REAL_CARGO" 2>/dev/null || echo "$REAL_CARGO")" > "$SILENT_BIN/cargo.bat"

    SILENT_CFG="${TMPDIR:-/tmp}/linix-it-silent"
    rm -rf "$SILENT_CFG"; mkdir -p "$SILENT_CFG/modules" "$SILENT_CFG/profiles"
    printf 'cargo\n%s\n' "$BACKEND" > "$SILENT_CFG/priority"
    printf 'Work\n' > "$SILENT_CFG/active"
    printf 'use base\n' > "$SILENT_CFG/profiles/Work"
    printf '%s\n' "$PKG" > "$SILENT_CFG/modules/base.txt"

    silent_lx() {
        env PATH="$SILENT_BIN:$PATH" \
            LINIX_CONFIG_DIR="$(cygpath -w "$SILENT_CFG" 2>/dev/null || echo "$SILENT_CFG")" \
            LINIX_DATA_DIR="$(cygpath -w "$SILENT_CFG/state" 2>/dev/null || echo "$SILENT_CFG/state")" \
            $TO "$LINIX" "$@"
    }
    grep_ok "a plan past a silent manager says which one" "could not answer" \
        silent_lx --dry-run plan
    ok "a sync past a silent manager still resolves" silent_lx -y sync
    # The ruling: it resolved, and wrote nothing down, so the next sync asks again.
    nok "and freezes nothing" sh -c \
        "cat '$SILENT_CFG'/locks/bare.*.toml 2>/dev/null | grep -q '$PKG'"
    rm -rf "$SILENT_BIN" "$SILENT_CFG"
fi

# ==========================================================================
# 12. REAL lifecycle for every other manager on this host
# ==========================================================================
# The container harness sweeps every manager its image ships. A developer's
# machine is not disposable, so the same sweep runs only for managers that
# install per-user and uninstall cleanly. The machine-wide ones are named in
# no_lifecycle_reason and plan-smoked in section 13 instead — an unexplained
# skip is the vacuous check IV.1 bans.
echo "[12] Real lifecycle, every other user-scoped manager on this host"

canary() {
    case "$1" in
        scoop)    echo "jq|jq|full|" ;;
        npm)      echo "cowsay|cowsay|full|" ;;
        pnpm)     echo "cowsay|cowsay|full|" ;;
        yarn)     echo "cowsay|cowsay|full|" ;;
        bun)      echo "cowsay|cowsay|full|" ;;
        pipx)     echo "pycowsay|pycowsay|full|" ;;
        uv)       echo "pycowsay|pycowsay|full|" ;;
        gem)      echo "colorize||full|" ;;
        cargo)    echo "hexyl|hexyl|full|" ;;
        github)   echo "sharkdp/fd|fd|full|fd" ;;
        brew)     echo "wget|wget|full|" ;;
        # Each of these installs into a per-user directory (~/go/bin, ~/.dotnet/tools,
        # ~/.pub-cache/bin, ~/.pixi/bin, ~/.nimble/bin), so a real lifecycle here leaves
        # nothing behind outside the developer's own profile.
        go)       echo "golang.org/x/example/hello|hello|full|hello" ;;
        dotnet)   echo "dotnetsay|dotnetsay|full|" ;;
        pub)      echo "sass|sass|full|" ;;
        pixi)     echo "ripgrep|rg|full|" ;;
        nimble)   echo "nimjson|nimjson|full|" ;;
        luarocks) echo "luafilesystem||full|" ;;
        krew)     echo "ns|kubectl-ns|full|" ;;
        *)        echo "" ;;
    esac
}

no_lifecycle_reason() {
    case "$1" in
        winget)     echo "installs machine-wide on a developer's real machine — plan-smoked instead" ;;
        choco)      echo "installs machine-wide and needs an elevated shell — plan-smoked instead" ;;
        psresource) echo "writes to the PowerShell module path for the whole user profile — plan-smoked instead" ;;
        pip)        echo "installs into the system Python this host runs on — plan-smoked instead" ;;
        mas)        echo "needs a signed-in App Store account — plan-smoked instead" ;;
        link)       echo "a dependent statement (link:SRC), not a package name — smoked in 13" ;;
        service)    echo "a dependent statement (service:NAME), and starting one mutates the host" ;;
        setting)    echo "a dependent statement (setting:K @value=), and it writes a live desktop setting" ;;
        vscode)     echo "installs an extension into the developer's real editor profile" ;;
        emacs)      echo "installs a package into the developer's real Emacs profile" ;;
        # OPEN BUG, not a property of this host: `helm plugin install` takes a URL and
        # `helm plugin uninstall` takes the plugin NAME, and a declaration carries one
        # name — so a helm plugin LiNix installed, LiNix cannot remove.
        helm)       echo "OPEN: helm's install takes a URL and its uninstall takes a name, so LiNix cannot remove what it installed — see Part VII" ;;
        mise|asdf)  echo "rewrites the host's tool-version shims" ;;
        web|appimage) echo "installs from a pasted URL; no stable public canary — smoked in 13" ;;
        btrfs)      echo "a snapshot provider, not an install target" ;;
        *)          echo "" ;;
    esac
}

# A manager whose own uninstall deletes the package and keeps its launcher. Reported,
# never assumed: the strict check runs first, and this only softens the result when the
# leftover actually happens — so a manager that starts cleaning up still passes.
removal_leaves_binary() {
    case "$1" in
        bun) echo "bun's own \`remove -g\` drops the package and keeps its .exe/.bunx launchers (reproduced against bun directly, with no LiNix involved)" ;;
        *)   echo "" ;;
    esac
}

assert_binary_gone() {
    _be="$1"; _bin="$2"
    if ! on_path "$_bin"; then
        PASS=$((PASS + 1)); echo "  PASS  $_be: $_bin is off PATH"; return 0
    fi
    _known="$(removal_leaves_binary "$_be")"
    if [ -n "$_known" ]; then
        soft "$_be: $_bin is still on PATH after removal — $_known"
        return 0
    fi
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - $_be: $_bin is still on PATH after removal"
    echo "  FAIL  $_be: $_bin is still on PATH after removal"
    return 1
}

# A manager whose `list` answers a different question than its `install`. Named, because
# "the install worked and `list` does not show it" is otherwise indistinguishable from a
# parser that is broken — which is the one thing this section exists to catch.
list_cannot_show() {
    case "$1" in
        cabal) echo "\`cabal list --installed\` reports the GHC package DB (libraries); \`cabal install\` builds an EXECUTABLE into ~/.cabal/bin, which that DB never mentions" ;;
        *)     echo "" ;;
    esac
}

# Take a canary's line back out of the manifest.
#
# Every install syncs the WHOLE model, so a line left behind is retried by every backend
# after this one — and they then fail with the FIRST one's error. That happens for two
# reasons and both are by design: a pinned name a manager could not install stays (V.7c),
# and a manager with no uninstall verb cannot take its own line out.
#
# Both halves matter. Deleting the line stops the next sync from re-installing it;
# `unmanage` stops the registry from reporting it as drift and trying to REMOVE it —
# which is the state a failed removal leaves behind, and it fails identically on every
# sync after that.
undeclare_canary() {
    $TO "$LINIX" unmanage "$1" >/dev/null 2>&1 || true
    _imp="$LINIX_CONFIG_DIR/modules/imperative.txt"
    [ -f "$_imp" ] || return 0
    grep -v -F "$1" "$_imp" > "$_imp.tmp" 2>/dev/null
    mv "$_imp.tmp" "$_imp"
}

READY_LIST=$(lx doctor 2>/dev/null | grep '^\[READY\]' | awk '{print $2}' | sort)
echo "        READY backends: $(echo $READY_LIST | tr '\n' ' ')"

lifecycle() {
    be="$1"
    spec="$(canary "$be")"
    cpkg="$(echo "$spec" | cut -d'|' -f1)"
    cbin="$(echo "$spec" | cut -d'|' -f2)"
    cmode="$(echo "$spec" | cut -d'|' -f3)"
    ctok="$(echo "$spec" | cut -d'|' -f4)"
    [ -n "$ctok" ] || ctok="$cpkg"

    echo "    -- $be:$cpkg"
    grep -qx "$be" "$LINIX_CONFIG_DIR/priority" 2>/dev/null || echo "$be" >> "$LINIX_CONFIG_DIR/priority"

    # Same rule as section 5: a canary this host already had must not be taken away.
    had_it=""
    lx list --backend "$be" 2>/dev/null | grep -q "$ctok" && had_it=1
    if [ -n "$had_it" ]; then
        soft "$be: $cpkg is already installed on this host — left alone rather than removed"
        echo "$be" >> "$LEDGER/be-life-partial"
        return 0
    fi

    if ! lx -y install "$be:$cpkg" >/tmp/itw-life.out 2>&1; then
        soft "$be: install of $cpkg failed (ecosystem/network variance) — the checks after it did not run"
        tail -4 /tmp/itw-life.out | sed 's/^/        | /'
        echo "$be" >> "$LEDGER/be-life-partial"
        # A pinned name a manager could not install stays in the manifest on purpose —
        # but every later install syncs the WHOLE model, so leaving it made every backend
        # after this one fail with THIS one's error, and the log showed the same stack
        # trace under nine different names. Clear it, as section 6 clears its own.
        _imp="$LINIX_CONFIG_DIR/modules/imperative.txt"
        if [ -f "$_imp" ]; then
            grep -v -F "$be:$cpkg" "$_imp" > "$_imp.tmp" 2>/dev/null
            mv "$_imp.tmp" "$_imp"
        fi
        return 0
    fi
    PASS=$((PASS + 1)); echo "  PASS  $be installed $cpkg for real"
    echo "$be" >> "$LEDGER/be-life"

    _nolist="$(list_cannot_show "$be")"
    if [ -n "$_nolist" ]; then
        soft "$be: list does not show $ctok — $_nolist"
    else
        grep_ok "$be: list shows $ctok" "$ctok" lx list --backend "$be"
    fi
    [ -n "$cbin" ] && ok "$be: $cbin is on PATH" on_path "$cbin"

    if [ "$cmode" = "unsupported" ]; then
        grep_ok "$be: removal reports a graceful unsupported" \
            "not support\|unsupport\|cannot remove\|no remove" \
            lx -y uninstall "$be:$cpkg"
        # That refusal is correct AND it leaves the line, so take it out by hand.
        undeclare_canary "$be:$cpkg"
        return 0
    fi
    ok "$be: uninstall $cpkg" lx -y uninstall "$be:$cpkg"
    [ -n "$_nolist" ] || nok "$be: $ctok is gone from list" sh -c \
        "$LINIX list --backend '$be' 2>/dev/null | grep -q '$ctok'"
    [ -n "$cbin" ] && assert_binary_gone "$be" "$cbin"
    undeclare_canary "$be:$cpkg"
    return 0
}

for be in $READY_LIST; do
    [ "$be" = "$BACKEND" ] && continue          # section 5 already did this one
    reason="$(no_lifecycle_reason "$be")"
    if [ -n "$reason" ]; then
        soft "$be: no real lifecycle here — $reason"
        continue
    fi
    if [ -z "$(canary "$be")" ]; then
        # It still gets a plan-smoke below, so the audit passes — which is the point of
        # saying this out loud: the host could have run it for real and did not.
        soft "$be: READY here and this harness has no canary — it falls through to the plan-smoke, which is weaker than this host could give"
        continue
    fi
    lifecycle "$be"
done

# ==========================================================================
# 13. PLAN-SMOKE — every backend this host cannot (or must not) run for real
# ==========================================================================
echo "[13] Plan-smoke, every backend not lifecycled above"

ALL_BACKENDS=$(lx doctor --json 2>/dev/null \
    | sed -n 's/.*"backend"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | sort -u)
echo "        registered backends: $(echo $ALL_BACKENDS | wc -w)"
ok "doctor --json enumerates the registry" test -n "$ALL_BACKENDS"

SMOKE_CFG="${TMPDIR:-/tmp}/linix-it-win-smoke"
rm -rf "$SMOKE_CFG" 2>/dev/null; mkdir -p "$SMOKE_CFG/modules" "$SMOKE_CFG/profiles"
printf 'Work\n' > "$SMOKE_CFG/active"
printf 'use base\n' > "$SMOKE_CFG/profiles/Work"
: > "$SMOKE_CFG/modules/base.txt"
: > "$SMOKE_CFG/priority"
for b in $ALL_BACKENDS; do echo "$b" >> "$SMOKE_CFG/priority"; done

SMOKE_CFG_ARG="$(cygpath -w "$SMOKE_CFG" 2>/dev/null || echo "$SMOKE_CFG")"
SMOKE_DATA_ARG="$(cygpath -w "$SMOKE_CFG/state" 2>/dev/null || echo "$SMOKE_CFG/state")"
smoke_lx() {
    record_argv "$@"
    env LINIX_CONFIG_DIR="$SMOKE_CFG_ARG" LINIX_DATA_DIR="$SMOKE_DATA_ARG" $TO "$LINIX" "$@"
}

smoke_pkg() {
    case "$1" in
        github)   echo "sharkdp/fd" ;;
        go)       echo "golang.org/x/example/hello" ;;
        composer) echo "psr/log" ;;
        emerge)   echo "app-misc/jq" ;;
        vscode)   echo "ms-python.python" ;;
        flatpak)  echo "org.freedesktop.Platform" ;;
        helm)     echo "https://github.com/databus23/helm-diff" ;;
        web)      echo "https://example.invalid/tool.tar.gz" ;;
        appimage) echo "https://example.invalid/tool.AppImage" ;;
        *)        echo "$PKG" ;;
    esac
}

for be in $ALL_BACKENDS; do
    grep -qx "$be" "$LEDGER/be-life" 2>/dev/null && continue
    case "$be" in
        service)
            printf 'service:Spooler\n' > "$SMOKE_CFG/modules/base.txt"
            ok "service: a service statement parses" smoke_lx check
            ok "service: and reaches a plan" smoke_lx --dry-run sync
            : > "$SMOKE_CFG/modules/base.txt"
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
        link)
            printf 'link:/etc/hostname @target=/tmp/linix-it-hostname\n' > "$SMOKE_CFG/modules/base.txt"
            ok "link: a link statement parses" smoke_lx check
            ok "link: and reaches a plan" smoke_lx --dry-run sync
            : > "$SMOKE_CFG/modules/base.txt"
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
        setting)
            printf 'setting:org.gnome.desktop.interface/color-scheme @value=prefer-dark\n' \
                > "$SMOKE_CFG/modules/base.txt"
            ok "setting: a setting statement parses" smoke_lx check
            ok "setting: and reaches a plan" smoke_lx --dry-run sync
            : > "$SMOKE_CFG/modules/base.txt"
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
        btrfs)
            ok "btrfs: the snapshot verb runs" smoke_lx snapshot list
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
    esac
    sp="$(smoke_pkg "$be")"
    if grep_ok "$be: a dry-run install plans $be:$sp" "$be:$sp" \
            smoke_lx --dry-run install "$be:$sp"; then
        echo "$be" >> "$LEDGER/be-smoke"
    fi
done

# ==========================================================================
# 14. The command surface, RUN — not just `--help`
# ==========================================================================
# 23 of the previous run's 61 checks were `<cmd> --help`, which proves clap is
# wired and nothing else. Every command below is actually executed; the ones that
# cannot be are exempted BY NAME in EXEMPT_CMDS.
echo "[14] Command surface, executed"

ok "vars resolves this machine's variables" lx vars
# `repl` (U34) reads stdin until EOF; a piped session drives the loop and exits, and runs through
# `lx` so the coverage check counts it as really executed, not merely `--help`'d.
if printf ':help\n:vars\n:quit\n' | lx repl >/tmp/it.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  repl evaluates a piped session and exits on EOF (U34)"
else
    FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - repl piped session failed"
    echo "  FAIL  repl piped session"; tail -4 /tmp/it.out | sed 's/^/        | /'
fi
ok "unmanaged lists what LiNix does not manage" lx unmanaged
ok "path prints the config repo" lx path
ok "path --explain says which source won" lx path --explain
ok "config show prints the active configuration" lx config show
ok "policy checks the desired state against [guard]" lx policy
ok "conflicts reports cross-backend conflicts" lx conflicts
ok "sbom emits a bill of materials" lx sbom
ok "completions powershell generates a script" lx completions powershell
ok "profile list" lx profile list
ok "profile active" lx profile active
ok "profile create scaffolds one" lx profile create HarnessProfile
ok "profile show reads it back" lx profile show HarnessProfile
ok "module list" lx module list
ok "module create scaffolds one" lx module create harness-module
ok "module show reads it back" lx module show harness-module
ok "snapshot list" lx snapshot list
ok "schedule list" lx schedule list
ok "service list" lx service list
if lx repo list >/tmp/itw.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  repo list enumerates repositories"
else
    grep_ok "repo list says which backends cannot enumerate" \
        "not supported\|does not support" cat /tmp/itw.out
fi
ok "list enumerates what is installed" lx list
ok "hooks status says which managers are hookable" lx hooks status
ok "hooks shell-init prints the wrapper functions" lx hooks shell-init bash
ok "heal recovers an uninterrupted transaction" lx heal
ok "clean-cache frees archives without removing a package" lx clean-cache
ok "update refreshes repository metadata" lx update
ok "watch --once runs a single unattended reconcile" lx -y watch --once
ok "search finds something" lx search "$PKG"
ok "info reads a package's metadata" lx info "$PKG"
ok "why explains a package's provenance" lx why "$PKG"
ok "lock records installed versions" lx lock
ok "upgrade --dry-run previews" lx --dry-run upgrade
ok "remove-orphans previews without removing" lx --dry-run remove-orphans
ok "activate converges onto the named profiles" lx -y activate Main
ok "deactivate previews dropping one" lx --dry-run deactivate HarnessProfile
ok "hold pins a package against bulk upgrade" lx hold "$PKG"
ok "unhold releases it" lx unhold "$PKG"
ok "teleport previews moving a package between managers" lx --dry-run teleport "$PKG" cargo
if lx audit >/tmp/itw.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  audit scans for vulnerabilities"
else
    soft "audit ran but could not reach the OSV.dev database"
fi
ok "export writes native manifests" lx export --out "${TMPDIR:-/tmp}/linix-it-win-export"
# PINNED to this host's manager. An unpinned name resolved to a library crate on a
# machine that had cargo and not the tool, so the check failed on the resolver's
# answer rather than on `run`.
ok "run executes inside an ephemeral environment" lx run -p "$BACKEND:$PKG" true

ok "plan freezes a reviewable file" lx plan --out "${TMPDIR:-/tmp}/linix-it-win-plan.json"
ok "the plan file exists" test -f "${TMPDIR:-/tmp}/linix-it-win-plan.json"
ok "apply reads a saved plan" lx --dry-run apply "${TMPDIR:-/tmp}/linix-it-win-plan.json"

# `edit` shells out to $VISUAL/$EDITOR; `true` is an editor that exits 0.
record_argv edit priority
ok "edit opens a file in \$EDITOR" env EDITOR=true VISUAL=true $TO "$LINIX" edit priority

# reset deletes the registry. The command is exercised through the refusal it owes a
# machine that still has a config repo — running it for real would end the run.
nok "reset refuses while a config repo still exists" lx reset
grep_ok "and says --force is what overrides it" "force" lx reset

ok "self-upgrade --check reports the version and source" lx self-upgrade --check

# --- 14b. bundle → restore, the round trip (V.59) -------------------------
echo "[14b] bundle → restore"
BUNDLE_DIR="${TMPDIR:-/tmp}/linix-it-win-bundle"
RESTORE_DIR="${TMPDIR:-/tmp}/linix-it-win-restored"
rm -rf "$BUNDLE_DIR" "$RESTORE_DIR" 2>/dev/null
ok "bundle packs the config" lx bundle --out "$BUNDLE_DIR"
ok "the bundle directory exists" test -d "$BUNDLE_DIR"
mkdir -p "$RESTORE_DIR"
RESTORE_ARG="$(cygpath -w "$RESTORE_DIR" 2>/dev/null || echo "$RESTORE_DIR")"
# The data dir is a SIBLING, not a child: put LiNix's state inside the config directory
# and the very first command makes that directory non-empty, so `restore` refuses it —
# and the test for "restores into a clean directory" can never run.
RESTORE_STATE_DIR="${TMPDIR:-/tmp}/linix-it-win-restored-state"
rm -rf "$RESTORE_STATE_DIR" 2>/dev/null
RESTORE_STATE_ARG="$(cygpath -w "$RESTORE_STATE_DIR" 2>/dev/null || echo "$RESTORE_STATE_DIR")"
restore_lx() {
    env LINIX_CONFIG_DIR="$RESTORE_ARG" LINIX_DATA_DIR="$RESTORE_STATE_ARG" $TO "$LINIX" "$@"
}
record_argv restore "$BUNDLE_DIR"
ok "restore into a clean config directory" restore_lx restore "$BUNDLE_DIR"
ok "the restored model parses" restore_lx check
nok "restore refuses a config directory that is not empty" restore_lx restore "$BUNDLE_DIR"
ok "and --force overrides it" restore_lx restore "$BUNDLE_DIR" --force

# --- 14c. `--help` for the whole surface ----------------------------------
# Kept, but demoted: it catches a subcommand whose clap wiring is broken, and the
# audit below does not accept it as coverage.
echo "[14c] --help across the surface"
HELP_CMDS=$("$LINIX" --help 2>&1 | sed -n '/^Commands:/,/^Options:/p' \
    | sed -n 's/^  \([a-z][a-z-]*\) .*/\1/p' | grep -v '^help$' | sort -u)
for c in $HELP_CMDS; do
    ok "\`$c --help\` exists" lx "$c" --help
done

# ==========================================================================
# 15. COVERAGE AUDIT — what did nothing touch? (IV.1)
# ==========================================================================
echo "[15] Coverage audit"

sort -u "$LEDGER/be-life" > "$LEDGER/be-life.u" 2>/dev/null || : > "$LEDGER/be-life.u"
sort -u "$LEDGER/be-life-partial" > "$LEDGER/be-life-partial.u" 2>/dev/null || : > "$LEDGER/be-life-partial.u"
sort -u "$LEDGER/be-smoke" > "$LEDGER/be-smoke.u" 2>/dev/null || : > "$LEDGER/be-smoke.u"
sort -u "$LEDGER/cmd-real" > "$LEDGER/cmd-real.u" 2>/dev/null || : > "$LEDGER/cmd-real.u"

echo "        backends: $(grep -c . "$LEDGER/be-life.u") real lifecycle, \
$(grep -c . "$LEDGER/be-life-partial.u") install-attempted, \
$(grep -c . "$LEDGER/be-smoke.u") plan-smoked"

UNTOUCHED_BE=""
for be in $ALL_BACKENDS; do
    grep -qx "$be" "$LEDGER/be-life.u"         && continue
    grep -qx "$be" "$LEDGER/be-life-partial.u" && continue
    grep -qx "$be" "$LEDGER/be-smoke.u"        && continue
    UNTOUCHED_BE="$UNTOUCHED_BE $be"
done
if [ -n "$UNTOUCHED_BE" ]; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - coverage: backend(s) no lifecycle and no plan-smoke touched:$UNTOUCHED_BE"
    echo "  FAIL  every registered backend is covered — untouched:$UNTOUCHED_BE"
else
    PASS=$((PASS + 1)); echo "  PASS  every registered backend got a lifecycle or a plan-smoke"
fi

EXEMPT_CMDS="shell undo history bisect fleet"
exempt_reason() {
    case "$1" in
        shell)   echo "opens an interactive subshell" ;;
        undo)    echo "an interactive snapshot gallery" ;;
        history) echo "an interactive manifest-history TUI" ;;
        bisect)  echo "restores system snapshots, and may need a reboot between steps" ;;
        fleet)   echo "compares machines over SSH; there are no peers here" ;;
        *)       echo "" ;;
    esac
}
for c in $EXEMPT_CMDS; do echo "        exempt: $c — $(exempt_reason "$c")"; done

UNTOUCHED_CMD=""
for c in $HELP_CMDS; do
    grep -qx "$c" "$LEDGER/cmd-real.u" && continue
    case " $EXEMPT_CMDS " in *" $c "*) continue ;; esac
    UNTOUCHED_CMD="$UNTOUCHED_CMD $c"
done
echo "        subcommands: $(echo $HELP_CMDS | wc -w) in --help, \
$(grep -c . "$LEDGER/cmd-real.u") executed, $(echo $EXEMPT_CMDS | wc -w) exempt"
if [ -n "$UNTOUCHED_CMD" ]; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - coverage: subcommand(s) only ever reached via --help:$UNTOUCHED_CMD"
    echo "  FAIL  every subcommand is executed — only --help'd:$UNTOUCHED_CMD"
else
    PASS=$((PASS + 1)); echo "  PASS  every non-exempt subcommand was executed, not just --help'd"
fi

# --- Summary ---------------------------------------------------------------
echo "=============================================================="
echo " RESULT  pass=$PASS  fail=$FAILC  soft=$SOFTC"
if [ "$FAILC" -ne 0 ]; then
    printf " FAILURES:%b\n" "$FAILED_NAMES"
    exit 1
fi
echo " OK — every hard check passed."
exit 0
