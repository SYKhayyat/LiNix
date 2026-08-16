#!/usr/bin/env bash
# ============================================================================
# Shall v7 native Windows/macOS sweep — host-native backends (scoop/winget/
# choco/brew) driven through the real `shall` binary. These OSes can't run in a
# Linux container, so this mirrors the container harness (run-in-container.sh)
# for the host, section for section — including its COVERAGE AUDIT.
#
#   scripts/integration-windows.sh [backend] [package]
#   e.g. scripts/integration-windows.sh scoop jq        # user-scoped, reversible
#        SHALL=./target/release/shall.exe scripts/integration-windows.sh
#
# scoop is the safe default (user-scoped, trivially reversible). Shall's own
# state is isolated via SHALL_CONFIG_DIR / SHALL_DATA_DIR; real package installs
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
SHALL="${SHALL:-shall}"

export SHALL_CONFIG_DIR="${TMPDIR:-/tmp}/shall-it-win-config"
export SHALL_DATA_DIR="${TMPDIR:-/tmp}/shall-it-win-state"
rm -rf "$SHALL_CONFIG_DIR" "$SHALL_DATA_DIR" 2>/dev/null
mkdir -p "$SHALL_CONFIG_DIR" "$SHALL_DATA_DIR"

# The coverage ledger. Files, not variables: `grep_ok` runs its command in a
# pipeline, and a pipeline is a subshell whose variable writes die with it.
LEDGER="${TMPDIR:-/tmp}/shall-it-win-ledger"
rm -rf "$LEDGER" 2>/dev/null; mkdir -p "$LEDGER"
: > "$LEDGER/cmd-real"; : > "$LEDGER/cmd-help"
: > "$LEDGER/be-life"; : > "$LEDGER/be-life-partial"; : > "$LEDGER/be-life-unmeasured"; : > "$LEDGER/be-smoke"

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
# An automated sweep has nobody at the keyboard, so it must not hand anything a keyboard.
#
# Shall gives a mutation's child `Stdio::inherit()` when its own stdin is a terminal
# (`core/executor.rs`), so a manager that asks a question inherits *this* terminal and waits
# for an answer that is never coming. Measured on this host: the same install is 48ms with no
# terminal on stdin and 21.9s with one, where 21.9s is the whole `command_idle_timeout_secs`
# bound elapsing. At the shipped default of 900 that is a fifteen-minute silence.
#
# `exec` rather than a redirect on each call, because the per-call form has to be remembered
# at ~200 sites and the three background holders are not call sites at all. An explicit pipe
# still wins over this, so `printf … | lx repl` is unaffected.
#
# `SHALL_IT_KEEP_TTY=1` puts the terminal back, and exists so the stall can still be
# REPRODUCED on demand. A fix that also deletes the only way to observe the bug leaves nobody
# able to show it ever existed, or to notice it coming back.
HAD_TTY=no; [ -t 0 ] && HAD_TTY=yes
if [ "${SHALL_IT_KEEP_TTY:-0}" = "1" ]; then
    echo "stdin: keeping the terminal (SHALL_IT_KEEP_TTY=1) — this run can stall on purpose"
else
    exec < /dev/null
fi

# --- Stall capture ---------------------------------------------------------
#
# Four times a full sweep sat with an idle `shall` and a log that had stopped, and four times
# the process was killed to get moving again — which is what destroyed the only evidence that
# could name the cause. The capture has to be armed BEFORE the run, not written afterwards.
#
# Non-destructive on purpose: it photographs and never kills. A command that is merely slow
# and one that is wedged look identical in a single frame, so the snapshot measures CPU over a
# window and lists the child tree; the report tells them apart, not the threshold.
STALL_DIR="${TMPDIR:-/tmp}/shall-it-win-stall"
rm -rf "$STALL_DIR" 2>/dev/null; mkdir -p "$STALL_DIR"
STALL_REPORT="$STALL_DIR/stalls.txt"
STALL_CURRENT="$STALL_DIR/current"
: > "$STALL_CURRENT"
STALL_AFTER="${SHALL_IT_STALL_AFTER:-150}"   # above the slowest honest call measured here (141s)
STALL_EVERY="${SHALL_IT_STALL_EVERY:-60}"
STALL_SNAPSHOT="$(dirname "$0")/stall-snapshot.ps1"
STALL_PID=""

stall_watch() {
    _snaps=0
    while :; do
        sleep 15
        [ -s "$STALL_CURRENT" ] || { _snaps=0; continue; }
        _started=$(awk '{print $1; exit}' "$STALL_CURRENT" 2>/dev/null)
        case "$_started" in ''|*[!0-9]*) continue ;; esac
        _age=$(( $(date +%s) - _started ))
        [ "$_age" -ge "$STALL_AFTER" ] || continue
        # One at STALL_AFTER, then one every STALL_EVERY, capped: a genuinely long build must
        # not produce a thousand snapshots, and eight is enough to show a trend.
        _due=$(( (_age - STALL_AFTER) / STALL_EVERY + 1 ))
        [ "$_due" -gt "$_snaps" ] || continue
        [ "$_snaps" -ge 8 ] && continue
        _snaps=$((_snaps + 1))
        _what=$(cut -d' ' -f2- < "$STALL_CURRENT")
        echo "  ....  STALL WATCH: \`shall $_what\` has been running ${_age}s — snapshot $_snaps"
        "$STALL_PS" -NoProfile -ExecutionPolicy Bypass \
            -File "$(cygpath -w "$STALL_SNAPSHOT")" \
            -OutFile "$(cygpath -w "$STALL_REPORT")" \
            -Note "in flight ${_age}s: shall $_what" >/dev/null 2>&1
    done
}

# Armed only where it can work. Named when it cannot, because a capture that silently did not
# run reads exactly like a run with nothing to capture.
# `powershell.exe` is not always on PATH under a stripped shell, and the System32 copy is
# always there when Windows is. Resolved once, by name then by path: an arming check that
# depends on a PATH entry disarms the whole capture over a shell setting.
STALL_PS=""
if command -v powershell.exe >/dev/null 2>&1; then STALL_PS="powershell.exe"
elif command -v powershell >/dev/null 2>&1; then STALL_PS="powershell"
elif [ -x "/c/windows/System32/WindowsPowerShell/v1.0/powershell.exe" ]; then
    STALL_PS="/c/windows/System32/WindowsPowerShell/v1.0/powershell.exe"
fi

if [ -r "$STALL_SNAPSHOT" ] && [ -n "$STALL_PS" ] \
   && command -v cygpath >/dev/null 2>&1; then
    stall_watch & STALL_PID=$!
    trap 'kill "$STALL_PID" 2>/dev/null' EXIT INT TERM
    echo "stall capture armed: >${STALL_AFTER}s in one call photographs the process tree into"
    echo "  $STALL_REPORT"
    # Which arm this run is. A stall that only happens with a terminal on stdin is a different
    # finding from one that happens without, and the report must say which run it was.
    echo "  terminal on stdin at startup: $HAD_TTY; handed to Shall: ${SHALL_IT_KEEP_TTY:-0}"
else
    echo "stall capture NOT armed (needs stall-snapshot.ps1, powershell.exe and cygpath);"
    echo "  a stall this run will be an observation with no cause, again."
fi

lx() {
    record_argv "$@"
    echo "$(date +%s) $*" > "$STALL_CURRENT"
    $TO "$SHALL" "$@"
    _lx_rc=$?
    : > "$STALL_CURRENT"
    return $_lx_rc
}

PASS=0; FAILC=0; SOFTC=0; FAILED_NAMES=""

# An identity for section 9's `git init`, when this machine has none.
#
# **Per-process, never `git config --global`.** This harness runs on a real machine — that is
# its whole point — so writing a global identity would replace the owner's. The container twin
# did exactly that once it started being run on the host, and thirteen commits went out under
# the wrong name before anyone noticed (2026-07-28).
#
# Only when git has no identity, so a developer's own is left alone. Without it, a clean CI
# runner fails `git init` with `unable to auto-detect email address` — Shall's message is
# right and there is nobody there to act on it — and `diff` and `rollback` never run, which
# then fails the coverage audit for a reason that has nothing to do with them.
if ! git config user.email >/dev/null 2>&1; then
    export GIT_AUTHOR_NAME="Shall Integration" GIT_AUTHOR_EMAIL="integration@shall.invalid"
    export GIT_COMMITTER_NAME="Shall Integration" GIT_COMMITTER_EMAIL="integration@shall.invalid"
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
# and the check reports full coverage. Measured under a do-nothing `shall` stub, the audit
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
    "$@" >/tmp/itw.out 2>&1; rc=$?
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
    if grep -q "$pat" /tmp/itw.out; then
        PASS=$((PASS + 1)); echo "  PASS  $desc (refused, saying so)"; return 0
    fi
    FAILC=$((FAILC + 1)); FAILED_NAMES="$FAILED_NAMES
    - $desc (failed without saying /$pat/)"
    echo "  FAIL  $desc (rc=$rc, but nothing in the output said /$pat/ — it failed for some other reason)"
    excerpt; return 1
}

refuses_with_3() { # description command...
    desc="$1"; shift
    "$@" >/tmp/itw.out 2>&1; rc=$?
    if [ "$rc" = 3 ]; then
        PASS=$((PASS + 1)); echo "  PASS  $desc (refused on purpose, exit 3)"; return 0
    fi
    FAILC=$((FAILC + 1))
    if [ "$rc" = 0 ]; then
        _rw="it succeeded"
    elif never_ran "$rc"; then
        _rw="rc=$rc: the command never ran"
    else
        _rw="rc=$rc: a failure, not the documented refusal (README.md: 3 means refused on purpose)"
    fi
    FAILED_NAMES="$FAILED_NAMES\n    - $desc ($_rw)"
    echo "  FAIL  $desc ($_rw)"
    excerpt; return 1
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
#   timeout    the build ran out of time (124). Not a verdict on the backend.
#   transient  failed once, succeeded on retry. The caller CONTINUES the lifecycle — skipping
#              it is how list, PATH, remove and gone-from-list went unrun for every backend
#              whose install was flaky.
#   exhausted  Shall classed the failure passing and it did not pass in this window — a rate
#              limit with 20 minutes left on it. SOFT, and recorded as a lifecycle this run
#              could not measure, which is not the same fact as a lifecycle that got worse.
#   defect     failed permanently, or failed twice with nothing classifying it. Hard.
#
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
    lx -y install "$_ci_spec" >/tmp/itw-retry.out 2>&1
    _ci_rc2=$?
    if [ "$_ci_rc2" -eq 0 ]; then
        soft "$_ci_be: install of $_ci_spec failed once and succeeded on retry — transient"
        CLASS=transient; return 0
    fi
    if [ "$_ci_class" = transient ]; then
        soft "$_ci_be: install of $_ci_spec is classed transient and did not clear on a retry — exhausted, not a defect (rc=$_ci_rc, $_ci_rc2)"
        excerpt /tmp/itw-retry.out 6
        # Recorded so the ratchet can tell a lifecycle it could not MEASURE from one that got
        # worse. Without this a rate limit ratchets a platform's coverage down permanently.
        echo "$_ci_be" >> "$LEDGER/be-life-unmeasured"
        CLASS=exhausted; return 0
    fi
    hard "$_ci_be: install of $_ci_spec failed twice, unclassified — a defect, not ecosystem variance (rc=$_ci_rc, $_ci_rc2)"
    excerpt /tmp/itw-retry.out 6
    CLASS=defect
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
echo " Shall v7 Windows/macOS harness — backend=$BACKEND package=$PKG"
echo " SHALL=$SHALL"
echo "=============================================================="

# Runnable, not merely present — and runnable THROUGH the wrapper every check below uses.
# `command -v` answers about the binary alone, so a missing `timeout` left every one of
# the sweep's own invocations exiting 127 while this line reported the binary was fine.
if ! $TO "$SHALL" --version >/dev/null 2>&1; then
    echo "FATAL: '${TO:+$TO }$SHALL --version' did not run — nothing below would be tested."
    command -v "$SHALL" >/dev/null 2>&1 \
        || echo "       not on PATH: set SHALL to the built binary, or build it. Looked for '$SHALL'"
    [ -n "$TO" ] || echo "       (no timeout wrapper in use, so the binary itself is the fault)"
    exit 2
fi
[ -n "$TO" ] || soft "no \`timeout\` nor \`gtimeout\` on this host — commands run unbounded"

# --- 1. Bootstrap ----------------------------------------------------------
echo "[1] Bootstrap"
ok "init scaffolds the repo" lx init
ok "priority file exists" test -f "$SHALL_CONFIG_DIR/priority"
ok "active file exists" test -f "$SHALL_CONFIG_DIR/active"
grep_ok "priority names this backend" "$BACKEND" cat "$SHALL_CONFIG_DIR/priority"

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
echo "[4] purge-undeclared, before adopt"
refuses_with_3 "purge-undeclared is refused on a machine Shall has not adopted" lx -y purge-undeclared
grep_ok "and it is the unadopted-machine ratio that refused" \
    "adopt\|allow-mass-purge" lx -y purge-undeclared

# --- 5. Install -> list -> remove (real, reversible on scoop) --------------
echo "[5] Real lifecycle"
# This host is not disposable. If it already had the package, the uninstall below would
# take away something the developer chose, so it is put back at the end and the run says
# so rather than leaving a hole nobody notices.
PKG_WAS_HERE=""
# `on_path` here and `binary_present` below, deliberately: this runs BEFORE the install, so
# there is no log in which anything could have named a directory yet.
on_path "$PKG" && PKG_WAS_HERE=1
# And WHERE it resolved, which is what tells a binary this install put there from one that was
# already on PATH under the same name (G-3). The lifecycle sweep below has always recorded this
# for the removal half; section 5 recorded nothing.
#
# **Empty when the host already had the package**, and that is the whole subtlety. This sweep
# does not remove a developer's software, so when `$PKG` was already installed the resolution
# CANNOT change and comparing against it calls a correct run a failure. Measured on the macOS
# runner, which ships wget: `FAIL brew: wget resolves to /opt/homebrew/bin/wget, which was
# already there before this install`. The canary loop below needs no such guard — it skips a
# canary this host already has (`left alone rather than removed`), so anything it does install
# is genuinely new.
PKG_PREPATH=""
[ -z "$PKG_WAS_HERE" ] && PKG_PREPATH="$(path_of "$PKG")"

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
    assert_binary_reachable "$BACKEND" "$PKG" /tmp/itw-life0.out "$PKG_PREPATH"
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
nok "installing a nonexistent package fails" lx -y install "$BACKEND:shall-no-such-pkg-zzz"
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
IMPERATIVE="$SHALL_CONFIG_DIR/modules/imperative.txt"
if [ -f "$IMPERATIVE" ]; then
    nok "the unresolvable name is out of the manifest" \
        grep -q "shall-no-such-pkg-zzz" "$IMPERATIVE"
fi

# --- 7. Adopt (II.9: Windows managers install no deps, so adopt is exact) --
echo "[7] Adopt"
ADOPTED_FILE="$SHALL_CONFIG_DIR/modules/adopted.txt"
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
# only that the binary still exists — which it would whatever Shall did.
$TO "$SHALL" -y uninstall shall >/dev/null 2>&1 || true
ok "Shall survives an uninstall attempt" on_path "$SHALL"
# **This establishes its premise instead of assuming the host's package census supplies
# one.** It used to assert only that `purge-undeclared` refuses after adopt, and what did
# the refusing was the ratio (II.11) — whose denominator is the undeclared crawl. When the
# crawl stopped surveying `service:` (correctly: `priority` names package managers, and a
# sweep must not propose to delete every running service), several hundred entries left that
# denominator on every host, the ratio rose over 0.1, and the refusal went away. Nothing
# about the guard changed. The assertion had been reading a property of the runner image.
#
# So the premise is built here: one package that IS undeclared on this host is written into
# `protected_packages`, and the refusal that names it is the thing under test. That holds on
# a machine with four undeclared packages and on one with four hundred.
PREFS="$SHALL_CONFIG_DIR/preferences.toml"
# The list is printed before the ratio is consulted, so it is there whether the command
# would have gone on to refuse or to sweep.
#
# **The first name that cannot break the file it is written into, not simply the first.**
# winget reports MSIX packages as `MSIX\AdobeAcrobatDCCoreApp_23.1.0.0_x64__pc75e8sa7ep4e`,
# and a backslash in a TOML basic string is an escape — `\A` is not one, so the whole
# preferences file failed to parse and the command exited 1. A refusal that never happened
# because the config was unreadable would have read as this test passing had it been
# asserting on the exit code alone.
VICTIM=$($TO "$SHALL" --dry-run purge-undeclared 2>/dev/null \
    | sed -n 's/^  [a-z0-9]*:\([A-Za-z0-9][A-Za-z0-9._+-]*\)$/\1/p' | head -1)
if [ -n "$VICTIM" ]; then
    # Restored by copy, not by editing the file back. A `sed` that deletes from `[guard]`
    # to EOF also deletes whatever a later step put there, and this harness runs 300 more
    # checks against this config.
    [ -f "$PREFS" ] && cp "$PREFS" "$PREFS.beforeguard"
    printf '\n[guard]\nprotected_packages = ["%s"]\n' "$VICTIM" >> "$PREFS"
    echo "        protecting an undeclared package by name: $VICTIM"
    refuses_with_3 "purge-undeclared refuses a sweep that would take a protected package" \
        lx -y purge-undeclared
    grep_ok "and the refusal names the protection rather than some other failure" \
        "protected" lx -y purge-undeclared
    if [ -f "$PREFS.beforeguard" ]; then
        mv "$PREFS.beforeguard" "$PREFS"
    else
        rm -f "$PREFS"
    fi
else
    soft "purge-undeclared: nothing is undeclared on this host, so the protected-package refusal has no subject"
fi

# --- 9. Git history + rollback --------------------------------------------
echo "[9] Git history + rollback"
if ok "git init" lx git init; then
    ok "git status reads the repo" lx git status
    # Driven through the binary, not `sh -c "lx …"`: `lx` is a function and a subshell
    # never sees it, so the old form ran nothing and reported whatever came after.
    $TO "$SHALL" -y sync >/dev/null 2>&1 || true
    ok "sync commits" lx git log --limit 5
    # `shall` matches the config path, the binary name and half the error messages.
    # `shall:` is the commit-subject prefix and nothing else — grep for what only the
    # right answer contains (IV.1), especially with a config dir named shall-it-win-*.
    grep_ok "git log shows a shall commit" "shall:" lx git log --limit 10
    ok "git commit records the current state on demand" lx git commit -m "shall: harness checkpoint"
    ok "diff HEAD runs" lx diff HEAD
    ok "rollback HEAD accepted" lx -y rollback HEAD
fi

# --- 10. rebuild asserts, and writes no commit (K14) ----------------------
echo "[10] rebuild"
commits() { git -C "$SHALL_CONFIG_DIR" rev-list --count HEAD 2>/dev/null || echo 0; }
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
LOCKFILE=$(ls "$SHALL_CONFIG_DIR"/locks/bare.*.toml 2>/dev/null | head -1)
echo "        lock file: ${LOCKFILE:-<none>}"
ok  "a chain is legal"           lx --dry-run install "$BACKEND,cargo:$PKG"
ok  "a chain may end in list"    lx --dry-run install "$BACKEND,list:$PKG"
ok  "list alone is legal"        lx --dry-run install "list:$PKG"
nok_saying "an empty slot is refused" "has an empty backend"   lx --dry-run install "$BACKEND,,cargo:$PKG"
nok_saying "an unknown link is refused" "is not a backend Shall uses" lx --dry-run install "$BACKEND,nope:$PKG"
nok_saying "list must come last" "must come last"        lx --dry-run install "list,$BACKEND:$PKG"
nok_saying "a name repeated is refused" "is named twice" lx --dry-run install "$BACKEND,$BACKEND:$PKG"
nok_saying "a pattern cannot span one" "must match in exactly one backend"  lx --dry-run install "$BACKEND,cargo:re:^$PKG"
# A manager no Windows host has: a pin to it must say so rather than no-op.
nok "a pin to a manager this host lacks is not silent" lx -y install "apt:$PKG"
ok  "unlock backends --list runs"  lx unlock backends --list
ok  "unlocking an unfrozen name is not an error" lx unlock backends shall-never-frozen-zzz
# Z2: the axis is not optional in the sense that matters — a bare name is not one.
nok_saying "a name where the axis goes is refused" "invalid value" lx unlock shall-never-frozen-zzz

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
    SILENT_BIN="${TMPDIR:-/tmp}/shall-it-silent-bin"
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

    SILENT_CFG="${TMPDIR:-/tmp}/shall-it-silent"
    rm -rf "$SILENT_CFG"; mkdir -p "$SILENT_CFG/modules" "$SILENT_CFG/profiles"
    printf 'cargo\n%s\n' "$BACKEND" > "$SILENT_CFG/priority"
    printf 'Work\n' > "$SILENT_CFG/active"
    printf 'use base\n' > "$SILENT_CFG/profiles/Work"
    printf '%s\n' "$PKG" > "$SILENT_CFG/modules/base.txt"

    silent_lx() {
        env PATH="$SILENT_BIN:$PATH" \
            SHALL_CONFIG_DIR="$(cygpath -w "$SILENT_CFG" 2>/dev/null || echo "$SILENT_CFG")" \
            SHALL_DATA_DIR="$(cygpath -w "$SILENT_CFG/state" 2>/dev/null || echo "$SILENT_CFG/state")" \
            $TO "$SHALL" "$@"
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

# The ceiling for the block below. It may only go DOWN; raising it is Q4's item 4 happening.
#
# Was 15, measured: that many backends were in neither `canary()` nor `no_lifecycle_reason()`.
# The nightly of 2026-08-11 printed them by name — `14 backend(s) have no path to a real
# lifecycle (ceiling 15) — cabal composer conda flatpak macports mix nix opam pkg pkg_add pkgin
# snap spack stack` — under a line calling itself the release blocker, and that had been the
# whole of the report for weeks: a count, with no statement about any individual member.
#
# **A count is not an answer, and fourteen unexplained skips is the vacuous check IV.1 bans,
# fourteen times.** Every one of those names now says either what runs it or why nothing here
# can, so the expected count is nought. Derived rather than measured, and stated as such: the
# nightly is what confirms it, and it fails BY NAME if a fifteenth appears.
LIFECYCLE_GAP_CEILING=0
canary() {
    case "$1" in
        scoop)    echo "jq|jq|full|" ;;
        # zoxide, and not jq/rg/fd: those three are this host's scoop, pixi and github canaries,
        # so a leftover from any of them would answer winget's check for it. It is also a
        # portable zip package, which winget installs under the user's own profile with the argv
        # Shall already sends — measured 2026-07-30, install and uninstall both clean, with no
        # `--scope` flag and no elevation.
        #
        # **No binary is asserted, and the reason is not that it is missing.** A winget portable
        # package lands in `%LOCALAPPDATA%\Microsoft\WinGet\Links`, and winget adds that
        # directory to the *persisted* user PATH — `Path environment variable modified; restart
        # your shell to use the new value`. A shell that is already running cannot see it, so
        # `on_path` asks a question whose honest answer is "yes, in your next shell". Nor does
        # the off-PATH fallback apply: that reads Shall's own "installs its executables into
        # DIR, which is not on your PATH" warning, and Shall is right not to print it here —
        # the directory IS on PATH, just not on this process's copy of it.
        #
        # `list --backend winget` is the presence assertion, and the lifecycle still proves
        # install → list → uninstall → gone. Measured by hand 2026-07-30: the alias is created,
        # `winget list` shows it, uninstall removes it, and `winget list` then finds nothing.
        winget)   echo "ajeetdsouza.zoxide||full|" ;;
        # Chocolatey only reaches this row on an elevated shell; `no_lifecycle_reason` says so
        # and skips it otherwise. `bat` is nothing else's canary on this platform.
        #
        # It is NOT dependency-free, and the line that said so was never measured: an elevated
        # `choco install -y bat` pulls eleven packages — three chocolatey extensions, five KB
        # hotfixes, `less` and `vcredist140` — and any one of them can fail or ask for a reboot
        # while `bat` itself never installs. That is the whole of what CI 30684191791 found, and
        # it is why the canary stays as it is: a dependency-free package would pass this row
        # without ever exercising the path that broke.
        choco)    echo "bat|bat|full|" ;;
        # A PowerShell module, so there is no binary on PATH — the field is empty rather than
        # faked, and `list --backend psresource` is the presence assertion.
        psresource) echo "powershell-yaml||full|" ;;
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
        gem)      echo "colorize||full|" ;;
        cargo)    echo "hexyl|hexyl|full|" ;;
        github)   echo "sharkdp/fd|fd|full|fd" ;;
        # The three the disposable-host rule opened up. Each is a manager that works fine here
        # and was declined out of respect for the developer's machine, so each gets a canary
        # that only runs where `disposable_host` is true.
        #
        # No binary check on any of them: a pip module, a VS Code extension and an Emacs package
        # are none of them a program on PATH, and asserting one would be asserting a guess.
        # `list` is the presence proof, as it is for `helm` and `mise`.
        pip)      echo "six||full|" ;;
        vscode)   echo "ms-python.python||full|" ;;
        emacs)    echo "hydra||full|" ;;
        # A pinned release asset, so a red run means one thing. The list-token is the name Shall
        # records for a `web:` install, which is derived from the URL rather than chosen here.
        web)      echo "https://github.com/bootandy/dust/releases/download/v1.1.1/dust-v1.1.1-x86_64-pc-windows-msvc.zip|dust|full|dust" ;;
        mise)     echo "jq||full|" ;;
        asdf)     echo "jq||full|" ;;
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

# True when this process can write the machine-wide locations an installer needs. `net session`
# is the cheapest reliable answer on a shell that has no `id -u` worth trusting: it needs the
# Server service and fails with "Access is denied" for a non-elevated token.
is_elevated() {
    net session >/dev/null 2>&1
}

# Is this host disposable — a runner that is destroyed after the job, rather than somebody's
# machine?
#
# Four exemptions below are host-RESPECT rather than impossibility: Shall can install a VS Code
# extension, an Emacs package or a system-Python module perfectly well, and this sweep declines
# to do it on a developer's box because it would leave their editor and their Python changed.
# On a runner there is nobody to inconvenience, and `Q4` is explicit that an exemption must be
# something the harness genuinely cannot do — not something it would rather not.
#
# Detected, never assumed: `CI` is set by GitHub Actions and by every other runner worth the
# name, and a developer who wants the wide sweep can set it for one run.
disposable_host() { [ -n "${CI:-}" ]; }

no_lifecycle_reason() {
    case "$1" in
        # winget, choco and psresource were excused here until 2026-07-30 on the grounds that
        # they touch the real machine — which is true of scoop, npm and cargo too, and those
        # have had real lifecycles all along. The owner ruled the excuse away: install and
        # uninstall, like every other manager. What is left is DETECTED rather than assumed, so
        # a host that can run one gets a real lifecycle and only a host that genuinely cannot
        # prints a reason. An assumed skip is a check nobody will ever revisit.
        choco)      is_elevated || echo "chocolatey writes to C:\\ProgramData and this shell is not elevated, so the install would fail on permissions rather than on anything Shall did — re-run from an elevated shell to lifecycle it" ;;
        psresource) powershell -NoProfile -Command "exit (\$null -eq (Get-Command Install-PSResource -ErrorAction SilentlyContinue))" >/dev/null 2>&1 \
                        || echo "this host has no PSResourceGet cmdlets, so there is no manager here to lifecycle — Shall's own health check prints the one command that installs it" ;;
        mas)        echo "needs a signed-in App Store account — plan-smoked instead" ;;
        pip)        disposable_host || echo "installs into the system Python this host runs on, and this host is somebody's — plan-smoked instead" ;;
        link)       echo "a dependent statement (link:SRC), not a package name — smoked in 13" ;;
        service)    echo "a dependent statement (service:NAME), and starting one mutates the host" ;;
        setting)    echo "a dependent statement (setting:K @value=), and it writes a live desktop setting" ;;
        vscode)     disposable_host || echo "installs an extension into the developer's real editor profile" ;;
        emacs)      disposable_host || echo "installs a package into the developer's real Emacs profile" ;;
        mise|asdf)  disposable_host || echo "rewrites the host's tool-version shims" ;;
        # `web:` had no lifecycle ANYWHERE, and the reason given was "no stable public
        # canary" — which was a search nobody had done rather than a fact. A pinned GitHub
        # release asset is exactly a stable public URL; `dust` is chosen because nothing else
        # in this table installs it, so the binary it leaves cannot be confused with another
        # backend's canary. `appimage` keeps the reason it has: Windows cannot run one.
        appimage)   echo "an AppImage is a Linux artifact; this host cannot run one — lifecycled on the storage image instead" ;;
        # It IS an install target — `btrfs:PATH` runs `subvolume create` — and the sentence that
        # stood here until 2026-07-31 ("a snapshot provider, not an install target") is the one
        # that kept three destructive backends unrun for months. The Linux harness was corrected
        # on 2026-07-30 and this copy was not, which is the twin-site failure this repo keeps
        # finding. What actually excuses it is the machine, not the backend: creating a subvolume
        # needs a real btrfs filesystem, and Windows has none to give.
        btrfs|lvm|zfs) echo "$1 needs a real block device on a Linux filesystem; the privileged \`storage\` image lifecycles it for real, and this host cannot" ;;

        # ---- The fourteen this sweep counted and never named ----------------
        #
        # Every line below is either "another harness runs it" or a confession. A backend that
        # is lifecycled in the `tools` container is NOT untested — it is untested *here*, and
        # saying which is the difference between a coverage gap and a coverage map. The ones
        # with no runner anywhere are the honest half, and they are what the README has to say
        # out loud before a release (Q4).

        # Linux-only, by the kernel and the daemon they need. There is no version of these that
        # runs on macOS or Windows, so this is a wall rather than a cost.
        flatpak)    echo "Flatpak is a Linux runtime with a session bus; this host is not Linux — the container matrix names its own reason for it" ;;
        snap)       echo "snapd is a Linux daemon over systemd; this host is not Linux, and no image in the matrix runs systemd either — argv-tested only, everywhere" ;;

        # No BSD host exists anywhere in this project's CI. Named so the gap is a stated fact
        # rather than an absence somebody has to notice.
        pkg)        echo "FreeBSD's native manager, and there is no FreeBSD host in this matrix at all — argv-tested only, everywhere" ;;
        pkg_add)    echo "OpenBSD's native manager, and there is no OpenBSD host in this matrix at all — argv-tested only, everywhere" ;;
        pkgin)      echo "the pkgsrc binary manager (NetBSD, SmartOS), and there is no pkgsrc host in this matrix at all — argv-tested only, everywhere" ;;

        # **The one on this list a runner could actually attempt**, and nobody has. Written as a
        # cost rather than a wall on purpose: `nix` carried "impossible" for months on reasoning
        # nobody re-derived, and it turned out to be a price (Q17).
        macports)   echo "MacPorts installs a whole second package tree under /opt/local beside Homebrew and has never been attempted on a runner — a cost nobody has priced, not a wall" ;;

        # Lifecycled for real in the `tools` container image, which ships each of these and has
        # a canary for it. Installing them here would be testing their installers on somebody's
        # machine, which is what `disposable_host` exists to refuse.
        cabal|composer|conda|mix|nix|opam|spack)
                    echo "$1 gets a real install/list/binary/remove in the \`tools\` container image, which ships it and has a canary for it; installing it here would test its installer rather than Shall" ;;

        # The same image, and the same reason the container harness gives: the toolchain can be
        # baked in, the package's build cannot.
        stack)      echo "a Haskell package builds from source, so the toolchain can be baked into an image and the build cannot — smoked here, and named the same way in the container harness" ;;

        *)          echo "" ;;
    esac
}

# A manager whose own uninstall deletes the package and keeps its launcher. Reported,
# never assumed: the strict check runs first, and this only softens the result when the
# leftover actually happens — so a manager that starts cleaning up still passes.
removal_leaves_binary() {
    case "$1" in
        bun) echo "bun's own \`remove -g\` drops the package and keeps its .exe/.bunx launchers (reproduced against bun directly, with no Shall involved)" ;;
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
    $TO "$SHALL" unmanage "$1" >/dev/null 2>&1 || true
    _imp="$SHALL_CONFIG_DIR/modules/imperative.txt"
    [ -f "$_imp" ] || return 0
    grep -v -F "$1" "$_imp" > "$_imp.tmp" 2>/dev/null
    mv "$_imp.tmp" "$_imp"
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
    grep -qx "$be" "$SHALL_CONFIG_DIR/priority" 2>/dev/null || echo "$be" >> "$SHALL_CONFIG_DIR/priority"

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
        $TO "$SHALL" unmanage "$be:$cpkg" >/dev/null 2>&1 || true
        _imp="$SHALL_CONFIG_DIR/modules/imperative.txt"
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
    [ -n "$cbin" ] && assert_binary_reachable "$be" "$cbin" /tmp/itw-life.out "$_prepath"

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
        "$SHALL list --backend '$be' 2>/dev/null | grep -q '$ctok'"
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

# The dependent statements this harness drives for REAL, as case labels.
#
# Its twin in `docker/integration/run-in-container.sh` holds the same table for the container
# matrix, and `lifecycle_coverage_union_tests` reads BOTH — because a dependent statement can be
# unreachable on one platform and trivial on the other, which is exactly the case here.
dependent_lifecycle() {
    case "$1" in
        setting)  echo "section 12b: written to HKCU, read back, changed, undeclared, reset" ;;
        *)        echo "" ;;
    esac
}

# ==========================================================================
# 12b. A DEPENDENT STATEMENT DRIVEN FOR REAL — setting:
# ==========================================================================
# `setting:` was exempted in both harnesses as *"it writes to a live desktop settings store
# (dconf/gsettings) that no image here runs a bus for"*. True of Linux, and it was never asked
# of Windows: `setting_stores.toml` says the store here is `reg`, which needs no bus, no session
# and no desktop. A per-user value under a key nothing else reads is as harmless as the temp
# directory this harness already writes into.
#
# **The teardown is the half worth having.** Removing a `setting:` line runs the store's `reset`,
# which returns the key to the store's own default — on Windows, `reg delete`. Nothing had ever
# observed that happen against a real registry.
echo "[12b] setting: driven for real, through the Windows registry"
if command -v reg >/dev/null 2>&1; then
    _set_f0=$FAILC
    SET_SUBKEY='Software\ShallIntegrationCanary'
    SET_NAME=Mode
    SET_MOD="$SHALL_CONFIG_DIR/modules/imperative.txt"

    # **`reg` is called through this, because a bare `reg query ... /v NAME` cannot work here.**
    # This harness runs under Git Bash, whose MSYS layer rewrites any argument that looks like an
    # absolute path — and `/v` and `/f` look exactly like one. `reg` then answers `ERROR: Invalid
    # syntax` to every call. Measured on Windows 11: the same query is `Invalid syntax` bare and
    # returns the value under `MSYS_NO_PATHCONV=1`.
    #
    # It cost four wrong verdicts on this section's first night, and only two of them looked like
    # failures. The two `grep_ok`s reported the value missing from a registry that had it; the two
    # `nok` controls — "the value does not exist before sync", "the value is really gone" — PASSED,
    # because a command that always fails is indistinguishable from a key that is really absent.
    # A control satisfied by its own instrument being broken is the vacuous check this repository
    # keeps finding, arriving here as the thing that hid the other two.
    #
    # `scripts/unix-check.sh` already sets this variable for the same reason.
    winreg() { MSYS_NO_PATHCONV=1 reg "$@"; }

    # And the positive control, because the paragraph above is the whole argument for it: prove
    # `reg query` can read a value that certainly exists, before believing it about one that
    # should not. `CurrentVersion\ProgramFilesDir` is present on every Windows since NT.
    grep_ok "reg query can read the registry at all (the control the controls needed)" \
        "REG_SZ" \
        winreg query 'HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion' /v ProgramFilesDir

    winreg delete "HKCU\\$SET_SUBKEY" /f >/dev/null 2>&1
    # The control: the value must be absent before the sync that writes it, or every assertion
    # below passes over whatever a previous run left behind.
    nok "the registry value does not exist before sync" \
        winreg query "HKCU\\$SET_SUBKEY" /v "$SET_NAME"
    printf 'setting:%s/%s @value=prefer-dark\n' "$SET_SUBKEY" "$SET_NAME" >> "$SET_MOD"
    ok "sync applies a declared setting" lx -y sync
    grep_ok "the value is really in the registry" "prefer-dark" \
        winreg query "HKCU\\$SET_SUBKEY" /v "$SET_NAME"
    # A changed declaration must reach the store. A `setting:` that only ever wrote on first
    # sight would look identical to a working one until the day somebody edited the value.
    grep -v -F "setting:$SET_SUBKEY/$SET_NAME " "$SET_MOD" > "$SET_MOD.tmp" 2>/dev/null
    mv "$SET_MOD.tmp" "$SET_MOD"
    printf 'setting:%s/%s @value=prefer-light\n' "$SET_SUBKEY" "$SET_NAME" >> "$SET_MOD"
    ok "sync applies a CHANGED setting" lx -y sync
    grep_ok "the new value replaced the old one" "prefer-light" \
        winreg query "HKCU\\$SET_SUBKEY" /v "$SET_NAME"
    grep -v -F "setting:$SET_SUBKEY/$SET_NAME " "$SET_MOD" > "$SET_MOD.tmp" 2>/dev/null
    mv "$SET_MOD.tmp" "$SET_MOD"
    ok "sync resets a setting whose declaration is gone" lx -y sync
    nok "the value is really gone from the registry" \
        winreg query "HKCU\\$SET_SUBKEY" /v "$SET_NAME"
    winreg delete "HKCU\\$SET_SUBKEY" /f >/dev/null 2>&1
    # Credited only when the whole block passed: a ledger row written before the assertions
    # is the harness telling the ratchet a number the machine did not give it.
    [ "$FAILC" = "$_set_f0" ] && echo "setting" >> "$LEDGER/be-life"
else
    soft "setting: no \`reg\` on this host, which is not a Windows host — nothing to drive"
fi

# ==========================================================================
# 13. PLAN-SMOKE — every backend this host cannot (or must not) run for real
# ==========================================================================
echo "[13] Plan-smoke, every backend not lifecycled above"

ALL_BACKENDS=$(lx check health --json 2>/dev/null \
    | sed -n 's/.*"backend"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | sort -u)
echo "        registered backends: $(echo $ALL_BACKENDS | wc -w)"
ok "check health --json enumerates the registry" test -n "$ALL_BACKENDS"

SMOKE_CFG="${TMPDIR:-/tmp}/shall-it-win-smoke"
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
    env SHALL_CONFIG_DIR="$SMOKE_CFG_ARG" SHALL_DATA_DIR="$SMOKE_DATA_ARG" $TO "$SHALL" "$@"
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
        # The storage effectors get the same plan-smoke as everything else, carrying the options
        # that make them declarations rather than bare names (Q18). This host has no such
        # filesystem, which is what a plan-smoke is for: the grammar, the planner and the argv
        # are exercised without a device.
        btrfs)    echo "/mnt/fs/data@quota=10G,mount=/srv" ;;
        lvm)      echo "vg0/data@size=10G" ;;
        zfs)      echo "tank/data@quota=10G,mount=/mnt/data" ;;
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
# `eval` is the one output that will acquire consumers Shall cannot see, so the thing
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
ok "check unmanaged lists what Shall does not manage" lx check unmanaged
ok "path prints the config repo" lx path
ok "path --explain says which source won" lx path --explain
ok "config show prints the active configuration" lx config show
ok "policy checks the desired state against [guard]" lx policy
ok "check conflicts reports cross-backend conflicts" lx check conflicts
# `adapters` (S78) — the eight extension surfaces. Twin of the block in
# `docker/integration/run-in-container.sh`; change one, change the other. A fresh machine has no
# `adapters/` directory at all, which is the state almost every machine is in and the one a
# survey must report without complaining about.
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
printf '[[backend]]\nname = "mymgr"\ninstall = "cmd /c exit 0"\n' > "$SHALL_CONFIG_DIR/adapters/backends.toml"
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
ok "sbom emits a bill of materials" lx sbom
# `try` rehearses in a container. Named against an image that cannot exist, so the
# answer is a refusal on every host: with no runtime it refuses for want of one, with a
# runtime it refuses for want of the image — and neither spends ten minutes building.
refuses_with_3 "try refuses to rehearse on an image that is not there" lx try --image shall-it-no-such-image
grep_ok "try's refusal says what it refused" "refusing to rehearse" lx try --image shall-it-no-such-image
# `add` vendors a source's modules. A local path is the network-free case: it copies the
# module in and reports it. The line names the package this run already manages, so
# vendoring it declares nothing new.
SHARE_SRC="${TMPDIR:-/tmp}/shall-it-share"
rm -rf "$SHARE_SRC" 2>/dev/null; mkdir -p "$SHARE_SRC/modules"
printf '%s:%s\n' "$BACKEND" "$PKG" > "$SHARE_SRC/modules/shared.txt"
ok "add vendors a module from a local source" lx add "$SHARE_SRC"
ok "add brought the module file in" test -f "$SHALL_CONFIG_DIR/modules/shared.txt"
nok "add refuses a source that does not exist" lx add "${TMPDIR:-/tmp}/shall-it-no-such-source"
# Proved, then taken back out: a module left behind changes what every section after
# this one plans, and this section is about `add`, not about the model.
rm -f "$SHALL_CONFIG_DIR/modules/shared.txt"
ok "completions powershell generates a script" lx completions powershell
ok "profile list" lx profile list
ok "profile active" lx profile active
ok "profile create scaffolds one" lx profile create HarnessProfile
# "scaffolds" is a claim about the disk and the line above only reads an exit code. Found by
# running this harness against a `shall` that does nothing and exits 0: both `create` checks
# and both `show` checks passed, because not one of the four ever looked at a file.
ok "profile create wrote the profile" test -f "$SHALL_CONFIG_DIR/profiles/HarnessProfile"
ok "profile show reads it back" lx profile show HarnessProfile
ok "module list" lx module list
ok "module create scaffolds one" lx module create harness-module
ok "module create wrote the module" test -f "$SHALL_CONFIG_DIR/modules/harness-module.txt"
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
# `heal` when there is nothing to recover — and the check has to say WHICH, because by the time
# it runs this sweep has deliberately failed an install and that leaves a `Failed` entry in the
# journal, which `get_incomplete_actions` counts as incomplete. The old check asserted rc=0 and
# therefore passed or failed on whether an earlier deliberate failure happened to still be
# there: heads on ubuntu, tails on tools and macOS, in one CI run. Proved not a regression by
# building the pre-session tree in a worktree and getting the same rc=1.
#
# What W36 ruled is what is asserted: heal NAMES what it could not recover and does not exit 0
# while saying so. Both states are legitimate; the two that cannot both be true are not.
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
if lx check security >/tmp/itw.out 2>&1; then
    PASS=$((PASS + 1)); echo "  PASS  check security scans for vulnerabilities"
else
    soft "check security ran but could not reach the OSV.dev database"
fi
ok "export writes native manifests" lx export --out "${TMPDIR:-/tmp}/shall-it-win-export"
# PINNED to this host's manager. An unpinned name resolved to a library crate on a
# machine that had cargo and not the tool, so the check failed on the resolver's
# answer rather than on `run`.
ok "run executes inside an ephemeral environment" lx run -p "$BACKEND:$PKG" true

# `plan` exits 2 when it finds work (`H2`, owner 2026-08-13) — it is a read-only command
# that looked, which is what 2 means. `answers` is the helper for exactly that: 0 or 2 is
# an answer, 1 and 3 are not.
answers "plan freezes a reviewable file" lx plan --out "${TMPDIR:-/tmp}/shall-it-win-plan.json"
ok "the plan file exists" test -f "${TMPDIR:-/tmp}/shall-it-win-plan.json"
ok "apply reads a saved plan" lx --dry-run apply "${TMPDIR:-/tmp}/shall-it-win-plan.json"

# `edit` shells out to $VISUAL/$EDITOR; `true` is an editor that exits 0.
record_argv edit priority
ok "edit opens a file in \$EDITOR" env EDITOR=true VISUAL=true $TO "$SHALL" edit priority

# reset deletes the registry. The command is exercised through the refusal it owes a
# machine that still has a config repo — running it for real would end the run.
refuses_with_3 "reset refuses while a config repo still exists" lx reset
grep_ok "and says --force is what overrides it" "force" lx reset

ok "self-upgrade --check reports the version and source" lx self-upgrade --check

# --- 14b. bundle → restore, the round trip (V.59) -------------------------
echo "[14b] bundle → restore"
BUNDLE_DIR="${TMPDIR:-/tmp}/shall-it-win-bundle"
RESTORE_DIR="${TMPDIR:-/tmp}/shall-it-win-restored"
rm -rf "$BUNDLE_DIR" "$RESTORE_DIR" 2>/dev/null
ok "bundle packs the config" lx bundle --out "$BUNDLE_DIR"
ok "the bundle directory exists" test -d "$BUNDLE_DIR"
mkdir -p "$RESTORE_DIR"
RESTORE_ARG="$(cygpath -w "$RESTORE_DIR" 2>/dev/null || echo "$RESTORE_DIR")"
# The data dir is a SIBLING, not a child: put Shall's state inside the config directory
# and the very first command makes that directory non-empty, so `restore` refuses it —
# and the test for "restores into a clean directory" can never run.
RESTORE_STATE_DIR="${TMPDIR:-/tmp}/shall-it-win-restored-state"
rm -rf "$RESTORE_STATE_DIR" 2>/dev/null
RESTORE_STATE_ARG="$(cygpath -w "$RESTORE_STATE_DIR" 2>/dev/null || echo "$RESTORE_STATE_DIR")"
restore_lx() {
    env SHALL_CONFIG_DIR="$RESTORE_ARG" SHALL_DATA_DIR="$RESTORE_STATE_ARG" $TO "$SHALL" "$@"
}
record_argv restore "$BUNDLE_DIR"
ok "restore into a clean config directory" restore_lx restore "$BUNDLE_DIR"
answers "the restored model parses" restore_lx check
refuses_with_3 "restore refuses a config directory that is not empty" restore_lx restore "$BUNDLE_DIR"
ok "and --force overrides it" restore_lx restore "$BUNDLE_DIR" --force

# --- 14c. `--help` for the whole surface ----------------------------------
# Kept, but demoted: it catches a subcommand whose clap wiring is broken, and the
# audit below does not accept it as coverage.
echo "[14c] --help across the surface"
HELP_CMDS=$("$SHALL" --help 2>&1 | sed -n '/^Commands:/,/^Options:/p' \
    | sed -n 's/^  \([a-z][a-z-]*\) .*/\1/p' | grep -v '^help$' | sort -u)
for c in $HELP_CMDS; do
    ok "\`$c --help\` exists" lx "$c" --help
done

# ==========================================================================
# 14d. A REAL CRASH IN THE MIDDLE OF A TRANSACTION (GRADER §5)
# ==========================================================================
# The twin of `run-in-container.sh`'s 16d, and it is here because a check on one harness is a
# check on one platform — round 7's own finding, about `winget`. The write-ahead log, the
# recovery and the data lock are all cross-platform code with a Windows-specific file layer
# under them, which is exactly where "it works on Linux" stops being evidence.
#
# Three things this platform cannot do, each MEASURED here rather than assumed:
#   * no group kill — `setsid` does not exist on Windows, so the package manager cannot be
#     taken down with Shall and the hostile third iteration the container runs is absent.
#   * no `sudo` section — `run_on` inserts `sudo` only when `!cfg!(windows)`, so there is no
#     privileged path on this platform to drive. Its absence is correct, not a gap.
#   * the canaries are scoop's, because scoop is user-scoped and reversible; a machine-wide
#     manager is not something to crash halfway through on somebody's real computer.
echo "[14d] SIGKILL mid-transaction, then heal"

JOURNAL="$SHALL_DATA_DIR/journal.jsonl"

# How many operations are still OPEN — and the emphasis is the whole point.
#
# `journal.jsonl` is APPEND-ONLY: one line per state change, carrying the same id each time, so
# a single successful install writes `InProgress` and then `Completed` and both lines stay.
# Counting `InProgress` lines therefore answers "how many operations ever started", which on a
# healthy run is every one of them. The first draft of the container twin did exactly that and
# reported 32 operations open on a run where heal had resolved all of them — a finding
# manufactured by the instrument. So the question is about the LAST line for each id.
journal_status_tally() { # awk-condition over the final status of each id
    [ -f "$JOURNAL" ] || { echo 0; return 0; }
    sed -n 's/.*"id":"\([^"]*\)".*"status":"\([^"]*\)".*/\1 \2/p' "$JOURNAL" \
        | awk -v want="$1" '
            { last[$1] = $2 }
            END { n = 0; for (k in last) if (index(want, last[k]) > 0) n++; print n + 0 }'
}
journal_open()       { journal_status_tally "InProgress Abandoned"; }
journal_incomplete() { journal_status_tally "InProgress Abandoned Failed"; }
# Operations the log has closed. The `completed` iteration below kills the run once this
# rises, which is the window neither of the other two can reach: an install the log already
# calls done, in a process that has not yet written the ownership registry.
journal_completed()  { journal_status_tally "Completed"; }
journal_open_names() {
    [ -f "$JOURNAL" ] || return 0
    sed -n 's/.*"id":"\([^"]*\)".*"status":"\([^"]*\)".*/\1 \2/p' "$JOURNAL" \
        | awk '{ last[$1] = $2 } END { for (k in last) if (last[k] == "InProgress" || last[k] == "Abandoned") print "        | " k }'
}

# scoop packages whose binary IS the package name, and none of them another canary here: `jq`
# is scoop's own, `rg` is pixi's, `fd` is github's and `zoxide` is winget's. Two canaries
# sharing a binary name is the G-3 collision by construction.
#
# FILTERED against the machine: this is a developer's real computer, and a package it already
# has is not a transaction step. What is left is what this host can turn into one.
CRASH_PKGS=""
for _c in gron grex tokei; do
    on_path "$_c" || CRASH_PKGS="$CRASH_PKGS $_c"
done
CRASH_N=0
for _c in $CRASH_PKGS; do CRASH_N=$((CRASH_N + 1)); done

# Whether this host can actually INSTALL that fixture, which is a different question from
# whether the binaries are absent from PATH. Set by the control sync below, read by the
# killed-holder recovery check — which declares the same fixture and judges a *sync* by its exit
# code, so a run that failed because the fixture is unavailable would be that sentence about
# something else. Found on the container side, where slackware carries none of its three
# candidates; kept in step here because the two harnesses have the same section and the same
# premise.
CRASH_FIXTURE_OK=0

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
crash_installed() { _n=0; for _p in $CRASH_PKGS; do on_path "$_p" && _n=$((_n + 1)); done; echo "$_n"; }
crash_missing()   { _m=""; for _p in $CRASH_PKGS; do on_path "$_p" || _m="$_m $_p"; done; echo "$_m"; }
# UNINSTALL FIRST, undeclare second. `shall uninstall` refuses a package no active file
# declares — *"nothing was uninstalled: it is not declared in any active file"* — so taking the
# line out first makes every cleanup refuse. Measured on the container twin, where it cost two
# thirds of the section's coverage before anyone noticed.
crash_wipe() {
    : > /tmp/crash-wipe-win.out
    for _p in $CRASH_PKGS; do
        on_path "$_p" || continue
        {
            echo "--- uninstall $BACKEND:$_p"
            echo "    declared in imperative.txt: $(grep -cx "$BACKEND:$_p" "$IMPERATIVE" 2>/dev/null || echo 0)"
        } >> /tmp/crash-wipe-win.out
        $TO "$SHALL" -y uninstall "$BACKEND:$_p" >> /tmp/crash-wipe-win.out 2>&1
        echo "    rc=$? and $_p is now $(on_path "$_p" && echo 'STILL on PATH' || echo 'gone')" >> /tmp/crash-wipe-win.out
    done
    crash_undeclare
}

# crash_run <tag> <when>
#   when   `open`       kill the moment the log opens a new entry — the manager may not have started
#          <n>          kill once n of the canaries have reached the filesystem
#          `completed`  kill once the log has CLOSED an operation this run opened
#
# The third one is `S87`'s window, and it is not a variation on the other two. Ownership is
# held in memory through a sync and written to `registry.json` once, at the end; the log is
# written per operation. A kill between those two leaves the package installed, `Completed` in
# the log — so recovery has nothing to replay — and owned by nobody, and the one command for
# removing it then plans no change and reports success. Polling the filesystem cannot aim at
# that window: a canary reaches disk well before its entry closes.
crash_run() {
    _tag="$1"; _when="$2"
    _open_before=$(journal_open)
    _done_before=$(journal_completed)
    crash_declare
    record_argv sync

    # No `timeout` wrapper: killing the wrapper would leave Shall running and this section
    # would then be measuring an orphan. The spin budget below is the bound instead.
    "$SHALL" -y sync >"/tmp/crash-win-$_tag.out" 2>&1 &
    _pid=$!
    _spins=0
    while [ "$_spins" -lt 3000 ]; do
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
    kill -9 "$_pid" 2>/dev/null
    wait "$_pid" 2>/dev/null

    # The DELTA, not the total: what this iteration is answerable for is what it added.
    _opened=$((_open_at_kill - _open_before))
    _closed=$((_done_at_kill - _done_before))
    if [ "$_when" = completed ]; then
        # This iteration is answerable for a CLOSED operation, not an open one — a kill that
        # leaves nothing open is exactly what it aims at, so measuring it by `_opened` would
        # skip the run every time it worked.
        if [ "$_closed" -lt 1 ]; then
            soft "crash/$_tag: the kill closed no operation in the write-ahead log ($_done_before before, $_done_at_kill after), so this iteration measured nothing"
            crash_wipe
            return 0
        fi
        PASS=$((PASS + 1))
        echo "  PASS  crash/$_tag: SIGKILL landed with $_closed operation(s) closed in the write-ahead log and $_opened still open, with $(crash_installed) of $CRASH_N canaries on disk"
    elif [ "$_opened" -lt 1 ]; then
        soft "crash/$_tag: the kill opened no new entry in the write-ahead log ($_open_before before, $_open_at_kill after), so this iteration measured no recovery"
        crash_wipe
        return 0
    else
        PASS=$((PASS + 1))
        echo "  PASS  crash/$_tag: SIGKILL left $_opened newly-opened operation(s) in the write-ahead log ($_open_at_kill open in all), with $(crash_installed) of $CRASH_N canaries on disk"
    fi

    _hout=$(lx heal 2>&1); _hrc=$?

    if printf '%s' "$_hout" | grep -q "could not be recovered"; then
        if [ "$_hrc" -ne 0 ]; then
            PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: heal named what it could not recover, and said so in the exit code"
        else
            hard "crash/$_tag: heal reported an unrecovered operation and exited 0 (W36)"
        fi
    elif [ "$_hrc" -eq 0 ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: heal recovered the interrupted operation(s)"
    else
        hard "crash/$_tag: heal exited $_hrc without naming anything it could not recover"
    fi

    if printf '%s' "$_hout" | grep -q 'CommandFailed {\|absent_name:\|retry: Permanent\|retry: Transient'; then
        hard "crash/$_tag: heal printed the journal's own struct at the user — $(printf '%s' "$_hout" | grep -o 'CommandFailed {\|absent_name:\|retry: [A-Za-z]*' | head -1)"
    else
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: heal's report is in the user's words, not the journal's"
    fi

    _still=$(journal_open)
    if [ "$_still" -gt 0 ] && ! printf '%s' "$_hout" | grep -q "could not be recovered"; then
        hard "crash/$_tag: $_still operation(s) are still open after heal (this crash opened $_opened of them), and heal named none of them"
        journal_open_names | head -5
    else
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: nothing is open in the log that heal did not name"
    fi

    answers "crash/$_tag: the model still parses after the crash" lx check

    # **Every canary the crash left on the machine is under management.** `S87`: the ownership
    # registry is written once, at the end of a run, and the log is written per operation — so
    # a kill in between leaves a package installed and owned by nobody, and nothing downstream
    # notices. The sync converges (the package IS installed) and the preview plans nothing
    # (there is nothing to plan); the damage shows up only at the cleanup below, as a removal
    # that reports success and takes nothing away.
    _unowned=""
    for _p in $CRASH_PKGS; do
        on_path "$_p" || continue
        $TO "$SHALL" why "$BACKEND:$_p" 2>&1 | grep -q "not under Shall management" && _unowned="$_unowned $_p"
    done
    if [ -z "$_unowned" ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: every canary the crash left on the machine is under Shall management"
    else
        hard "crash/$_tag: the crash left$_unowned installed and under nobody's management, so the command for removing them will report success and take nothing away"
    fi

    if lx -y sync >"/tmp/crash-conv-win-$_tag.out" 2>&1 && [ "$(crash_installed)" -eq "$CRASH_N" ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: the sync after the crash converged onto all $CRASH_N canaries"
        # Asked of the PLAN, not of a phrase: a converged machine that also reports one
        # protected package it left alone prints a different sentence and has done nothing
        # wrong. What "nothing left to do" means is zero planned changes.
        lx --dry-run sync >"/tmp/crash-plan-win-$_tag.out" 2>&1
        if ! grep -q "Planned changes" "/tmp/crash-plan-win-$_tag.out" \
           || grep -qi "already up to date\|nothing to do" "/tmp/crash-plan-win-$_tag.out" \
           || grep -q "install 0 *remove 0" "/tmp/crash-plan-win-$_tag.out"; then
            PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: and the preview after that plans no change at all"
        else
            hard "crash/$_tag: a converged machine still has a plan — $(grep -i 'install\|remove' "/tmp/crash-plan-win-$_tag.out" | head -2 | tr '\n' ' ')"
        fi
    else
        hard "crash/$_tag: the sync after the crash did not converge — still missing:$(crash_missing)"
        excerpt "/tmp/crash-conv-win-$_tag.out" 6
    fi

    crash_wipe
    _left=$(crash_installed)
    if [ "$_left" -eq 0 ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/$_tag: the canaries are off the machine again"
    else
        hard "crash/$_tag: the cleanup uninstall left $_left of$CRASH_PKGS still on PATH"
        sed 's/^/        | /' /tmp/crash-wipe-win.out
        # The twin of the container's diagnostic: `already up to date` over an installed package
        # is either "Shall thinks it is not installed" or "Shall thinks it is not managed", and
        # only `why` separates them. Added here in the same change, because this branch reported
        # the same sentence on the macOS leg with the same missing fact.
        for _p in $CRASH_PKGS; do
            on_path "$_p" || continue
            echo "        ? why $BACKEND:$_p"
            $TO "$SHALL" why "$BACKEND:$_p" 2>&1 | sed 's/^/        ? /' | head -6
        done
    fi
}

# Before anything is killed: what does the log look like after an ordinary run? Fourteen
# sections of installs and removals have run above this line and every one of them either
# finished or failed, and both outcomes close their entry.
_baseline_open=$(journal_open)
_baseline_total=$(journal_status_tally "InProgress Abandoned Failed Completed")
if [ "$_baseline_total" -lt 1 ]; then
    # An audit of an empty set passes without examining anything — the collapse
    # `too_few_to_audit` exists for, and the one check of this kind that survived a shall
    # which fails everything on the container twin.
    hard "journal: the write-ahead log has no entries at all after fourteen sections of installs and removals — nothing recorded an operation, so there is nothing to audit"
elif [ "$_baseline_open" -eq 0 ]; then
    PASS=$((PASS + 1)); echo "  PASS  journal: an ordinary run left nothing open in the write-ahead log ($_baseline_total recorded, $(journal_incomplete) failed-and-retryable)"
else
    hard "journal: $_baseline_open operation(s) are still open in the write-ahead log and nothing crashed"
    journal_open_names | head -5
fi

# An entry `heal` cannot act on AT ALL — the branch a crash cannot produce on its own. Built
# from a REAL journal line with its backend renamed, never hand-written: an `Install` entry
# that omits `options` lands in the corrupt-log branch instead.
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
        hard "heal: an operation was left unresolved and heal exited 0 (W36's family)"
    fi
    # Taken back out AFTER the assertions, never before one.
    grep -v shallnosuchmgr "$JOURNAL" > "$JOURNAL.tmp" 2>/dev/null
    mv "$JOURNAL.tmp" "$JOURNAL"
fi

if [ "$CRASH_N" -lt 2 ]; then
    soft "crash/heal: this host already has gron, grex and tokei, so a sync over them is not a multi-step transaction — named rather than run vacuously"
else
    # The control. If this cannot converge the fixture is wrong, and every iteration below
    # would be measuring the fixture instead of the write-ahead log.
    crash_declare
    if lx -y sync >/tmp/crash-control-win.out 2>&1 && [ "$(crash_installed)" -eq "$CRASH_N" ]; then
        PASS=$((PASS + 1)); echo "  PASS  crash/heal: the control sync installs all $CRASH_N canaries ($CRASH_PKGS)"
        CRASH_FIXTURE_OK=1
        crash_wipe
        crash_run open open
        crash_run midway 1
        # And the one neither of those can reach: the log has CLOSED an operation, and the run
        # has not yet written down what it owns (`S87`).
        crash_run completed completed
        soft "crash/groupkill: Windows has no \`setsid\`, so Shall cannot be put in a process group of its own and the package manager cannot be killed with it — the container twin runs that iteration"
    else
        soft "crash/heal: the control sync did not install$CRASH_PKGS on this host, so the crash loop has no fixture — $(tail -c 300 /tmp/crash-control-win.out | tr '\n' ' ')"
        crash_wipe
    fi
fi

# ==========================================================================
# 14e. TWO RUNS AT ONCE, AND KILLING THE ONE THAT HOLDS THE LOCK (GRADER §6)
# ==========================================================================
# `DataLock` is an OS lock on an open handle, and on Windows that is a different kernel
# primitive from the one the container twin exercises. Both were asserted only by unit tests
# inside ONE process, which is the one place a file lock cannot fail.
#
# The holder is a real Shall rather than a `flock`. `flock` exists in this shell, but it locks
# through the POSIX emulation layer and Shall locks through `fs2` — whether those two contend
# is an assumption, and an unverified assumption in the holder makes every assertion below a
# statement about MSYS. A real Shall holder needs no such belief.
echo "[14e] Two runs at once, and killing the lock holder"

LOCKOWNER="$SHALL_DATA_DIR/shall.lock.owner"
since() { echo $(( $(date +%s) - $1 )); }

if [ "$CRASH_N" -lt 1 ]; then
    soft "two-writers: no free canary on this host, so a holding sync would have no work and could not be caught holding"
else
    rm -f "$LOCKOWNER"
    crash_declare
    "$SHALL" -y sync >/tmp/lock-holder-win.out 2>&1 &
    _holder=$!
    _spins=0
    while [ "$_spins" -lt 600 ] && [ ! -s "$LOCKOWNER" ]; do
        kill -0 "$_holder" 2>/dev/null || break
        sleep "$CRASH_POLL"; _spins=$((_spins + 1))
    done

    if [ ! -s "$LOCKOWNER" ] || ! kill -0 "$_holder" 2>/dev/null; then
        soft "two-writers: the holder finished before it could be caught holding, so there was nothing to contend with"
        kill -9 "$_holder" 2>/dev/null; wait "$_holder" 2>/dev/null
    else
        _stamp="$(cat "$LOCKOWNER")"
        if printf '%s' "$_stamp" | grep -q "pid"; then
            PASS=$((PASS + 1)); echo "  PASS  lock: the holder published its own command and pid — $_stamp"
        else
            hard "lock: the holder's stamp names no pid — it says '$_stamp'"
        fi

        # A second writer, while the first is demonstrably holding. Waiting with no reason given
        # is indistinguishable from hanging, so the message is the assertion.
        _t0=$(date +%s)
        $TO "$SHALL" -y sync >/tmp/two-writers-win.out 2>&1
        _rc=$?
        _waited=$(since "$_t0")
        if grep -q "waiting for the data directory" /tmp/two-writers-win.out; then
            PASS=$((PASS + 1)); echo "  PASS  two-writers: the second run announced the wait instead of going quiet (${_waited}s)"
        elif kill -0 "$_holder" 2>/dev/null; then
            hard "two-writers: a second Shall ran alongside a live one and never named the holder"
            excerpt /tmp/two-writers-win.out 6
        else
            soft "two-writers: the first run finished before the second reached the lock, so there was no overlap to measure"
        fi
        kill -9 "$_holder" 2>/dev/null; wait "$_holder" 2>/dev/null

        # SIGKILL the holder. `Drop` never runs, so the stamp outlives the process that wrote
        # it — and if anything ever decided to wait by reading that FILE rather than by trying
        # the lock, this is where it costs two minutes.
        rm -f "$LOCKOWNER"
        "$SHALL" -y sync >/tmp/lock-holder2-win.out 2>&1 &
        _corpse=$!
        _spins=0
        while [ "$_spins" -lt 600 ] && [ ! -s "$LOCKOWNER" ]; do
            kill -0 "$_corpse" 2>/dev/null || break
            sleep "$CRASH_POLL"; _spins=$((_spins + 1))
        done
        if [ ! -s "$LOCKOWNER" ]; then
            soft "lock: the second holder never published a stamp, so the killed-holder check had no corpse to leave behind"
            kill -9 "$_corpse" 2>/dev/null; wait "$_corpse" 2>/dev/null
        else
            kill -9 "$_corpse" 2>/dev/null; wait "$_corpse" 2>/dev/null
            if [ -s "$LOCKOWNER" ]; then
                PASS=$((PASS + 1)); echo "  PASS  lock: the stamp outlived the process that wrote it, which is the state under test"
            else
                soft "lock: the stamp was already gone after the kill, so the corpse case is weaker than intended"
            fi
            # **The subject here is Shall's own lock, so the package manager's is cleared
            # first.** Killing a holder mid-sync also orphans the manager it had started, which
            # keeps its own lock and leaves it behind — so the sync below would fail on *that*
            # lock and this check would report it as a Shall lock that was not released. The
            # container twin learned this and this script did not, which is the same one-of-two
            # split as the assertion below it. `heal` is the command whose job that repair is
            # (II.50), and it is untimed: what is measured is the sync.
            $TO "$SHALL" -y heal >/tmp/lock-corpse-heal-win.out 2>&1 || true
            _t0=$(date +%s)
            $TO "$SHALL" -y sync >/tmp/lock-corpse-win.out 2>&1
            _rc=$?
            _took=$(since "$_t0")
            # **The exit code is asked LAST, and under its own name.** It used to be asked
            # first, so any failure of the sync was reported as a lock that had not been
            # released. The macOS nightly failed exactly that way for six nights running:
            #
            #     FAIL  lock: a run after a killed holder failed (rc=1) instead of taking
            #           the free lock
            #           | Error: `brew` failed (exit 1): No available formula with the
            #             name "tokei@14.0.0".
            #
            # The lock was taken correctly and instantly. What failed was a version pin naming
            # a Homebrew formula that does not exist — and the only red line in a 296-assertion
            # run pointed at the wrong mechanism. A check whose name and whose cause are
            # unrelated is the defect `GRADER.md` exists to catch. Two questions, two sentences.
            #
            # The lock question is answered by the clock and by the message, not by rc: Shall
            # announces its own wait, so a run that was refused by a dead holder's stamp says so.
            if grep -q "waiting for the data directory\|is locked by" /tmp/lock-corpse-win.out 2>/dev/null; then
                hard "lock: the next run waited on the data directory after the holder was killed — the stale stamp file was believed over the lock (${_took}s)"
                excerpt /tmp/lock-corpse-win.out 6
            elif [ "$_took" -ge 30 ]; then
                hard "lock: the next run took ${_took}s after a holder that was already dead, without saying what it was waiting for"
                excerpt /tmp/lock-corpse-win.out 6
            # **Positive evidence that the lock was taken, not merely that nothing complained.**
            # `DataLock`'s `Drop` deletes the stamp, so the corpse's file is gone once something
            # has taken and released the lock — and is still there if nothing did. Without this
            # the whole branch passed against a stub that does nothing and exits 0, which is a
            # check that cannot fail and therefore proves nothing when it passes.
            elif [ -s "$LOCKOWNER" ]; then
                hard "lock: the killed holder's stamp is still on disk after the next run — nothing took and released the lock, so this check had nothing to measure"
                excerpt /tmp/lock-corpse-win.out 6
            else
                PASS=$((PASS + 1)); echo "  PASS  lock: a killed holder's lock died with it — the next run took ${_took}s, took the lock and released it"
            fi
            # The other question, asked separately because it is a different question — and asked
            # at all only where the fixture it syncs is installable here.
            if [ "$CRASH_FIXTURE_OK" -eq 0 ]; then
                soft "crash-recovery sync: the fixture ($CRASH_PKGS) does not install on this host, so the recovery sync's exit code says nothing about recovery"
            elif [ "$_rc" -ne 0 ] && [ "$_rc" -ne 2 ]; then
                hard "crash-recovery sync: the run after a killed holder failed (rc=$_rc) — the lock was free, so this is a sync defect and not a lock defect"
                echo "        heal said: $(tail -c 300 /tmp/lock-corpse-heal-win.out | tr '\n' ' ')"
                excerpt /tmp/lock-corpse-win.out 6
            fi
        fi
    fi
    crash_wipe
fi

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
    NO_PATH="$NO_PATH $be"
done
NO_PATH_N=$(echo $NO_PATH | wc -w)
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
        echo "        Each failed a real install for a reason Shall classed as passing, and did"
        echo "        not clear on a retry — a rate-limit window, a held lock. The floor is NOT"
        echo "        lowered for these: the next run on a clear window measures them again."
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

# --- Stalls -----------------------------------------------------------------
#
# Soft, not hard: a slow host is not a defect and this must not turn a red run into a red run
# for the wrong reason. But it is never silent — the four stalls that started this were
# invisible in the result line, and a sweep that says `OK` while it sat idle for an hour is
# reporting on a run that did not happen the way it reads.
kill "$STALL_PID" 2>/dev/null; trap - EXIT INT TERM
if [ -s "$STALL_REPORT" ]; then
    STALL_N=$(grep -c '^STALL SNAPSHOT' "$STALL_REPORT" 2>/dev/null || echo 0)
    soft "stall capture: $STALL_N snapshot(s) taken — a call ran past ${STALL_AFTER}s"
    echo "        $STALL_REPORT"
    echo "        Read the child list: a tree at cpuMsInWindow=0 with a live child is Shall"
    echo "        waiting on that child; the child's command line names what was asked."
    # Only the `shall` trees. The report also carries a flat watched-name table whose rows start
    # at column 0, and matching those printed a dozen of the harness's own `bash` processes
    # instead of the wedged process — the one thing this excerpt exists to show.
    grep -E '^(SHALL PID| +pid=.*cpuMsInWindow)' "$STALL_REPORT" | head -12 | sed 's/^/        /'
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
