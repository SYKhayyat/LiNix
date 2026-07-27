#!/usr/bin/env bash
# ============================================================================
# The integration harnesses' own predicates, tested.
#
# The harnesses run nightly and in containers, so a wrong verdict inside one of
# them is found a day late and by whoever reads the log — which is how `go` was
# reported as leaving a binary behind on a removal that had worked, and how a
# macOS sweep in which nothing executed at all reported passes.
#
# Both faults were in two-line predicates. This runs those predicates against the
# cases that got them wrong, in a second, on every push.
#
# The function bodies are LIFTED from the harnesses rather than copied, so this
# cannot drift from what CI actually runs.
#
#   scripts/harness-logic-test.sh [harness.sh ...]
# ============================================================================
set -u

SOURCES="$*"
if [ -z "$SOURCES" ]; then
    _here="$(cd "$(dirname "$0")/.." && pwd)"
    SOURCES="$_here/docker/integration/run-in-container.sh $_here/scripts/integration-windows.sh"
fi

TOTAL=0; BAD=0

# Pull one function's definition out of a harness, so this file tests what CI runs rather
# than a copy of it. Handles both the one-line `f() { …; }` form and the braces-on-their-
# own-line form, and refuses anything implausibly long: a range that runs off the end of a
# function swallows the rest of the script, and `eval` would then RUN it — which is how a
# first attempt at this printed the harness's banner and died on an unset variable.
lift() {
    _name="$1"; _file="$2"
    awk -v fn="$_name" '
        $0 ~ "^" fn "\\(\\) \\{" { inside = 1 }
        inside { print; n++ }
        inside && /\}[[:space:]]*$/ && (n > 1 || /\{.*\}/) { exit }
        n > 40 { exit 1 }
    ' "$_file"
}

run_against() {
    SRC="$1"
    echo "== $SRC"
    [ -f "$SRC" ] || { echo "  FATAL: no such harness"; BAD=$((BAD + 1)); return 1; }

    PASS=0; FAILC=0; SOFTC=0; FAILED_NAMES=""
    soft() { SOFTC=$((SOFTC + 1)); }
    removal_leaves_binary() { case "$1" in bun) echo "bun keeps its launcher" ;; *) echo "" ;; esac; }

    FAKE=""; FAKE_NAME="hello"
    path_of() { case "$1" in "$FAKE_NAME") echo "$FAKE" ;; *) echo "" ;; esac; }

    for _fn in never_ran assert_binary_gone on_path; do
        _body="$(lift "$_fn" "$SRC")"
        if [ -z "$_body" ]; then
            echo "  FATAL: could not lift $_fn() from this harness"
            TOTAL=$((TOTAL + 1)); BAD=$((BAD + 1)); return 1
        fi
        eval "$_body"
    done
    command -v assert_binary_gone >/dev/null || { echo "  FATAL: could not lift assert_binary_gone"; BAD=$((BAD + 1)); return 1; }
    command -v never_ran >/dev/null || { echo "  FATAL: could not lift never_ran"; BAD=$((BAD + 1)); return 1; }

    check() { # check <label> <pass|fail|soft>
        _label="$1"; _want="$2"
        TOTAL=$((TOTAL + 1))
        _p0=$PASS; _f0=$FAILC; _s0=$SOFTC
        assert_binary_gone "$_be" "$_bin" "$_was" >/dev/null 2>&1
        if   [ "$PASS"  -gt "$_p0" ]; then _got=pass
        elif [ "$FAILC" -gt "$_f0" ]; then _got=fail
        elif [ "$SOFTC" -gt "$_s0" ]; then _got=soft
        else _got=none; fi
        if [ "$_got" = "$_want" ]; then
            echo "  ok    $_label -> $_got"
        else
            echo "  BAD   $_label -> $_got (wanted $_want)"; BAD=$((BAD + 1))
        fi
    }

    # The regression: cabal's canary binary is `hello` and cabal has no uninstall verb,
    # so its copy is there before go installs its own and still there after go removes
    # it. Asking PATH scored another manager's deliberate leftover as go's failure.
    _be=go; _bin=hello; _was="/root/.cabal/bin/hello"; FAKE="/root/.cabal/bin/hello"
    check "another manager's copy predates and outlives the install" pass

    # The defect the check exists to catch, which must still be caught.
    _be=go; _bin=hello; _was=""; FAKE="/root/go/bin/hello"
    check "removal left its own binary behind" fail

    _be=go; _bin=hello; _was=""; FAKE=""
    check "clean removal" pass

    # A documented leftover softens only when it actually happens.
    _be=bun; _bin=hello; _was=""; FAKE="/root/.bun/bin/hello"
    check "documented leftover softens" soft
    _be=bun; _bin=hello; _was=""; FAKE=""
    check "documented quirk that did not happen still passes strictly" pass

    # Resolution that MOVED to this backend's own directory is its leftover, not
    # the pre-existing copy.
    _be=go; _bin=hello; _was="/root/.cabal/bin/hello"; FAKE="/root/go/bin/hello"
    check "resolution moved to this backend's own dir" fail

    # A command that could not run is not a refusal.
    for rc in 127 126 124; do
        TOTAL=$((TOTAL + 1))
        if never_ran "$rc"; then echo "  ok    rc=$rc counts as never-ran"
        else echo "  BAD   rc=$rc must count as never-ran"; BAD=$((BAD + 1)); fi
    done
    for rc in 0 1 2 3; do
        TOTAL=$((TOTAL + 1))
        if never_ran "$rc"; then echo "  BAD   rc=$rc must be a real verdict"; BAD=$((BAD + 1))
        else echo "  ok    rc=$rc is a real verdict"; fi
    done

    # A predicate answers yes or no. `command -v` says "not found" with 1 under bash and
    # 127 under dash and busybox ash — and 127 is what `nok` must read as "never ran", so
    # an unnormalised on_path made three container jobs fail on a name that was correctly
    # absent. This case only bites where /bin/sh is dash: ubuntu's runner, which is where
    # this file runs.
    TOTAL=$((TOTAL + 1))
    on_path linix-no-such-binary-zzz; _rc=$?
    if [ "$_rc" = 1 ]; then
        echo "  ok    on_path says no with 1, not with the shell's not-found code"
    else
        echo "  BAD   on_path answered 'no' with rc=$_rc — nok cannot tell that from 'never ran'"
        BAD=$((BAD + 1))
    fi
    TOTAL=$((TOTAL + 1))
    if on_path sh; then echo "  ok    on_path still finds a name that is there"
    else echo "  BAD   on_path could not find sh"; BAD=$((BAD + 1)); fi
}

for src in $SOURCES; do run_against "$src"; done

# ---------------------------------------------------------------------------
# Every subcommand a harness invokes must exist in the binary.
#
# This is not pedantry: `linix doctor`, `status`, `absent`, `unmanaged`, `conflicts` and
# `audit` were folded into `check <section>`, the host harness was never updated, and
# clap answered "unrecognized subcommand" with exit 2. One of those calls builds
# READY_LIST — so the entire real-lifecycle section and the entire plan-smoke section
# iterated over an empty list and reported nothing wrong. A stale name does not announce
# itself as missing coverage; it announces itself as no coverage at all.
#
# Runs only when a binary is given, so the predicate tests above stay runnable anywhere.
BIN="${LINIX_BIN:-}"
if [ -n "$BIN" ] && "$BIN" --version >/dev/null 2>&1; then
    echo "== subcommands invoked vs subcommands that exist ($BIN)"
    _real="$("$BIN" --help 2>&1 | sed -n '/^Commands:/,/^Options:/p' \
        | awk '{print $1}' | grep -E '^[a-z]' | sort -u)"
    for src in $SOURCES; do
        _used="$(grep -oE '(\b(lx|lx_slow|smoke_lx|silent_lx|restore_lx))( +-[-a-zA-Z]+)* +[a-z][a-z-]*' "$src" \
            | awk '{print $NF}' | sort -u)"
        _unknown=""
        for _v in $_used; do
            printf '%s\n' "$_real" | grep -qx "$_v" || _unknown="$_unknown $_v"
        done
        TOTAL=$((TOTAL + 1))
        if [ -z "$_unknown" ]; then
            echo "  ok    $(basename "$src") invokes only subcommands that exist"
        else
            echo "  BAD   $(basename "$src") invokes subcommands the binary does not have:$_unknown"
            BAD=$((BAD + 1))
        fi
    done
else
    echo "== subcommands invoked vs subcommands that exist: SKIPPED (set LINIX_BIN to a built binary)"
fi

echo "--------------------------------------------------------------"
echo " harness predicates: $((TOTAL - BAD))/$TOTAL ok"
[ "$BAD" = 0 ] || { echo " FAILED"; exit 1; }
