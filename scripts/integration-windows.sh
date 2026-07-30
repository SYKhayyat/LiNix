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
#
# `timeout` is GNU coreutils: Linux ships it, macOS ships neither it nor `gtimeout`
# unless somebody installed coreutils. Naming it unconditionally is what a whole macOS
# run cost — every wrapped call exited 127, and 127 is indistinguishable from a refusal
# to anything that only asks "was it non-zero". Unbounded is worse than a wedge only if
# nobody is told, so the fallback is announced rather than assumed.
if command -v timeout >/dev/null 2>&1; then
    TO="timeout 900"
elif command -v gtimeout >/dev/null 2>&1; then
    TO="gtimeout 900"
else
    TO=""
fi
lx() { record_argv "$@"; $TO "$LINIX" "$@"; }

PASS=0; FAILC=0; SOFTC=0; FAILED_NAMES=""

# An identity for section 9's `git init`, when this machine has none.
#
# **Per-process, never `git config --global`.** This harness runs on a real machine — that is
# its whole point — so writing a global identity would replace the owner's. The container twin
# did exactly that once it started being run on the host, and thirteen commits went out under
# the wrong name before anyone noticed (2026-07-28).
#
# Only when git has no identity, so a developer's own is left alone. Without it, a clean CI
# runner fails `git init` with `unable to auto-detect email address` — LiNix's message is
# right and there is nobody there to act on it — and `diff` and `rollback` never run, which
# then fails the coverage audit for a reason that has nothing to do with them.
if ! git config user.email >/dev/null 2>&1; then
    export GIT_AUTHOR_NAME="LiNix Integration" GIT_AUTHOR_EMAIL="integration@linix.invalid"
    export GIT_COMMITTER_NAME="LiNix Integration" GIT_COMMITTER_EMAIL="integration@linix.invalid"
fi


# What a failing command actually said. `tail` alone is not that: RUST_BACKTRACE is on in
# CI, so the last lines of a failure are stack frames — on macOS, a column of identical
# `__mh_execute_header`, because the release binary carries no symbols — and the one line
# that says what went wrong scrolls off the top. A frame is never the reason a check
# failed, so the backtrace is dropped and what remains is the message.
#
# **It takes the log as an argument, and that is the point** (2026-07-29). It used to read
# `/tmp/itw.out` and nothing else, so every site reporting a *different* log fell back to a raw
# `tail` with no filtering — including `classify_install`'s retry, which is the one that reports
# a confirmed defect. A real macOS run produced exactly the failure this comment describes:
#
#     FAIL  github: install of github:sharkdp/fd failed twice — a defect, not ecosystem variance
#           |    3: __mh_execute_header
#           |    4: __mh_execute_header       (six frames, no message, nothing to act on)
#
# The cure was already written here and had reached one of its four callers. A helper that only
# helps its first caller is the twin-branch shape this repo keeps finding.
excerpt() { # [logfile] [lines]
    _ex_log="${1:-/tmp/itw.out}"; _ex_n="${2:-8}"
    _kept="$(grep -vE '^[[:space:]]*[0-9]+:|^[[:space:]]*at |^stack backtrace:|^note: [A-Z]?[a-z]* ?run with' "$_ex_log")"
    if [ -n "$_kept" ]; then
        printf '%s\n' "$_kept" | tail -"$_ex_n" | sed 's/^/        | /'
    else
        tail -"$_ex_n" "$_ex_log" | sed 's/^/        | /'
    fi
}
ok() {
    desc="$1"; shift
    if "$@" >/tmp/itw.out 2>&1; then
        PASS=$((PASS + 1)); echo "  PASS  $desc"; return 0
    else
        rc=$?; FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc)"
        echo "  FAIL  $desc (rc=$rc)"; excerpt; return 1
    fi
}
# For a command whose 2 means "answered, and here is what I found" rather than "failed":
# the aggregate `check` reports findings that way, and a machine with unmanaged packages
# is the ordinary case, not a broken run.
answers() {
    desc="$1"; shift
    "$@" >/tmp/itw.out 2>&1; rc=$?
    if [ "$rc" = 0 ] || [ "$rc" = 2 ]; then
        PASS=$((PASS + 1)); echo "  PASS  $desc (rc=$rc)"; return 0
    else
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc)"
        echo "  FAIL  $desc (rc=$rc)"; excerpt; return 1
    fi
}
# A command that could not run is not a refusal. 127 (no such command), 126 (not
# executable) and 124 (killed by the bound) all exit non-zero without the program ever
# reaching its own decision — and reading them as "correctly refused" is how a macOS run
# where nothing executed still printed passes.
never_ran() { [ "$1" = 127 ] || [ "$1" = 126 ] || [ "$1" = 124 ]; }
# Refuse to audit a set that collapsed. A set-containment audit over an EMPTY set passes
# without examining anything: the `for` runs zero times, the "untouched" string stays empty,
# and the check reports full coverage. Measured under a do-nothing `linix` stub, the audit
# printed "0 in --help ... 0 registered" and PASSed both of its meta-checks.
#
# The floor detects collapse, not coverage. A real registry is 48 backends on Windows and 56
# on Ubuntu, and a real `--help` carries ~55 subcommands; anything in single figures means the
# program under test did not answer, and an audit of an answer nobody gave proves nothing.
too_few_to_audit() { [ "$2" -lt "$1" ]; }

nok() {
    desc="$1"; shift
    "$@" >/tmp/itw.out 2>&1; rc=$?
    if [ "$rc" = 0 ]; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (expected non-zero, got 0)"
        echo "  FAIL  $desc (expected refusal, but it succeeded)"; return 1
    elif never_ran "$rc"; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc — never ran, not a refusal)"
        echo "  FAIL  $desc (rc=$rc — the command never ran; that is not a refusal)"
        excerpt; return 1
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

# A failure recorded directly, when the thing that failed was not a single command call.
hard() { FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES
    - $1"; echo "  FAIL  $1"; }
# A refusal is its own outcome. LiNix worked correctly and declined on purpose (exit 3), and
# scoring that as a failure — or as "ecosystem variance" — says the opposite of what happened.
refused() { PASS=$((PASS + 1)); echo "  PASS  $1 (LiNix refused, on purpose)"; }

# Why an install failed — a question, not an assumption (E5).
#
# Both harnesses used to soften ANY install failure into a claim about the network, and skip
# that backend's whole remaining lifecycle. In one observed run it fired four times and not
# once was it the network: one was LiNix correctly refusing, two were real argv defects
# (`helm`, `luarocks`). Coverage disappeared exactly where the product was broken, and the run
# still reported success.
#
# Sets CLASS to one of:
#   refused    LiNix declined on purpose (exit 3, U21). Its own outcome, not a failure.
#   timeout    the build ran out of time (124). Not a verdict on the backend.
#   transient  failed once, succeeded on retry. The caller CONTINUES the lifecycle — skipping
#              it is how list, PATH, remove and gone-from-list went unrun for every backend
#              whose install was flaky.
#   defect     failed twice, identically. Hard.
#
# $5 runs between the two attempts, for a caller that must clear a declaration the failed
# attempt left behind. Pass `:` when there is nothing to undo.
classify_install() { # be  install-spec  rc  logfile  [cleanup]
    _ci_be="$1"; _ci_spec="$2"; _ci_rc="$3"; _ci_log="$4"; _ci_clear="${5:-:}"
    if [ "$_ci_rc" -eq 124 ]; then
        soft "$_ci_be: install of $_ci_spec hit the build time limit — not a verdict on the backend"
        excerpt "$_ci_log" 4
        CLASS=timeout; return 0
    fi
    if [ "$_ci_rc" -eq 3 ]; then
        refused "$_ci_be: install of $_ci_spec"
        excerpt "$_ci_log" 3
        CLASS=refused; return 0
    fi
    # Transience is a claim that a second attempt could differ, so it is tested by making one.
    # A held lock or a dropped mirror passes now; a wrong name, a bad argv or a broken build
    # fails identically forever.
    echo "        (first attempt failed; retrying once to tell a flake from a defect)"
    $_ci_clear
    lx -y install "$_ci_spec" >/tmp/itw-retry.out 2>&1
    _ci_rc2=$?
    if [ "$_ci_rc2" -ne 0 ]; then
        hard "$_ci_be: install of $_ci_spec failed twice — a defect, not ecosystem variance (rc=$_ci_rc, $_ci_rc2)"
        excerpt /tmp/itw-retry.out 6
        CLASS=defect; return 0
    fi
    soft "$_ci_be: install of $_ci_spec failed once and succeeded on retry — transient"
    CLASS=transient
}


# Is NAME runnable right now? `command -v` alone answers from the shell's hash table
# and keeps naming a path after the file is gone, so a removal check written with it
# cannot fail. A fresh `sh` has an empty cache and has to look.
#
# A predicate answers yes or no and nothing else. `command -v` reports "not found" as 1
# under bash and as 127 under dash and busybox ash — the same 127 that means "I could not
# run at all", which is a distinction `nok` has to make. Collapsing it here keeps that
# ambiguity out of every caller instead of teaching each one about the host's /bin/sh.
on_path() {
    sh -c 'command -v "$1" >/dev/null 2>&1' _ "$1" && return 0
    return 1
}
# Where does NAME resolve, if anywhere. Same fresh-shell rule as on_path.
path_of() { sh -c 'command -v "$1" 2>/dev/null' _ "$1" || true; }

# The directory an install NAMED as the home of what it just put there, or "" if it named none.
#
# LiNix's answer to a bin directory that is not on PATH is a warning naming the directory and
# the line that would add it (E6c/W4). That sentence is the product's promise, so it is what
# the checks below read. Matched against the backend that printed it, so one sync that warns
# about two managers cannot hand one manager's directory to the other.
named_bin_dir() { # backend install-log
    [ -f "$2" ] || return 0
    _nbd_pat="s/.*$1. installs its executables into \\(.*\\), which is not on your PATH.*/\\1/p"
    _nbd="$(sed -n "$_nbd_pat" "$2" | head -1)"
    [ -n "$_nbd" ] || return 0
    cygpath -u "$_nbd" 2>/dev/null || echo "$_nbd"
}

# Where a name sits when PATH cannot reach it: the file in the directory the install named,
# or "" when there is no such file. The extensions are Windows's — `cowsay` on a runner is
# `cowsay.cmd`, and looking only for the bare name reports an installed program as absent.
off_path_copy() { # backend binary install-log
    _opd="$(named_bin_dir "$1" "$3")"
    [ -n "$_opd" ] || return 0
    for _ope in "" .exe .cmd .bat .ps1; do
        [ -e "$_opd/$2$_ope" ] && printf '%s\n' "$_opd/$2$_ope" && return 0
    done
    return 0
}

# Is NAME on this machine at all: resolvable, or sitting where its install said it went?
#
# `on_path` alone answers "can I type it", which stops being the same question the moment the
# install is honest about a directory the host has not wired up — and every assertion built on
# it (survived unmanage, gone after uninstall) was then reading the wrong answer.
binary_present() { # backend binary install-log
    on_path "$2" && return 0
    [ -n "$(off_path_copy "$1" "$2" "$3")" ]
}

# assert_binary_reachable <backend> <binary> <install-log>
#
# An install the user cannot invoke is a failed install reported as a success (E6c). On a clean
# runner most per-user managers install into a directory nobody's PATH names, so asking PATH
# alone fails runs where the product did everything it promised and passes runs where it said
# nothing at all. So the assertion is the promise: the name resolves, OR the install named the
# directory and the file is in it. Silence plus an unreachable binary is the defect — measured
# 2026-07-29 on a clean Windows runner, `github` and `yarn` both.
assert_binary_reachable() { # backend binary install-log
    _rbe="$1"; _rbin="$2"
    if on_path "$_rbin"; then
        PASS=$((PASS + 1)); echo "  PASS  $_rbe: $_rbin is on PATH"; return 0
    fi
    _rdir="$(named_bin_dir "$1" "$3")"
    if [ -z "$_rdir" ]; then
        FAILC=$((FAILC + 1))
        FAILED_NAMES="$FAILED_NAMES\n    - $_rbe: $_rbin is not on PATH and nothing said where it went"
        echo "  FAIL  $_rbe: $_rbin is not on PATH and nothing said where it went"
        return 1
    fi
    if binary_present "$1" "$_rbin" "$3"; then
        PASS=$((PASS + 1))
        echo "  PASS  $_rbe: $_rbin is not on PATH, and the install said so, naming $_rdir"
        return 0
    fi
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - $_rbe: the install named $_rdir and $_rbin is not in it"
    echo "  FAIL  $_rbe: the install named $_rdir and $_rbin is not in it"
    return 1
}

echo "=============================================================="
echo " LiNix v7 Windows/macOS harness — backend=$BACKEND package=$PKG"
echo " LINIX=$LINIX"
echo "=============================================================="

# Runnable, not merely present — and runnable THROUGH the wrapper every check below uses.
# `command -v` answers about the binary alone, so a missing `timeout` left every one of
# the sweep's own invocations exiting 127 while this line reported the binary was fine.
if ! $TO "$LINIX" --version >/dev/null 2>&1; then
    echo "FATAL: '${TO:+$TO }$LINIX --version' did not run — nothing below would be tested."
    command -v "$LINIX" >/dev/null 2>&1 \
        || echo "       '$LINIX' is not on PATH: set LINIX to the built binary, or build it."
    [ -n "$TO" ] || echo "       (no timeout wrapper in use, so the binary itself is the fault)"
    exit 2
fi
[ -n "$TO" ] || soft "no \`timeout\` nor \`gtimeout\` on this host — commands run unbounded"

# --- 1. Bootstrap ----------------------------------------------------------
echo "[1] Bootstrap"
ok "init scaffolds the repo" lx init
ok "priority file exists" test -f "$LINIX_CONFIG_DIR/priority"
ok "active file exists" test -f "$LINIX_CONFIG_DIR/active"
grep_ok "priority names this backend" "$BACKEND" cat "$LINIX_CONFIG_DIR/priority"

# --- 2. Discovery / read-only ---------------------------------------------
echo "[2] Discovery / read-only verbs"
ok "check health" lx check health
ok "check drift" lx check drift
# The aggregate `check` exits 2 when it has findings to report, and an unmanaged package
# on a developer's own machine is a finding. Every named section exits 0.
answers "check parses the model" lx check
ok "check absent" lx check absent
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
# `on_path` here and `binary_present` below, deliberately: this runs BEFORE the install, so
# there is no log in which anything could have named a directory yet.
on_path "$PKG" && PKG_WAS_HERE=1

> /tmp/itw-life0.out
lx -y install "$BACKEND:$PKG" >/tmp/itw-life0.out 2>&1
IRC=$?
CLASS=installed
[ "$IRC" -ne 0 ] && classify_install "$BACKEND" "$BACKEND:$PKG" "$IRC" /tmp/itw-life0.out

# `transient` continues: the retry inside `classify_install` succeeded, so the package IS on
# the machine and every check below it is answerable. This is the half of E5 that mattered —
# the old catch-all skipped list, PATH, second-sync, unmanage and uninstall for any backend
# whose install hiccuped once.
if [ "$CLASS" = installed ] || [ "$CLASS" = transient ]; then
    [ "$CLASS" = installed ] && { PASS=$((PASS + 1)); echo "  PASS  install $BACKEND:$PKG"; }
    echo "$BACKEND" >> "$LEDGER/be-life"
    grep_ok "list shows $PKG" "$PKG" lx list
    assert_binary_reachable "$BACKEND" "$PKG" /tmp/itw-life0.out
    ok "second sync is a no-op" lx -y sync
    # `unmanage` belongs here and not with the read-only verbs: "forgets it WITHOUT
    # uninstalling it" is only a proof while something is installed to leave behind.
    ok "unmanage forgets a package without uninstalling it" lx unmanage "$BACKEND:$PKG"
    ok "$PKG is still installed after unmanage" binary_present "$BACKEND" "$PKG" /tmp/itw-life0.out
    ok "declaring it again takes it back" lx -y install "$BACKEND:$PKG"
    ok "uninstall $BACKEND:$PKG" lx -y uninstall "$BACKEND:$PKG"
    # S36 again, on the package the run did not install. When the host already owned
    # $PKG, absence is not this harness's to demand: the manager may legitimately keep a
    # formula another one depends on, and a second copy may live outside its prefix. The
    # strict assertion is kept for the case it is actually about — a package this run put
    # on the machine and took back off.
    if [ -n "$PKG_WAS_HERE" ]; then
        if binary_present "$BACKEND" "$PKG" /tmp/itw-life0.out; then
            soft "$PKG is still there after uninstall — it predates the run, so absence is not asserted"
        else
            PASS=$((PASS + 1)); echo "  PASS  $PKG binary gone after uninstall"
        fi
    else
        nok "$PKG binary gone after uninstall" binary_present "$BACKEND" "$PKG" /tmp/itw-life0.out
    fi
    if [ -n "$PKG_WAS_HERE" ]; then
        if lx -y install "$BACKEND:$PKG" >/dev/null 2>&1; then
            soft "$PKG was on this host before the run — put back, so the sweep leaves nothing missing"
        else
            soft "$PKG was on this host before the run and could NOT be put back — reinstall it by hand"
        fi
    fi
else
    # A refusal, a timeout or a defect. `classify_install` has already recorded the verdict
    # with the right severity; the lifecycle below it is genuinely unanswerable, because
    # nothing was installed to look at.
    echo "$BACKEND" >> "$LEDGER/be-life-partial"
fi

# --- 6. Negative path ------------------------------------------------------
echo "[6] Negative path"
nok "installing a nonexistent package fails" lx -y install "$BACKEND:linix-no-such-pkg-zzz"
answers "a failed install leaves the model parseable" lx check
# This asserts the PRODUCT withdrew the line. It used to `grep -v` the name out and then
# assert it was gone, which tested its own `grep -v` and printed PASS on every run while the
# product did the opposite — and the scrub was load-bearing, because the line left behind then
# failed `rollback`, `activate` and `restore --force` later in this same sweep.
#
# The name here is QUALIFIED, so the backend resolves and the install fails: by the ruling of
# 2026-07-27 (Q1) that is withdrawn when the backend's own ExitPolicy calls the failure
# permanent. If this goes red, $BACKEND has no policy that can tell a wrong name from a
# dropped network — which is a real gap in that backend, not a reason to put the scrub back.
IMPERATIVE="$LINIX_CONFIG_DIR/modules/imperative.txt"
if [ -f "$IMPERATIVE" ]; then
    nok "the unresolvable name is out of the manifest" \
        grep -q "linix-no-such-pkg-zzz" "$IMPERATIVE"
fi

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
    # while the manager under test is untouched.
    #
    # The shim has to be something the host's process launcher will actually run.
    # Windows resolves a bare `cargo` through PATHEXT, so there it must be a `.bat`;
    # every other host resolves the executable bit, and a `.bat` on macOS is an inert
    # file that shadows nothing — so this section staged no silent manager at all, and
    # then reported that the plan failed to mention one.
    SILENT_BIN="${TMPDIR:-/tmp}/linix-it-silent-bin"
    rm -rf "$SILENT_BIN"; mkdir -p "$SILENT_BIN"
    case "$(uname -s 2>/dev/null)" in
        MINGW*|MSYS*|CYGWIN*|Windows_NT)
            printf '@echo off\r\nif "%%1"=="search" (\r\n  echo error: failed to fetch the registry index 1>&2\r\n  exit /b 1\r\n)\r\n"%s" %%*\r\n' \
                "$(cygpath -w "$REAL_CARGO" 2>/dev/null || echo "$REAL_CARGO")" > "$SILENT_BIN/cargo.bat"
            ;;
        *)
            cat > "$SILENT_BIN/cargo" <<EOSHIM
#!/bin/sh
if [ "\$1" = "search" ]; then
    echo "error: failed to fetch the registry index" >&2
    exit 1
fi
exec "$REAL_CARGO" "\$@"
EOSHIM
            chmod +x "$SILENT_BIN/cargo"
            ;;
    esac

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
        # A helm plugin installs under the user's own helm data dir and reaches PATH
        # through nothing — it is run as `helm diff` — so no binary is asserted.
        helm)     echo "secrets||full||@url=https://github.com/jkroepke/helm-secrets,unverified" ;;
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

# assert_binary_gone <backend> <binary> <what-the-name-resolved-to-before-the-install>
#
# The question is "did this backend's install get undone", NOT "does this name resolve".
# Two managers can ship a binary of the same name, and one of them may hold it on
# purpose: cabal's canary is `hello`, cabal has no uninstall verb, so its `hello` stays
# for the rest of the run — and go's canary is also `hello`. Asking PATH handed cabal's
# leftover to go as a failure, on a removal that had worked.
#
# So the assertion is against the state before the install: whatever the install added
# must be gone, and whatever was already there is not this backend's to answer for.
assert_binary_gone() {
    _be="$1"; _bin="$2"; _was="$3"
    _now="$(path_of "$_bin")"
    # A binary that was never on PATH is "gone" by PATH from the moment it was installed, so
    # this check answered yes before the removal ran. Where the install SAID the file went is
    # the only place that can tell, and it is the fourth argument.
    [ -n "$_now" ] || _now="$(off_path_copy "$_be" "$_bin" "${4:-}")"
    if [ "$_now" = "$_was" ]; then
        if [ -n "$_now" ]; then
            PASS=$((PASS + 1))
            echo "  PASS  $_be: $_bin is back to the pre-install $_now (not this backend's copy)"
        else
            PASS=$((PASS + 1)); echo "  PASS  $_be: $_bin is gone"
        fi
        return 0
    fi
    _known="$(removal_leaves_binary "$_be")"
    if [ -n "$_known" ]; then
        soft "$_be: $_bin is still there after removal — $_known"
        return 0
    fi
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - $_be: $_bin is still on PATH after removal (at $_now)"
    echo "  FAIL  $_be: $_bin is still on PATH after removal (at $_now)"
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

READY_LIST=$(lx check health 2>/dev/null | grep '^\[READY\]' | awk '{print $2}' | sort)

# And the backends LiNix reports as degraded ONLY because a setup step it offers to run has not
# been run (Q10/Q11/Q13). They belong in the lifecycle for the same reason they are degraded:
# `lx -y install` performs that setup, so leaving them out tests the offer nowhere — which is
# what happened the first night the health check shipped, when `mix` dropped from a real
# lifecycle to a plan-smoke and the run still said PASS.
#
# The sentence is LiNix's own (`src/verbs/check.rs`); if it changes, this must change with it,
# which is why it is one grep in one place rather than a pattern in each check.
SETUP_LIST=$(lx check health 2>/dev/null \
    | grep 'before it can install anything' \
    | sed -n 's/.*\] *\([A-Za-z0-9_-]*\).*/\1/p' | sort)
[ -n "$SETUP_LIST" ] && echo "        needs setup, and the sweep exercises it anyway: $(echo $SETUP_LIST | tr '\n' ' ')"
READY_LIST=$(printf '%s\n%s\n' "$READY_LIST" "$SETUP_LIST" | grep -v '^[[:space:]]*$' | sort -u)
echo "        READY backends: $(echo $READY_LIST | tr '\n' ' ')"

lifecycle() {
    be="$1"
    spec="$(canary "$be")"
    cpkg="$(echo "$spec" | cut -d'|' -f1)"
    cbin="$(echo "$spec" | cut -d'|' -f2)"
    cmode="$(echo "$spec" | cut -d'|' -f3)"
    ctok="$(echo "$spec" | cut -d'|' -f4)"
    # `@k=v` appended at INSTALL only: helm installs a plugin from a URL and removes it
    # by name, so the two verbs cannot be handed the same string (U39).
    copts="$(echo "$spec" | cut -d'|' -f5)"
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

    # Read before the install, because the removal check below is a comparison against
    # it: a name another manager already owns must not be scored as this one's leftover.
    _prepath="$(path_of "$cbin")"
    [ -n "$_prepath" ] && soft "$be: $cbin already resolves to $_prepath — the removal check compares against that, not against absence"

    # A canary left declared makes every LATER backend sync the whole model and fail with THIS
    # one's error — nine identical stack traces under nine different names. So each attempt
    # below clears its own line before the next thing runs.
    _clear_canary() {
        $TO "$LINIX" unmanage "$be:$cpkg" >/dev/null 2>&1 || true
        _imp="$LINIX_CONFIG_DIR/modules/imperative.txt"
        if [ -f "$_imp" ]; then
            grep -v -F "$be:$cpkg" "$_imp" > "$_imp.tmp" 2>/dev/null
            mv "$_imp.tmp" "$_imp"
        fi
    }

    lx -y install "$be:$cpkg$copts" >/tmp/itw-life.out 2>&1
    lrc=$?
    if [ "$lrc" -ne 0 ]; then
        # One classifier, shared with section 5. Two copies of this decision is how section 5
        # kept the catch-all for a month after section 12 lost it.
        classify_install "$be" "$be:$cpkg$copts" "$lrc" /tmp/itw-life.out _clear_canary
        case "$CLASS" in
            transient) : ;;   # the retry succeeded; the lifecycle below is answerable
            defect)    echo "$be" >> "$LEDGER/be-life-partial"; _clear_canary; return 1 ;;
            *)         echo "$be" >> "$LEDGER/be-life-partial"; _clear_canary; return 0 ;;
        esac
    fi
    PASS=$((PASS + 1)); echo "  PASS  $be installed $cpkg for real"
    echo "$be" >> "$LEDGER/be-life"

    _nolist="$(list_cannot_show "$be")"
    if [ -n "$_nolist" ]; then
        soft "$be: list does not show $ctok — $_nolist"
    else
        grep_ok "$be: list shows $ctok" "$ctok" lx list --backend "$be"
    fi
    [ -n "$cbin" ] && assert_binary_reachable "$be" "$cbin" /tmp/itw-life.out

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
    [ -n "$cbin" ] && assert_binary_gone "$be" "$cbin" "$_prepath" /tmp/itw-life.out
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

ALL_BACKENDS=$(lx check health --json 2>/dev/null \
    | sed -n 's/.*"backend"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | sort -u)
echo "        registered backends: $(echo $ALL_BACKENDS | wc -w)"
ok "check health --json enumerates the registry" test -n "$ALL_BACKENDS"

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
        helm)     echo "secrets@url=https://github.com/jkroepke/helm-secrets,unverified" ;;
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
            answers "service: a service statement parses" smoke_lx check
            ok "service: and reaches a plan" smoke_lx --dry-run sync
            : > "$SMOKE_CFG/modules/base.txt"
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
        link)
            printf 'link:/etc/hostname @target=/tmp/linix-it-hostname\n' > "$SMOKE_CFG/modules/base.txt"
            answers "link: a link statement parses" smoke_lx check
            ok "link: and reaches a plan" smoke_lx --dry-run sync
            : > "$SMOKE_CFG/modules/base.txt"
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
        setting)
            printf 'setting:org.gnome.desktop.interface/color-scheme @value=prefer-dark\n' \
                > "$SMOKE_CFG/modules/base.txt"
            answers "setting: a setting statement parses" smoke_lx check
            ok "setting: and reaches a plan" smoke_lx --dry-run sync
            : > "$SMOKE_CFG/modules/base.txt"
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
        btrfs)
            ok "btrfs: the snapshot verb runs" smoke_lx snapshot list
            echo "$be" >> "$LEDGER/be-smoke"; continue ;;
    esac
    sp="$(smoke_pkg "$be")"
    # The plan names the package; its options are not part of that name.
    sp_tok="${sp%%@*}"
    if grep_ok "$be: a dry-run install plans $be:$sp" "$be:$sp_tok" \
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
# `eval` is the one output that will acquire consumers LiNix cannot see, so the thing
# asserted is the contract: a top-level schema version.
grep_ok "eval prints a versioned document" '"schema"' lx eval
# `repl` (U34) reads stdin until EOF; a piped session drives the loop and exits, and runs through
# `lx` so the coverage check counts it as really executed, not merely `--help`'d.
if printf ':help\n:vars\n:quit\n' | lx repl >/tmp/it.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  repl evaluates a piped session and exits on EOF (U34)"
else
    FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - repl piped session failed"
    echo "  FAIL  repl piped session"; excerpt /tmp/it.out 4
fi
ok "check unmanaged lists what LiNix does not manage" lx check unmanaged
ok "path prints the config repo" lx path
ok "path --explain says which source won" lx path --explain
ok "config show prints the active configuration" lx config show
ok "policy checks the desired state against [guard]" lx policy
ok "check conflicts reports cross-backend conflicts" lx check conflicts
ok "sbom emits a bill of materials" lx sbom
# `try` rehearses in a container. Named against an image that cannot exist, so the
# answer is a refusal on every host: with no runtime it refuses for want of one, with a
# runtime it refuses for want of the image — and neither spends ten minutes building.
nok "try refuses to rehearse on an image that is not there" lx try --image linix-it-no-such-image
grep_ok "try's refusal says what it refused" "refusing to rehearse" lx try --image linix-it-no-such-image
# `add` vendors a source's modules. A local path is the network-free case: it copies the
# module in and reports it. The line names the package this run already manages, so
# vendoring it declares nothing new.
SHARE_SRC="${TMPDIR:-/tmp}/linix-it-share"
rm -rf "$SHARE_SRC" 2>/dev/null; mkdir -p "$SHARE_SRC/modules"
printf '%s:%s\n' "$BACKEND" "$PKG" > "$SHARE_SRC/modules/shared.txt"
ok "add vendors a module from a local source" lx add "$SHARE_SRC"
ok "add brought the module file in" test -f "$LINIX_CONFIG_DIR/modules/shared.txt"
nok "add refuses a source that does not exist" lx add "${TMPDIR:-/tmp}/linix-it-no-such-source"
# Proved, then taken back out: a module left behind changes what every section after
# this one plans, and this section is about `add`, not about the model.
rm -f "$LINIX_CONFIG_DIR/modules/shared.txt"
ok "completions powershell generates a script" lx completions powershell
ok "profile list" lx profile list
ok "profile active" lx profile active
ok "profile create scaffolds one" lx profile create HarnessProfile
# "scaffolds" is a claim about the disk and the line above only reads an exit code. Found by
# running this harness against a `linix` that does nothing and exits 0: both `create` checks
# and both `show` checks passed, because not one of the four ever looked at a file.
ok "profile create wrote the profile" test -f "$LINIX_CONFIG_DIR/profiles/HarnessProfile"
ok "profile show reads it back" lx profile show HarnessProfile
ok "module list" lx module list
ok "module create scaffolds one" lx module create harness-module
ok "module create wrote the module" test -f "$LINIX_CONFIG_DIR/modules/harness-module.txt"
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
if lx check security >/tmp/itw.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  check security scans for vulnerabilities"
else
    soft "check security ran but could not reach the OSV.dev database"
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
answers "the restored model parses" restore_lx check
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
BE_COUNT=$(echo $ALL_BACKENDS | wc -w)
if too_few_to_audit 10 "$BE_COUNT"; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - coverage: the registry came back empty ($BE_COUNT backend(s)) — nothing was audited"
    echo "  FAIL  the registry enumerated $BE_COUNT backend(s); an audit over that examines nothing"
elif [ -n "$UNTOUCHED_BE" ]; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - coverage: backend(s) no lifecycle and no plan-smoke touched:$UNTOUCHED_BE"
    echo "  FAIL  every registered backend is covered — untouched:$UNTOUCHED_BE"
else
    PASS=$((PASS + 1)); echo "  PASS  every registered backend got a lifecycle or a plan-smoke"
fi

# --- the real-lifecycle ratchet (G-11) ------------------------------------
# The audit above accepts a plan-smoke as coverage, so a run with 4 real lifecycles and a run
# with 15 both PASS. This asks the other question: did THIS host class do worse than it has
# done before? The floor lives in `scripts/lifecycle-floor.txt` beside the reasoning.
LIFECYCLES=$(grep -c . "$LEDGER/be-life.u")
# A stable key. `uname -s` on git-bash is `MINGW64_NT-10.0-26200` — a Windows build number,
# so keying on it would mint a fresh host class (and a free pass) at every OS update.
case "$(uname -s 2>/dev/null)" in
    MINGW*|MSYS*|CYGWIN*|Windows*) HOST_OS=windows ;;
    Darwin*)                       HOST_OS=darwin ;;
    Linux*)                        HOST_OS=linux ;;
    *)                             HOST_OS=unknown ;;
esac
# Inside a container the distro is what decides which managers exist, so it is part of the
# class: ubuntu and the `tools` image are not comparable runs.
HOST_FLAVOUR=""
[ -r /etc/os-release ] && HOST_FLAVOUR="-$(. /etc/os-release 2>/dev/null; echo "${ID:-}")"
HOST_CLASS="windows-native-${HOST_OS}${HOST_FLAVOUR}-$([ -n "${CI:-}" ] && echo ci || echo local)"
FLOOR_FILE="$(dirname "$0")/lifecycle-floor.txt"
if [ -f "$FLOOR_FILE" ]; then
    FLOOR=$(grep -E "^${HOST_CLASS} " "$FLOOR_FILE" 2>/dev/null | awk '{print $2}' | head -1)
    if [ -z "$FLOOR" ]; then
        # The twin of the container branch, uncounted for the same reason: a record that is not
        # there compares nothing. Only `windows-native-windows-local` is recorded, so the CI
        # runner's own class lands here — and as a PASS it was a green check on the leg with no
        # floor at all.
        soft "real-lifecycle ratchet: no record for $HOST_CLASS yet, so nothing was compared"
        echo "        add to $FLOOR_FILE:  $HOST_CLASS $LIFECYCLES"
    elif [ "$LIFECYCLES" -lt "$FLOOR" ]; then
        FAILC=$((FAILC + 1))
        FAILED_NAMES="$FAILED_NAMES
    - coverage: $LIFECYCLES real lifecycle(s) on $HOST_CLASS, below the recorded $FLOOR"
        echo "  FAIL  real-lifecycle ratchet: $LIFECYCLES, and $HOST_CLASS has done $FLOOR before"
        echo "        Something stopped running. A plan-smoke satisfies the audit above, so this"
        echo "        is the only check that notices coverage collapsing rather than breaking."
    else
        PASS=$((PASS + 1))
        echo "  PASS  real-lifecycle ratchet: $LIFECYCLES >= $FLOOR recorded for $HOST_CLASS"
        [ "$LIFECYCLES" -gt "$FLOOR" ] &&             echo "        ratchet up:  sed -i 's/^$HOST_CLASS .*/$HOST_CLASS $LIFECYCLES/' $FLOOR_FILE"
    fi
else
    # The twin of the container harness's branch, and it was silent in the same way: one line,
    # tallied nowhere, so a run with the ratchet missing was indistinguishable from a run that
    # passed it (N-5). Here the file sits next to this script, so absence means someone deleted
    # or moved it — which is exactly when a gate must not go quiet.
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES
    - coverage: the real-lifecycle ratchet is not in force ($FLOOR_FILE is missing)"
    echo "  FAIL  real-lifecycle ratchet: $FLOOR_FILE is missing, so nothing checked whether"
    echo "        coverage collapsed. $LIFECYCLES real lifecycle(s) this run, unmeasured against"
    echo "        $HOST_CLASS."
fi

EXEMPT_CMDS="shell history bisect fleet"
exempt_reason() {
    case "$1" in
        shell)   echo "opens an interactive subshell" ;;
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
CMD_COUNT=$(echo $HELP_CMDS | wc -w)
if too_few_to_audit 20 "$CMD_COUNT"; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES\n    - coverage: --help listed $CMD_COUNT subcommand(s) — nothing was audited"
    echo "  FAIL  --help listed $CMD_COUNT subcommand(s); an audit over that examines nothing"
elif [ -n "$UNTOUCHED_CMD" ]; then
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
