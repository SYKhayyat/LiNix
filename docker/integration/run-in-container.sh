#!/bin/sh
# ============================================================================
# Shall v7 integration harness — runs INSIDE a disposable container as root.
#
#   Usage: run-in-container.sh <native-backend> [package]
#   e.g.   run-in-container.sh apt jq
#
# Driven entirely through the real `shall` binary against the distro's native
# package manager AND against every other package manager the image ships.
# Isolation is by env var (SHALL_CONFIG_DIR / SHALL_DATA_DIR), so Shall's own
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
SHALL="${SHALL:-shall}"
TO="timeout 300"
# A source-building manager (cargo, opam, nimble, spack, go) compiles the canary
# from scratch; 300s is a build that has barely started. 900s is long enough for a
# real build and short enough that one wedged manager cannot eat the whole matrix —
# and a run that hits it says so by name rather than blaming the network.
TO_LONG="timeout 900"

# --- Isolation: Shall's config + data are throwaway; the II.1 repo lives here.
export SHALL_CONFIG_DIR="/tmp/shall-it-config"
export SHALL_DATA_DIR="/tmp/shall-it-state"
rm -rf "$SHALL_CONFIG_DIR" "$SHALL_DATA_DIR"
mkdir -p "$SHALL_CONFIG_DIR" "$SHALL_DATA_DIR"

# **A wait budget equal to the caller's whole timeout leaves nothing for the work.**
# `manager_lock_wait_secs` defaults to 300 — sized for a person's `dnf upgrade`, with no outer
# clock — and `$TO` above is `timeout 300`. A run that waits out an orphaned manager therefore
# hits the harness's own limit at the exact second the wait would have expired, and is killed
# mid-transaction with nothing installed: measured on fedora as `crash/midway: the cleanup
# uninstall left 3 of pv dos2unix ncdu still on PATH`, a sentence about a cleanup that never got
# to run. Every crash section here kills Shall on purpose and so manufactures exactly this
# orphan, over and over.
#
# Thirty seconds: long enough that the wait is genuinely exercised (the checks that watch for
# `shall: waiting for` still see it), short enough to leave the command nine tenths of its
# clock. The product's default is not the thing under test here; that it waits at all, and says
# so, is.
cat > "$SHALL_CONFIG_DIR/preferences.toml" <<'PREFS'
manager_lock_wait_secs = 30
PREFS

# --- The coverage ledger. Files, not variables: `grep_ok` runs its command in a
# pipeline, and a pipeline is a subshell whose variable writes die with it — so a
# ledger kept in a variable would silently forget every command greped for.
LEDGER=/tmp/shall-it-ledger
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

lx()      { record_argv "$@"; $TO "$SHALL" "$@"; }
lx_slow() { record_argv "$@"; $TO_LONG "$SHALL" "$@"; }

# Shall commits as you (II.13) and injects no identity of its own, so git needs
# to know who that is. A bare container has no identity and every commit fails.
# An identity for the `git init` section, as environment rather than as `git config --global`.
#
# This script is written for a disposable container, where a global write is harmless. It is
# not only run there: `scripts/harness-mutation-test.sh` executes it on the host to measure
# whether its checks can fail, and both release scripts now do that — so on 2026-07-28 this
# pair silently replaced a developer's real git identity, and thirteen commits went out
# authored `Shall Integration <integration@shall.invalid>` instead of by their author.
#
# `GIT_AUTHOR_*`/`GIT_COMMITTER_*` are per-process: they cover this run and touch nothing the
# user owns. And they are set only when git has no identity, so a machine that has one keeps
# it — what the harness exercises is then what its owner actually runs.
if ! git config user.email >/dev/null 2>&1; then
    export GIT_AUTHOR_NAME="Shall Integration" GIT_AUTHOR_EMAIL="integration@shall.invalid"
    export GIT_COMMITTER_NAME="Shall Integration" GIT_COMMITTER_EMAIL="integration@shall.invalid"
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
# and the check reports full coverage. Measured under a do-nothing `shall` stub, the audit
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
    elif [ "$rc" = 3 ]; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES\n    - $desc (rc=3: a deliberate refusal where a failure was expected)"
        echo "  FAIL  $desc (rc=3: Shall refused on purpose; if that is the outcome under test, assert it with refuses_with_3)"
        return 1
    else
        PASS=$((PASS + 1)); echo "  PASS  $desc (failed, as it must)"; return 0
    fi
}

# The other half of `nok`, and the reason it is a separate word: Shall has a dedicated exit code
# for declining on purpose (`Exit::Refused` = 3, U21) and `nok` could not tell it from a crash.
# Measured by the round-6 grader against a stub that answers `--version` and fails everything
# else: SIXTEEN of seventeen surviving checks were refusal checks, every one of them scored
# "correctly refused" because the stub exited 1. The distinction the product publishes is the
# distinction the harness has to assert.
# `nok`, plus the sentence. A negative check that asserts only "non-zero" cannot tell the
# product refusing your input from the binary being broken — measured: a stub that fails
# everything left twelve of these passing (G-8). The pattern is the manager-independent half
# of Shall's own message, so this stays true wherever the sweep runs.
nok_saying() { # description pattern command...
    desc="$1"; pat="$2"; shift 2
    "$@" >/tmp/it.out 2>&1; rc=$?
    if [ "$rc" = 0 ]; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES
    - $desc (expected a failure, got 0)"
        echo "  FAIL  $desc (expected a failure, but it succeeded)"; return 1
    fi
    if never_ran "$rc"; then
        FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES
    - $desc (rc=$rc: never ran)"
        echo "  FAIL  $desc (rc=$rc: the command never ran; that is not a failure)"
        excerpt; return 1
    fi
    if grep -q "$pat" /tmp/it.out; then
        PASS=$((PASS + 1)); echo "  PASS  $desc (refused, saying so)"; return 0
    fi
    FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES
    - $desc (failed without saying /$pat/)"
    echo "  FAIL  $desc (rc=$rc, but nothing in the output said /$pat/ — it failed for some other reason)"
    excerpt; return 1
}

refuses_with_3() { # description command...
    desc="$1"; shift
    "$@" >/tmp/it.out 2>&1; rc=$?
    if [ "$rc" = 3 ]; then
        PASS=$((PASS + 1)); echo "  PASS  $desc (refused on purpose, exit 3)"; return 0
    fi
    FAILC=$((FAILC + 1))
    if [ "$rc" = 0 ]; then
        _rw="it succeeded"
    elif never_ran "$rc"; then
        _rw="rc=$rc: the command never ran"
    else
        _rw="rc=$rc: a failure, not the documented refusal (readme.md: 3 means refused on purpose)"
    fi
    FAILED_NAMES="$FAILED_NAMES\n    - $desc ($_rw)"
    echo "  FAIL  $desc ($_rw)"
    excerpt; return 1
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
# A refusal is its own outcome. Shall worked correctly and declined on purpose (exit 3), and
# scoring that as a failure — or as "ecosystem variance" — says the opposite of what happened.
refused() { PASS=$((PASS + 1)); echo "  PASS  $1 (Shall refused, on purpose)"; }

# Why an install failed — a question, not an assumption (E5).
#
# Both harnesses used to soften ANY install failure into a claim about the network, and skip
# that backend's whole remaining lifecycle. In one observed run it fired four times and not
# once was it the network: one was Shall correctly refusing, two were real argv defects
# (`helm`, `luarocks`). Coverage disappeared exactly where the product was broken, and the run
# still reported success.
#
# Sets CLASS to one of:
#   refused    Shall declined on purpose (exit 3, U21). Its own outcome, not a failure.
#   timeout    the build ran out of clock (124). The harness's limit, not a verdict.
#   transient  failed once, succeeded on retry. The caller CONTINUES the lifecycle — skipping
#              it is how list, PATH, remove and gone-from-list went unrun for every backend
#              whose install was flaky.
#   exhausted  Shall classed the failure passing and it did not pass in this window — a rate
#              limit with 20 minutes left on it. SOFT, and recorded as a lifecycle this run
#              could not measure, which is not the same fact as a lifecycle that got worse.
#   defect     failed permanently, or failed twice with nothing classifying it. Hard.
#
# Twinned with scripts/integration-windows.sh's, and lifted out of both by
# scripts/harness-logic-test.sh so the two cannot drift into disagreeing about a verdict.
#
# TRANSIENCE IS READ, NOT RE-DERIVED (R-3). It is a claim that a second attempt could differ,
# and Shall already answers it — `Retryability`, from the backend's own exit policy. Until
# 2026-07-30 nothing downstream could see that answer, so this function re-derived it by
# RETRYING THE INSTALL IMMEDIATELY. That proxy is wrong for exactly the failures the
# classification gets right: a GitHub rate limit with 1236 seconds left on the window cannot
# succeed one second later, so it scored `defect`, the macOS leg went red, and the
# real-lifecycle ratchet fell 8 -> 7 and went red behind it. Two red jobs over an answer the
# program had already computed.
#
# So `shall-failure-class:` is read, and the retry is kept only where it still adds evidence:
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
    _ci_class="$(sed -n 's/^shall-failure-class: //p' "$_ci_log" | tail -1)"
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
# Shall's answer to a bin directory that is not on PATH is a warning naming the directory and
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

# assert_binary_reachable <backend> <binary> <install-log> <what-the-name-resolved-to-before>
#
# An install the user cannot invoke is a failed install reported as a success (E6c). On a clean
# runner most per-user managers install into a directory nobody's PATH names, so asking PATH
# alone fails runs where the product did everything it promised and passes runs where it said
# nothing at all. So the assertion is the promise: the name resolves, OR the install named the
# directory and the file is in it. Silence plus an unreachable binary is the defect — measured
# 2026-07-29 on a clean Windows runner, `github` and `yarn` both.
#
# The fourth argument is the question "is a binary of this name reachable" cannot answer: WHOSE.
# Two managers ship a binary of the same name — cabal's canary is `hello` and so is go's — and
# `go: hello is on PATH` passed on the tools image against /root/.cabal/bin/hello, which cabal
# had installed four lifecycles earlier (G-3). Its twin `assert_binary_gone` was given this
# value and this one was not, in the same three lines of the same function.
assert_binary_reachable() { # backend binary install-log prior-resolution
    # `$3` is used where it stands rather than named: every variable this function sets is a
    # global in a POSIX shell, and `harness-logic-test.sh` lifts these bodies and runs them
    # against globals of its own. Naming the log clobbered the test's `$_rlog` and broke three
    # unrelated predicates that had nothing to do with the change.
    _rbe="$1"; _rbin="$2"; _rprev="${4:-}"
    _rnow="$(path_of "$_rbin")"

    # It resolves somewhere it did not resolve before: this install is what put it there.
    if [ -n "$_rnow" ] && [ "$_rnow" != "$_rprev" ]; then
        PASS=$((PASS + 1)); echo "  PASS  $_rbe: $_rbin is on PATH (at $_rnow)"; return 0
    fi

    # Either nothing resolves, or the name still resolves to whatever owned it before. PATH
    # cannot answer for this backend in either case, so the evidence is the directory the
    # install named — asked directly, because `binary_present` starts by asking PATH and PATH
    # is the thing that is lying here.
    _rdir="$(named_bin_dir "$_rbe" "$3")"
    _rcopy=""
    [ -n "$_rdir" ] && _rcopy="$(off_path_copy "$_rbe" "$_rbin" "$3")"
    if [ -n "$_rcopy" ]; then
        PASS=$((PASS + 1))
        if [ -n "$_rnow" ]; then
            echo "  PASS  $_rbe: $_rbin still resolves to the pre-existing $_rnow, and this backend's own copy is at $_rcopy"
        else
            echo "  PASS  $_rbe: $_rbin is not on PATH, and the install said so, naming $_rdir"
        fi
        return 0
    fi

    FAILC=$((FAILC + 1))
    if [ -n "$_rnow" ]; then
        _rwhy="$_rbin resolves to $_rnow, which was already there before this install — nothing here shows $_rbe installed anything"
    elif [ -z "$_rdir" ]; then
        _rwhy="$_rbin is not on PATH and nothing said where it went"
    else
        _rwhy="the install named $_rdir and $_rbin is not in it"
    fi
    FAILED_NAMES="$FAILED_NAMES\n    - $_rbe: $_rwhy"
    echo "  FAIL  $_rbe: $_rwhy"
    return 1
}

echo "=============================================================="
echo " Shall v7 harness — backend=$BACKEND package=$PKG"
echo "=============================================================="

# A missing binary is not a failing run, it is an unrun one — and it does not
# look like one. `nok` reads "command not found" as the refusal it was hoping
# for, and `grep_ok` for /shall/ matches the words "failed to run command
# 'shall'", so an image with no binary reported nine passes. Stop here instead.
if ! $SHALL --version >/dev/null 2>&1; then
    echo "FATAL: not runnable in this image — nothing below was tested. Looked for '$SHALL'"
    echo "       The image must put the built binary on PATH (see the Dockerfiles)."
    exit 1
fi

# --- 1. Bootstrap the II.1 repo -------------------------------------------
echo "[1] Bootstrap"
ok "init scaffolds the repo" lx init
ok "priority file exists" test -f "$SHALL_CONFIG_DIR/priority"
grep_ok "priority names this backend" "$BACKEND" cat "$SHALL_CONFIG_DIR/priority"
ok "active file exists" test -f "$SHALL_CONFIG_DIR/active"

# --- 2. Discovery (read-only) ---------------------------------------------
echo "[2] Discovery / read-only verbs"
ok "check health" lx check health
ok "check drift" lx check drift
ok "plan (no changes yet)" lx plan --dry-run
answers "check parses the model" lx check
ok "check absent lists nothing" lx check absent
ok "protected lists guarded packages" lx protected
grep_ok "protected includes a system essential" "shall\|libc\|systemd\|kernel\|bash" lx protected

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
echo "[4] purge-undeclared, before adopt (the state that makes it a test)"
refuses_with_3 "purge-undeclared is refused on a machine Shall has not adopted" lx -y purge-undeclared
# WHICH rule refused matters: a `nok` that accepts any non-zero exit accepts a panic
# and an unknown flag just as happily. Before adopt the ratio rule is the one that
# fires, and it says so by name.
grep_ok "and it is the unadopted-machine ratio that refused" \
    "adopt\|allow-mass-purge" lx -y purge-undeclared

# --- 5. Imperative install -> list -> coherence ---------------------------
echo "[5] Install"
if [ -n "$SMOKE" ]; then
    skip_smoke "install $PKG, and the list/PATH checks that read its result"
else
    # Where the name resolved BEFORE the install, which is what tells a binary this install
    # put there from one that was already on PATH under the same name (G-3).
    PKG_PREPATH="$(path_of "$PKG")"
    ok "install $PKG" lx -y install "$PKG"
    # `ok` leaves the install's output in /tmp/it.out and the next check overwrites it; the
    # reachability assertion reads what the install SAID, so it gets its own copy.
    cp /tmp/it.out /tmp/it-life0.out 2>/dev/null || true
    grep_ok "list shows $PKG" "$PKG" lx list
    assert_binary_reachable "$BACKEND" "$PKG" /tmp/it-life0.out "$PKG_PREPATH"
    echo "$BACKEND" >> "$LEDGER/be-life"
fi

# --- 6. Idempotency --------------------------------------------------------
echo "[6] Idempotency"
# Runs under SMOKE too: with nothing installed the model is empty, and a sync over an
# empty model must still exit 0 rather than find work that is not there.
ok "second sync is a no-op (exit 0)" lx -y sync

# --- 7. Negative path ------------------------------------------------------
echo "[7] Negative path"
nok "installing a nonexistent package fails" lx -y install "shall-no-such-pkg-zzz"
# The failure must not be left in the manifest. Every later command parses the
# model, so one unresolvable line wedges the config until someone hand-edits it.
ok "a failed install leaves the model parseable" lx check drift
# This asserts the PRODUCT withdrew the line. It used to `grep -v` the name out first and
# then assert it was absent, which tested its own `grep -v` and printed PASS on every run
# while the product did the opposite. If this goes red, Shall stopped withdrawing an
# unresolvable name — do not put the scrub back.
#
# The name here is deliberately unqualified: nothing claims it, so it is `Unresolvable` and
# withdrawing it is the behaviour both harnesses have always agreed on. The qualified form
# (`<backend>:<typo>`, which resolves and then fails to install) is a different question and
# is asserted in the host harness.
IMPERATIVE="$SHALL_CONFIG_DIR/modules/imperative.txt"
if [ -f "$IMPERATIVE" ]; then
    nok "the unresolvable name is out of the manifest" \
        grep -q "shall-no-such-pkg-zzz" "$IMPERATIVE"
fi

# --- 8. Adopt (Part IV proof) ---------------------------------------------
echo "[8] Adopt"
ADOPTED_FILE="$SHALL_CONFIG_DIR/modules/adopted.txt"
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
        # Never MORE than the manual set: adopt may drop a name (already declared elsewhere,
        # unwritable), but a count above it means something not user-chosen was swept in.
        # Protected and OS-essential names are NOT dropped — they are adopted like any other
        # (E7, Q47), which moves this count toward the bound rather than away from it.
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
# The victim comes from Shall's OWN protected list, intersected with what this image actually
# has installed. It was hardcoded to `bash` until 2026-07-30, and Void Linux ships no bash at
# all — so on the first Void run this check asserted the survival of a package that had never
# been there and printed `FAIL bash survives an uninstall attempt`, which reads exactly like
# the guard having deleted /bin/bash. A proof that cannot run must say so, not accuse.
_installed_names() { lx list --backend "$BACKEND" 2>/dev/null | awk '{print $2}'; }
GUARD_VICTIM=""
_have="$(_installed_names)"
# A protected name is matched against the manager's own spelling AND its last path component,
# because Portage names a package `app-shells/bash` while `shall protected` says `bash`. Without
# that the intersection on gentoo is empty however much bash is installed, the guard proof
# examines nothing, and `uninstall` — which on a SMOKE_ONLY image is executed ONLY by the guard
# victim — is reported as never run. Two failures, one missing slash. Measured on CI 30680916682.
for _p in $(lx protected 2>/dev/null | sed -n 's/^  \([a-z0-9][a-z0-9._+-]*\)$/\1/p'); do
    _match="$(printf '%s\n' "$_have" | while IFS= read -r _n; do
        [ "$_n" = "$_p" ] && { printf '%s' "$_n"; break; }
        [ "${_n##*/}" = "$_p" ] && { printf '%s' "$_n"; break; }
    done)"
    if [ -n "$_match" ]; then GUARD_VICTIM="$_match"; break; fi
done
if [ -z "$GUARD_VICTIM" ]; then
    FAILC=$((FAILC + 1))
    FAILED_NAMES="$FAILED_NAMES
    - guard: no protected package is installed on this image, so the guard proof examined nothing"
    echo "  FAIL  not one of Shall's protected packages is installed here, so nothing could be"
    echo "        protected and this check proved nothing. Add one to the image, or widen"
    echo "        default_protected_packages so it names something this distro ships."
else
    echo "        guard victim: $GUARD_VICTIM (from \`shall protected\`, and installed here)"
    lx -y uninstall "$GUARD_VICTIM" >/dev/null 2>&1 || true
    ok "$GUARD_VICTIM survives an uninstall attempt" \
        sh -c "$SHALL list --backend '$BACKEND' 2>/dev/null | awk '{print \$2}' | grep -qx '$GUARD_VICTIM'"
fi
# Post-adopt the ratio no longer fires, so the bare command must still not be a
# silent mass-delete — the refusal is asserted in both states, for different reasons.
refuses_with_3 "purge-undeclared is still not a silent mass-delete after adopt" lx -y purge-undeclared
# WHICH rule refuses is still asserted, but the answer depends on how much `adopt`
# could take on this image: where it adopted well the protected set decides, where it
# adopted little the ratio still does. Both are named answers; "some error" is not, and
# that is what a bare `nok` would accept.
grep_ok "and the refusal after adopt still names its rule" \
    "protected\|essential\|allow-mass-removal\|allow-mass-purge" lx -y purge-undeclared

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
ok "the install left a commit behind" git -C "$SHALL_CONFIG_DIR" rev-parse HEAD
# Subjects are deliberately generic (II.13 puts the detail in the diff), so the
# package name is not in the log — match the subject prefix Shall actually writes.
grep_ok "git log shows a shall commit" "shall:" lx git log --limit 10
ok "git commit records the current state on demand" lx git commit -m "shall: harness checkpoint"
ok "diff against a commit runs" lx diff HEAD
ok "rollback to HEAD is accepted" lx -y rollback HEAD
fi
fi

# --- 12. rebuild asserts, and writes no commit (K14) ----------------------
echo "[12] rebuild"
# Git is asked directly, not `shall git log`: a rebuild that committed by some
# other route would still move HEAD, and only git can say so.
commits() { git -C "$SHALL_CONFIG_DIR" rev-list --count HEAD 2>/dev/null || echo 0; }
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
LOCKFILE=$(ls "$SHALL_CONFIG_DIR"/locks/bare.*.toml 2>/dev/null | head -1)
echo "        lock file: ${LOCKFILE:-<none>}"
# Per-host: the answer is about this machine, so the filename has to be too, or
# two machines sharing a config overwrite each other on every sync.
ok "the lock is named for this host" test -n "$LOCKFILE"
grep_ok "an unpinned name froze to $BACKEND" "\"$BACKEND\"" cat "$LOCKFILE"

# A lock written by another machine is not an answer about this one.
printf '[resolved]\n%s = "shall-no-such-backend"\n' "$PKG" \
    > "$SHALL_CONFIG_DIR/locks/bare.some-other-box.toml"
ok "sync ignores another host's lock file" lx -y sync
ok "and leaves it alone" test -f "$SHALL_CONFIG_DIR/locks/bare.some-other-box.toml"
rm -f "$SHALL_CONFIG_DIR/locks/bare.some-other-box.toml"
fi

# The chain grammar. `list` is the priority file; a comma separates candidates.
ok  "a chain is legal"            lx --dry-run install "$BACKEND,cargo:$PKG"
ok  "a chain may end in list"     lx --dry-run install "$BACKEND,list:$PKG"
ok  "list alone is legal"         lx --dry-run install "list:$PKG"
nok_saying "an empty slot is refused" "has an empty backend"    lx --dry-run install "$BACKEND,,cargo:$PKG"
nok_saying "an unknown link is refused" "is not a backend Shall uses"  lx --dry-run install "$BACKEND,nope:$PKG"
nok_saying "list must come last" "must come last"         lx --dry-run install "list,$BACKEND:$PKG"
nok_saying "a name repeated is refused" "is named twice"  lx --dry-run install "$BACKEND,$BACKEND:$PKG"
nok_saying "a pattern cannot span one" "must match in exactly one backend"   lx --dry-run install "$BACKEND,cargo:re:^$PKG"

# A pin naming a manager this host does not have must fail out loud, not quietly
# decide there is nothing to do — that silence is the bug chains exist to end.
FOREIGN=dnf; [ "$BACKEND" = "dnf" ] && FOREIGN=apt
command -v "$FOREIGN" >/dev/null 2>&1 \
    && soft "$FOREIGN exists on this image — cannot test a pin to a missing manager" \
    || nok "a pin to a manager this host lacks is not silent" lx -y install "$FOREIGN:$PKG"

if [ -z "$SMOKE" ]; then
grep_ok "unlock backends --list names the frozen package" "$PKG" lx unlock backends --list
ok "unlock backends forgets one name" lx unlock backends "$PKG"
nok "the entry is really gone" grep -q "$PKG" "$LOCKFILE"
fi
ok "unlocking a name that was never frozen is not an error" lx unlock backends shall-never-frozen-zzz

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

    SILENT_CFG=/tmp/shall-it-silent
    rm -rf "$SILENT_CFG"; mkdir -p "$SILENT_CFG/modules" "$SILENT_CFG/profiles"
    printf 'cargo\n%s\n' "$BACKEND" > "$SILENT_CFG/priority"
    printf 'Work\n' > "$SILENT_CFG/active"
    printf 'use base\n' > "$SILENT_CFG/profiles/Work"
    printf '%s\n' "$PKG" > "$SILENT_CFG/modules/base.txt"

    silent_lx() {
        env PATH="/tmp/silent-bin:$PATH" SHALL_CONFIG_DIR="$SILENT_CFG" \
            SHALL_DATA_DIR=/tmp/shall-it-silent-state $TO "$SHALL" "$@"
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
    rm -rf /tmp/silent-bin "$SILENT_CFG" /tmp/shall-it-silent-state
fi

# ==========================================================================
# 13c. REAL DEVICES for the storage effectors (btrfs, lvm, zfs)
# ==========================================================================
# These three operate on block devices, and until 2026-07-30 none of them had ever been run.
# The harness called btrfs "a snapshot provider, not an install target" — which is not what the
# code does. `btrfs:PATH` runs `btrfs subvolume create`, `lvm:VG/LV` runs `lvcreate`, and both
# have an ordinary install → list → remove cycle that needs nothing but a real device.
#
# Inert unless the image asked for it (`SHALL_IT_STORAGE`), because it needs `--privileged` and
# nothing else here is. The devices are loopback files made in the container and destroyed with
# it. Owner-authorised 2026-07-30 (Q17).
#
# Every step is checked: a failure leaves the backend without a canary and PRINTS why, rather
# than leaving a half-made volume for the lifecycle to trip over in a way that reads like a
# Shall defect.
STORAGE_BTRFS=""
STORAGE_LVM=""
STORAGE_ZFS=""

setup_storage_devices() {
    [ -n "${SHALL_IT_STORAGE:-}" ] || return 0
    [ -z "$SMOKE" ] || { skip_smoke "loopback devices for the storage effectors"; return 0; }
    echo "[13c] Loopback devices for the storage effectors"

    if ! command -v mkfs.btrfs >/dev/null 2>&1; then
        soft "btrfs: no mkfs.btrfs in this image, so there is no filesystem to make a subvolume in"
    elif ! modprobe btrfs >/dev/null 2>&1 && ! grep -qw btrfs /proc/filesystems; then
        soft "btrfs: this kernel has no btrfs — a container borrows the HOST's kernel, so that is a fact about the machine and not about Shall"
    else
        rm -f /var/tmp/shall-btrfs.img
        truncate -s 512M /var/tmp/shall-btrfs.img
        mkdir -p /mnt/shall-btrfs
        if mkfs.btrfs -q -f /var/tmp/shall-btrfs.img >/dev/null 2>&1 \
           && mount -o loop /var/tmp/shall-btrfs.img /mnt/shall-btrfs >/dev/null 2>&1; then
            STORAGE_BTRFS=/mnt/shall-btrfs
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
        rm -f /var/tmp/shall-lvm.img
        truncate -s 512M /var/tmp/shall-lvm.img
        # Three commands, three separate reasons, and they used to share one message: "could not
        # build a volume group on a loopback device here" is a sentence you cannot act on. It
        # said the same thing whether the kernel had no free loop device, LVM's filter rejected
        # the one it got, or the group name was already taken — and on 2026-07-31 that cost an
        # hour chasing a leak that did not exist, because the message named nothing.
        _why=""
        _loop="$(losetup -f --show /var/tmp/shall-lvm.img 2>&1)"
        if [ ! -b "$_loop" ]; then
            _why="losetup: $_loop"
        elif ! _out="$(pvcreate -f -y "$_loop" 2>&1)"; then
            _why="pvcreate on $_loop: $(echo "$_out" | tr '\n' ' ')"
        elif ! _out="$(vgcreate shallvg "$_loop" 2>&1)"; then
            _why="vgcreate shallvg on $_loop: $(echo "$_out" | tr '\n' ' ')"
        fi
        if [ -z "$_why" ]; then
            # A volume group is not enough, and assuming it was cost a whole run: `lvcreate`
            # needs a device node to zero the new volume through, and in a container with no
            # udev and a tmpfs /dev there is nobody to make one. It aborts with "device not
            # cleared" — a fact about the machine that reads exactly like a Shall defect in the
            # lifecycle below. So the probe is a real volume, made and destroyed with Shall's
            # own argv, and its failure is reported as the environment's (Q17).
            if _probe="$(lvcreate -n shallprobe -L 8M shallvg 2>&1)"; then
                lvremove -y shallvg/shallprobe >/dev/null 2>&1
                STORAGE_LVM=shallvg
                PASS=$((PASS + 1)); echo "  PASS  lvm: volume group $STORAGE_LVM exists on $_loop and can hold a volume"
            else
                soft "lvm: the volume group exists and \`lvcreate\` cannot make a volume in it, which is this container and not Shall — $(echo "$_probe" | tr '\n' ' ')"
            fi
        else
            soft "lvm: could not build a volume group on a loopback device here — $_why"
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
        rm -f /var/tmp/shall-zfs.img
        truncate -s 512M /var/tmp/shall-zfs.img
        if zpool create -f shallpool /var/tmp/shall-zfs.img >/dev/null 2>&1; then
            STORAGE_ZFS=shallpool
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
    rm -f /var/tmp/shall-btrfs.img /var/tmp/shall-lvm.img /var/tmp/shall-zfs.img
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
# Install failure is SOFT (a registry outage is not a Shall bug); everything
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
# 10 on the `tools` image and 11 on every other one, and this constant is shared — so lowering
# it to 10 on the tools measurement turned the `storage` job red for a reason that had nothing
# to do with storage: `nix` is installed in `tools` and nowhere else, so only that image can
# reach 10. Measured on CI 30680916682, and put back the same hour.
#
# **The real fix is a ceiling per image**, the way `scripts/lifecycle-floor.txt` is a floor per
# host class and for exactly the same reason — one number over unlike machines is a number that
# is wrong for all but one of them. Not done tonight, and named here rather than left as a
# surprise for whoever next lowers this line.
#
# **Left at 11 deliberately on 2026-08-11.** Ten names moved out of the gap and into
# `no_lifecycle_reason` that day — the other distributions' native managers, which had been
# counted and never named — so every image's number falls, and by different amounts. Lowering
# this to a number nobody has measured is the mistake the paragraph above records happening once
# already, against `storage`. The soft branch prints each image's new count; take the largest of
# them from the next full matrix run and lower this line to it.
LIFECYCLE_GAP_CEILING=11
canary() {
    case "$1" in
        # **One binary name per backend.** `cowsay` was the canary for npm, pnpm, yarn AND bun,
        # and `pycowsay` for both pipx and uv — so whichever installed first owned the name and
        # the others' PATH checks passed on somebody else's binary. G-3 made that visible
        # (`pnpm: cowsay resolves to .../bun/bin/cowsay, which was already there`) and the fix
        # is a distinct canary per backend, not a weaker check. Each binary was verified from
        # the registry (`npm view <pkg> bin`) rather than assumed.
        npm)      echo "cowsay|cowsay|full|" ;;
        pnpm)     echo "json|json|full|" ;;
        yarn)     echo "catj|catj|full|" ;;
        bun)      echo "sort-package-json|sort-package-json|full|" ;;
        pipx)     echo "pycowsay|pycowsay|full|" ;;
        uv)       echo "pyjokes|pyjoke|full|" ;;
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
        # `figlet` and not `hello`: cabal's canary is ALSO `hello`, cabal has no uninstall verb,
        # and its copy therefore sits on PATH for the rest of the run — so nix's binary check
        # could never be clean, and the harness said so out loud the first time nix was
        # installed (`nix: hello resolves to /root/.cabal/bin/hello, which was already there`).
        # Two canaries sharing a binary name is the G-3 collision by construction; the fix is a
        # name nothing else in this image installs, not a weaker check.
        nix)      echo "figlet|figlet|full|" ;;
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
        # The list-token is EMPTY, which means `list` must say the same string `install` was
        # given. That is the assertion, and it is the whole point: `btrfs subvolume list`
        # reports a path relative to the filesystem root, so until 2026-07-30 a subvolume
        # installed as /mnt/shall-btrfs/canary came back as /canary and `sync` re-created it
        # every run. A token of `/canary` would pass either way — it is a substring of the full
        # path — so the weaker form could not tell the fix from the bug.
        #
        # `@quota=` and `@mount=` ride along because a subvolume that is only ever created is
        # half the backend: the mount half writes /etc/fstab, and until Q18 was ruled no line
        # could carry the option that reaches it, so it had never run once. The mount point is
        # deliberately NOT under $STORAGE_BTRFS — a second path to the same subvolume is what
        # `list` has to collapse, and mounting it inside the filesystem it lives on would not
        # produce one.
        btrfs)    [ -n "$STORAGE_BTRFS" ] && echo "$STORAGE_BTRFS/canary||full||@quota=100M,mount=/mnt/shall-btrfs-canary" ;;
        # `@size=` is not optional: `lvm:` refuses without one, by name, and the option rides
        # in the install-only field because `lvremove` takes the volume and not the size.
        lvm)      [ -n "$STORAGE_LVM" ] && echo "$STORAGE_LVM/canary||full||@size=64M" ;;
        zfs)      [ -n "$STORAGE_ZFS" ] && echo "$STORAGE_ZFS/canary||full||@quota=100M,mount=/mnt/shall-zfs-canary" ;;
        # An AppImage is installed by URL, so the "name" is the URL — and that is the whole
        # reason this row was empty for months. Pinned to a tag rather than `continuous`: a
        # moving artifact makes a red run mean two things at once.
        #
        # No binary check. Shall symlinks the downloaded file under a name derived from the URL,
        # and asserting a name this table guessed would be asserting the guess. `list` is the
        # presence proof, as it is for `mise` and `helm`.
        appimage) echo "https://github.com/AppImage/AppImageKit/releases/download/13/appimagetool-x86_64.AppImage||full|appimagetool" ;;
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
            [ -n "${SHALL_IT_STORAGE:-}" ] \
                || echo "needs a real block device, which only the \`storage\` image (--privileged) provides — plan-smoked here" ;;
        web)      echo "installs from a pasted URL; no stable public canary — smoked in 15" ;;
        appimage) echo "needs FUSE, which a plain container does not have — smoked in 15" ;;
        # A PRICE, not a wall, and the difference is the whole lesson of the `nix` finding
        # (Q17). Baking the toolchain into the image does not help: `stack install` builds the
        # PACKAGE from source too, and the smallest thing on Hackage is minutes per run on
        # every image forever. Re-derive this before believing it — that is what nobody did
        # for `nix`.
        stack)    echo "a Haskell package builds from source; the toolchain can be baked in, the build cannot — a cost, not an impossibility — smoked in 15" ;;
        # Two claims, and only one of them is a wall. The runtime size is a PRICE; the session
        # bus is real but `dbus-run-session flatpak --user` may close it, which is an
        # experiment nobody has run. Written as two facts so the next person can attack the
        # half that is attackable.
        flatpak)  echo "the smallest app pulls a multi-GB runtime (a cost), and there is no session bus here (a wall) — try dbus-run-session with --user before believing this" ;;
        # Detected, not assumed: on a distro without the marker a system pip install is
        # ordinary and gets the full lifecycle. Naming it keeps a permanent, expected
        # refusal from reading as ecosystem flakiness run after run.
        pip)      ls /usr/lib/python3*/EXTERNALLY-MANAGED >/dev/null 2>&1 \
                      && echo "this distro marks its Python EXTERNALLY-MANAGED (PEP 668), so a system pip install is refused by design" ;;

        # ---- Native managers of a distro this matrix does not build ---------
        #
        # These were counted and never named: the opensuse leg reported `10 backend(s) have no
        # path to a real lifecycle` and left the reader to work out that six of them are other
        # distributions' package managers. A count is not an answer. Each is now either "another
        # harness runs it" or a confession, and the confessions are what the README owes a
        # reader before a release (Q4).
        #
        # No image conditionals here on purpose: none of these is READY on any image in the
        # matrix, so a per-image reason would be six copies of the same sentence. `emerge` and
        # the pacman family are the exceptions and say why in their own line.
        brew)     echo "Homebrew is lifecycled on the macOS runner (scripts/integration-windows.sh brew); Linuxbrew is a second install of the same manager and no image in this matrix builds it" ;;
        emerge)   echo "Portage runs on the gentoo image and runs there SMOKE_ONLY — a real emerge builds the package from source, which is minutes per canary on every run for ever" ;;
        yay|paru) echo "an AUR helper builds from source AND refuses to run as root, which is what every container in this matrix is; the arch image lifecycles pacman underneath it — argv-tested only" ;;
        eopkg)    echo "Solus's native manager, and there is no Solus image in this matrix — argv-tested only" ;;
        slackpkg) echo "Slackware's native manager, and there is no Slackware image in this matrix — argv-tested only" ;;
        guix)     echo "GNU Guix needs its own daemon and store, the same shape as the nix finding (Q17) and without nix's answer — argv-tested only" ;;
        pkg)      echo "FreeBSD's native manager, and there is no FreeBSD host anywhere in this project's CI — argv-tested only" ;;
        pkg_add)  echo "OpenBSD's native manager, and there is no OpenBSD host anywhere in this project's CI — argv-tested only" ;;
        pkgin)    echo "the pkgsrc binary manager (NetBSD, SmartOS), and there is no pkgsrc host anywhere in this project's CI — argv-tested only" ;;
        macports) echo "macOS-only, and the macOS runner has not attempted it either — a cost nobody has priced, named in the same words there" ;;

        *)        echo "" ;;
    esac
}

# A manager whose own uninstall deletes the package and keeps its launcher. Reported,
# never assumed: the strict check runs first, and this only softens the result when the
# leftover actually happens — so a manager that cleans up properly still has to.
removal_leaves_binary() {
    case "$1" in
        bun) echo "bun's own \`remove -g\` drops the package and keeps its launcher (reproduced against bun directly, with no Shall involved)" ;;
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

# And the backends Shall reports as degraded ONLY because a setup step it offers to run has not
# been run (Q10/Q11/Q13). They belong in the lifecycle for the same reason they are degraded:
# `lx -y install` performs that setup, so leaving them out tests the offer nowhere — which is
# what happened the first night the health check shipped, when `mix` dropped from a real
# lifecycle to a plan-smoke and the run still said PASS.
#
# The sentence is Shall's own (`src/verbs/check.rs`); if it changes, this must change with it,
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
    $TO "$SHALL" unmanage "$1" >/dev/null 2>&1 || true
    _imp="$SHALL_CONFIG_DIR/modules/imperative.txt"
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
    grep -qx "$be" "$SHALL_CONFIG_DIR/priority" 2>/dev/null || echo "$be" >> "$SHALL_CONFIG_DIR/priority"

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
    # parser or argv fault from here on is a Shall bug and nothing else.
    _nolist="$(list_cannot_show "$be")"
    if [ -n "$_nolist" ]; then
        soft "$be: list does not show $ctok — $_nolist"
    else
        grep_ok "$be: list shows $ctok" "$ctok" lx list --backend "$be"
    fi
    [ -n "$cbin" ] && assert_binary_reachable "$be" "$cbin" /tmp/life.out "$_prepath"

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
        "$SHALL list --backend '$be' 2>/dev/null | grep -q '$ctok'"
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
# 14b. A DECLARED SIZE THAT CHANGES (Q19)
# ==========================================================================
# Section 14's lifecycle is install → list → remove, so it proves a volume can be *made* at a
# declared size and never that the number can be edited. Q19 ruled that it can: a bigger `@size`
# grows the volume, a smaller one is refused unless the line carries `@allow_shrink=true`.
#
# **That path cannot be proven by argv.** `lvextend --resizefs` shells out to `fsadm`, which
# talks to the filesystem, and the whole safety story — a shrink gives up bytes nothing is using,
# and a filesystem that cannot shrink stops the operation before the volume moves — lives in what
# that program does, not in the arguments it is handed. V.106's lesson from the run before this
# one: a backend that has never been executed has not been reviewed, it has been proofread.
#
# The converged re-run at the end is the assertion that matters most. D13's failure mode is a
# comparison that gets units wrong and reports a change on every sync **for ever**, and it is
# invisible to a test that syncs once.
storage_resize_lifecycle() {
    [ -n "$STORAGE_LVM" ] || return 0
    [ -z "$SMOKE" ] || { skip_smoke "the resize path (this image installs nothing)"; return 0; }
    echo "[14b] A declared size that changes (Q19)"

    _vol="$STORAGE_LVM/resizer"
    _dev="/dev/$STORAGE_LVM/resizer"
    _size_of() { lvs --noheadings --units b --nosuffix -o lv_size "$1" 2>/dev/null | tr -d ' 	'; }
    _cleanup_resizer() {
        undeclare_canary "lvm:$_vol"
        lvremove -y "$_vol" >/dev/null 2>&1
    }

    grep -qx lvm "$SHALL_CONFIG_DIR/priority" 2>/dev/null || echo lvm >> "$SHALL_CONFIG_DIR/priority"

    if ! lx_slow -y install "lvm:$_vol@size=64M" >/tmp/resize.out 2>&1; then
        soft "lvm: could not create the resize canary — $(tr '\n' ' ' < /tmp/resize.out)"
        _cleanup_resizer
        return 0
    fi

    # A raw volume has nothing for `--resizefs` to carry, and growing one is *meant* to fail —
    # so the canary gets a filesystem, and a machine that cannot give it one says so rather than
    # scoring the image's gap as a Shall defect.
    if ! mkfs.ext4 -q -F "$_dev" >/dev/null 2>&1; then
        soft "lvm: no ext4 on the resize canary (mkfs.ext4 failed or is absent), so --resizefs has no filesystem and the grow below would fail for the image's reason"
        _cleanup_resizer
        return 0
    fi

    # **The edit is made in the file and applied by `sync`, not by re-running `install`.** That is
    # the path Q19 is about: `install` names a spec and hands it to the backend, so it would prove
    # the resize argv and skip the half that was actually broken — the planner deciding that a
    # declaration whose volume already exists under its name still needs work.
    _mod="$SHALL_CONFIG_DIR/modules/imperative.txt"
    if ! grep -q "lvm:$_vol" "$_mod" 2>/dev/null; then
        soft "lvm: the resize canary is not declared in $_mod, so the edit-then-sync path cannot be driven here"
        _cleanup_resizer
        return 0
    fi
    _resize_to() {
        sed -i "s|\(lvm:$_vol@\)size=[^ ,]*|\1size=$1|" "$_mod"
        grep -q "size=$1" "$_mod"
    }

    _before="$(_size_of "$_vol")"
    if _resize_to 128M && lx_slow -y sync >/tmp/resize.out 2>&1; then
        _after="$(_size_of "$_vol")"
        if [ -n "$_after" ] && [ -n "$_before" ] && [ "$_after" -gt "$_before" ]; then
            PASS=$((PASS + 1)); echo "  PASS  lvm: a bigger @size grew $_vol, $_before -> $_after bytes"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES\n    - lvm: @size=128M reported success and the volume is still $_after bytes"
            echo "  FAIL  lvm: @size=128M reported success and the volume is still $_after bytes (was $_before)"
        fi
    else
        FAILC=$((FAILC + 1))
        FAILED_NAMES="$FAILED_NAMES\n    - lvm: growing a declared volume failed"
        echo "  FAIL  lvm: growing $_vol to 128M failed — $(tr '\n' ' ' < /tmp/resize.out)"
    fi

    # The point of the whole feature: a machine that already matches its declaration is not
    # work. A unit comparison that is wrong in either direction shows up here and nowhere else.
    _grown="$(_size_of "$_vol")"
    if lx_slow -y sync >/tmp/resize.out 2>&1; then
        if [ "$(_size_of "$_vol")" = "$_grown" ]; then
            PASS=$((PASS + 1)); echo "  PASS  lvm: a second sync over the same declaration left the volume alone"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES\n    - lvm: a converged sync resized the volume again"
            echo "  FAIL  lvm: a converged sync resized $_vol again"
        fi
    else
        soft "lvm: the converged re-sync did not exit 0 — $(tr '\n' ' ' < /tmp/resize.out)"
    fi

    # Shrinking unasked: the refusal is the feature. Asserted against the volume, not only
    # against the exit code — a command that says no and shrinks anyway is the worst outcome.
    if _resize_to 64M && lx_slow -y sync >/tmp/resize.out 2>&1; then
        FAILC=$((FAILC + 1))
        FAILED_NAMES="$FAILED_NAMES\n    - lvm: a smaller @size shrank the volume with no @allow_shrink"
        echo "  FAIL  lvm: a smaller @size was accepted with no @allow_shrink"
    elif grep -q "allow_shrink" /tmp/resize.out; then
        if [ "$(_size_of "$_vol")" = "$_grown" ]; then
            PASS=$((PASS + 1)); echo "  PASS  lvm: a smaller @size is refused by name and the volume is untouched at $_grown bytes"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES\n    - lvm: the shrink was refused and the volume changed anyway"
            echo "  FAIL  lvm: the shrink was refused and $_vol changed anyway"
        fi
    else
        FAILC=$((FAILC + 1))
        FAILED_NAMES="$FAILED_NAMES\n    - lvm: a smaller @size failed without naming @allow_shrink"
        echo "  FAIL  lvm: a smaller @size failed and the message never named @allow_shrink — $(tr '\n' ' ' < /tmp/resize.out)"
    fi

    sed -i "s|\(lvm:$_vol@size=64M\)|\1,allow_shrink=true|" "$_mod"
    if grep -q "allow_shrink=true" "$_mod" && lx_slow -y sync >/tmp/resize.out 2>&1; then
        _shrunk="$(_size_of "$_vol")"
        if [ -n "$_shrunk" ] && [ "$_shrunk" -lt "$_grown" ]; then
            PASS=$((PASS + 1)); echo "  PASS  lvm: @allow_shrink shrank $_vol, $_grown -> $_shrunk bytes"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES\n    - lvm: @allow_shrink reported success and the volume is still $_shrunk bytes"
            echo "  FAIL  lvm: @allow_shrink reported success and $_vol is still $_shrunk bytes"
        fi
    else
        # ext4 shrinks; xfs cannot, and fsadm refuses before touching the volume. Either is a
        # fact about the filesystem, so it is reported and not scored — but the volume must be
        # intact, because "refused" and "half shrunk" are the two outcomes this flag separates.
        if [ "$(_size_of "$_vol")" = "$_grown" ]; then
            soft "lvm: @allow_shrink did not shrink here and left the volume intact at $_grown bytes — $(tr '\n' ' ' < /tmp/resize.out)"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES\n    - lvm: a failed shrink left the volume changed"
            echo "  FAIL  lvm: the shrink failed and $_vol is no longer $_grown bytes"
        fi
    fi

    ok "lvm: the resize canary uninstalls" lx_slow -y uninstall "lvm:$_vol"
    _cleanup_resizer
}

storage_resize_lifecycle

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

SMOKE_CFG=/tmp/shall-it-smoke
rm -rf "$SMOKE_CFG"; mkdir -p "$SMOKE_CFG/modules" "$SMOKE_CFG/profiles"
printf 'Work\n' > "$SMOKE_CFG/active"
printf 'use base\n' > "$SMOKE_CFG/profiles/Work"
: > "$SMOKE_CFG/modules/base.txt"
: > "$SMOKE_CFG/priority"
for b in $ALL_BACKENDS; do echo "$b" >> "$SMOKE_CFG/priority"; done

smoke_lx() {
    record_argv "$@"
    env SHALL_CONFIG_DIR="$SMOKE_CFG" SHALL_DATA_DIR=/tmp/shall-it-smoke-state \
        $TO "$SHALL" "$@"
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
            printf 'link:/etc/hostname @target=/tmp/shall-it-hostname\n' > "$SMOKE_CFG/modules/base.txt"
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
# `eval` is the one output that will acquire consumers Shall cannot see, so the
# thing asserted is the contract: a top-level schema version, and valid JSON.
grep_ok "eval prints a versioned document" '"schema"' lx eval
ok "eval emits valid JSON" sh -c "$SHALL eval | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null || $SHALL eval | head -1 | grep -q '{'"
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
refuses_with_3 "try refuses when there is no container runtime" lx try
grep_ok "try's refusal names what is missing" "podman" lx try
ok "check unmanaged lists what Shall does not manage" lx check unmanaged
ok "path prints the config repo" lx path
ok "path --explain says which source won" lx path --explain
ok "config show prints the active configuration" lx config show
ok "policy checks the desired state against [guard]" lx policy
ok "check conflicts reports cross-backend conflicts" lx check conflicts
# With no event hooks declared, approvals is clean and exits 0 (not 2).
ok "check approvals is clean with no hooks" lx check approvals
# `adapters` (S78) — the eight extension surfaces. The container starts with no `adapters/`
# directory at all, which is the state almost every machine is in and the one a survey must
# report without complaining about.
grep_ok "adapters names every surface it knows" "firewall" lx adapters
grep_ok "adapters says an unextended machine has extended nothing" "extended nothing" lx adapters
grep_ok "adapters refuses a name that is not a surface, and lists the real ones" "is not an extension surface" lx adapters nosuchsurface
# **The failure a plugin system has that a built-in does not**: a file that is present,
# approved, valid TOML and read by nobody, because the array key is `backends` and the reader
# wants `backend`. Every other signal says fine. `rows in force` is the only one that does not.
mkdir -p "$SHALL_CONFIG_DIR/adapters"
printf '[[backends]]\nname = "mymgr"\n' > "$SHALL_CONFIG_DIR/adapters/backends.toml"
grep_ok "an adapter file nobody approved is reported unapproved" "unapproved" lx adapters backends
lx lock >/dev/null 2>&1
grep_ok "a table nobody opens is 'no rows', not 'in use'" "no rows" lx adapters backends
printf '[[backend]]\nname = "mymgr"\ninstall = "true --"\n' > "$SHALL_CONFIG_DIR/adapters/backends.toml"
lx lock >/dev/null 2>&1
grep_ok "a row of the right kind is in force" "in use" lx adapters backends
# Malformed degrades rather than refusing (owner ruling, 2026-08-10), and `check adapters` is
# where that fact is an exit code instead of a warning nobody re-reads.
printf 'this is not toml at all\n' > "$SHALL_CONFIG_DIR/adapters/backends.toml"
# Approved first, and that ordering is the point: `standing_of` asks II.12 before it asks the
# parser, so an unapproved file reads `unapproved` whatever is inside it. Without this line the
# check below tests the approval ledger and calls it a parse result.
lx lock >/dev/null 2>&1
grep_ok "an unreadable adapter file is reported malformed" "malformed" lx adapters backends
# The ruling: `sync` degrades rather than refusing. Asserted on the words Shall prints, not on
# exit 0 — a check that only wants exit 0 is a check a do-nothing binary passes, which is what
# the mutation gate exists to say. The exit code half is `a_malformed_adapter_does_not_refuse_a_
# sync` in the Rust suite, where it is a real assertion rather than a survivor.
grep_ok "a malformed adapter warns, naming the file, and the sync goes on" "is not in use" lx sync --dry-run
# The exit code is `a_malformed_adapter_does_not_refuse_a_sync` in the Rust suite. Here it is
# the report, because a `nok` cannot tell a refusal from a crash — which is the second thing the
# mutation gate measures, and it was right about these two as well.
grep_ok "check adapters names a file that is not in force" "not in force" lx check adapters
rm -f "$SHALL_CONFIG_DIR/adapters/backends.toml"
grep_ok "check adapters is clean once the file is gone" "extended nothing" lx check adapters
# `add` vendors a source's modules. A local path with a module is the network-free case; it
# copies the module in and reports it.
mkdir -p /tmp/shall-share/modules
printf 'apt:jq\n' > /tmp/shall-share/modules/shared.txt
ok "add vendors a module from a local source" lx add /tmp/shall-share
ok "add brought the module file in" test -f "$SHALL_CONFIG_DIR/modules/shared.txt"
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
# Not every manager can enumerate its repositories (apt has no listing command Shall
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
# `heal` when there is nothing to recover — and the check has to say WHICH, because by the time
# it runs this sweep has deliberately failed an install (section 7) and that leaves a Failed
# entry in the journal. `get_incomplete_actions` counts Failed as incomplete, so heal correctly
# reports it and exits non-zero. The old check asserted rc=0 and therefore passed or failed on
# whether an earlier deliberate failure happened to still be in the journal — a coin toss that
# came up heads on ubuntu and tails on tools and macOS in the same CI run.
#
# What W36 actually ruled is the thing worth asserting: heal NAMES what it could not recover and
# does not exit 0 while saying so. Both outcomes are legitimate here; a heal that says nothing
# and exits non-zero, or one that names a failure and exits 0, is not.
# Through `lx`, not `$TO "$SHALL"`: the wrapper is what records a subcommand as EXECUTED,
# and the first version of this check called the binary directly — so the coverage audit
# reported `heal` as only ever --help'd, which is precisely the claim this sweep exists
# to refuse. Measured: `FAIL every subcommand is executed — only --help'd: heal`.
_heal_out=$(lx heal 2>&1); _heal_rc=$?
if printf '%s' "$_heal_out" | grep -q "could not be recovered"; then
    if [ "$_heal_rc" -ne 0 ]; then
        PASS=$((PASS + 1)); echo "  PASS  heal names what it could not recover, and says so in the exit code"
    else
        hard "heal reported an unrecovered operation and exited 0 (W36)"
    fi
elif [ "$_heal_rc" -eq 0 ]; then
    PASS=$((PASS + 1)); echo "  PASS  heal had nothing to recover and said nothing"
else
    hard "heal exited $_heal_rc without naming anything it could not recover"
fi
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
ok "export writes native manifests" lx export --out /tmp/shall-it-export
# The package is PINNED to this image's native manager. An unpinned `jq` resolved to
# `cargo:jq` on an image that had cargo and no jq — a library crate that installs no
# program — so the check failed on the resolver's answer, not on `run`.
if [ -n "$SMOKE" ]; then
    skip_smoke "run's ephemeral environment (it would install the package it needs)"
else
    ok "run executes inside an ephemeral environment" lx run -p "$BACKEND:$PKG" true
fi

# plan → apply: the frozen plan is the one that gets applied.
ok "plan freezes a reviewable file" lx plan --out /tmp/shall-it-plan.json
ok "the plan file exists" test -f /tmp/shall-it-plan.json
ok "apply reads a saved plan" lx --dry-run apply /tmp/shall-it-plan.json

# `edit` shells out to $VISUAL/$EDITOR; `true` is an editor that exits 0.
record_argv edit priority
ok "edit opens a file in \$EDITOR" env EDITOR=true VISUAL=true $TO "$SHALL" edit priority

# reset deletes the registry. The command is exercised through the refusal it owes
# a machine that still has a config repo — running it for real would end the run.
refuses_with_3 "reset refuses while a config repo still exists" lx reset
grep_ok "and says --force is what overrides it" "force" lx reset

# self-upgrade --check only reports; it rebuilds nothing.
ok "self-upgrade --check reports the version and source" lx self-upgrade --check

# --- 16b. bundle → restore, the round trip (V.59) -------------------------
echo "[16b] bundle → restore"
rm -rf /tmp/shall-it-bundle /tmp/shall-it-restored
ok "bundle packs the config" lx bundle --out /tmp/shall-it-bundle
ok "the bundle directory exists" test -d /tmp/shall-it-bundle
mkdir -p /tmp/shall-it-restored
ok "restore into a clean config directory" \
    env SHALL_CONFIG_DIR=/tmp/shall-it-restored SHALL_DATA_DIR=/tmp/shall-it-restored-state \
        $TO "$SHALL" restore /tmp/shall-it-bundle
record_argv restore /tmp/shall-it-bundle
answers "the restored model parses" \
    env SHALL_CONFIG_DIR=/tmp/shall-it-restored SHALL_DATA_DIR=/tmp/shall-it-restored-state \
        $TO "$SHALL" check
refuses_with_3 "restore refuses a config directory that is not empty" \
    env SHALL_CONFIG_DIR=/tmp/shall-it-restored SHALL_DATA_DIR=/tmp/shall-it-restored-state \
        $TO "$SHALL" restore /tmp/shall-it-bundle
ok "and --force overrides it" \
    env SHALL_CONFIG_DIR=/tmp/shall-it-restored SHALL_DATA_DIR=/tmp/shall-it-restored-state \
        $TO "$SHALL" restore /tmp/shall-it-bundle --force

# --- 16c. `--help` for the whole surface ----------------------------------
# Kept, but demoted: it catches a subcommand whose clap wiring is broken, and the
# audit below does not accept it as coverage.
echo "[16c] --help across the surface"
HELP_CMDS=$($SHALL --help 2>&1 | sed -n '/^Commands:/,/^Options:/p' \
    | sed -n 's/^  \([a-z][a-z-]*\) .*/\1/p' | grep -v '^help$' | sort -u)
for c in $HELP_CMDS; do
    ok "\`$c --help\` exists" lx "$c" --help
done

# ==========================================================================
# 16d. A REAL CRASH IN THE MIDDLE OF A TRANSACTION (GRADER §5)
# ==========================================================================
# The journal is a write-ahead log and nothing had ever killed Shall while one was open.
# `heal` was driven only over a journal left by an ORDINARY run — the one state a WAL is not
# for — and the 2026-08-04 grade named it directly: *"the WAL's entire reason to exist,
# untested under a real crash."*
#
# `kill -9`, never `-15`: a SIGTERM runs whatever Shall does on the way out, which is the
# graceful path and is what every other check here already exercises. Only SIGKILL leaves the
# state this section is about — an entry on disk that says a package is being installed and a
# process that will never come back to say how it went.
echo "[16d] SIGKILL mid-transaction, then heal"

JOURNAL="$SHALL_DATA_DIR/journal.jsonl"

# How many operations are still OPEN — and the emphasis is the whole point.
#
# `journal.jsonl` is APPEND-ONLY: one line per state change, carrying the same id each time, so
# a single successful install writes `InProgress` and then `Completed` and both lines stay.
# Counting `InProgress` lines therefore answers "how many operations ever started", which on a
# healthy run is every one of them. The first draft of this section did exactly that and
# reported **32 operations open** on a run where heal had resolved all of them — a finding
# manufactured by the instrument. Measured and corrected 2026-08-04: the same id appears as
# `InProgress`, `Failed` and `Completed` in one file.
#
# So the question is about the LAST line for each id, which is what `get_incomplete_actions`
# reads. `InProgress`/`Abandoned` is "open"; `Failed` is resolved-and-retryable and is counted
# separately, because heal treats them differently and so must this.
journal_status_tally() { # awk-condition over the final status of each id
    [ -f "$JOURNAL" ] || { echo 0; return 0; }
    sed -n 's/.*"id":"\([^"]*\)".*"status":"\([^"]*\)".*/\1 \2/p' "$JOURNAL" \
        | awk -v want="$1" '
            { last[$1] = $2 }
            END { n = 0; for (k in last) if (index(want, last[k]) > 0) n++; print n + 0 }'
}
journal_open()       { journal_status_tally "InProgress Abandoned"; }
journal_incomplete() { journal_status_tally "InProgress Abandoned Failed"; }
# Operations the log has closed. The `completed` iteration below kills the run once this rises,
# which is the window neither of the other two can reach: an install the log already calls done,
# in a process that has not yet written the ownership registry.
journal_completed()  { journal_status_tally "Completed"; }
# The names behind the number, for a failure message that can be acted on.
journal_open_names() {
    [ -f "$JOURNAL" ] || return 0
    sed -n 's/.*"id":"\([^"]*\)".*"status":"\([^"]*\)".*/\1 \2/p' "$JOURNAL" \
        | awk '{ last[$1] = $2 } END { for (k in last) if (last[k] == "InProgress" || last[k] == "Abandoned") print "        | " k }'
}

# Candidates: package name == binary name, present in the repos of all six distros this
# harness images, and none of them a section-14 canary. `figlet` was the first draft and it is
# `nix`'s canary on the tools image — two canaries sharing a binary name is the G-3 collision,
# and this section's cleanup would then be deciding nix's result.
#
# FILTERED against the machine rather than assumed: `pv` in particular ships pre-installed on
# some images, and a "crash during the install of something already installed" is no install
# at all. What is left is what this host can actually make into transaction steps.
CRASH_PKGS=""
for _c in pv dos2unix ncdu; do
    on_path "$_c" || CRASH_PKGS="$CRASH_PKGS $_c"
done
CRASH_N=0
for _c in $CRASH_PKGS; do CRASH_N=$((CRASH_N + 1)); done

# Asked, not assumed: busybox `sleep` takes a fraction and some builtins do not. A poll that
# silently rounds up to a whole second walks straight past the window this section aims at.
CRASH_POLL=0.1
sleep 0.1 2>/dev/null || CRASH_POLL=1

crash_declare() {
    for _p in $CRASH_PKGS; do
        grep -qx "$BACKEND:$_p" "$IMPERATIVE" 2>/dev/null || echo "$BACKEND:$_p" >> "$IMPERATIVE"
    done
}
crash_undeclare() {
    [ -f "$IMPERATIVE" ] || return 0
    for _p in $CRASH_PKGS; do
        grep -v -x "$BACKEND:$_p" "$IMPERATIVE" > "$IMPERATIVE.tmp" 2>/dev/null
        mv "$IMPERATIVE.tmp" "$IMPERATIVE"
    done
}
# The machine, asked directly. Never `shall list` — that is the program on trial.
crash_installed() { _n=0; for _p in $CRASH_PKGS; do on_path "$_p" && _n=$((_n + 1)); done; echo "$_n"; }
crash_missing()   { _m=""; for _p in $CRASH_PKGS; do on_path "$_p" || _m="$_m $_p"; done; echo "$_m"; }
# UNINSTALL FIRST, undeclare second — and the order is not cosmetic.
#
# `shall uninstall` refuses a package no active file declares: *"nothing was uninstalled:
# `apt:ncdu` is not declared in any active file."* That is the product being careful, and the
# first version of this function took the line out first and then asked for the removal, so
# every cleanup refused, three packages stayed installed, and the two crash iterations after it
# had no work to interrupt and quietly measured nothing. One inverted pair of lines cost two
# thirds of this section's coverage.
#
# The uninstall output is KEPT for the same reason: without it, a cleanup that left three
# packages behind could only report that it had.
# Putting a package manager back together after its own transaction was killed mid-write. The
# command is the one the manager itself asks for, and it is per-manager because the state it
# repairs is: dpkg has a half-configured package list, rpm has a stale database lock.
repair_manager() {
    case "$BACKEND" in
        apt)          dpkg --configure -a >/tmp/repair.out 2>&1; return 0 ;;
        dnf|zypper)   rpm --rebuilddb >/tmp/repair.out 2>&1; return 0 ;;
        pacman)       rm -f /var/lib/pacman/db.lck >/tmp/repair.out 2>&1; return 0 ;;
        apk|xbps)     return 0 ;;   # both write atomically; there is nothing half-written to fix
        *)            return 1 ;;
    esac
}

crash_wipe() {
    : > /tmp/crash-wipe.out
    for _p in $CRASH_PKGS; do
        on_path "$_p" || continue
        # The declaration state BEFORE the attempt. `uninstall` refuses a package no active file
        # declares, so "did it remove it" and "was it there to remove" are two different
        # questions and a failure that cannot tell them apart is a failure nobody can act on.
        {
            echo "--- uninstall $BACKEND:$_p"
            echo "    declared in imperative.txt: $(grep -cx "$BACKEND:$_p" "$IMPERATIVE" 2>/dev/null || echo 0)"
            echo "    active modules: $(tr '\n' ' ' < "$SHALL_CONFIG_DIR/active" 2>/dev/null)"
        } >> /tmp/crash-wipe.out
        $TO "$SHALL" -y uninstall "$BACKEND:$_p" >> /tmp/crash-wipe.out 2>&1
        echo "    rc=$? and $_p is now $(on_path "$_p" && echo 'STILL on PATH' || echo 'gone')" >> /tmp/crash-wipe.out
    done
    crash_undeclare
}

# crash_run <tag> <when> [group]
#   when   `open`       kill the moment the log has an entry — the manager may not have started
#          <n>          kill once n of the canaries have reached the filesystem
#          `completed`  kill once the log has CLOSED an operation this run opened
#   group  non-empty kills the process GROUP, so the package manager dies mid-write too
#
# The third one is `S87`'s window, and it is not a variation on the other two. Ownership is
# held in memory through a sync and written to `registry.json` once, at the end; the log is
# written per operation. A kill between those two leaves the package installed, `Completed` in
# the log — so recovery has nothing to replay — and owned by nobody, and the one command for
# removing it then plans no change and reports success. Polling the filesystem cannot aim at
# that window: a canary reaches disk well before its entry closes, so `midway` always killed
# too early, and twelve iterations of the other two produced it zero times.
crash_run() {
    _tag="$1"; _when="$2"; _grp="${3:-}"
    _open_before=$(journal_open)
    _done_before=$(journal_completed)
    crash_declare
    record_argv sync

    # No `timeout` wrapper. Killing the wrapper leaves Shall running and this section would
    # then be measuring an orphan; the spin budget below is the bound instead.
    if [ -n "$_grp" ]; then
        setsid $SHALL -y sync >"/tmp/crash-$_tag.out" 2>&1 &
    else
        $SHALL -y sync >"/tmp/crash-$_tag.out" 2>&1 &
    fi
    _pid=$!

    _spins=0
    while [ "$_spins" -lt 1800 ]; do
        if [ "$_when" = open ]; then
            [ "$(journal_open)" -gt "$_open_before" ] && break
        elif [ "$_when" = completed ]; then
            [ "$(journal_completed)" -gt "$_done_before" ] && break
        else
            [ "$(crash_installed)" -ge "$_when" ] && break
        fi
        kill -0 "$_pid" 2>/dev/null || break
        sleep "$CRASH_POLL"; _spins=$((_spins + 1))
    done

    _open_at_kill=$(journal_open)
    _done_at_kill=$(journal_completed)
    if [ -n "$_grp" ]; then
        kill -9 "-$_pid" 2>/dev/null
    fi
    kill -9 "$_pid" 2>/dev/null
    wait "$_pid" 2>/dev/null

    # The DELTA, not the total. The first draft compared against zero and passed on entries
    # that were already open before this iteration started — so it would have reported a crash
    # it never caused. What this iteration is answerable for is what it added.
    _opened=$((_open_at_kill - _open_before))
    _closed=$((_done_at_kill - _done_before))
    if [ "$_when" = completed ]; then
        # This iteration is answerable for a CLOSED operation, not an open one — a kill that
        # leaves nothing open is exactly what it is aiming at. Measuring it by `_opened` would
        # skip the run every time it worked.
        if [ "$_closed" -lt 1 ]; then
            soft "crash/$_tag: the kill closed no operation in the write-ahead log ($_done_before before, $_done_at_kill after), so this iteration measured nothing"
            crash_wipe
            return 0
        fi
        PASS=$((PASS + 1))
        echo "  PASS  crash/$_tag: SIGKILL landed with $_closed operation(s) closed in the write-ahead log and $_opened still open, with $(crash_installed) of $CRASH_N canaries on disk"
    elif [ "$_opened" -lt 1 ]; then
        # Honest, and deliberately NOT a pass. The kill landed outside a transaction, so
        # nothing here exercised recovery — scoring it green is the vacuous check IV.1 exists
        # to refuse.
        soft "crash/$_tag: the kill opened no new entry in the write-ahead log ($_open_before before, $_open_at_kill after), so this iteration measured no recovery"
        crash_wipe
        return 0
    else
        PASS=$((PASS + 1))
        echo "  PASS  crash/$_tag: SIGKILL left $_opened newly-opened operation(s) in the write-ahead log ($_open_at_kill open in all), with $(crash_installed) of $CRASH_N canaries on disk"
    fi

    _hout=$(lx heal 2>&1); _hrc=$?

    # (1) heal's exit code answers for its own failures. W36's rule, and this is the only
    #     place in the sweep where a recovery can genuinely fail, so it is the only place
    #     that check has ever been able to mean anything.
    if printf '%s' "$_hout" | grep -q "could not be recovered"; then
        if [ "$_hrc" -ne 0 ]; then
            PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: heal named what it could not recover and said so in the exit code"
        else
            hard "crash/$_tag: heal reported an unrecovered operation and exited 0 (W36)"
        fi
    elif [ "$_hrc" -eq 0 ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: heal recovered the interrupted operation(s)"
    else
        hard "crash/$_tag: heal exited $_hrc without naming anything it could not recover"
    fi

    # (2) It says so in the user's words. `Some(CommandFailed { retry: Permanent,
    #     absent_name: true })` is the journal's vocabulary and W36 found it printed at a
    #     person; no ordinary run can reach that branch, which is why it survived.
    if printf '%s' "$_hout" | grep -q 'CommandFailed {\|absent_name:\|retry: Permanent\|retry: Transient'; then
        hard "crash/$_tag: heal printed the journal's own struct at the user — $(printf '%s' "$_hout" | grep -o 'CommandFailed {\|absent_name:\|retry: [A-Za-z]*' | head -1)"
    else
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: heal's report is in the user's words, not the journal's"
    fi

    # (3) Nothing is left open that heal did not name. An entry that stays InProgress is a
    #     legitimate outcome — heal must not close what it could not verify — but silence
    #     over one is how a machine and its log disagree for ever.
    _still=$(journal_open)
    if [ "$_still" -gt 0 ] && ! printf '%s' "$_hout" | grep -q "could not be recovered"; then
        hard "crash/$_tag: $_still operation(s) are still open after heal (this crash opened $_opened of them), and heal named none of them"
        journal_open_names | head -5
    else
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: nothing is open in the log that heal did not name"
    fi

    # (4) The model still parses. A crash that wedges the config is E1's family one layer down.
    answers "crash/$_tag: the model still parses after the crash" lx check

    # (4b) **Every canary the crash left on the machine is under management.** `S87`: the
    #      ownership registry is written once, at the end of a run, and the log is written per
    #      operation — so a kill in between leaves a package installed and owned by nobody.
    #      Nothing downstream notices. The sync converges (the package IS installed), the
    #      preview plans nothing (there is nothing to plan), and the damage only shows up when
    #      somebody tries to remove it, three checks later, as a cleanup that reports success
    #      and takes nothing away. Asked here, where the answer still names a cause.
    _unowned=""
    for _p in $CRASH_PKGS; do
        on_path "$_p" || continue
        lx why "$BACKEND:$_p" 2>&1 | grep -q "not under Shall management" && _unowned="$_unowned $_p"
    done
    if [ -z "$_unowned" ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: every canary the crash left on the machine is under Shall management"
    else
        hard "crash/$_tag: the crash left$_unowned installed and under nobody's management, so the command for removing them will report success and take nothing away"
    fi

    # (5) The promise the whole loop exists to test: the next sync converges onto the
    #     declaration, whatever the crash left behind.
    if lx_slow -y sync >"/tmp/crash-conv-$_tag.out" 2>&1 && [ "$(crash_installed)" -eq "$CRASH_N" ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: the sync after the crash converged onto all $CRASH_N canaries"

        # (6) ...and there is nothing left to do. A crash that leaves permanent phantom drift is
        #     indistinguishable from a converged machine until you ask twice, which is the half
        #     a single re-sync cannot see.
        #
        #     Asked of the PLAN, not of a phrase. The first version grepped for "already up to
        #     date" and went red the day `sudo` joined the image — a converged sync that also
        #     reports one protected package it left alone prints a different sentence and has
        #     done nothing wrong. What "nothing left to do" means is zero planned changes.
        if lx_slow -y sync >"/tmp/crash-idem-$_tag.out" 2>&1; then
            lx --dry-run sync >"/tmp/crash-plan-$_tag.out" 2>&1
            # A preview with no `Planned changes` block planned nothing. It is asserted that way
            # rather than by a phrase because a converged machine that also has one unmanaged
            # protected package prints the "Left in place" report and no "already up to date" —
            # correct on both counts, and the second version of this check went red on it.
            if ! grep -q "Planned changes" "/tmp/crash-plan-$_tag.out" \
               || grep -qi "already up to date\|nothing to do" "/tmp/crash-plan-$_tag.out" \
               || grep -q "install 0 *remove 0" "/tmp/crash-plan-$_tag.out"; then
                PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: and the preview after that plans no change at all"
            else
                hard "crash/$_tag: a converged machine still has a plan — $(grep -i 'install\|remove' "/tmp/crash-plan-$_tag.out" | head -2 | tr '\n' ' ')"
            fi
        else
            hard "crash/$_tag: the sync after convergence failed"
            excerpt "/tmp/crash-idem-$_tag.out" 6
        fi
    elif [ -n "$_grp" ]; then
        # The package manager itself was killed mid-write. Shall cannot undo that, and the
        # contract here is smaller and still real: it must say what is wrong in words a person
        # can act on, and not with a panic. Silence and a stack trace are the two failures.
        if grep -qi "interrupt\|dpkg\|database\|lock\|run.*configure\|repair" "/tmp/crash-conv-$_tag.out"; then
            PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: the manager was left broken and Shall named the manager's own state"
        elif grep -q "panicked at\|RUST_BACKTRACE" "/tmp/crash-conv-$_tag.out"; then
            hard "crash/$_tag: killing the package manager mid-write made Shall panic"
            excerpt "/tmp/crash-conv-$_tag.out" 6
        else
            hard "crash/$_tag: the sync after a mid-write kill neither converged nor said why"
            excerpt "/tmp/crash-conv-$_tag.out" 6
        fi
    else
        hard "crash/$_tag: the sync after the crash did not converge — still missing:$(crash_missing)"
        excerpt "/tmp/crash-conv-$_tag.out" 6
        # **What the manager's own lock was doing at the time** (`Q50`). The commonest cause of
        # this failure is a lock the killed run left behind, and `heal` clears one it can prove
        # nobody holds — so when the sync still does not converge, the two facts a reader needs
        # are whether a lock is there and whether anything is holding it. Printed here rather
        # than reasoned about: this failure was diagnosed twice from a passing local run and the
        # answer was wrong both times.
        for _lk in /var/lib/pacman/db.lck /var/cache/dnf/metadata_lock.pid /run/zypp.pid; do
            [ -e "$_lk" ] && echo "        lock still present: $(ls -l "$_lk" 2>&1)"
        done
        for _d in /proc/[0-9]*; do
            _c=$(cat "$_d/comm" 2>/dev/null) || continue
            case "$_c" in
                pacman|dnf|zypper)
                    echo "        a $_c is running: $_d state=$(awk '{print $3}' "$_d/stat" 2>/dev/null)" ;;
            esac
        done
        # And what `heal` said when it ran, a few lines earlier. It is captured into `$_hout`
        # for three assertions and then dropped, so the one command whose job is to undo what
        # the crash did was the only step of this sequence with no visible output at all.
        printf '%s
' "$_hout" | sed 's/^/        heal said: /' | head -12
    fi

    # The group kill leaves the PACKAGE MANAGER half-written, which is the whole point of it —
    # and Shall's own message says how to put that right (`dpkg was interrupted, you must
    # manually run 'dpkg --configure -a'`). Following that sentence is itself the check: advice
    # that does not work is worse than no advice.
    #
    # It also has to happen, or every section after this one inherits a machine whose package
    # manager cannot install anything. On the run that first got this far, one deliberate kill
    # turned four later checks red across three sections for a reason that was nothing to do
    # with them — a harness that breaks the machine on purpose owes it a repair.
    if [ -n "$_grp" ]; then
        if repair_manager && lx_slow -y sync >/tmp/crash-repair.out 2>&1; then
            PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: the repair Shall named put the manager back, and the next sync worked"
        elif repair_manager; then
            hard "crash/$_tag: the manager's own repair ran and the sync after it still failed"
            excerpt /tmp/crash-repair.out 6
        else
            soft "crash/$_tag: no repair command is known for \`$BACKEND\`, so the machine stays as the kill left it and the checks after this inherit it"
        fi
    fi

    crash_wipe
    _left=$(crash_installed)
    if [ "$_left" -eq 0 ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: the canaries are off the machine again"
    else
        # A removal that reports success and leaves the file is the worst outcome this whole
        # section can produce, so it is hard and it prints what the removal actually said.
        hard "crash/$_tag: the cleanup uninstall left $_left of$CRASH_PKGS still on PATH"
        # The WHOLE log, not a tail. A ten-line tail of three uninstalls shows the end of the
        # last one and nothing about which of them refused or why — which is how one inverted
        # pair of lines in this function cost two thirds of this section's coverage and took
        # two full runs to name.
        sed 's/^/        | /' /tmp/crash-wipe.out
        # **And WHO OWNS the ones left behind.** `already up to date` over an installed package
        # has exactly two causes and the wipe log cannot tell them apart: Shall believes the
        # package is not installed, or Shall believes it is not managed. `why` answers the second
        # directly — the ownership record is what recovery is documented to have missed once
        # before (`sync/mod.rs`, the ledger note on the recovery path), and on the run that
        # produced this branch it was the first question anyone asked and the one fact the log
        # did not carry.
        for _p in $CRASH_PKGS; do
            on_path "$_p" || continue
            echo "        ? why $BACKEND:$_p"
            $TO "$SHALL" why "$BACKEND:$_p" 2>&1 | sed 's/^/        ? /' | head -6
        done
    fi
}

# Before anything is killed: what does the log look like after an ordinary run?
#
# Nothing above this line crashed. Every sync, install and uninstall in sections 1–16 either
# finished or failed, and both outcomes close their entry — so a `Completed`/`Failed` log is
# what a clean run should leave, and every `InProgress` in it is an operation that reached a
# manager and was never resolved. `needs_recovery()` keys on exactly that, which means a leak
# here makes `heal` permanently believe it has work.
#
# It is checked HERE rather than inside the loop because otherwise the crash checks inherit it
# and get the blame: the first draft of this section reported "23 operations open" as the
# crash's doing, and 23 of them were already there.
_baseline_open=$(journal_open)
_baseline_total=$(journal_status_tally "InProgress Abandoned Failed Completed")
if [ "$_baseline_total" -lt 1 ] && [ -n "$SMOKE" ]; then
    # **The premise of the check below is false on a SMOKE_ONLY image.** It reads "sixteen
    # sections of installs and removals have run above this line", and on gentoo none of them
    # did — Portage builds from source, so that image installs and removes nothing by design.
    # An empty write-ahead log there is the correct state, and calling it a hard failure is a
    # sentence about a run that never happened. It failed every scheduled night for at least a
    # week on exactly this.
    skip_smoke "journal: the write-ahead log is empty, and there is nothing to audit"
elif [ "$_baseline_total" -lt 1 ]; then
    # An audit of an empty set passes without examining anything — the same collapse
    # `too_few_to_audit` exists for. Sixteen sections of installs and removals have run above
    # this line, so an empty log means the binary under test never recorded an operation, and
    # "nothing is open" is then true of a program that did nothing at all. Measured: this was
    # the one check in this round that survived a shall which fails everything.
    hard "journal: the write-ahead log has no entries at all after sixteen sections of installs and removals — nothing recorded an operation, so there is nothing to audit"
elif [ "$_baseline_open" -eq 0 ]; then
    PASS=$((PASS + 1)); echo "  PASS  journal: an ordinary run left nothing open in the write-ahead log ($_baseline_total recorded, $(journal_incomplete) failed-and-retryable)"
else
    hard "journal: $_baseline_open operation(s) are still open in the write-ahead log and nothing crashed — every command in this run either finished or failed, and both close their entry"
    journal_open_names | head -5
fi

# An entry `heal` cannot act on AT ALL, which is the branch a crash cannot produce on its own.
#
# The recovery loop is `if let Some(cap) = registry.get(&backend) { if let Some(handler) =
# cap.as_installable() { … } }` — two nested `if let`s and **no `else` on either**. An entry
# naming a manager this machine does not have is therefore neither recovered, nor failed, nor
# mentioned, and `heal` returns Ok. That is W36's finding one branch over: W36 was "says it
# failed and exits 0", this is "says nothing and exits 0".
#
# Built from a REAL journal line with its backend renamed, never hand-written: an `Install`
# entry that omits `options` lands in the corrupt-log branch instead, which is a different
# check answering a question nobody asked.
_ghost="$(grep '"action":{"Install"' "$JOURNAL" 2>/dev/null | tail -1)"
if [ -z "$_ghost" ]; then
    soft "heal: no real install entry to build an unreachable one from, so the silent-skip branch was not driven"
else
    printf '%s\n' "$_ghost" \
        | sed -e 's/"id":"[^"]*"/"id":"shallnosuchmgr:ghost:00000000000000000000000000000001"/' \
              -e 's/"backend":"[^"]*"/"backend":"shallnosuchmgr"/g' \
              -e 's/"name":"[^"]*"/"name":"ghost"/' \
              -e 's/"status":"[^"]*"/"status":"InProgress"/' \
        >> "$JOURNAL"
    _hout=$(lx heal 2>&1); _hrc=$?
    if printf '%s' "$_hout" | grep -q "shallnosuchmgr"; then
        PASS=$((PASS + 1)); echo "  PASS  heal: an operation it cannot act on is named rather than skipped in silence"
    else
        hard "heal: an entry naming a manager this machine does not have was skipped without a word (rc=$_hrc)"
        printf '%s\n' "$_hout" | tail -4 | sed 's/^/        | /'
    fi
    if [ "$_hrc" -ne 0 ]; then
        PASS=$((PASS + 1)); echo "  PASS  heal: and it says so in the exit code rather than reporting success"
    else
        hard "heal: an operation was left unresolved and heal exited 0 — \`shall heal && echo ok\` prints ok (W36's family)"
    fi
    # Taken back out AFTER the assertions, never before one. The scrub that runs first and then
    # asserts absence is E2, and this harness deleted its last one for that reason.
    grep -v shallnosuchmgr "$JOURNAL" > "$JOURNAL.tmp" 2>/dev/null
    mv "$JOURNAL.tmp" "$JOURNAL"
fi

if [ -n "$SMOKE" ]; then
    skip_smoke "the crash-and-heal loop (it installs and removes real packages)"
elif [ "$CRASH_N" -lt 2 ]; then
    soft "crash/heal: this image already has pv, dos2unix and ncdu, so a sync over them is not a multi-step transaction — named rather than run vacuously"
else
    # The control. If this cannot converge the fixture is wrong, and every iteration below
    # would be measuring the fixture instead of the write-ahead log.
    crash_declare
    if lx_slow -y sync >/tmp/crash-control.out 2>&1 && [ "$(crash_installed)" -eq "$CRASH_N" ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/heal: the control sync installs all $CRASH_N canaries ($CRASH_PKGS)"
        crash_wipe

        # The log is open and the manager has done nothing yet.
        crash_run open open
        # The manager is part-way through: at least one canary has reached the filesystem.
        crash_run midway 1
        # And the one neither of those can reach: the log has CLOSED an operation, and the run
        # has not yet written down what it owns (`S87`).
        crash_run completed completed
        # And the hostile one — the manager dies too, mid-write. `setsid` is what puts Shall
        # in a group of its own so the kill reaches the child; an image without it says so
        # rather than quietly running the gentler test twice.
        if on_path setsid; then
            crash_run groupkill 1 group
        else
            soft "crash/groupkill: this image has no \`setsid\`, so Shall cannot be put in a process group of its own and the package manager cannot be killed with it"
        fi
    else
        soft "crash/heal: the control sync did not install$CRASH_PKGS on this image, so the crash loop has no fixture — $(tr '\n' ' ' < /tmp/crash-control.out | tail -c 300)"
        crash_wipe
    fi
fi

# ==========================================================================
# 16e. TWO RUNS AT ONCE, AND KILLING THE ONE THAT HOLDS THE LOCK (GRADER §6)
# ==========================================================================
# `DataLock` is an OS lock on an open handle and `main.rs` waits 120s for it. Both facts were
# asserted only by unit tests inside one process, which is the one place a file lock cannot
# fail: `try_lock_exclusive` on a handle this process already holds is a different code path
# from a second *process* arriving. Nothing had ever started two Shalles.
#
# The third check is the one with teeth. The lock is released by the kernel when the holder
# dies, and the *stamp* beside it is written by a `Drop` that SIGKILL never runs — so after a
# crash the file on disk still names a process that no longer exists. A wait driven off that
# file rather than off the lock would hang for two minutes on a corpse.
echo "[16e] Two runs at once, and killing the lock holder"

LOCKFILE="$SHALL_DATA_DIR/shall.lock"
LOCKOWNER="$SHALL_DATA_DIR/shall.lock.owner"

# The elapsed-time oracle. `date +%s` is whole seconds everywhere, including busybox.
since() { echo $(( $(date +%s) - $1 )); }

if [ -n "$SMOKE" ]; then
    skip_smoke "the two-writers checks (they need a mutating command to hold the lock)"
elif ! on_path flock; then
    # Named, not skipped silently. Without `flock` the holder cannot be made deterministic and
    # every check below would be a race dressed up as an assertion.
    soft "two-writers: this image has no \`flock\`, so a second writer cannot be held against a lock this harness controls"
else
    # --- (a) a second writer waits, and says who it is waiting for -----------
    # `flock` takes the same OS lock on the same file Shall does, so the holder is this
    # harness rather than a second Shall whose lifetime is a guess.
    flock "$LOCKFILE" -c 'sleep 12' &
    _holder=$!
    sleep 1
    _t0=$(date +%s)
    $TO "$SHALL" -y sync >/tmp/two-writers.out 2>&1
    _rc=$?
    _waited=$(since "$_t0")
    kill -9 "$_holder" 2>/dev/null; wait "$_holder" 2>/dev/null

    if grep -q "waiting for the data directory" /tmp/two-writers.out; then
        PASS=$((PASS + 1)); echo "  PASS  two-writers: the second run announced the wait instead of going quiet"
    else
        hard "two-writers: a second writer met a held lock and said nothing about waiting"
        excerpt /tmp/two-writers.out 6
    fi
    # It waited for the holder rather than walking past it. Under 8s means it did not wait for
    # a 12s holder at all, which is the failure this check exists for — a lock that is taken
    # and not honoured is worse than no lock, because the message says it is safe.
    if [ "$_waited" -ge 8 ]; then
        PASS=$((PASS + 1)); echo "  PASS  two-writers: it waited ${_waited}s for the holder rather than writing alongside it"
    else
        hard "two-writers: the second run got past a held lock in ${_waited}s (the holder was to keep it for 12s)"
    fi
    # And it got through in the end: a wait that turns into a permanent refusal is a
    # different bug wearing the same message.
    #
    # The `_waited` clause is not decoration. A binary that returns instantly also exits 0, so
    # "it succeeded" alone passes for a program that never took the lock at all — measured, it
    # was one of five checks here that survived a do-nothing binary. Succeeding is only the
    # right answer if it waited first.
    if { [ "$_rc" -eq 0 ] || [ "$_rc" -eq 2 ]; } && [ "$_waited" -ge 8 ]; then
        PASS=$((PASS + 1)); echo "  PASS  two-writers: and it proceeded once the lock was free (rc=$_rc, after ${_waited}s)"
    elif [ "$_rc" -eq 0 ] || [ "$_rc" -eq 2 ]; then
        hard "two-writers: it succeeded in ${_waited}s without ever waiting for the holder"
    else
        hard "two-writers: the second run never recovered after the holder released (rc=$_rc)"
        excerpt /tmp/two-writers.out 6
    fi

    # --- (b) and (c) share one holder, and it is a REAL Shall ----------------
    # The first draft of (c) held the lock with `flock … -c 'sleep 300'` and killed the `flock`
    # process. `sleep` is its CHILD and inherits the open descriptor, so the lock was still held
    # by a live process — Shall waited its whole 120s and said so, correctly, and the check
    # scored the product for the harness's mistake. It is the same trap the release notes
    # already record about killing a wrapper instead of the script. So the holder is Shall
    # itself, which is also the only holder whose stamp is worth reading.
    #
    # The holder is given WORK, and is then killed the instant its stamp appears. A sync with
    # nothing to do was the first design and it finished before the poll could catch it holding
    # — the check reported "no holder to measure" and proved nothing. The stamp is written when
    # the lock is taken, which is before planning, so the kill still lands well before a package
    # manager is spawned: a holder killed mid-`apt` would leave *dpkg's* lock behind and the run
    # after it would fail for a reason that is not the one under test.
    rm -f "$LOCKOWNER"
    crash_declare
    $SHALL -y sync >/tmp/lock-holder.out 2>&1 &
    _holder=$!
    _spins=0
    while [ "$_spins" -lt 600 ] && [ ! -s "$LOCKOWNER" ]; do
        kill -0 "$_holder" 2>/dev/null || break
        sleep "$CRASH_POLL"; _spins=$((_spins + 1))
    done

    if [ ! -s "$LOCKOWNER" ] || ! kill -0 "$_holder" 2>/dev/null; then
        soft "lock: the holder finished before it could be caught holding, so the by-pid and killed-holder checks had nothing to measure"
        kill -9 "$_holder" 2>/dev/null; wait "$_holder" 2>/dev/null
    else
        _stamp="$(cat "$LOCKOWNER")"
        # It names the command and the pid, not "another shall". A wait with no name is
        # indistinguishable from a hang, which is the whole reason the stamp file exists.
        if printf '%s' "$_stamp" | grep -q "pid $_holder"; then
            PASS=$((PASS + 1)); echo "  PASS  lock: the holder published its own command and pid — $_stamp"
        else
            hard "lock: the holder's stamp does not name pid $_holder — it says '$_stamp'"
        fi

        # --- (c) SIGKILL the holder ------------------------------------------
        # `Drop` never runs, so `shall.lock.owner` outlives the process that wrote it. The lock
        # itself is an OS lock on an open handle and the kernel drops it — so if anything ever
        # decided to wait by reading that FILE, this is where it costs two minutes.
        kill -9 "$_holder" 2>/dev/null; wait "$_holder" 2>/dev/null
        if [ -s "$LOCKOWNER" ]; then
            PASS=$((PASS + 1)); echo "  PASS  lock: the stamp outlived the process that wrote it, which is the state under test"
        else
            soft "lock: the stamp was already gone after the kill, so the corpse case below is weaker than intended"
        fi

        # **The subject here is Shall's own lock, so the package manager's is cleared first.**
        # Killing a holder mid-sync also orphans the `pacman` it had started, which keeps its own
        # `db.lck` and then leaves it behind — so the sync below failed on *that* lock and this
        # check reported it as a Shall lock that was not released. A check that can fail for a
        # reason unrelated to its own sentence proves nothing when it passes either. `heal` is
        # the command whose job that repair is (II.50), and it is untimed: what is being measured
        # is the sync, and specifically that it does not wait 120s on a corpse.
        $TO "$SHALL" -y heal >/tmp/lock-corpse-heal.out 2>&1 || true
        _t0=$(date +%s)
        $TO "$SHALL" -y sync >/tmp/lock-corpse.out 2>&1
        _rc=$?
        _took=$(since "$_t0")
        # **What must not happen is a wait on the DATA DIRECTORY**, and that is what is
        # asserted — not a stopwatch, and NOT the exit code. A killed holder also orphans the
        # package manager it had started, and the next run legitimately waits for that manager
        # to finish (II.51), announcing it as it goes. On opensuse that was 62 seconds of
        # correct behaviour, and a bare `>= 30` reported it as "the stale stamp file was
        # believed over the lock" — a sentence about a mechanism that had not run. The wait
        # Shall is forbidden from making here is the one on its own lock, and it says which
        # wait it is making.
        #
        # **The exit code is asked LAST, and under its own name.** It used to be asked first,
        # so any failure of the sync — a package that no longer exists upstream, a repository
        # that was down — was reported as `lock: a run after a killed holder failed … instead
        # of taking the free lock`. The macOS nightly failed exactly that way for six nights
        # over a version pin naming a Homebrew formula that does not exist: the lock was taken
        # correctly, and the check that reported a lock defect was the only red line in the run.
        # A check whose name and whose cause are unrelated is the defect `GRADER.md` exists to
        # catch. The lock question and the sync question are separate questions and now have
        # separate sentences.
        if grep -q "waiting for the data directory" /tmp/lock-corpse.out 2>/dev/null; then
            hard "lock: the next run waited on the data directory after the holder was killed — the stale stamp file was believed over the lock (${_took}s)"
            excerpt /tmp/lock-corpse.out 6
        elif [ "$_took" -ge 120 ]; then
            hard "lock: the next run took ${_took}s, which is the data-lock timeout, without saying it was waiting for anything"
            excerpt /tmp/lock-corpse.out 6
        # **Positive evidence that the lock was taken, not merely that nothing complained.**
        # `DataLock`'s `Drop` deletes the stamp, so the corpse's file is gone once something has
        # taken and released the lock — and is still there if nothing did. Without this the
        # whole branch passed against a stub that does nothing and exits 0, which is a check
        # that cannot fail and therefore proves nothing when it passes.
        elif [ -s "$LOCKOWNER" ]; then
            hard "lock: the killed holder's stamp is still on disk after the next run — nothing took and released the lock, so this check had nothing to measure"
            excerpt /tmp/lock-corpse.out 6
        else
            _why=""
            grep -q "shall: waiting for" /tmp/lock-corpse.out 2>/dev/null && _why=" (it waited for the package manager the killed run had started, and said so)"
            PASS=$((PASS + 1)); echo "  PASS  lock: a killed holder's lock died with it — the next run took ${_took}s, took the lock and released it${_why}"
        fi
        # The other question, asked separately because it is a different question.
        if [ "$_rc" -ne 0 ] && [ "$_rc" -ne 2 ]; then
            hard "crash-recovery sync: the run after a killed holder failed (rc=$_rc) — the lock was free, so this is a sync defect and not a lock defect"
            echo "        heal said: $(tr '\n' ' ' < /tmp/lock-corpse-heal.out | tail -c 300)"
            excerpt /tmp/lock-corpse.out 6
        fi
    fi
    crash_wipe
fi

# ==========================================================================
# 16f. sudo, WITH A REAL PASSWORD, ON A REAL TERMINAL (GRADER §4)
# ==========================================================================
# Every check above this line runs as root, and `run_on` inserts `sudo` only when
# `!Self::is_root()` — so the whole privileged path is dead code in this container and has
# never executed anywhere. It is on every mutation of every system manager.
#
# It cannot be tested through a pipe. `ChildStdin::Interactive` exists so that sudo can ask
# for a password on the terminal Shall was started from; with no terminal there is nothing to
# ask, and the check would be measuring the absence of the thing it is for. So this section
# makes a user with a password, and drives Shall on a pty.
echo "[16f] sudo with a real password, on a pty"

SUDO_USER_NAME=shallsudo
SUDO_PW=shall-harness-pw
SUDO_CFG=/tmp/shall-sudo-config
SUDO_STATE=/tmp/shall-sudo-state
SUDO_DRIVER=/tmp/shall-sudo-drive.sh

# Every one of these is DETECTED. An image without `sudo`, without a way to set a password, or
# without a pty tool is a fact about the image, and Q17's rule is that an exemption has to be
# something the harness genuinely cannot do — measured here, never assumed from the distro.
sudo_blocker() {
    # THE FIRST CHECK, and it is not about capability. This section creates a real user, sets a
    # real password and writes a real `/etc/sudoers.d` file — and this script is not only run in
    # a container: `scripts/harness-mutation-test.sh` executes it ON THE HOST, from both release
    # scripts, to measure whether its checks can fail. That pair once silently replaced a
    # developer's git identity and thirteen commits went out under it. A sudoer is worse.
    #
    # `SHALL_IT_IMAGE` is set by every integration Dockerfile and by nothing else, and
    # `the_review_apparatus_is_rust_tests` asserts each image declares it — so it is the one marker
    # that means "this machine is disposable" and cannot drift without a red gate.
    [ -n "${SHALL_IT_IMAGE:-}" ] \
        || { echo "this is not a disposable image (no SHALL_IT_IMAGE), and these checks add a user and a sudoers file"; return 0; }
    on_path sudo      || { echo "this image has no \`sudo\`"; return 0; }
    on_path chpasswd  || { echo "this image cannot set a password (no \`chpasswd\`)"; return 0; }
    on_path script    || { echo "this image has no \`script\`, so Shall cannot be given a terminal"; return 0; }
    on_path useradd || on_path adduser || { echo "this image cannot create a user"; return 0; }
    echo ""
}

# `-e` — exit with the CHILD's status — is util-linux's and busybox's `script` does not have
# it. Every assertion below reads an exit code, and without `-e` that code is `script`'s own:
# the checks would all pass on an image where Shall failed every time. So it is a blocker and
# not a fallback, asked of the binary rather than guessed from the distro.
SCRIPT_FLAGS=""
script -qec true /dev/null >/dev/null 2>&1 && SCRIPT_FLAGS="-qec"

# The command goes in a FILE, never in a quoted string handed to `su -c`. `script -qec '…'`
# inside `su -c "…"` is three levels of quoting over a command that itself contains quotes,
# and the failure mode is not an error — it is a shell that runs something subtly different
# and a check that passes for the wrong reason.
write_sudo_driver() { # $1 = the shall argv, as one string
    cat > "$SUDO_DRIVER" <<EOF
#!/bin/sh
export SHALL_CONFIG_DIR="$SUDO_CFG"
export SHALL_DATA_DIR="$SUDO_STATE"
export HOME="/home/$SUDO_USER_NAME"
script $SCRIPT_FLAGS "$SHALL $1" /dev/null
EOF
    chmod 0755 "$SUDO_DRIVER"
}

# Runs Shall as the unprivileged user, on a pty, with $1 typed at whatever asks. `$2` is what
# gets typed; an empty string means the terminal is there and nobody types anything, which is
# the case that separates "asked and got no answer" from "never asked".
run_as_sudoer() { # shall-argv  password-to-type  outfile
    write_sudo_driver "$1"
    if [ -n "$2" ]; then
        printf '%s\n' "$2" | su "$SUDO_USER_NAME" -c "$SUDO_DRIVER" >"$3" 2>&1
    else
        su "$SUDO_USER_NAME" -c "$SUDO_DRIVER" </dev/null >"$3" 2>&1
    fi
}

SUDO_WHY="$(sudo_blocker)"
if [ -n "$SMOKE" ]; then
    skip_smoke "the sudo checks (they run a real privileged mutation)"
elif [ -n "$SUDO_WHY" ]; then
    soft "sudo: not driven here — $SUDO_WHY"
elif [ -z "$SCRIPT_FLAGS" ]; then
    soft "sudo: this image's \`script\` has no -e, so it reports its own exit status rather than Shall's and every check below would pass regardless of what Shall did"
else
    userdel -r "$SUDO_USER_NAME" >/dev/null 2>&1 || true
    if on_path useradd; then
        useradd -m -s /bin/sh "$SUDO_USER_NAME" >/dev/null 2>&1
    else
        adduser -D -s /bin/sh "$SUDO_USER_NAME" >/dev/null 2>&1
    fi
    printf '%s:%s\n' "$SUDO_USER_NAME" "$SUDO_PW" | chpasswd >/dev/null 2>&1

    # A drop-in rather than a group. `sudo` is `sudo` on debian and `wheel` everywhere else,
    # and this line says the same thing on all of them. Deliberately WITHOUT `NOPASSWD`: a
    # passwordless sudoer would make every check below pass without a password ever being
    # typed, which is the whole subject.
    mkdir -p /etc/sudoers.d
    printf '%s ALL=(ALL) ALL\n' "$SUDO_USER_NAME" > /etc/sudoers.d/shall-harness
    chmod 0440 /etc/sudoers.d/shall-harness

    rm -rf "$SUDO_CFG" "$SUDO_STATE"
    mkdir -p "$SUDO_CFG" "$SUDO_STATE"
    # **The bound under test is set here, small, rather than left at its default.**
    #
    # Checks (3) and (4) below ask whether Shall bounds the wait for a password nobody is going
    # to type. The default bound is 120s, so with it in force the correct behaviour takes 120
    # seconds — and the assertions, written when the answer was the 900s command-idle timeout,
    # read `>= 120` and called the bound firing exactly on time a wedge. Four minutes of nightly
    # spent proving a constant.
    #
    # Setting it to 5 tests the mechanism instead of the default's value: sudo asks, nobody
    # answers, and Shall has to give up in about five seconds. A run that takes 30 is not
    # honouring the setting, which is the defect either way and is now the thing being measured.
    printf 'sudo_password_timeout_secs = 5\n' > "$SUDO_CFG/preferences.toml"
    chown -R "$SUDO_USER_NAME" "$SUDO_CFG" "$SUDO_STATE" 2>/dev/null || true

    # Prove the fixture before trusting any result from it. A user who cannot sudo at all, or
    # whose password does not work, would make "Shall failed" and "the image is wrong"
    # indistinguishable — and the first reading is the flattering one.
    cat > /tmp/shall-sudo-probe.sh <<EOF
#!/bin/sh
sudo -k
script $SCRIPT_FLAGS "sudo id -u" /dev/null
EOF
    chmod 0755 /tmp/shall-sudo-probe.sh
    # `tr -d '\r'`: a pty terminates lines with CRLF, so an exact match against `0` fails on
    # output that is correct. That is the shape of bug that makes a fixture look broken.
    if printf '%s\n' "$SUDO_PW" | su "$SUDO_USER_NAME" -c /tmp/shall-sudo-probe.sh 2>&1 | tr -d '\r' | grep -qx 0; then
        # NOT counted as a pass. It is a statement about `sudo` and this image, with no Shall in
        # it at all — so it survives a do-nothing binary by construction, and a precondition
        # that spends coverage budget is a check pretending to be one.
        echo "        fixture ok: $SUDO_USER_NAME reaches uid 0 by typing a password"
        SUDO_READY=1
    else
        soft "sudo: the unprivileged user could not sudo even outside Shall, so this image cannot answer the question"
        SUDO_READY=""
    fi

    if [ -n "$SUDO_READY" ]; then
        # `init` first: without a priority file V.15 refuses the backend, and the refusal
        # would be scored as a sudo failure.
        run_as_sudoer "-y init" "$SUDO_PW" /tmp/sudo-init.out

        # --- (1) the prompt reaches the screen -------------------------------
        # `update` and not `install`: it needs root on every system manager, costs seconds,
        # and depends on no package name. What is under test is the escalation, not apt.
        su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
        run_as_sudoer "update" "$SUDO_PW" /tmp/sudo-update.out
        _rc=$?
        if grep -qi "password for\|Password:" /tmp/sudo-update.out; then
            PASS=$((PASS + 1)); echo "  PASS  sudo: the password prompt reached the terminal"
        else
            hard "sudo: a privileged mutation ran as a non-root user and no password prompt ever reached the screen"
            excerpt /tmp/sudo-update.out 8
        fi
        # --- (2) and the password reached sudo -------------------------------
        if [ "$_rc" -eq 0 ]; then
            PASS=$((PASS + 1)); echo "  PASS  sudo: the typed password reached sudo and the privileged command ran"
        else
            hard "sudo: the password was typed at a real prompt and the privileged command still failed (rc=$_rc)"
            excerpt /tmp/sudo-update.out 8
        fi

        # --- (3) a wrong password fails loudly, and quickly -------------------
        # The two failures that matter are silence and a hang. sudo retries three times before
        # giving up, so the bound is generous and still far under `timeout`'s.
        su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
        _t0=$(date +%s)
        run_as_sudoer "update" "definitely-not-the-password" /tmp/sudo-wrong.out
        _rc=$?
        _took=$(since "$_t0")
        if [ "$_rc" -eq 0 ]; then
            hard "sudo: a wrong password produced a successful privileged command"
        elif [ "$_took" -ge 30 ]; then
            hard "sudo: a wrong password left Shall waiting ${_took}s against a \`sudo_password_timeout_secs\` of 5 — the bound is not being honoured"
        elif grep -qi "sorry, try again\|incorrect password\|authentication fail\|sudo" /tmp/sudo-wrong.out; then
            PASS=$((PASS + 1)); echo "  PASS  sudo: a wrong password fails in ${_took}s and says which program refused"
        else
            hard "sudo: a wrong password failed without naming sudo anywhere in the output"
            excerpt /tmp/sudo-wrong.out 8
        fi

        # --- (4) a terminal nobody types at is not a hang ---------------------
        # The state a CI job is in. sudo has a tty, asks, and gets EOF; the honest outcome is
        # a prompt and a prompt failure, and the one unacceptable outcome is a wedge.
        su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
        _t0=$(date +%s)
        run_as_sudoer "update" "" /tmp/sudo-silent.out
        _rc=$?
        _took=$(since "$_t0")
        # The output has to name the program that asked. Without that clause "it failed" is
        # true of a binary that fails at everything, and this passed against one.
        if [ "$_took" -ge 30 ]; then
            hard "sudo: a terminal with nobody at it wedged Shall for ${_took}s against a \`sudo_password_timeout_secs\` of 5 — the bound is not being honoured"
        elif [ "$_rc" -ne 0 ] && grep -qi "password\|sudo" /tmp/sudo-silent.out; then
            PASS=$((PASS + 1)); echo "  PASS  sudo: an unanswered prompt is a bounded failure (${_took}s) that names sudo, not a wedge"
        elif [ "$_rc" -ne 0 ]; then
            hard "sudo: the run failed with nobody at the terminal and never mentioned a password or sudo — that is a different failure"
            excerpt /tmp/sudo-silent.out 6
        else
            soft "sudo: an unanswered prompt still succeeded — this host's sudo timestamp was still warm"
        fi

        # --- (5) one run, one password ----------------------------------------
        # WITHIN a single Shall run, not across two. The first version of this check ran
        # `update` twice and expected the second not to ask — and sudo asked, correctly: its
        # timestamp is per-tty (`tty_tickets`, on by default) and each drive here gets a fresh
        # pty. That check was measuring sudo's own design and calling it a Shall defect.
        #
        # What Shall owes is that ONE command which escalates more than once asks at most once.
        # A sync carrying both an install and a removal runs the manager twice, so the count of
        # prompts in one transcript is the measurement.
        #
        # **The long-run keepalive is still not covered and this does not pretend to cover it.**
        # `start_sudo_keepalive` exists for a sync that outlives sudo's 15-minute timestamp, and
        # nothing here runs for fifteen minutes. Naming that is the point: a proxy that cannot
        # fail for the reason it claims to test is the vacuous check this harness exists to
        # refuse.
        # What THIS section declares under `$SUDO_CFG`, tracked as it goes.
        #
        # The cleanup below used to ask the machine instead — "which canaries are on PATH" — and
        # call the answer "the section's own packages". Sections before this one install canaries
        # too, under a *different* config root, so an empty manifest here correctly removes none
        # of them and the check called that a survival. It failed on ubuntu the first run where
        # 16e happened to leave all three installed rather than one, which is a coin toss, not a
        # defect. A check that can fail for a reason unrelated to its sentence is the vacuous
        # check this harness exists to refuse.
        _sudo_mine=""
        _mine_add() { _sudo_mine="$_sudo_mine $1"; }
        _mine_drop() {
            _kept=""
            for _m in $_sudo_mine; do [ "$_m" = "$1" ] || _kept="$_kept $_m"; done
            _sudo_mine="$_kept"
        }
        if [ -n "$CRASH_PKGS" ]; then
            _a=""; _b=""
            for _c in $CRASH_PKGS; do
                if [ -z "$_a" ]; then _a="$_c"; elif [ -z "$_b" ]; then _b="$_c"; fi
            done
        fi
        if [ -z "${_b:-}" ]; then
            soft "sudo: fewer than two canaries free on this host, so no single run performs both an install and a removal"
        else
            su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
            run_as_sudoer "-y install $BACKEND:$_a" "$SUDO_PW" /tmp/sudo-seed.out
            _mine_add "$_a"
            su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
            # One run, two manager invocations: $_a comes out, $_b goes in.
            printf '%s:%s\n' "$BACKEND" "$_b" > "$SUDO_CFG/modules/imperative.txt"
            _mine_drop "$_a"; _mine_add "$_b"
            chown "$SUDO_USER_NAME" "$SUDO_CFG/modules/imperative.txt" 2>/dev/null || true
            run_as_sudoer "-y sync" "$SUDO_PW" /tmp/sudo-onerun.out
            _rc=$?
            _asks=$(grep -c "password for" /tmp/sudo-onerun.out 2>/dev/null)
            [ -n "$_asks" ] || _asks=0
            if [ "$_rc" -ne 0 ]; then
                soft "sudo: the two-operation run did not complete (rc=$_rc), so the prompt count says nothing — $(tr '\n' ' ' < /tmp/sudo-onerun.out | tail -c 200)"
            elif [ "$_asks" -le 1 ]; then
                PASS=$((PASS + 1)); echo "  PASS  sudo: one run that installed and removed asked for a password $_asks time(s)"
            else
                hard "sudo: one run asked for a password $_asks times — the timestamp is not being held across a sync's manager calls"
            fi
        fi

        # --- (6) and a real package, installed by a user who is not root ------
        # The end of the path: escalation, the manager, and a file on disk. Whichever crash
        # canary this host does not already have — and if it has them all, that is said out
        # loud rather than quietly dropping the only check here that touches a package.
        _sudo_pkg=""
        for _c in $CRASH_PKGS; do on_path "$_c" || { _sudo_pkg="$_c"; break; }; done
        if [ -z "$_sudo_pkg" ]; then
            soft "sudo: every crash canary is installed on this host, so the privileged install had nothing to install"
        else
            su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
            run_as_sudoer "-y install $BACKEND:$_sudo_pkg" "$SUDO_PW" /tmp/sudo-install.out
            _rc=$?
            _sudo_installed=""
            if [ "$_rc" -eq 0 ] && on_path "$_sudo_pkg"; then
                PASS=$((PASS + 1)); echo "  PASS  sudo: a non-root user installed $BACKEND:$_sudo_pkg, and the file is on disk"
                _sudo_installed=1
                _mine_add "$_sudo_pkg"
            else
                hard "sudo: the privileged install of $BACKEND:$_sudo_pkg reported rc=$_rc and $(on_path "$_sudo_pkg" && echo 'the binary is there anyway' || echo 'nothing reached PATH')"
                excerpt /tmp/sudo-install.out 8
            fi
            # Removal is the same path and the guard sits on it; a run that only ever installs
            # leaves the privileged REMOVE untested, which is the more dangerous half.
            #
            # Gated on the install having happened. "The binary is not on PATH" is true before
            # any install too, so without this the check passes against a binary that does
            # nothing at all — measured: it was one of five survivors of the mutation gate.
            if [ -z "$_sudo_installed" ]; then
                soft "sudo: the privileged removal was not driven, because the install it would undo did not happen"
            else
                su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
                run_as_sudoer "-y uninstall $BACKEND:$_sudo_pkg" "$SUDO_PW" /tmp/sudo-remove.out
                _mine_drop "$_sudo_pkg"
                if on_path "$_sudo_pkg"; then
                    hard "sudo: the privileged uninstall of $BACKEND:$_sudo_pkg left the binary on PATH"
                    excerpt /tmp/sudo-remove.out 8
                else
                    PASS=$((PASS + 1)); echo "  PASS  sudo: and the same user removed it again"
                fi
            fi
        fi
    fi

    # Put the machine back, declaratively — an empty manifest and a sync, which is the same
    # path a user takes and one more exercise of the privileged REMOVE. Doing it with `apt`
    # directly would tidy up without testing anything, and would leave the sections after this
    # one guessing why their canaries were already installed.
    if [ -n "${SUDO_READY:-}" ]; then
        # What this section actually put on the machine, read BEFORE the cleanup. An empty
        # manifest removing nothing is not a pass — and "nothing is installed" is the state a
        # do-nothing binary leaves too, which is how this check survived the mutation gate.
        #
        # `$_sudo_mine`, not every canary on PATH: only what this section declared under
        # `$SUDO_CFG` is this section's to take back. See where `_sudo_mine` is built.
        _had=""
        for _c in ${_sudo_mine:-}; do on_path "$_c" && _had="$_had $_c"; done
        : > "$SUDO_CFG/modules/imperative.txt"
        chown "$SUDO_USER_NAME" "$SUDO_CFG/modules/imperative.txt" 2>/dev/null || true
        su "$SUDO_USER_NAME" -c "sudo -k" >/dev/null 2>&1
        run_as_sudoer "-y sync" "$SUDO_PW" /tmp/sudo-cleanup.out
        _left=""
        for _c in $_had; do on_path "$_c" && _left="$_left $_c"; done
        if [ -z "$_had" ]; then
            soft "sudo: this section installed nothing, so an empty manifest had nothing to take back"
        elif [ -z "$_left" ]; then
            PASS=$((PASS + 1)); echo "  PASS  sudo: an empty manifest took back everything this section installed —$_had"
        else
            hard "sudo: the section's own packages survived an empty manifest —$_left"
            excerpt /tmp/sudo-cleanup.out 8
        fi
    fi

    rm -f /etc/sudoers.d/shall-harness
    userdel -r "$SUDO_USER_NAME" >/dev/null 2>&1 || true
fi

# ==========================================================================
# 16g. A SNAPSHOT, A MUTATION, AND A RESTORE — on a real device (GRADER §5)
# ==========================================================================
# `SnapshotProvider::restore` had never executed. Section 13b gives btrfs and lvm real block
# devices and section 14 installs into them, but a lifecycle is install → list → remove and by
# construction never restores anything. So the most destructive effector in the program — the
# one that puts a filesystem back — was argv-tested and unrun, which is exactly what `GRADER`
# §5 says needs a disposable machine.
#
# lvm and not btrfs, and the reason is in the shipped rows rather than in a preference: btrfs's
# built-in row is create-only (`restore_how`, no `restore` argv) because a live btrfs rollback
# replaces the root subvolume, and zfs needs an out-of-tree kernel module this container's host
# does not have. lvm is U27's exemplar USER row — thirty lines of data in `adapters/` — so
# driving it proves the plugin door as well as the effector, which is the K17/U1 rule.
echo "[16g] Snapshot → mutate → restore, on a real device"

LVM_ORIGIN=""
if [ -n "$SMOKE" ]; then
    skip_smoke "the snapshot restore round-trip (it makes and merges a real volume)"
elif [ -z "$STORAGE_LVM" ]; then
    soft "restore: no volume group on this image — 13b said why, and only the \`storage\` image (--privileged) builds one"
elif ! command -v mkfs.ext4 >/dev/null 2>&1; then
    soft "restore: no mkfs.ext4 here, so the origin volume has no filesystem to put a marker file in"
else
    LVM_ORIGIN=restoreorigin
    LVM_MNT=/mnt/shall-restore
    lvremove -y "$STORAGE_LVM/$LVM_ORIGIN" >/dev/null 2>&1
    mkdir -p "$LVM_MNT"
    if ! lvcreate -y -n "$LVM_ORIGIN" -L 128M "$STORAGE_LVM" >/tmp/restore-setup.out 2>&1 \
       || ! mkfs.ext4 -q -F "/dev/$STORAGE_LVM/$LVM_ORIGIN" >>/tmp/restore-setup.out 2>&1; then
        soft "restore: could not build an origin volume with a filesystem here — $(tr '\n' ' ' < /tmp/restore-setup.out | tail -c 200)"
        LVM_ORIGIN=""
    fi
fi

if [ -n "$LVM_ORIGIN" ]; then
    # The provider, as a row in the user's own adapters file. `restores_running_system` is the
    # one value U27 ruled can never be inferred — Shall will not run a "restore" for a provider
    # that did not say it can finish one — so this line is the whole capability.
    mkdir -p "$SHALL_CONFIG_DIR/adapters"
    cat > "$SHALL_CONFIG_DIR/adapters/snapshot.toml" <<EOSNAP
[[snapshot]]
name = "lvm"
detect = "lvcreate"
source = "$STORAGE_LVM/$LVM_ORIGIN"
id_template = "shall_{label}_{ts}"
create = ["lvcreate", "-y", "-s", "-n", "{id}", "-L", "64M", "{source}"]
list = ["lvs", "--noheadings", "-o", "lv_name", "$STORAGE_LVM"]
list_pattern = "(shall_\\\\S+)"
delete = ["lvremove", "-y", "$STORAGE_LVM/{id}"]
restore = ["lvconvert", "--merge", "-y", "$STORAGE_LVM/{id}"]
restores_running_system = true
EOSNAP
    # II.12: a row in a pulled config is argv a shared repo can run, so it passes the hook
    # ledger. Approving it is part of the path a real user walks, not a step around it.
    ok "restore: \`lock\` approves the snapshot provider row" lx lock

    _marker_before="$LVM_MNT/before-the-snapshot"
    _marker_after="$LVM_MNT/after-the-snapshot"
    mount "/dev/$STORAGE_LVM/$LVM_ORIGIN" "$LVM_MNT" >/dev/null 2>&1
    echo "this file existed when the snapshot was taken" > "$_marker_before"
    sync; umount "$LVM_MNT" >/dev/null 2>&1

    # --- (a) a snapshot is taken, for real, by an ordinary mutating sync -------
    # Not by a test hook: `auto_snapshot` runs before every mutating sync, so this is the same
    # code path a user's `shall sync` takes. The canaries are declared to give it work — a sync
    # with nothing to do has nothing to snapshot before.
    crash_declare
    lx_slow -y sync >/tmp/restore-sync.out 2>&1
    _snaps="$(lvs --noheadings -o lv_name "$STORAGE_LVM" 2>/dev/null | tr -d ' ' | grep '^shall_' | head -1)"
    if [ -n "$_snaps" ]; then
        PASS=$((PASS + 1)); echo "  PASS  restore: a mutating sync took a real LVM snapshot — $_snaps"
        grep_ok "restore: snapshot list reports it" "shall_" lx snapshot list

        # --- (b) the control: no terminal, no gallery -------------------------
        # Choosing from a gallery needs somewhere to choose, and the refusal must say so and
        # name the commands that do the same job without asking. Exit 3, because Shall declined
        # on purpose rather than broke.
        #
        # It runs HERE and not before the snapshot exists: with an empty gallery the command
        # says "No system snapshots found" and exits 0, correctly and long before it looks for a
        # terminal — so asserting the refusal against an empty gallery asserts nothing.
        refuses_with_3 "restore: the gallery refuses a shell with no terminal" lx snapshot restore
        grep_ok "restore: and names what to use instead" \
            "snapshot list\|rollback" lx snapshot restore
    elif grep -q "already up to date" /tmp/restore-sync.out; then
        soft "restore: the sync had nothing to do, so there was no mutation for a pre-sync snapshot to precede — the fixture, not the provider"
    else
        hard "restore: a mutating sync took no snapshot, so the provider row was loaded and never used"
        excerpt /tmp/restore-sync.out 8
    fi

    if [ -n "$_snaps" ] && [ -n "$SCRIPT_FLAGS" ]; then
        # --- (c) the regression test for today's fix, driven live -------------
        # `show_diff_and_confirm` matched the provider BY NAME — `btrfs` or `timeshift`, and
        # `Unsupported snapshot backend: <anything else>` — so every provider U27 turned into a
        # row was refused before the confirmation, whatever its row declared. Typing a word that
        # is not RESTORE is the safe half of the same drive: it must reach the prompt and then
        # do nothing.
        cat > /tmp/shall-restore-drive.sh <<EOF
#!/bin/sh
export SHALL_CONFIG_DIR="$SHALL_CONFIG_DIR"
export SHALL_DATA_DIR="$SHALL_DATA_DIR"
printf '\\nno\\n' | script $SCRIPT_FLAGS "$SHALL snapshot restore" /dev/null
EOF
        chmod 0755 /tmp/shall-restore-drive.sh
        sh /tmp/shall-restore-drive.sh >/tmp/restore-abort.out 2>&1
        if grep -q "Unsupported snapshot backend" /tmp/restore-abort.out; then
            hard "restore: the gallery still refuses a provider for its name — the row declares a live restore and the command does not read the row"
            excerpt /tmp/restore-abort.out 6
        elif grep -q "Type 'RESTORE'" /tmp/restore-abort.out; then
            PASS=$((PASS + 1)); echo "  PASS  restore: a provider declared in a user row reaches the confirmation"
        else
            hard "restore: the gallery never reached its confirmation prompt"
            excerpt /tmp/restore-abort.out 8
        fi
        # And it did nothing. A confirmation that acts on the wrong answer is worse than one
        # that never asked, so this is asked of LVM and not of Shall's output.
        if lvs --noheadings -o lv_name "$STORAGE_LVM" 2>/dev/null | tr -d ' ' | grep -q '^shall_'; then
            PASS=$((PASS + 1)); echo "  PASS  restore: answering anything but RESTORE left the snapshot alone"
        else
            hard "restore: the snapshot was consumed by a confirmation that was answered 'no'"
        fi

        # --- (d) the round trip, and the filesystem is the witness -------------
        # The mutation is made HERE, between the snapshot and the restore, so that "the restore
        # put it back" is a statement about bytes on a device rather than about a message.
        mount "/dev/$STORAGE_LVM/$LVM_ORIGIN" "$LVM_MNT" >/dev/null 2>&1
        rm -f "$_marker_before"
        echo "this file was written after the snapshot" > "$_marker_after"
        sync; umount "$LVM_MNT" >/dev/null 2>&1

        cat > /tmp/shall-restore-drive.sh <<EOF
#!/bin/sh
export SHALL_CONFIG_DIR="$SHALL_CONFIG_DIR"
export SHALL_DATA_DIR="$SHALL_DATA_DIR"
printf '\\nRESTORE\\n' | script $SCRIPT_FLAGS "$SHALL snapshot restore" /dev/null
EOF
        chmod 0755 /tmp/shall-restore-drive.sh
        sh /tmp/shall-restore-drive.sh >/tmp/restore-do.out 2>&1

        # An LVM merge of an unmounted origin happens immediately; a mounted one is deferred to
        # the next activation, which is why the origin is unmounted above.
        mount "/dev/$STORAGE_LVM/$LVM_ORIGIN" "$LVM_MNT" >/dev/null 2>&1
        _back=""; _gone=""
        [ -f "$_marker_before" ] && _back=1
        [ -f "$_marker_after" ] || _gone=1
        umount "$LVM_MNT" >/dev/null 2>&1
        if [ -n "$_back" ] && [ -n "$_gone" ]; then
            PASS=$((PASS + 1)); echo "  PASS  restore: the filesystem is back to the snapshot — the pre-snapshot file returned and the post-snapshot file is gone"
        elif [ -n "$_back" ]; then
            hard "restore: the pre-snapshot file came back and the post-snapshot file survived — a half-merge"
            excerpt /tmp/restore-do.out 8
        else
            hard "restore: the restore reported and the device did not change — the pre-snapshot file is still missing"
            excerpt /tmp/restore-do.out 10
        fi
    elif [ -z "$SCRIPT_FLAGS" ]; then
        soft "restore: this image's \`script\` has no -e, so the gallery cannot be driven on a terminal here"
    fi

    umount "$LVM_MNT" >/dev/null 2>&1
    lvremove -y "$STORAGE_LVM/$LVM_ORIGIN" >/dev/null 2>&1
    rm -f "$SHALL_CONFIG_DIR/adapters/snapshot.toml"
    crash_wipe
fi

# ==========================================================================
# 17. COVERAGE AUDIT — what did nothing touch? (IV.1)
# ==========================================================================
# The only check here that can notice what is MISSING from the list above it. A
# backend or a command added next year fails this until it is covered.
echo "[17] Coverage audit"

# What the image SAID it shipped, against what is here.
#
# Every manager above the `COPY` in the Dockerfile is installed best-effort — an arch or a
# mirror can take one of thirty away and the rest are still worth testing — and until this
# check a failed install was indistinguishable from a manager nobody ever tried. `nix` printed
# `SKIP nix install` in step 12 of a 26-step build for months while the coverage ledger recorded
# it as having **no path to a real lifecycle anywhere**: a cost we chose not to notice, filed as
# an impossibility (Q4).
#
# Named as a soft rather than a hard failure on purpose. Some of these genuinely cannot install
# on some architectures, and turning that into a red run would teach people to delete the line
# rather than fix the install — which is how the exemption lists got long in the first place.
# What it must never do again is stay invisible.
if [ -f /etc/shall-image-managers ]; then
    _absent="$(awk '$2 == "ABSENT" {printf "%s ", $1}' /etc/shall-image-managers)"
    if [ -n "$_absent" ]; then
        soft "the image tried to install these and they are not here: $_absent"
        echo "        a manager that failed to install is MISSING, not impossible — read the"
        echo "        build log for its step before excusing the backend that needs it."
    else
        PASS=$((PASS + 1))
        echo "  PASS  every manager this image installs is present"
    fi
fi

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
if [ -z "$SMOKE" ] && [ -n "${SHALL_IT_IMAGE:-}" ]; then
    for be in $ALL_BACKENDS; do
        case ",$(primary_manager_image "$be" | tr -d ' ')," in
            *",$SHALL_IT_IMAGE,"*) ;;
            *) continue ;;
        esac
        if grep -qx "$be" "$LEDGER/be-life" "$LEDGER/be-life-partial" \
                "$LEDGER/be-life-unmeasured" 2>/dev/null; then
            PASS=$((PASS + 1))
            echo "  PASS  $be: the image that claims its lifecycle is this one, and it ran"
        else
            FAILC=$((FAILC + 1))
            FAILED_NAMES="$FAILED_NAMES
    - coverage: primary_manager_image says $be is lifecycled on the $SHALL_IT_IMAGE image, and this run of it never touched $be"
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
# Shall itself classified as passing and a retry did not clear (a rate-limit window, a held
# lock). That is not the same fact as "this host did fewer lifecycles", and the ratchet must not
# confuse them: a GitHub rate limit on the macOS leg dropped the count 8 -> 7 and turned this
# gate red, and the obvious repair — lowering the floor to 7 — would have ratcheted a
# platform's coverage down permanently over a window that had already moved (R-3).
#
# Excused only for a class Shall computed, and only BY NAME, printed below. A backend that
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
# the other permanently wrong. Each Dockerfile declares its own `SHALL_IT_IMAGE`; os-release
# remains the fallback for an image that has not.
HOST_FLAVOUR=""
if [ -n "${SHALL_IT_IMAGE:-}" ]; then
    HOST_FLAVOUR="-$SHALL_IT_IMAGE"
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
        # A count is not a finding. The floor is one number, so this cannot name what the last
        # run did that this one did not — but it can name what ran and what the image is
        # missing, which is the same answer for the case that actually happens: a best-effort
        # install failed at build time and took its manager's whole lifecycle with it. Reading
        # `25, and it has done 26 before` cost a diff of two nightlies' logs to learn the word
        # `helm`; every fact needed to print that word was already in this container.
        echo "        did a real lifecycle here: $(tr '\n' ' ' < "$LEDGER/be-life.u")"
        if [ -f /etc/shall-image-managers ]; then
            _gone="$(awk '$2 == "ABSENT" {printf "%s ", $1}' /etc/shall-image-managers)"
            [ -n "$_gone" ] && echo "        the image says these are ABSENT: $_gone" &&
                echo "        A manager the image failed to install cannot have a lifecycle. If one" &&
                echo "        of these was here last run, that is the whole shortfall — fix the" &&
                echo "        install in the Dockerfile rather than the number in $FLOOR_FILE."
        fi
    elif [ "$LIFECYCLES" -lt "$FLOOR" ]; then
        # Short of the floor, and the shortfall is exactly the backends nothing could measure.
        # Reported at full volume and never silently: a run that excuses coverage has to say so,
        # or "silent truncation reads as covered everything when it did not".
        soft "real-lifecycle ratchet: $LIFECYCLES of $FLOOR on $HOST_CLASS, and $UNMEASURED backend(s) could not be measured this run"
        echo "        unmeasurable: $(tr '
' ' ' < "$LEDGER/be-life-unmeasured.u")"
        echo "        Each failed a real install for a reason Shall classed as passing, and did"
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
