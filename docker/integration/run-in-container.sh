#!/bin/sh
# ============================================================================
# LiNix v7 integration harness — runs INSIDE a disposable container as root.
#
#   Usage: run-in-container.sh <native-backend> [package]
#   e.g.   run-in-container.sh apt jq
#
# Driven entirely through the real `linix` binary against the distro's native
# package manager AND against every other package manager the image ships.
# Isolation is by env var (LINIX_CONFIG_DIR / LINIX_DATA_DIR), so LiNix's own
# state is a throwaway; real system packages ARE installed and removed (that is
# the point — it is a disposable container).
#
# HARD assertions: every check either passes or fails the whole run (exit 1 at
# the end if any failed). A short, honest list of "soft" checks (genuinely
# network/ecosystem-optional) is reported but never fails the run.
#
# The run ends in a COVERAGE AUDIT (IV.1) that hard-fails on any backend or any
# subcommand nothing touched. That is the only check here that can notice what
# is *missing* from the list above it — a fixed set of checks cannot.
# ============================================================================
set -u

BACKEND="${1:?usage: run-in-container.sh <backend> [package]}"
PKG="${2:-jq}"
LINIX="${LINIX:-linix}"
TO="timeout 300"
# A source-building manager (cargo, opam, nimble, spack, go) compiles the canary
# from scratch; 300s is a build that has barely started. 900s is long enough for a
# real build and short enough that one wedged manager cannot eat the whole matrix —
# and a run that hits it says so by name rather than blaming the network.
TO_LONG="timeout 900"

# --- Isolation: LiNix's config + data are throwaway; the II.1 repo lives here.
export LINIX_CONFIG_DIR="/tmp/linix-it-config"
export LINIX_DATA_DIR="/tmp/linix-it-state"
rm -rf "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"
mkdir -p "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"

# --- The coverage ledger. Files, not variables: `grep_ok` runs its command in a
# pipeline, and a pipeline is a subshell whose variable writes die with it — so a
# ledger kept in a variable would silently forget every command greped for.
LEDGER=/tmp/linix-it-ledger
rm -rf "$LEDGER"; mkdir -p "$LEDGER"
: > "$LEDGER/cmd-real"; : > "$LEDGER/cmd-help"
: > "$LEDGER/be-life"; : > "$LEDGER/be-life-partial"; : > "$LEDGER/be-life-unmeasured"; : > "$LEDGER/be-smoke"

# Record which subcommand an invocation actually ran, so the audit can name what
# nothing touched. Global flags are skipped; the two that take a value skip it too.
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

lx()      { record_argv "$@"; $TO "$LINIX" "$@"; }
lx_slow() { record_argv "$@"; $TO_LONG "$LINIX" "$@"; }

# LiNix commits as you (II.13) and injects no identity of its own, so git needs
# to know who that is. A bare container has no identity and every commit fails.
# An identity for the `git init` section, as environment rather than as `git config --global`.
#
# This script is written for a disposable container, where a global write is harmless. It is
# not only run there: `scripts/harness-mutation-test.sh` executes it on the host to measure
# whether its checks can fail, and both release scripts now do that — so on 2026-07-28 this
# pair silently replaced a developer's real git identity, and thirteen commits went out
# authored `LiNix Integration <integration@linix.invalid>` instead of by their author.
#
# `GIT_AUTHOR_*`/`GIT_COMMITTER_*` are per-process: they cover this run and touch nothing the
# user owns. And they are set only when git has no identity, so a machine that has one keeps
# it — what the harness exercises is then what its owner actually runs.
if ! git config user.email >/dev/null 2>&1; then
    export GIT_AUTHOR_NAME="LiNix Integration" GIT_AUTHOR_EMAIL="integration@linix.invalid"
    export GIT_COMMITTER_NAME="LiNix Integration" GIT_COMMITTER_EMAIL="integration@linix.invalid"
fi

PASS=0
FAILC=0
SOFTC=0
FAILED_NAMES=""

# What a failing command actually said. `tail` alone is not that: RUST_BACKTRACE is on in
# CI, so the last lines of a failure are stack frames — on macOS, a column of identical
# `__mh_execute_header`, because the release binary carries no symbols — and the one line
# that says what went wrong scrolls off the top. A frame is never the reason a check
# failed, so the backtrace is dropped and what remains is the message.
#
# **It takes the log as an argument, and that is the point** (2026-07-29). It used to read
# `/tmp/it.out` and nothing else, so every site reporting a *different* log fell back to a raw
# `tail` with no filtering — including `classify_install`'s retry, which is the one that reports
# a confirmed defect. Measured on a real macOS run of the twin harness:
#
#     FAIL  github: install of github:sharkdp/fd failed twice — a defect, not ecosystem variance
#           |    3: __mh_execute_header
#           |    4: __mh_execute_header       (six frames, no message, nothing to act on)
#
# The cure was already written here and had reached one of its four callers.
excerpt() { # [logfile] [lines]
    _ex_log="${1:-/tmp/it.out}"; _ex_n="${2:-8}"
    _kept="$(grep -vE '^[[:space:]]*[0-9]+:|^[[:space:]]*at |^stack backtrace:|^note: [A-Z]?[a-z]* ?run with' "$_ex_log")"
    if [ -n "$_kept" ]; then
        printf '%s\n' "$_kept" | tail -"$_ex_n" | sed 's/^/        | /'
    else
        tail -"$_ex_n" "$_ex_log" | sed 's/^/        | /'
    fi
}
# ok "desc" cmd...   — passes when cmd exits 0.
ok() {
    desc="$1"; shift
    if "$@" >/tmp/it.out 2>&1; then
        PASS=$((PASS + 1)); echo "  PASS  $desc"; return 0
    else
        rc=$?; FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc)"
        echo "  FAIL  $desc (rc=$rc)"; excerpt; return 1
    fi
}

# answers "desc" cmd...  — passes when cmd gives an ANSWER: 0 (converged) or 2
# (differences). Fails on 1 (failed) and 3 (refused).
#
# U21's exit table makes "it ran" and "it found nothing to do" two different results.
# A read-only command that looked and found work exits 2 on purpose, so an assertion
# that the model *parses* must not also demand the machine be converged — in a fresh
# container it never is.
answers() {
    desc="$1"; shift
    "$@" >/tmp/it.out 2>&1; rc=$?
    if [ "$rc" = 0 ] || [ "$rc" = 2 ]; then
        PASS=$((PASS + 1)); echo "  PASS  $desc (rc=$rc)"; return 0
    else
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=$rc)"
        echo "  FAIL  $desc (rc=$rc)"; excerpt; return 1
    fi
}

# A command that could not run is not a refusal. 127 (no such command), 126 (not
# executable) and 124 (killed by `timeout`) all exit non-zero without the program ever
# reaching its own decision. The FATAL preflight below catches the image-wide case; this
# catches the per-check one, and it is what let a macOS sweep with no `timeout` report
# passes for checks that never executed.
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

# nok "desc" cmd...  — passes when cmd exits NON-zero (a refusal/negative path).
nok() {
    desc="$1"; shift
    "$@" >/tmp/it.out 2>&1; rc=$?
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

# grep_ok "desc" pattern cmd... — passes when cmd's output contains pattern.
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
#   timeout    the build ran out of clock (124). The harness's limit, not a verdict.
#   transient  failed once, succeeded on retry. The caller CONTINUES the lifecycle — skipping
#              it is how list, PATH, remove and gone-from-list went unrun for every backend
#              whose install was flaky.
#   exhausted  LiNix classed the failure passing and it did not pass in this window — a rate
#              limit with 20 minutes left on it. SOFT, and recorded as a lifecycle this run
#              could not measure, which is not the same fact as a lifecycle that got worse.
#   defect     failed permanently, or failed twice with nothing classifying it. Hard.
#
# Twinned with scripts/integration-windows.sh's, and lifted out of both by
# scripts/harness-logic-test.sh so the two cannot drift into disagreeing about a verdict.
#
# TRANSIENCE IS READ, NOT RE-DERIVED (R-3). It is a claim that a second attempt could differ,
# and LiNix already answers it — `Retryability`, from the backend's own exit policy. Until
# 2026-07-30 nothing downstream could see that answer, so this function re-derived it by
# RETRYING THE INSTALL IMMEDIATELY. That proxy is wrong for exactly the failures the
# classification gets right: a GitHub rate limit with 1236 seconds left on the window cannot
# succeed one second later, so it scored `defect`, the macOS leg went red, and the
# real-lifecycle ratchet fell 8 -> 7 and went red behind it. Two red jobs over an answer the
# program had already computed.
#
# So `linix-failure-class:` is read, and the retry is kept only where it still adds evidence:
#
#   permanent  -> a defect now. Retrying a 404 to confirm it is still a 404 costs a minute and
#                 tells nobody anything.
#   transient  -> retry ONCE, because "a second attempt could differ" is worth testing where
#                 testing it is cheap. A repeat is NOT a defect: it is exhausted, which is what
#                 `Retryability::Exhausted` means — the claim was tested and did not hold, and
#                 "this can never work" is more than was measured.
#   unknown    -> retry once and treat a repeat as a defect. Nothing classified it, so here the
#                 retry IS the evidence.
#
# A missing class line is a defect too: every failing command emits one, so its absence means
# the binary under test is not the tree that was built.
# $5 runs between the two attempts, to clear a declaration the failed attempt left behind.
classify_install() { # be  install-spec  rc  logfile  [cleanup]
    _ci_be="$1"; _ci_spec="$2"; _ci_rc="$3"; _ci_log="$4"; _ci_clear="${5:-:}"
    if [ "$_ci_rc" -eq 124 ]; then
        soft "$_ci_be: install of $_ci_spec hit the ${TO_LONG##* }s build limit — not a verdict on the backend"
        excerpt "$_ci_log" 4
        CLASS=timeout; return 0
    fi
    if [ "$_ci_rc" -eq 3 ]; then
        refused "$_ci_be: install of $_ci_spec"
        excerpt "$_ci_log" 3
        CLASS=refused; return 0
    fi
    _ci_class="$(sed -n 's/^linix-failure-class: //p' "$_ci_log" | tail -1)"
    if [ -z "$_ci_class" ]; then
        hard "$_ci_be: install of $_ci_spec failed and printed no failure class (rc=$_ci_rc)"
        excerpt "$_ci_log" 6
        CLASS=defect; return 0
    fi
    if [ "$_ci_class" = permanent ]; then
        hard "$_ci_be: install of $_ci_spec failed permanently — a defect, not ecosystem variance (rc=$_ci_rc)"
        excerpt "$_ci_log" 6
        CLASS=defect; return 0
    fi
    echo "        (first attempt failed, class=$_ci_class; retrying once)"
    $_ci_clear
    lx_slow -y install "$_ci_spec" >/tmp/life2.out 2>&1
    _ci_rc2=$?
    if [ "$_ci_rc2" -eq 0 ]; then
        soft "$_ci_be: install of $_ci_spec failed once and succeeded on retry — transient"
        CLASS=transient; return 0
    fi
    if [ "$_ci_class" = transient ]; then
        soft "$_ci_be: install of $_ci_spec is classed transient and did not clear on a retry — exhausted, not a defect (rc=$_ci_rc, $_ci_rc2)"
        excerpt /tmp/life2.out 6
        # Recorded so the ratchet can tell a lifecycle it could not MEASURE from one that got
        # worse. Without this a rate limit ratchets a platform's coverage down permanently.
        echo "$_ci_be" >> "$LEDGER/be-life-unmeasured"
        CLASS=exhausted; return 0
    fi
    hard "$_ci_be: install of $_ci_spec failed twice, unclassified — a defect, not ecosystem variance (rc=$_ci_rc, $_ci_rc2)"
    excerpt /tmp/life2.out 6
    CLASS=defect
}


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
ok "check health" lx check health
ok "check drift" lx check drift
ok "plan (no changes yet)" lx plan --dry-run
answers "check parses the model" lx check
ok "check absent lists nothing" lx check absent
ok "protected lists guarded packages" lx protected
grep_ok "protected includes a system essential" "linix\|libc\|systemd\|kernel\|bash" lx protected

# --- 3. Dry-run is preview-only -------------------------------------------
echo "[3] Dry-run safety"
ok "sync --dry-run does not error" lx --dry-run sync
ok "a dry-run install shows a plan" lx --dry-run install "$PKG"
# Asked of the machine, not of PATH: a preview that installed into a directory the host has
# not wired up is exactly as much of a defect, and `on_path` would report it as clean.
cp /tmp/it.out /tmp/it-dryrun.out 2>/dev/null || true
nok "dry-run did NOT actually install $PKG" binary_present "$BACKEND" "$PKG" /tmp/it-dryrun.out

# --- 4. The guard's ratio rule, on an UNADOPTED machine -------------------
# IV.1: this is the only state in which the check tests anything. After `adopt`
# the machine is nearly all managed, so "delete everything unmanaged" is a small
# removal and the ratio it exists to catch never fires.
echo "[4] purge-unmanaged, before adopt (the state that makes it a test)"
nok "purge-unmanaged is refused on a machine LiNix has not adopted" lx -y purge-unmanaged
# WHICH rule refused matters: a `nok` that accepts any non-zero exit accepts a panic
# and an unknown flag just as happily. Before adopt the ratio rule is the one that
# fires, and it says so by name.
grep_ok "and it is the unadopted-machine ratio that refused" \
    "adopt\|allow-mass-purge" lx -y purge-unmanaged

# --- 5. Imperative install -> list -> coherence ---------------------------
echo "[5] Install"
if [ -n "$SMOKE" ]; then
    skip_smoke "install $PKG, and the list/PATH checks that read its result"
else
    ok "install $PKG" lx -y install "$PKG"
    # `ok` leaves the install's output in /tmp/it.out and the next check overwrites it; the
    # reachability assertion reads what the install SAID, so it gets its own copy.
    cp /tmp/it.out /tmp/it-life0.out 2>/dev/null || true
    grep_ok "list shows $PKG" "$PKG" lx list
    assert_binary_reachable "$BACKEND" "$PKG" /tmp/it-life0.out
    echo "$BACKEND" >> "$LEDGER/be-life"
fi

# --- 6. Idempotency --------------------------------------------------------
echo "[6] Idempotency"
# Runs under SMOKE too: with nothing installed the model is empty, and a sync over an
# empty model must still exit 0 rather than find work that is not there.
ok "second sync is a no-op (exit 0)" lx -y sync

# --- 7. Negative path ------------------------------------------------------
echo "[7] Negative path"
nok "installing a nonexistent package fails" lx -y install "linix-no-such-pkg-zzz"
# The failure must not be left in the manifest. Every later command parses the
# model, so one unresolvable line wedges the config until someone hand-edits it.
ok "a failed install leaves the model parseable" lx check drift
# This asserts the PRODUCT withdrew the line. It used to `grep -v` the name out first and
# then assert it was absent, which tested its own `grep -v` and printed PASS on every run
# while the product did the opposite. If this goes red, LiNix stopped withdrawing an
# unresolvable name — do not put the scrub back.
#
# The name here is deliberately unqualified: nothing claims it, so it is `Unresolvable` and
# withdrawing it is the behaviour both harnesses have always agreed on. The qualified form
# (`<backend>:<typo>`, which resolves and then fails to install) is a different question and
# is asserted in the host harness.
IMPERATIVE="$LINIX_CONFIG_DIR/modules/imperative.txt"
if [ -f "$IMPERATIVE" ]; then
    nok "the unresolvable name is out of the manifest" \
        grep -q "linix-no-such-pkg-zzz" "$IMPERATIVE"
fi

# --- 8. Adopt (Part IV proof) ---------------------------------------------
echo "[8] Adopt"
ADOPTED_FILE="$LINIX_CONFIG_DIR/modules/adopted.txt"
nok "nothing is adopted before adopt runs" test -s "$ADOPTED_FILE"
ok "adopt takes manual packages" lx -y adopt
# Part IV: adopt takes the MANUAL set, not the whole dependency closure, and
# python3 (apt/dnf) survives. The count is COMPARED, not printed: `lx list` answers
# "what is installed", which adopt does not change, so reading it proves nothing —
# the adoption manifest is the only file that records what adopt decided.
if [ "$BACKEND" = "apt" ] || [ "$BACKEND" = "dnf" ] || [ "$BACKEND" = "pacman" ]; then
    if command -v python3 >/dev/null 2>&1; then
        # `on_path`, not `binary_present`: python3 is the image's own, installed by apt into
        # /usr/bin before this harness ran, so there is no install of ours to have named a
        # directory for.
        ok "python3 still installed after adopt" on_path python3
    else
        soft "python3 not on this image — cannot check the survival proof"
    fi
    # No `|| echo 0`: `grep -c` prints the count AND exits 1 when it is zero, so the
    # fallback would append a second line and every later `test -ge` would be a syntax
    # error instead of a comparison.
    ADOPTED=$(grep -vc '^[[:space:]]*#\|^[[:space:]]*$' "$ADOPTED_FILE" 2>/dev/null)
    [ -n "$ADOPTED" ] || ADOPTED=0
    MANUAL=0; INSTALLED_TOTAL=0
    case "$BACKEND" in
        apt)    MANUAL=$(apt-mark showmanual 2>/dev/null | grep -c .)
                INSTALLED_TOTAL=$(dpkg-query -W -f='.\n' 2>/dev/null | grep -c .) ;;
        dnf)    MANUAL=$(dnf repoquery --userinstalled --qf '%{name}\n' 2>/dev/null | grep -c .)
                INSTALLED_TOTAL=$(rpm -qa 2>/dev/null | grep -c .) ;;
        pacman) MANUAL=$(pacman -Qqe 2>/dev/null | grep -c .)
                INSTALLED_TOTAL=$(pacman -Qq 2>/dev/null | grep -c .) ;;
    esac
    # Compared against THIS manager's rows only. `adopted.txt` spans every backend on the
    # image — cargo crates, npm globals, gem gems — so measuring the whole file against
    # one manager's user-chosen list is comparing two different sets, and on arch it read
    # 15 adopted against pacman's 12 explicit and called that a fault.
    ADOPTED_NATIVE=$(grep -c "^$BACKEND:" "$ADOPTED_FILE" 2>/dev/null)
    [ -n "$ADOPTED_NATIVE" ] || ADOPTED_NATIVE=0
    echo "        adopted=$ADOPTED (of which $BACKEND: $ADOPTED_NATIVE)  \
$BACKEND manual set=$MANUAL  $BACKEND installed=$INSTALLED_TOTAL"
    ok "adopt wrote an adoption manifest" test -s "$ADOPTED_FILE"
    ok "adopt recorded at least one package" test "$ADOPTED" -ge 1
    if [ "$INSTALLED_TOTAL" -gt 0 ]; then
        ok "adopt took the manual set, not the whole dependency closure" \
            test "$ADOPTED_NATIVE" -lt "$INSTALLED_TOTAL"
    else
        soft "could not count installed packages on $BACKEND — closure proof skipped"
    fi
    if [ "$MANUAL" -gt 0 ]; then
        # Never MORE than the manual set: adopt may drop a name (protected, unwritable),
        # but a count above it means something not user-chosen was swept in.
        ok "adopt took no more than what $BACKEND calls user-chosen" \
            test "$ADOPTED_NATIVE" -le "$MANUAL"
    else
        soft "$BACKEND could not report its manual set — the upper-bound proof skipped"
    fi
fi

# --- 9. The guard (Part IV proofs) ----------------------------------------
echo "[9] The guard"
# A protected package is never removed. Only survival is asserted: whether the
# verb refuses or no-ops depends on whether it was declared, and an earlier
# form asserted an exit code so convoluted that a correct refusal failed it.
#
# The victim comes from LiNix's OWN protected list, intersected with what this image actually
# has installed. It was hardcoded to `bash` until 2026-07-30, and Void Linux ships no bash at
# all — so on the first Void run this check asserted the survival of a package that had never
# been there and printed `FAIL bash survives an uninstall attempt`, which reads exactly like
# the guard having deleted /bin/bash. A proof that cannot run must say so, not accuse.
_installed_names() { lx list --backend "$BACKEND" 2>/dev/null | awk '{print $2}'; }
GUARD_VICTIM=""
_have="$(_installed_names)"
for _p in $(lx protected 2>/dev/null | sed -n 's/^  \([a-z0-9][a-z0-9._+-]*\)$/\1/p'); do
    if printf '%s\n' "$_have" | grep -qx "$_p"; then GUARD_VICTIM="$_p"; break; fi
done
if [ -z "$GUARD_VICTIM" ]; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES
    - guard: no protected package is installed on this image, so the guard proof examined nothing"
    echo "  FAIL  not one of LiNix's protected packages is installed here, so nothing could be"
    echo "        protected and this check proved nothing. Add one to the image, or widen"
    echo "        default_protected_packages so it names something this distro ships."
else
    echo "        guard victim: $GUARD_VICTIM (from \`linix protected\`, and installed here)"
    lx -y uninstall "$GUARD_VICTIM" >/dev/null 2>&1 || true
    ok "$GUARD_VICTIM survives an uninstall attempt" \
        sh -c "$LINIX list --backend '$BACKEND' 2>/dev/null | awk '{print \$2}' | grep -qx '$GUARD_VICTIM'"
fi
# Post-adopt the ratio no longer fires, so the bare command must still not be a
# silent mass-delete — the refusal is asserted in both states, for different reasons.
nok "purge-unmanaged is still not a silent mass-delete after adopt" lx -y purge-unmanaged
# WHICH rule refuses is still asserted, but the answer depends on how much `adopt`
# could take on this image: where it adopted well the protected set decides, where it
# adopted little the ratio still does. Both are named answers; "some error" is not, and
# that is what a bare `nok` would accept.
grep_ok "and the refusal after adopt still names its rule" \
    "protected\|essential\|allow-mass-removal\|allow-mass-purge" lx -y purge-unmanaged

# --- 10. Remove -------------------------------------------------------------
echo "[10] Remove"
if [ -n "$SMOKE" ]; then
    skip_smoke "uninstall $PKG (nothing was installed to remove)"
else
    ok "uninstall $PKG" lx -y uninstall "$PKG"
    nok "$PKG binary gone after uninstall" binary_present "$BACKEND" "$PKG" /tmp/it-life0.out
fi

# --- 11. Git-backed history (Phase 4 / v7) --------------------------------
echo "[11] Git history + rollback"
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
ok "git status reads the repo" lx git status
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
ok "git commit records the current state on demand" lx git commit -m "linix: harness checkpoint"
ok "diff against a commit runs" lx diff HEAD
ok "rollback to HEAD is accepted" lx -y rollback HEAD
fi
fi

# --- 12. rebuild asserts, and writes no commit (K14) ----------------------
echo "[12] rebuild"
# Git is asked directly, not `linix git log`: a rebuild that committed by some
# other route would still move HEAD, and only git can say so.
commits() { git -C "$LINIX_CONFIG_DIR" rev-list --count HEAD 2>/dev/null || echo 0; }
# K2 (ruled 2026-07-24): a bare `rebuild` no longer REFUSES — it WARNS loudly and rebuilds
# `--all`. Checked with `--dry-run` so the harness does not actually churn every manual package
# on the image to prove a claim about the default scope. The warning is the safeguard the old
# refusal used to be, and it must be loud and it must not error.
ok "bare rebuild is accepted, not refused (K2)" lx --dry-run rebuild
grep_ok "bare rebuild warns it will rebuild EVERY declared package (K2)" \
    "EVERY declared package" lx --dry-run rebuild
if [ -n "$SMOKE" ]; then
    skip_smoke "the rebuild itself, and K14's no-commit proof (needs an installed package)"
else
BEFORE_COMMITS=$(commits)
# "unchanged" proves nothing if there was no history to change, and nothing if
# the rebuild never ran: both read 0 == 0. Require a commit to exist first.
ok "there is history for a rebuild to leave alone" test "$BEFORE_COMMITS" -ge 1
# Scoped to $PKG, not --all: the machine was adopted in section 8, so `--all`
# would churn every manual package on the image to prove a claim about one.
ok "rebuild $PKG runs" lx_slow -y rebuild "$PKG"
cp /tmp/it.out /tmp/it-rebuild.out 2>/dev/null || true
ok "$PKG is reinstalled, not left removed" binary_present "$BACKEND" "$PKG" /tmp/it-rebuild.out
AFTER_COMMITS=$(commits)
echo "        commits before=$BEFORE_COMMITS after=$AFTER_COMMITS"
ok "rebuild wrote no git commit (K14)" test "$BEFORE_COMMITS" = "$AFTER_COMMITS"
fi

# --- 13. Backend chains, the per-host lock, and unlock (II.7b) ------------
echo "[13] Chains and the per-host lock"
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

# --- 13b. A manager that could not answer is not one that said no (V.7c) --
echo "[13b] Silence is not a no"
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

# ==========================================================================
# 13b. REAL DEVICES for the storage effectors (btrfs, lvm, zfs)
# ==========================================================================
# These three operate on block devices, and until 2026-07-30 none of them had ever been run.
# The harness called btrfs "a snapshot provider, not an install target" — which is not what the
# code does. `btrfs:PATH` runs `btrfs subvolume create`, `lvm:VG/LV` runs `lvcreate`, and both
# have an ordinary install → list → remove cycle that needs nothing but a real device.
#
# Inert unless the image asked for it (`LINIX_IT_STORAGE`), because it needs `--privileged` and
# nothing else here is. The devices are loopback files made in the container and destroyed with
# it. Owner-authorised 2026-07-30 (Q17).
#
# Every step is checked: a failure leaves the backend without a canary and PRINTS why, rather
# than leaving a half-made volume for the lifecycle to trip over in a way that reads like a
# LiNix defect.
STORAGE_BTRFS=""
STORAGE_LVM=""
STORAGE_ZFS=""

setup_storage_devices() {
    [ -n "${LINIX_IT_STORAGE:-}" ] || return 0
    [ -z "$SMOKE" ] || { skip_smoke "loopback devices for the storage effectors"; return 0; }
    echo "[13b] Loopback devices for the storage effectors"

    if ! command -v mkfs.btrfs >/dev/null 2>&1; then
        soft "btrfs: no mkfs.btrfs in this image, so there is no filesystem to make a subvolume in"
    elif ! modprobe btrfs >/dev/null 2>&1 && ! grep -qw btrfs /proc/filesystems; then
        soft "btrfs: this kernel has no btrfs — a container borrows the HOST's kernel, so that is a fact about the machine and not about LiNix"
    else
        rm -f /var/tmp/linix-btrfs.img
        truncate -s 512M /var/tmp/linix-btrfs.img
        mkdir -p /mnt/linix-btrfs
        if mkfs.btrfs -q -f /var/tmp/linix-btrfs.img >/dev/null 2>&1 \
           && mount -o loop /var/tmp/linix-btrfs.img /mnt/linix-btrfs >/dev/null 2>&1; then
            STORAGE_BTRFS=/mnt/linix-btrfs
            PASS=$((PASS + 1)); echo "  PASS  btrfs: a real filesystem is mounted at $STORAGE_BTRFS"
        else
            soft "btrfs: mkfs or mount failed in this container, so the lifecycle has nowhere to run"
        fi
    fi

    if ! command -v lvcreate >/dev/null 2>&1; then
        soft "lvm: no lvcreate in this image"
    elif ! modprobe dm_mod >/dev/null 2>&1 && [ ! -e /dev/mapper/control ]; then
        soft "lvm: this kernel has no device-mapper — again a fact about the machine"
    else
        rm -f /var/tmp/linix-lvm.img
        truncate -s 512M /var/tmp/linix-lvm.img
        _loop="$(losetup -f --show /var/tmp/linix-lvm.img 2>/dev/null)"
        if [ -n "$_loop" ] \
           && pvcreate -f -y "$_loop" >/dev/null 2>&1 \
           && vgcreate linixvg "$_loop" >/dev/null 2>&1; then
            STORAGE_LVM=linixvg
            PASS=$((PASS + 1)); echo "  PASS  lvm: volume group $STORAGE_LVM exists on $_loop"
        else
            soft "lvm: could not build a volume group on a loopback device here"
        fi
    fi

    # ZFS is out of tree, so whether it is available is a property of the kernel the container
    # borrowed. On the WSL2 kernel this project is developed against, `modprobe -n zfs` says no.
    # That must read as "this machine cannot", never as "this backend is excused" (Q17).
    if ! command -v zpool >/dev/null 2>&1; then
        soft "zfs: no zpool in this image"
    elif ! modprobe zfs >/dev/null 2>&1; then
        soft "zfs: this kernel has no ZFS module — it is out-of-tree and the WSL2 kernel ships without it. This is the release blocker Q4 counts, not an exemption."
    else
        rm -f /var/tmp/linix-zfs.img
        truncate -s 512M /var/tmp/linix-zfs.img
        if zpool create -f linixpool /var/tmp/linix-zfs.img >/dev/null 2>&1; then
            STORAGE_ZFS=linixpool
            PASS=$((PASS + 1)); echo "  PASS  zfs: pool $STORAGE_ZFS is imported"
        else
            soft "zfs: the module is loaded and zpool create still failed"
        fi
    fi
}

teardown_storage_devices() {
    [ -n "$STORAGE_ZFS" ] && zpool destroy "$STORAGE_ZFS" >/dev/null 2>&1
    [ -n "$STORAGE_LVM" ] && vgremove -f "$STORAGE_LVM" >/dev/null 2>&1
    [ -n "$STORAGE_BTRFS" ] && umount "$STORAGE_BTRFS" >/dev/null 2>&1
    losetup -D >/dev/null 2>&1
    rm -f /var/tmp/linix-btrfs.img /var/tmp/linix-lvm.img /var/tmp/linix-zfs.img
    return 0
}

# ==========================================================================
# 14. REAL lifecycle for every other manager this image ships
# ==========================================================================
# The `tools` image installs fifteen ecosystem managers and its header promises
# each of them a real install → list → remove. Until this section existed the
# promise was prose: `run.sh` mapped tools→apt, so the image was `ubuntu` with a
# forty-minute build and every expansion backend was proven only against mocked
# output — which is the one thing that never drifts, while drift is where every
# real bug in Part VII came from.
#
# Install failure is SOFT (a registry outage is not a LiNix bug); everything
# after a successful install is HARD. That split is what caught the pixi
# `global remove` vs `global uninstall` bug a dry-run plan could never see.
setup_storage_devices
echo "[14] Real lifecycle, every other manager on this image"

# canary <backend> → "package|binary|remove-mode|list-token|install-options"
#   binary      empty when the package ships no executable — the PATH check is
#               then skipped rather than faked.
#   remove-mode full        uninstall must succeed and the name must be gone
#               unsupported the manager has no uninstall verb; the contract is a
#                           refusal that SAYS so, and that is what is asserted
#   list-token  what `list` calls it, when that differs from what install takes
#               (`go:golang.org/x/example/hello` is listed as `hello`); empty
#               means the two are the same.
#   install-options `@k=v` appended at INSTALL only. helm installs a plugin from a
#               URL and removes it by name, so the two verbs cannot be handed the
#               same string — which is exactly what this section exists to catch.
# The ceiling for the block below, and it may only go DOWN. Raising it is Q4's item 4 happening.
#
# **Measured, not guessed** — 12, read off the openSUSE run of 2026-07-30, the first run of this
# harness after `primary_manager_image` stopped it counting a distro's own manager as uncovered.
# The twelve: brew emerge eopkg guix lvm paru pkg pkg_add pkgin slackpkg yay zfs. Three of those
# have images being built for them; the BSDs need a userland no Linux container can host, and
# `emerge` is smoke-only by design.
LIFECYCLE_GAP_CEILING=12
canary() {
    case "$1" in
        npm)      echo "cowsay|cowsay|full|" ;;
        pnpm)     echo "cowsay|cowsay|full|" ;;
        yarn)     echo "cowsay|cowsay|full|" ;;
        bun)      echo "cowsay|cowsay|full|" ;;
        pipx)     echo "pycowsay|pycowsay|full|" ;;
        uv)       echo "pycowsay|pycowsay|full|" ;;
        pip)      echo "six||full|" ;;
        gem)      echo "colorize||full|" ;;
        cargo)    echo "hexyl|hexyl|full|" ;;
        go)       echo "golang.org/x/example/hello|hello|full|hello" ;;
        composer) echo "psr/log||full|log" ;;
        opam)     echo "ocamlfind|ocamlfind|full|" ;;
        luarocks) echo "luafilesystem||full|" ;;
        nimble)   echo "nimjson|nimjson|full|" ;;
        # `hello` and not a real tool: cabal builds from source, and the smallest
        # Haskell executable on Hackage is the difference between a four-minute
        # check and a forty-minute one.
        cabal)    echo "hello|hello|unsupported|" ;;
        # `hex` was never installable: measured 2026-07-29, `mix archive.install hex hex`
        # answers `No package with name hex (from: mix.exs) in registry` even once Hex is
        # there, so the check could not pass and its failure looked like the Hex defect.
        # phx_new is pinned because this image's Elixir is 1.14 and the current release
        # declares `~> 1.17` — the archive fetches, builds, and then refuses to run.
        mix)      echo "phx_new@version=1.6.16||full|phx_new" ;;
        # A helm plugin has no binary on PATH — it is reached as `helm diff` — so the
        # PATH check is skipped rather than faked. U39: the name is the identity, the
        # URL is install-time data.
        helm)     echo "secrets||full||@url=https://github.com/jkroepke/helm-secrets,unverified" ;;
        krew)     echo "ns|kubectl-ns|full|" ;;
        pixi)     echo "ripgrep|rg|full|" ;;
        spack)    echo "zlib||full|" ;;
        conda)    echo "six||full|" ;;
        # `nix:hello` and not the flake ref `nixpkgs#hello`: `#` opens a comment in the
        # one grammar, so a flake ref cannot be written in a manifest at all. The backend
        # builds `nixpkgs#<name>` itself from a plain name.
        nix)      echo "hello|hello|full|" ;;
        dotnet)   echo "dotnetsay|dotnetsay|full|" ;;
        pub)      echo "sass|sass|full|" ;;
        # mise appends the version itself; `jq@latest` here would be read as an option.
        #
        # No PATH check, for two independent reasons — either alone would make one vacuous
        # (IV.1: grep for something only the right answer contains). `jq` is also this image's
        # apt canary, so `command -v jq` answers about apt's copy whatever mise did; and a
        # mise tool only reaches PATH through `mise activate`, a shell integration this image
        # does not set up. The backend-scoped `list --backend mise` check below is the real
        # presence assertion, and it is the one that caught the `info` bug on 2026-07-24.
        mise)     echo "jq||full|" ;;
        # jq and not nodejs: both need `asdf plugin add` first, and jq's plugin downloads a
        # single binary in seconds where nodejs's fetches a release tarball. Measured end to
        # end in the tools image on 2026-07-29.
        asdf)     echo "jq||full|" ;;
        github)   echo "sharkdp/fd|fd|full|fd" ;;
        emacs)    echo "hydra||full|" ;;
        flatpak)  echo "org.freedesktop.Platform||full|" ;;
        snap)     echo "hello||full|" ;;
        vscode)   echo "ms-python.python||full|" ;;
        # The storage effectors. Each canary exists only when 13b built it a real device, so
        # this table never claims a lifecycle the machine could not give — and `btrfs` is an
        # install target, whatever the old exemption said: `btrfs:PATH` is `subvolume create`.
        #
        # The list-token is what `list` calls it, and for btrfs that is NOT the install path:
        # `btrfs subvolume list` reports a path relative to the filesystem root, so a subvolume
        # installed as /mnt/linix-btrfs/canary is listed as /canary.
        btrfs)    [ -n "$STORAGE_BTRFS" ] && echo "$STORAGE_BTRFS/canary||full|/canary" ;;
        # `@size=` is not optional: `lvm:` refuses without one, by name, and the option rides
        # in the install-only field because `lvremove` takes the volume and not the size.
        lvm)      [ -n "$STORAGE_LVM" ] && echo "$STORAGE_LVM/canary||full||@size=64M" ;;
        zfs)      [ -n "$STORAGE_ZFS" ] && echo "$STORAGE_ZFS/canary||full|" ;;
        appimage) echo "" ;;   # a URL, not a name — smoked in 15, not lifecycled
        web)      echo "" ;;
        *)        echo "" ;;
    esac
}

# The manager an image exists to test. Section 5 gives it a full install → list → binary →
# remove on that image, so it needs no `canary()` row — but the gap audit at the end of this
# script could not see that, and counted every distro manager as having no path to a lifecycle
# including the one this very run was about to lifecycle.
#
# Named per backend rather than read from `$BACKEND`, because the question the audit asks is
# "does a real lifecycle for this backend exist ANYWHERE", and on the ubuntu image the answer
# for `dnf` is yes, on the fedora image. A run cannot see the other images; this table can.
#
# `emerge` is deliberately absent. Gentoo is always SMOKE_ONLY — a source-building
# install→remove costs hours — so its image installs nothing and crediting it here would turn
# the release blocker into a caption, which is the whole of what Q4 forbids.
primary_manager_image() {
    case "$1" in
        apt)    echo "ubuntu, tools" ;;
        dnf)    echo "fedora" ;;
        pacman) echo "arch" ;;
        apk)    echo "alpine" ;;
        zypper) echo "opensuse" ;;
        xbps)   echo "void" ;;
        *)      echo "" ;;
    esac
}

# Backends that are READY but cannot run a real lifecycle in a plain container.
# Each is NAMED with the reason: an unexplained skip is the vacuous check again.
no_lifecycle_reason() {
    case "$1" in
        link)     echo "a dependent statement (link:SRC), not a package name — smoked in 15" ;;
        service)  echo "a dependent statement (service:NAME), not a package name — smoked in 15" ;;
        setting)  echo "a dependent statement (setting:K @value=), not a package name — smoked in 15" ;;
        # btrfs, lvm and zfs are install targets — `btrfs:PATH` runs `subvolume create`,
        # `lvm:VG/LV` runs `lvcreate`. The old text here said btrfs was "a snapshot provider, not
        # an install target", which is not what the code does, and that sentence is why the
        # three most destructive backends in the program had never been run (Q17).
        #
        # What they need is a real device, so the reason is whatever 13b could not build — and
        # it is DETECTED there and printed there. Silence here means either a canary exists or
        # 13b already said why not, and an unexplained skip is impossible in both directions.
        btrfs|lvm|zfs)
            [ -n "${LINIX_IT_STORAGE:-}" ] \
                || echo "needs a real block device, which only the \`storage\` image (--privileged) provides — plan-smoked here" ;;
        web)      echo "installs from a pasted URL; no stable public canary — smoked in 15" ;;
        appimage) echo "needs FUSE, which a plain container does not have — smoked in 15" ;;
        stack)    echo "its first install downloads a whole GHC toolchain (~2 GB) — smoked in 15" ;;
        flatpak)  echo "the smallest app pulls a multi-GB runtime, and there is no session bus here" ;;
        # Detected, not assumed: on a distro without the marker a system pip install is
        # ordinary and gets the full lifecycle. Naming it keeps a permanent, expected
        # refusal from reading as ecosystem flakiness run after run.
        pip)      ls /usr/lib/python3*/EXTERNALLY-MANAGED >/dev/null 2>&1 \
                      && echo "this distro marks its Python EXTERNALLY-MANAGED (PEP 668), so a system pip install is refused by design" ;;
        *)        echo "" ;;
    esac
}

# A manager whose own uninstall deletes the package and keeps its launcher. Reported,
# never assumed: the strict check runs first, and this only softens the result when the
# leftover actually happens — so a manager that cleans up properly still has to.
removal_leaves_binary() {
    case "$1" in
        bun) echo "bun's own \`remove -g\` drops the package and keeps its launcher (reproduced against bun directly, with no LiNix involved)" ;;
        *)   echo "" ;;
    esac
}

# assert_binary_gone <backend> <binary> <what-the-name-resolved-to-before-the-install>
#
# The question is "did this backend's install get undone", NOT "does this name resolve".
# Two managers can ship a binary of the same name, and one of them may hold it on
# purpose: cabal's canary is `hello`, cabal has no uninstall verb (remove-mode
# `unsupported`), so its ~/.cabal/bin/hello stays for the rest of the run — and go's
# canary binary is also `hello`. Asking PATH handed cabal's leftover to go as a failure,
# on a removal `list` had just confirmed worked.
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

# A manager whose `list` answers a different question than its `install`. Named, because
# "the install worked and `list` does not show it" is otherwise indistinguishable from a
# parser that is broken — which is the one thing this section exists to catch.
list_cannot_show() {
    case "$1" in
        cabal) echo "\`cabal list --installed\` reports the GHC package DB (libraries); \`cabal install hello\` builds an EXECUTABLE into ~/.cabal/bin, which that DB never mentions" ;;
        *)     echo "" ;;
    esac
}

# Take a canary's line back out of the manifest.
#
# Every install syncs the WHOLE model, so a line left behind is retried by every backend
# after this one — and they then fail with the FIRST one's error. That happens for two
# reasons and both are by design: a pinned name a manager could not install stays (V.7c),
# and a manager with no uninstall verb cannot take its own line out. So the harness cleans
# up after itself rather than letting one canary decide the next nine results.
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

# lifecycle <backend> — the real install → list → PATH → remove → gone cycle.
lifecycle() {
    be="$1"
    spec="$(canary "$be")"
    cpkg="$(echo "$spec" | cut -d'|' -f1)"
    cbin="$(echo "$spec" | cut -d'|' -f2)"
    cmode="$(echo "$spec" | cut -d'|' -f3)"
    ctok="$(echo "$spec" | cut -d'|' -f4)"
    copts="$(echo "$spec" | cut -d'|' -f5)"
    [ -n "$ctok" ] || ctok="$cpkg"

    echo "    -- $be:$cpkg"
    # V.15: an explicit `be:name` is refused unless `be` is listed. init writes the
    # READY set, but a manager that came up after init would not be there.
    grep -qx "$be" "$LINIX_CONFIG_DIR/priority" 2>/dev/null || echo "$be" >> "$LINIX_CONFIG_DIR/priority"

    # Read before the install, because the removal check below is a comparison against
    # it: a name another manager already owns must not be scored as this one's leftover.
    _prepath="$(path_of "$cbin")"
    [ -n "$_prepath" ] && soft "$be: $cbin already resolves to $_prepath — the removal check compares against that, not against absence"

    lx_slow -y install "$be:$cpkg$copts" >/tmp/life.out 2>&1
    lrc=$?
    if [ "$lrc" -ne 0 ]; then
        _canary_clear() { undeclare_canary "$be:$cpkg"; }
        classify_install "$be" "$be:$cpkg$copts" "$lrc" /tmp/life.out _canary_clear
        case "$CLASS" in
            transient) : ;;   # the retry succeeded; the lifecycle below is answerable
            defect)    echo "$be" >> "$LEDGER/be-life-partial"; undeclare_canary "$be:$cpkg"; return 1 ;;
            *)         echo "$be" >> "$LEDGER/be-life-partial"; undeclare_canary "$be:$cpkg"; return 0 ;;
        esac
    fi
    PASS=$((PASS + 1)); echo "  PASS  $be installed $cpkg for real"
    echo "$be" >> "$LEDGER/be-life"

    # Everything below is HARD: the install worked, so the manager answered, and a
    # parser or argv fault from here on is a LiNix bug and nothing else.
    _nolist="$(list_cannot_show "$be")"
    if [ -n "$_nolist" ]; then
        soft "$be: list does not show $ctok — $_nolist"
    else
        grep_ok "$be: list shows $ctok" "$ctok" lx list --backend "$be"
    fi
    [ -n "$cbin" ] && assert_binary_reachable "$be" "$cbin" /tmp/life.out

    if [ "$cmode" = "unsupported" ]; then
        # A manager with no uninstall verb must say so. Reporting success would
        # leave the package installed and the model claiming it is gone.
        grep_ok "$be: removal reports a graceful unsupported" \
            "not support\|unsupport\|cannot remove\|no remove" \
            lx -y uninstall "$be:$cpkg"
        # That refusal is correct AND it leaves the line, so take it out by hand.
        undeclare_canary "$be:$cpkg"
        return 0
    fi

    ok "$be: uninstall $cpkg" lx_slow -y uninstall "$be:$cpkg"
    [ -n "$_nolist" ] || nok "$be: $ctok is gone from list" sh -c \
        "$LINIX list --backend '$be' 2>/dev/null | grep -q '$ctok'"
    [ -n "$cbin" ] && assert_binary_gone "$be" "$cbin" "$_prepath" /tmp/life.out
    # A successful uninstall already removed the line; this covers the run where it
    # reported success and did not, which is the whole point of asserting the rest.
    undeclare_canary "$be:$cpkg"
    return 0
}

if [ -n "$SMOKE" ]; then
    skip_smoke "every other manager's real lifecycle (this image installs nothing)"
else
    for be in $READY_LIST; do
        [ "$be" = "$BACKEND" ] && continue          # section 5 already did this one
        reason="$(no_lifecycle_reason "$be")"
        if [ -n "$reason" ]; then
            soft "$be: no real lifecycle here — $reason"
            continue
        fi
        if [ -z "$(canary "$be")" ]; then
            # It still gets a plan-smoke below, so the audit passes — which is the point
            # of saying this out loud: the image could have run it for real and did not.
            soft "$be: READY here and this harness has no canary — it falls through to the plan-smoke, which is weaker than this image could give"
            continue
        fi
        lifecycle "$be"
    done
fi

# ==========================================================================
# 15. PLAN-SMOKE — every backend this image cannot run for real
# ==========================================================================
# A manager that is not installed here still has argv, a parser and a planner
# wiring that can break. A dry-run install proves that path without a machine
# that has the manager. V.15 refuses an unlisted backend, so the smoke config
# lists every one.
echo "[15] Plan-smoke, every backend this image cannot run"

ALL_BACKENDS=$(lx check health --json 2>/dev/null \
    | sed -n 's/.*"backend"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | sort -u)
echo "        registered backends: $(echo $ALL_BACKENDS | wc -w)"
ok "check health --json enumerates the registry" test -n "$ALL_BACKENDS"

SMOKE_CFG=/tmp/linix-it-smoke
rm -rf "$SMOKE_CFG"; mkdir -p "$SMOKE_CFG/modules" "$SMOKE_CFG/profiles"
printf 'Work\n' > "$SMOKE_CFG/active"
printf 'use base\n' > "$SMOKE_CFG/profiles/Work"
: > "$SMOKE_CFG/modules/base.txt"
: > "$SMOKE_CFG/priority"
for b in $ALL_BACKENDS; do echo "$b" >> "$SMOKE_CFG/priority"; done

smoke_lx() {
    record_argv "$@"
    env LINIX_CONFIG_DIR="$SMOKE_CFG" LINIX_DATA_DIR=/tmp/linix-it-smoke-state \
        $TO "$LINIX" "$@"
}

# smoke_pkg <backend> — a name whose grammar that backend accepts.
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
    # Already proven by a real lifecycle above — a dry run would add nothing.
    grep -qx "$be" "$LEDGER/be-life" 2>/dev/null && continue
    case "$be" in
        service)
            # A dependent statement: it is declared in a module and applied by
            # sync, and `install` correctly answers "not a package".
            printf 'service:cron\n' > "$SMOKE_CFG/modules/base.txt"
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
            # Not an install target: it provides snapshots. `snapshot list` is the
            # verb that reaches it, and doctor is what reports it at all.
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
# 16. The command surface, RUN — not just `--help`
# ==========================================================================
# 24 of the previous run's 82 checks were `<cmd> --help`, which proves clap is
# wired and nothing else. Every command below is actually executed; the ones that
# cannot be are exempted BY NAME with a reason, in EXEMPT_CMDS.
echo "[16] Command surface, executed"

ok "vars resolves this machine's variables" lx vars
# `eval` is the one output that will acquire consumers LiNix cannot see, so the
# thing asserted is the contract: a top-level schema version, and valid JSON.
grep_ok "eval prints a versioned document" '"schema"' lx eval
ok "eval emits valid JSON" sh -c "$LINIX eval | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null || $LINIX eval | head -1 | grep -q '{'"
# `repl` (U34) is the read side of the resolver — a REPL that reads stdin until EOF. A piped
# session drives the loop (`:help`, `:vars`) and exits on EOF, proving it runs headless; it goes
# through `lx` so the coverage check below counts it as really executed, not merely `--help`'d.
if printf ':help\n:vars\n:quit\n' | lx repl >/tmp/it.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  repl evaluates a piped session and exits on EOF (U34)"
else
    FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - repl piped session failed"
    echo "  FAIL  repl piped session"; excerpt /tmp/it.out 4
fi
# A container has no container runtime, which is exactly `try`'s refusal path —
# and the one a developer's own machine (which has docker) can never exercise.
nok "try refuses when there is no container runtime" lx try
grep_ok "try's refusal names what is missing" "podman" lx try
ok "check unmanaged lists what LiNix does not manage" lx check unmanaged
ok "path prints the config repo" lx path
ok "path --explain says which source won" lx path --explain
ok "config show prints the active configuration" lx config show
ok "policy checks the desired state against [guard]" lx policy
ok "check conflicts reports cross-backend conflicts" lx check conflicts
# With no event hooks declared, approvals is clean and exits 0 (not 2).
ok "check approvals is clean with no hooks" lx check approvals
# `add` vendors a source's modules. A local path with a module is the network-free case; it
# copies the module in and reports it.
mkdir -p /tmp/linix-share/modules
printf 'apt:jq\n' > /tmp/linix-share/modules/shared.txt
ok "add vendors a module from a local source" lx add /tmp/linix-share
ok "add brought the module file in" test -f "$LINIX_CONFIG_DIR/modules/shared.txt"
nok "add refuses a source that does not exist" lx add /no/such/source/here
ok "sbom emits a bill of materials" lx sbom
ok "completions bash generates a script" lx completions bash
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
# Not every manager can enumerate its repositories (apt has no listing command LiNix
# drives). Either it lists, or it says the backend cannot — an unexplained non-zero is
# still a failure.
if lx repo list >/tmp/it.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  repo list enumerates repositories"
else
    grep_ok "repo list says which backends cannot enumerate" \
        "not supported\|does not support" cat /tmp/it.out
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
ok "unmanage forgets a package without uninstalling it" lx unmanage "$PKG"
if [ -n "$SMOKE" ]; then
    skip_smoke "the proof that unmanage left the package behind (nothing was installed)"
else
    ok "$PKG is still installed after unmanage" binary_present "$BACKEND" "$PKG" /tmp/it-life0.out
fi
# The command runs either way; only reaching OSV.dev is optional, so a network
# failure is soft — and `ok` is not used, because it would count the failure too.
if lx check security >/tmp/it.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  check security scans for vulnerabilities"
else
    soft "check security ran but could not reach the OSV.dev database"
fi
ok "export writes native manifests" lx export --out /tmp/linix-it-export
# The package is PINNED to this image's native manager. An unpinned `jq` resolved to
# `cargo:jq` on an image that had cargo and no jq — a library crate that installs no
# program — so the check failed on the resolver's answer, not on `run`.
if [ -n "$SMOKE" ]; then
    skip_smoke "run's ephemeral environment (it would install the package it needs)"
else
    ok "run executes inside an ephemeral environment" lx run -p "$BACKEND:$PKG" true
fi

# plan → apply: the frozen plan is the one that gets applied.
ok "plan freezes a reviewable file" lx plan --out /tmp/linix-it-plan.json
ok "the plan file exists" test -f /tmp/linix-it-plan.json
ok "apply reads a saved plan" lx --dry-run apply /tmp/linix-it-plan.json

# `edit` shells out to $VISUAL/$EDITOR; `true` is an editor that exits 0.
record_argv edit priority
ok "edit opens a file in \$EDITOR" env EDITOR=true VISUAL=true $TO "$LINIX" edit priority

# reset deletes the registry. The command is exercised through the refusal it owes
# a machine that still has a config repo — running it for real would end the run.
nok "reset refuses while a config repo still exists" lx reset
grep_ok "and says --force is what overrides it" "force" lx reset

# self-upgrade --check only reports; it rebuilds nothing.
ok "self-upgrade --check reports the version and source" lx self-upgrade --check

# --- 16b. bundle → restore, the round trip (V.59) -------------------------
echo "[16b] bundle → restore"
rm -rf /tmp/linix-it-bundle /tmp/linix-it-restored
ok "bundle packs the config" lx bundle --out /tmp/linix-it-bundle
ok "the bundle directory exists" test -d /tmp/linix-it-bundle
mkdir -p /tmp/linix-it-restored
ok "restore into a clean config directory" \
    env LINIX_CONFIG_DIR=/tmp/linix-it-restored LINIX_DATA_DIR=/tmp/linix-it-restored-state \
        $TO "$LINIX" restore /tmp/linix-it-bundle
record_argv restore /tmp/linix-it-bundle
answers "the restored model parses" \
    env LINIX_CONFIG_DIR=/tmp/linix-it-restored LINIX_DATA_DIR=/tmp/linix-it-restored-state \
        $TO "$LINIX" check
nok "restore refuses a config directory that is not empty" \
    env LINIX_CONFIG_DIR=/tmp/linix-it-restored LINIX_DATA_DIR=/tmp/linix-it-restored-state \
        $TO "$LINIX" restore /tmp/linix-it-bundle
ok "and --force overrides it" \
    env LINIX_CONFIG_DIR=/tmp/linix-it-restored LINIX_DATA_DIR=/tmp/linix-it-restored-state \
        $TO "$LINIX" restore /tmp/linix-it-bundle --force

# --- 16c. `--help` for the whole surface ----------------------------------
# Kept, but demoted: it catches a subcommand whose clap wiring is broken, and the
# audit below does not accept it as coverage.
echo "[16c] --help across the surface"
HELP_CMDS=$($LINIX --help 2>&1 | sed -n '/^Commands:/,/^Options:/p' \
    | sed -n 's/^  \([a-z][a-z-]*\) .*/\1/p' | grep -v '^help$' | sort -u)
for c in $HELP_CMDS; do
    ok "\`$c --help\` exists" lx "$c" --help
done

# ==========================================================================
# 17. COVERAGE AUDIT — what did nothing touch? (IV.1)
# ==========================================================================
# The only check here that can notice what is MISSING from the list above it. A
# backend or a command added next year fails this until it is covered.
echo "[17] Coverage audit"

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

# --- the release blocker, counted (Q4) -----------------------------------
# `Q4` (owner, 2026-07-27) REJECTED labelling untested backends "experimental", and the reason
# is the rule: *this codebase does things; it does not cover for not doing them.* A label turns
# an unfinished job into a permanent disclaimer. So a backend with no real lifecycle in an
# automated gate is a **release blocker**, and its item 4 is *no new backend is added until the
# current set passes*.
#
# That ruling says the coverage is tracked in `plan.md`, and it was not — nothing in the repo
# could answer "which registered backends have no path to a real lifecycle at all". The
# per-run audit above cannot: it asks *lifecycle OR plan-smoke*, and a plan-smoke satisfies it.
# The `soft` in section 12 cannot either: it only looks at backends READY on THIS host, so a
# backend that is ready nowhere is never examined anywhere.
#
# Computed here instead, from the two tables that already exist: a backend has a path to a real
# lifecycle if `canary` gives it one, and an accounted-for reason not to if
# `no_lifecycle_reason` names one. In NEITHER table is the gap, and it is named rather than
# counted silently.
#
# A CEILING, ratcheted the same way the mutation budget is: it may only go down. Failing on
# today's number would paint every run red from the first one, which is how a gate becomes
# something people switch off; failing when it RISES is exactly Q4's item 4, enforceable now.
NO_PATH=""
for be in $ALL_BACKENDS; do
    [ -n "$(canary "$be")" ] && continue
    [ -n "$(no_lifecycle_reason "$be")" ] && continue
    # A distro's own manager is lifecycled by section 5 of the image built for it, which is a
    # real lifecycle and not a plan-smoke — but it happens on a DIFFERENT run of this same
    # script, so nothing in this process can observe it. Named in one table instead.
    [ -n "$(primary_manager_image "$be")" ] && continue
    NO_PATH="$NO_PATH $be"
done
NO_PATH_N=$(echo $NO_PATH | wc -w)

# `primary_manager_image` is a CLAIM about runs this process cannot see, and a claim nothing
# checks is how a coverage table starts lying. Each row names an image; on that image, this run
# is the one that can check it — so it does, and across the matrix every row is verified exactly
# once. Without this the table would excuse `zypper` from the gap on the strength of an image
# that might never have been built.
if [ -z "$SMOKE" ] && [ -n "${LINIX_IT_IMAGE:-}" ]; then
    for be in $ALL_BACKENDS; do
        case ",$(primary_manager_image "$be" | tr -d ' ')," in
            *",$LINIX_IT_IMAGE,"*) ;;
            *) continue ;;
        esac
        if grep -qx "$be" "$LEDGER/be-life" "$LEDGER/be-life-partial" \
                "$LEDGER/be-life-unmeasured" 2>/dev/null; then
            PASS=$((PASS + 1))
            echo "  PASS  $be: the image that claims its lifecycle is this one, and it ran"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES
    - coverage: primary_manager_image says $be is lifecycled on the $LINIX_IT_IMAGE image, and this run of it never touched $be"
            echo "  FAIL  $be is excused from the lifecycle gap because this image lifecycles it,"
            echo "        and this image did not. Either section 5 skipped it or the table is wrong."
        fi
    done
fi
# An audit over an empty set passes without examining anything (G2), and this one passed
# LOUDLY: under the do-nothing stub `ALL_BACKENDS` is empty, so nothing is in neither table,
# so the count is 0 and the `else` below congratulated the registry that came back blank. The
# mutation gate caught it on the first run after this check was written — 87 survivors against
# a budget of 86 — which is the gate doing to me exactly what it is for.
if too_few_to_audit 10 "$(echo $ALL_BACKENDS | wc -w)"; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES
    - coverage: the registry came back empty, so the lifecycle-gap ceiling examined nothing"
    echo "  FAIL  the lifecycle-gap ceiling cannot judge a registry that enumerated nothing"
elif [ -z "${LIFECYCLE_GAP_CEILING:-}" ]; then
    # Unrecorded, and reported as such rather than compared against a number nobody measured —
    # the same branch the real-lifecycle ratchet takes for a host class it has never seen. The
    # registry is platform-conditional (48 backends on Windows, 56 on Linux), so this harness's
    # number has to come from a run of this harness.
    soft "lifecycle-gap ceiling is not recorded for this harness: $NO_PATH_N backend(s) have no path to a real lifecycle —$NO_PATH"
    echo "        record it in this script:  LIFECYCLE_GAP_CEILING=$NO_PATH_N"
elif [ "$NO_PATH_N" -gt "$LIFECYCLE_GAP_CEILING" ]; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES
    - coverage: $NO_PATH_N backend(s) have no path to a real lifecycle, over the ceiling of $LIFECYCLE_GAP_CEILING"
    echo "  FAIL  $NO_PATH_N backend(s) can never get a real lifecycle from this harness, and the"
    echo "        ceiling is $LIFECYCLE_GAP_CEILING:$NO_PATH"
    echo "        Q4 item 4: no new backend until the current set passes. Give it a canary, or"
    echo "        name in no_lifecycle_reason() why it cannot have one."
elif [ "$NO_PATH_N" -gt 0 ]; then
    soft "$NO_PATH_N backend(s) have no path to a real lifecycle (ceiling $LIFECYCLE_GAP_CEILING) —$NO_PATH"
    echo "        Q4: this is the release blocker, not a caption. Lower the ceiling as they land."
else
    PASS=$((PASS + 1)); echo "  PASS  every registered backend has a canary or a stated reason it cannot have one"
fi

# --- the real-lifecycle ratchet (G-11) ------------------------------------
# The audit above accepts a plan-smoke as coverage, so a run with 4 real lifecycles and a run
# with 15 both PASS. This asks the other question: did THIS host class do worse than it has
# done before? The floor lives in `scripts/lifecycle-floor.txt` beside the reasoning.
LIFECYCLES=$(grep -c . "$LEDGER/be-life.u")
# Backends whose lifecycle this run could not MEASURE, because the install failed for a reason
# LiNix itself classified as passing and a retry did not clear (a rate-limit window, a held
# lock). That is not the same fact as "this host did fewer lifecycles", and the ratchet must not
# confuse them: a GitHub rate limit on the macOS leg dropped the count 8 -> 7 and turned this
# gate red, and the obvious repair — lowering the floor to 7 — would have ratcheted a
# platform's coverage down permanently over a window that had already moved (R-3).
#
# Excused only for a class LiNix computed, and only BY NAME, printed below. A backend that
# genuinely broke is classed `permanent` or `unknown`, is scored a defect, and is not in here —
# so a real collapse still fails this check.
sort -u "$LEDGER/be-life-unmeasured" > "$LEDGER/be-life-unmeasured.u" 2>/dev/null || : > "$LEDGER/be-life-unmeasured.u"
UNMEASURED=$(grep -c . "$LEDGER/be-life-unmeasured.u")
MEASURABLE=$((LIFECYCLES + UNMEASURED))
# A stable key. `uname -s` on git-bash is `MINGW64_NT-10.0-26200` — a Windows build number,
# so keying on it would mint a fresh host class (and a free pass) at every OS update.
case "$(uname -s 2>/dev/null)" in
    MINGW*|MSYS*|CYGWIN*|Windows*) HOST_OS=windows ;;
    Darwin*)                       HOST_OS=darwin ;;
    Linux*)                        HOST_OS=linux ;;
    *)                             HOST_OS=unknown ;;
esac
# Which image this is, not which distro it was built on. `/etc/os-release` answers `ubuntu` for
# both the ubuntu image (7 real lifecycles) and the `tools` image (25) — `tools` IS Ubuntu — so
# keying on it filed two incomparable runs under one record, and whichever wrote it last made
# the other permanently wrong. Each Dockerfile declares its own `LINIX_IT_IMAGE`; os-release
# remains the fallback for an image that has not.
HOST_FLAVOUR=""
if [ -n "${LINIX_IT_IMAGE:-}" ]; then
    HOST_FLAVOUR="-$LINIX_IT_IMAGE"
elif [ -r /etc/os-release ]; then
    HOST_FLAVOUR="-$(. /etc/os-release 2>/dev/null; echo "${ID:-}")"
fi
HOST_CLASS="container-${HOST_OS}${HOST_FLAVOUR}-$([ -n "${CI:-}" ] && echo ci || echo local)"
FLOOR_FILE="/src/scripts/lifecycle-floor.txt"
if [ -n "$SMOKE" ]; then
    # A smoke run installs nothing, so 0 is its correct answer and a floor over it could only be
    # 0 — a number no run can fall below. Judged here would be a check that cannot fail.
    soft "real-lifecycle ratchet: not judged — SMOKE_ONLY installs nothing, so $LIFECYCLES is not a coverage measurement"
elif [ -f "$FLOOR_FILE" ]; then
    FLOOR=$(grep -E "^${HOST_CLASS} " "$FLOOR_FILE" 2>/dev/null | awk '{print $2}' | head -1)
    if [ -z "$FLOOR" ]; then
        # Not a PASS: this branch reads a record that is not there, so it examines nothing. It
        # counted as a passing check while every container leg took it — the floor was mounted on
        # all five and in force on none — and the only thing that noticed was the mutation gate,
        # counting one more check that survives a do-nothing binary.
        soft "real-lifecycle ratchet: no record for $HOST_CLASS yet, so nothing was compared"
        echo "        add to $FLOOR_FILE:  $HOST_CLASS $LIFECYCLES"
    elif [ "$MEASURABLE" -lt "$FLOOR" ]; then
        FAILC=$((FAILC + 1))
        FAILED_NAMES="$FAILED_NAMES
    - coverage: $LIFECYCLES real lifecycle(s) on $HOST_CLASS, below the recorded $FLOOR"
        echo "  FAIL  real-lifecycle ratchet: $LIFECYCLES, and $HOST_CLASS has done $FLOOR before"
        echo "        Something stopped running. A plan-smoke satisfies the audit above, so this"
        echo "        is the only check that notices coverage collapsing rather than breaking."
        [ "$UNMEASURED" -gt 0 ] && echo "        ($UNMEASURED excused as unmeasurable, and it was still not enough.)"
    elif [ "$LIFECYCLES" -lt "$FLOOR" ]; then
        # Short of the floor, and the shortfall is exactly the backends nothing could measure.
        # Reported at full volume and never silently: a run that excuses coverage has to say so,
        # or "silent truncation reads as covered everything when it did not".
        soft "real-lifecycle ratchet: $LIFECYCLES of $FLOOR on $HOST_CLASS, and $UNMEASURED backend(s) could not be measured this run"
        echo "        unmeasurable: $(tr '
' ' ' < "$LEDGER/be-life-unmeasured.u")"
        echo "        Each failed a real install for a reason LiNix classed as passing, and did"
        echo "        not clear on a retry — a rate-limit window, a held lock. The floor is NOT"
        echo "        lowered for these: the next run on a clear window measures them again."
    else
        PASS=$((PASS + 1))
        echo "  PASS  real-lifecycle ratchet: $LIFECYCLES >= $FLOOR recorded for $HOST_CLASS"
        [ "$LIFECYCLES" -gt "$FLOOR" ] &&             echo "        ratchet up:  sed -i 's/^$HOST_CLASS .*/$HOST_CLASS $LIFECYCLES/' $FLOOR_FILE"
    fi
else
    # A counted failure, not a note. This branch printed one line and incremented neither PASS
    # nor FAILC, so the ratchet was absent from all four distro legs and the `tools` image and
    # every one of those runs was green — the gate reporting its own absence in a voice nothing
    # tallies (N-5). `.dockerignore` excludes `scripts/`, so the file reaches a container only by
    # being mounted; `run.sh` and every `docker run` in `ci.yml` mount it now.
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES
    - coverage: the real-lifecycle ratchet is not in force ($FLOOR_FILE is not in this container)"
    echo "  FAIL  real-lifecycle ratchet: $FLOOR_FILE is not here, so nothing checked whether"
    echo "        coverage collapsed. $LIFECYCLES real lifecycle(s) this run, unmeasured against"
    echo "        $HOST_CLASS. Mount it:  -v \"\$PWD/scripts/lifecycle-floor.txt:$FLOOR_FILE:ro\""
fi

# Commands that cannot be executed in a container, each with the reason. Anything
# not on this list must have been RUN — `--help` does not count.
EXEMPT_CMDS="shell history bisect fleet"
# A SMOKE run installs nothing, so no commit is ever written, and the two verbs
# that read one have nothing to read. Named here rather than silently passing:
# an exemption that appears only in one mode has to say which mode.
[ -n "$SMOKE" ] && EXEMPT_CMDS="$EXEMPT_CMDS rollback diff run"
exempt_reason() {
    case "$1" in
        shell)    echo "opens an interactive subshell" ;;
        history)  echo "an interactive manifest-history TUI" ;;
        bisect)   echo "restores system snapshots, and may need a reboot between steps" ;;
        fleet)    echo "compares machines over SSH; there are no peers here" ;;
        rollback) echo "SMOKE_ONLY: nothing was installed, so no commit exists to roll back to" ;;
        diff)     echo "SMOKE_ONLY: nothing was installed, so there are no two commits to diff" ;;
        run)      echo "SMOKE_ONLY: an ephemeral environment installs the package it provisions" ;;
        *)        echo "" ;;
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

# The container is thrown away either way, so this is not cleanup for its own sake: an
# unmounted loopback file left behind by a run that failed halfway makes the NEXT run's
# `mkfs` fail on a busy device, and that failure names btrfs rather than the run before it.
teardown_storage_devices

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
