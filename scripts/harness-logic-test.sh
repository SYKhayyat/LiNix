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
        n > 60 { exit 1 }
    ' "$_file"
}
# The cap is a runaway guard — a malformed function must not slurp the rest of the file — and
# not a size limit on harness functions. It was 40, and `classify_install` grew to 43 when it
# stopped re-deriving transience by retrying and started reading `shall-failure-class:`. A
# truncated lift is a `syntax error: unexpected end of file` and then `CLASS: unbound variable`,
# which reads as the harness being broken rather than this file's awk being short: worth
# knowing, because the first instinct was to shrink the function to fit the test measuring it.

run_against() {
    SRC="$1"
    echo "== $SRC"
    [ -f "$SRC" ] || { echo "  FATAL: no such harness"; BAD=$((BAD + 1)); return 1; }

    PASS=0; FAILC=0; SOFTC=0; FAILED_NAMES=""
    soft() { SOFTC=$((SOFTC + 1)); }
    removal_leaves_binary() { case "$1" in bun) echo "bun keeps its launcher" ;; *) echo "" ;; esac; }

    FAKE=""; FAKE_NAME="hello"
    path_of() { case "$1" in "$FAKE_NAME") echo "$FAKE" ;; *) echo "" ;; esac; }

    for _fn in never_ran assert_binary_gone on_path named_bin_dir off_path_copy binary_present assert_binary_reachable; do
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
    on_path shall-no-such-binary-zzz; _rc=$?
    if [ "$_rc" = 1 ]; then
        echo "  ok    on_path says no with 1, not with the shell's not-found code"
    else
        echo "  BAD   on_path answered 'no' with rc=$_rc — nok cannot tell that from 'never ran'"
        BAD=$((BAD + 1))
    fi
    TOTAL=$((TOTAL + 1))
    if on_path sh; then echo "  ok    on_path still finds a name that is there"
    else echo "  BAD   on_path could not find sh"; BAD=$((BAD + 1)); fi

    # ---- E6c/W4: "the binary is reachable" is a claim about the PRODUCT ------
    #
    # `on_path` alone asks the HOST. On a clean runner ~/go/bin, ~/.local/bin and yarn's
    # global directory are on nobody's PATH, so the old check failed three backends that
    # had done nothing wrong — and it would have passed all three had the machine happened
    # to be wired, which is the half that matters: an install that says NOTHING about an
    # unreachable binary is the defect (github and yarn, measured 2026-07-29).
    _rd="${TMPDIR:-/tmp}/shall-reach-$$"
    rm -rf "$_rd"; mkdir -p "$_rd"
    _rbin=shall-reach-zzz
    _rlog="$_rd/install.log"
    printf '%s\n' "  WARN shall::verbs::sync: \`go\` installs its executables into $_rd, which is not on your PATH — so what it just installed will answer \"command not found\"." > "$_rlog"

    outcome() { # label want cmd...
        _olabel="$1"; _owant="$2"; shift 2
        TOTAL=$((TOTAL + 1))
        _op0=$PASS; _of0=$FAILC; _os0=$SOFTC
        "$@" >/dev/null 2>&1
        if   [ "$PASS"  -gt "$_op0" ]; then _ogot=pass
        elif [ "$FAILC" -gt "$_of0" ]; then _ogot=fail
        elif [ "$SOFTC" -gt "$_os0" ]; then _ogot=soft
        else _ogot=none; fi
        if [ "$_ogot" = "$_owant" ]; then
            echo "  ok    $_olabel -> $_ogot"
        else
            echo "  BAD   $_olabel -> $_ogot (wanted $_owant)"; BAD=$((BAD + 1))
        fi
    }

    # The defect this check exists for: installed, unreachable, and nothing said so.
    outcome "unreachable and unexplained" fail assert_binary_reachable go "$_rbin" "$_rd/no-such.log"

    # Shall warned and the file is where it said: the product kept its promise, and the
    # host's PATH is not the product's to fix.
    : > "$_rd/$_rbin"
    outcome "unreachable, explained, and there" pass assert_binary_reachable go "$_rbin" "$_rlog"

    # Warned about a directory the binary is NOT in — the install claimed something untrue.
    rm -f "$_rd/$_rbin"
    outcome "explained and not there" fail assert_binary_reachable go "$_rbin" "$_rlog"

    # A Windows install writes cowsay.cmd, not cowsay. Looking only for the bare name reports
    # an installed program as missing.
    : > "$_rd/$_rbin.cmd"
    outcome "the platform's extension counts" pass assert_binary_reachable go "$_rbin" "$_rlog"
    rm -f "$_rd/$_rbin.cmd"

    # G-3, the reachability half of the collision `assert_binary_gone` has handled since
    # 2026-07-29. Measured on the tools image (CI 30566924407): `PASS go: hello is on PATH`
    # scored against /root/.cabal/bin/hello, which cabal installed four lifecycles earlier and
    # cannot uninstall. The check would have passed if the go install had done nothing at all.
    FAKE="/root/.cabal/bin/hello"
    outcome "the name resolves to the manager that already owned it" fail \
        assert_binary_reachable go hello "$_rd/no-such.log" "/root/.cabal/bin/hello"

    # Same collision, but this backend did install its own copy and said where it went. The
    # other manager holding the PATH entry is not this install's failure.
    : > "$_rd/hello"
    outcome "a collision the backend's own copy answers for" pass \
        assert_binary_reachable go hello "$_rlog" "/root/.cabal/bin/hello"
    rm -f "$_rd/hello"

    # The control, and it is the whole reason the comparison is against the PRIOR value rather
    # than against a list of known collisions: a name that resolves somewhere it did not before
    # is this install's doing, and must still pass with nothing else to go on.
    FAKE="/root/go/bin/hello"
    outcome "a resolution that changed is this install's" pass \
        assert_binary_reachable go hello "$_rd/no-such.log" "/root/.cabal/bin/hello"
    # The lifted bodies assign to globals — there are no locals in a POSIX shell — so the three
    # cases above left `$_rbin` reading `hello`. Restored, because the checks below share it.
    FAKE=""; _rbin=shall-reach-zzz

    # The warning belongs to the backend that printed it. One sync can warn about two
    # managers, and handing yarn's directory to go would answer for the wrong install.
    TOTAL=$((TOTAL + 1))
    if [ -z "$(named_bin_dir yarn "$_rlog")" ]; then
        echo "  ok    a directory is read only for the backend that named it"
    else
        echo "  BAD   named_bin_dir handed go's directory to yarn"; BAD=$((BAD + 1))
    fi

    # And the removal half. A binary that was never on PATH is "gone" by PATH before the
    # removal runs, so the old three-argument check passed while the file was still there.
    : > "$_rd/$_rbin"
    _be=go; _bin="$_rbin"; _was=""; FAKE=""
    TOTAL=$((TOTAL + 1))
    _p0=$PASS; _f0=$FAILC
    assert_binary_gone go "$_rbin" "" "$_rlog" >/dev/null 2>&1
    if [ "$FAILC" -gt "$_f0" ]; then
        echo "  ok    a leftover off PATH is still a leftover -> fail"
    else
        echo "  BAD   a leftover in the directory the install named passed as removed"; BAD=$((BAD + 1))
    fi
    rm -f "$_rd/$_rbin"
    TOTAL=$((TOTAL + 1))
    _f0=$FAILC
    assert_binary_gone go "$_rbin" "" "$_rlog" >/dev/null 2>&1
    if [ "$FAILC" -eq "$_f0" ]; then
        echo "  ok    a removal that really removed it still passes"
    else
        echo "  BAD   a clean removal was scored as a leftover"; BAD=$((BAD + 1))
    fi
    rm -rf "$_rd"
}

for src in $SOURCES; do run_against "$src"; done

# ---------------------------------------------------------------------------
# Every subcommand a harness invokes must exist in the binary.
#
# This is not pedantry: the `doctor`, `status`, `absent`, `unmanaged`, `conflicts` and
# `audit` were folded into `check <section>`, the host harness was never updated, and
# clap answered "unrecognized subcommand" with exit 2. One of those calls builds
# READY_LIST — so the entire real-lifecycle section and the entire plan-smoke section
# iterated over an empty list and reported nothing wrong. A stale name does not announce
# itself as missing coverage; it announces itself as no coverage at all.
#
# Runs only when a binary is given, so the predicate tests above stay runnable anywhere.
BIN="${SHALL_BIN:-}"
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
    # The other half, and the half that was missing. An exemption says "this subcommand
    # exists and cannot be driven here, for this reason" — so a name that does not exist
    # cannot be exempt, it can only be stale. `undo` sat in both lists after the command was
    # renamed to snapshot/rollback, and nothing looked, because the audit only ever checked
    # the names that were *used*. An unvalidated exemption list is where coverage goes to
    # disappear quietly: the printed "5 exempt" was wrong and read as reassurance.
    echo "== subcommands exempted vs subcommands that exist ($BIN)"
    for src in $SOURCES; do
        _exempt="$(sed -n 's/^[[:space:]]*EXEMPT_CMDS="\([^"]*\)".*/\1/p' "$src" | tr ' ' '\n' \
            | grep -E '^[a-z]' | sort -u)"
        [ -n "$_exempt" ] || continue
        _stale=""
        for _v in $_exempt; do
            printf '%s\n' "$_real" | grep -qx "$_v" || _stale="$_stale $_v"
        done
        TOTAL=$((TOTAL + 1))
        if [ -z "$_stale" ]; then
            echo "  ok    $(basename "$src") exempts only subcommands that exist"
        else
            echo "  BAD   $(basename "$src") exempts subcommands the binary does not have:$_stale"
            BAD=$((BAD + 1))
        fi
    done
else
    echo "== subcommands invoked vs subcommands that exist: SKIPPED (set SHALL_BIN to a built binary)"
fi

# E5's classifier, in both harnesses. The catch-all it replaced softened ANY install failure
# to "network/ecosystem variance" and skipped that backend's whole remaining lifecycle; in one
# observed run it fired four times and not once was it the network. The verdicts are what
# matter, so they are tested rather than the wording: a refusal is a pass, a timeout is a soft,
# and — the half that actually lost coverage — a transient must let the lifecycle CONTINUE.
echo "== an install failure is classified, not assumed to be the network"
for _src in $SOURCES; do
    TOTAL=$((TOTAL + 1))
    _body="$(lift classify_install "$_src")"
    if [ -z "$_body" ]; then
        echo "  BAD   $(basename "$_src") has no classify_install(): its install failures are unclassified"
        BAD=$((BAD + 1))
        continue
    fi
    (
        PASS=0; FAILC=0; SOFTC=0
        # The environment the lifted body runs in, not this script's own variables — both are
        # read by `run-in-container.sh`'s `classify_install` and its callers, and shellcheck
        # cannot see through the `eval` below to know that. Dropping them would make the lifted
        # code run against unset names, which is the failure this whole file exists to catch.
        #
        # One directive per line, and the assignments split off the line above for that reason:
        # a directive attaches to the next *command*, so on `A=1; B=2` it covers `A` and leaves
        # `B` reported — which is how the first attempt at this suppression did nothing.
        # shellcheck disable=SC2034
        FAILED_NAMES=""
        # shellcheck disable=SC2034
        TO_LONG="timeout 900"
        soft() { SOFTC=$((SOFTC + 1)); }
        hard() { FAILC=$((FAILC + 1)); }
        refused() { PASS=$((PASS + 1)); }
        eval "$_body"
        _log="$(mktemp)"; : > "$_log"

        _bad=0
        classify_install be spec 3 "$_log" >/dev/null 2>&1
        [ "$CLASS" = refused ] && [ "$PASS" -eq 1 ] || { echo "  BAD   exit 3 is not scored as a refusal (got '$CLASS')"; _bad=1; }
        classify_install be spec 124 "$_log" >/dev/null 2>&1
        [ "$CLASS" = timeout ] && [ "$SOFTC" -eq 1 ] || { echo "  BAD   exit 124 is not scored as a timeout (got '$CLASS')"; _bad=1; }
        # Neither may be counted as a hard failure: that is the "refusal reported as a defect"
        # half of E5, and it is as wrong as the variance catch-all was.
        [ "$FAILC" -eq 0 ] || { echo "  BAD   a refusal or a timeout was recorded as a hard failure"; _bad=1; }

        # R-3, both directions. The classifier reads `shall-failure-class:` instead of retrying
        # the install to guess at it, and the two branches fail in opposite ways: a permanent
        # failure retried is a minute wasted per backend, and a transient one scored a defect is
        # a red CI leg over a rate-limit window that has since moved.
        LEDGER="$(mktemp -d)"; : > "$LEDGER/be-life-unmeasured"
        lx() { echo "the retry must not be reached"; return 1; }
        lx_slow() { lx "$@"; }

        FAILC=0; SOFTC=0; PASS=0
        printf 'shall-failure-class: permanent\n' > "$_log"
        # To a file, not a `$( )`: command substitution runs in a subshell, so `CLASS` set
        # inside it never reaches this scope and the assertion below reads the PREVIOUS call's
        # answer. It did, and reported `timeout`.
        _out="$(mktemp)"
        classify_install be spec 1 "$_log" > "$_out" 2>&1
        [ "$CLASS" = defect ] || { echo "  BAD   a permanent failure is not a defect (got '$CLASS')"; _bad=1; }
        grep -q retrying "$_out" && { echo "  BAD   a permanent failure was retried anyway"; _bad=1; }
        rm -f "$_out"

        FAILC=0; SOFTC=0
        printf 'shall-failure-class: transient\n' > "$_log"
        classify_install be spec 1 "$_log" >/dev/null 2>&1
        [ "$CLASS" = exhausted ] || { echo "  BAD   a transient failure that did not clear is not exhausted (got '$CLASS')"; _bad=1; }
        [ "$FAILC" -eq 0 ] || { echo "  BAD   a transient failure that did not clear was scored a hard failure — this is the red macOS leg"; _bad=1; }
        grep -qx be "$LEDGER/be-life-unmeasured" || { echo "  BAD   an unmeasurable lifecycle was not recorded, so the ratchet cannot excuse it by name"; _bad=1; }

        # And a failure with no class at all is a defect, not a free pass: its absence means the
        # binary under test predates the line, so the run is not measuring the tree it claims to.
        FAILC=0
        : > "$_log"
        classify_install be spec 1 "$_log" >/dev/null 2>&1
        [ "$CLASS" = defect ] && [ "$FAILC" -eq 1 ] || { echo "  BAD   a failure with no class line was not scored a defect (got '$CLASS')"; _bad=1; }

        rm -rf "$LEDGER"
        rm -f "$_log"
        exit "$_bad"
    )
    if [ $? -eq 0 ]; then
        echo "  ok    $(basename "$_src") tells a refusal, a timeout, a defect and an unmeasurable apart"
    else
        BAD=$((BAD + 1))
    fi
done

# The drift register, in both harnesses. `classify_install` above degrades an ecosystem failure
# to `exhausted`, which the real-lifecycle ratchet then counts as coverage merely unmeasured —
# right for a rate-limit window, wrong for Hackage rotating its TUF root past what the image's
# cabal trusts, which no later run clears on its own. An excuse nothing ages is `|| true` with
# better manners, so the excuse now needs a dated line and expires.
#
# Both halves are arithmetic on dates, which is exactly the shape that is wrong on a leap year
# and right on every day somebody tests it by hand.
echo "== an ecosystem excuse is dated, and expires"
for _src in $SOURCES; do
    TOTAL=$((TOTAL + 1))
    _de="$(lift days_since_epoch "$_src")"
    _dv="$(lift drift_verdict "$_src")"
    if [ -z "$_de" ] || [ -z "$_dv" ]; then
        echo "  BAD   $(basename "$_src") has no drift register: an ecosystem failure is excused forever"
        BAD=$((BAD + 1))
        continue
    fi
    (
        # Read by the lifted `drift_verdict` and by nothing shellcheck can see through
        # the `eval` below — the same suppression, and the same reason, as `TO_LONG` above.
        # shellcheck disable=SC2034
        DRIFT_WINDOW_DAYS=14
        eval "$_de"
        eval "$_dv"
        _bad=0

        # Civil-to-days, against dates computed elsewhere. The leap cases are the point: the
        # formula shifts March to the start of the year precisely so 29 February needs no
        # special case, and a wrong shift is invisible on any date in the second half of a year.
        for _pair in 1970-01-01:0 2000-02-29:11016 2024-12-31:20088 2026-02-28:20512                      2026-03-01:20513 2026-08-21:20686; do
            _want="${_pair#*:}"
            _got="$(days_since_epoch "${_pair%%:*}")"
            [ "$_got" = "$_want" ] || { echo "  BAD   ${_pair%%:*} is $_got days, not $_want"; _bad=1; }
        done
        # A date nobody can parse must not become day zero, which is 1970 and therefore an
        # excuse fifty-six years past its expiry.
        for _junk in "" "2026-8-21" "yesterday" "20260821" "2026-13-01"; do
            days_since_epoch "$_junk" >/dev/null 2>&1 &&
                { echo "  BAD   '$_junk' was accepted as a date"; _bad=1; }
        done

        _reg="$(mktemp)"
        printf 'container-linux-tools-ci 25
' > "$_reg"
        printf 'drift container-linux-tools-ci cabal 2026-08-21
' >> "$_reg"
        _today=20686   # 2026-08-21, the day the line was written

        _v="$(drift_verdict container-linux-tools-ci cabal "$_reg" "$_today")"
        [ "${_v%% *}" = ok ] || { echo "  BAD   a register line dated today does not excuse (got '$1')"; _bad=1; }
        _v="$(drift_verdict container-linux-tools-ci cabal "$_reg" $((_today + 14)))"
        [ "${_v%% *}" = ok ] || { echo "  BAD   the last day of the window does not excuse (got '$1')"; _bad=1; }
        _v="$(drift_verdict container-linux-tools-ci cabal "$_reg" $((_today + 15)))"
        [ "${_v%% *}" = expired ] || { echo "  BAD   an excuse older than the window still excuses (got '$1')"; _bad=1; }

        # The three ways to have no excuse, which must never be the same as having one: another
        # backend, another host class, and a register that is not there at all. The host-class
        # case is the one that matters — a drift line for the tools image must not excuse the
        # same backend on ubuntu, where the lifecycle really did stop running.
        _v="$(drift_verdict container-linux-tools-ci stack "$_reg" "$_today")"
        [ "${_v%% *}" = unrecorded ] || { echo "  BAD   an unlisted backend was excused (got '$1')"; _bad=1; }
        _v="$(drift_verdict container-linux-ubuntu-ci cabal "$_reg" "$_today")"
        [ "${_v%% *}" = unrecorded ] || { echo "  BAD   one host class excused another's backend (got '$1')"; _bad=1; }
        _v="$(drift_verdict container-linux-tools-ci cabal /no/such/register "$_today")"
        [ "${_v%% *}" = unrecorded ] || { echo "  BAD   a missing register excused something (got '$1')"; _bad=1; }
        # A line dated tomorrow is a typo, and must buy nothing.
        _v="$(drift_verdict container-linux-tools-ci cabal "$_reg" $((_today - 1)))"
        [ "${_v%% *}" = unrecorded ] || { echo "  BAD   a future-dated line was honoured (got '$1')"; _bad=1; }
        # And a `drift` line whose date is rubbish must report, not excuse.
        printf 'drift container-linux-tools-ci opam soon
' >> "$_reg"
        _v="$(drift_verdict container-linux-tools-ci opam "$_reg" "$_today")"
        [ "${_v%% *}" = unrecorded ] || { echo "  BAD   an unparseable date was honoured (got '$1')"; _bad=1; }

        rm -f "$_reg"
        exit "$_bad"
    )
    if [ $? -eq 0 ]; then
        echo "  ok    $(basename "$_src") dates an ecosystem excuse and expires it"
    else
        BAD=$((BAD + 1))
    fi
done

# The coverage audit's floor. Both harnesses take `ALL_BACKENDS` and `HELP_CMDS` from the
# program under test and then assert set-containment — and a `for` over an empty list runs
# zero times, leaves the "untouched" string empty, and PASSes. Measured under a do-nothing
# stub: the audit printed "0 in --help … 0 registered" and passed both meta-checks. An audit
# that enumerates nothing scoring perfect coverage is the exact shape of every finding in this
# file, applied to the thing that was supposed to find them.
echo "== a collapsed registry cannot pass the coverage audit"
for _src in $SOURCES; do
    TOTAL=$((TOTAL + 1))
    _body="$(lift too_few_to_audit "$_src")"
    if [ -z "$_body" ]; then
        echo "  BAD   $(basename "$_src") has no too_few_to_audit(): its coverage audit has no floor"
        BAD=$((BAD + 1))
        continue
    fi
    eval "$_body"
    # 0 and 1 are collapse; a real registry is 48 on Windows and 56 on Ubuntu, and a real
    # `--help` is ~55 subcommands. Both directions, so a floor of zero cannot pass this.
    if too_few_to_audit 10 0 && too_few_to_audit 10 1 && ! too_few_to_audit 10 48; then
        echo "  ok    $(basename "$_src") refuses to audit a registry that came back empty"
    else
        echo "  BAD   $(basename "$_src") too_few_to_audit() does not tell collapse from a real registry"
        BAD=$((BAD + 1))
    fi
done

# The mutation gate's own collapse case. `grep -c` prints `0` AND exits 1 when it matches
# nothing, so `COUNT=$(grep -c … || echo 0)` captured the two-line string "0\n0". Both of the
# gate's guards then died with "integer expected", `[` returning an error took the else branch
# of each `if`, and the script fell through to its success message — reporting ok, exiting 0,
# in exactly the total-collapse case the guards exist to catch. A gate that cannot fail is the
# thing this whole file is about.
echo "== the mutation gate fails when the harness produced nothing at all"
TOTAL=$((TOTAL + 1))
_mg="$(dirname "$0")/harness-mutation-test.sh"
_silent="$(mktemp -d)/silent-harness.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$_silent"
chmod +x "$_silent"
if bash "$_mg" "$_silent" --check >/tmp/mg.out 2>&1; then
    echo "  BAD   the mutation gate passed a harness that emitted no checks at all:"
    sed -n 's/^/        /p' /tmp/mg.out | tail -5
    BAD=$((BAD + 1))
else
    echo "  ok    a harness that emits no checks fails the mutation gate"
fi
rm -rf "$(dirname "$_silent")"

# And the other side of the same gate (R-7). A ceiling on survivors cannot tell "the checks got
# stronger" from "the checks were deleted": pointed at a harness with three checks, the gate
# reported `ok: 2 survivors, within the budget of 92; 1 checks did their job` and exited 0. The
# collapse case above only fires when a harness emits NOTHING; a harness reduced from 121 checks
# to 3 emits plenty.
echo "== the mutation gate fails when a harness still runs but its assertions are gone"
TOTAL=$((TOTAL + 1))
_tinydir="$(mktemp -d)"
_tiny="$_tinydir/tiny-harness.sh"
cat > "$_tiny" <<'TINYEOF'
#!/usr/bin/env bash
LX="${SHALL:-shall}"
ok()  { echo "  PASS  $1"; }
nok() { echo "  FAIL  $1"; }
if "$LX" --version >/dev/null 2>&1; then ok "Shall runs"; else nok "Shall runs"; fi
if "$LX" init      >/dev/null 2>&1; then ok "init runs";  else nok "init runs";  fi
if "$LX" eval | grep -q schema;       then ok "eval emits a model"; else nok "eval emits a model"; fi
TINYEOF
chmod +x "$_tiny"
if bash "$_mg" "$_tiny" --check >/tmp/mg-floor.out 2>&1; then
    echo "  BAD   the mutation gate passed a harness with three checks and one real assertion:"
    sed -n 's/^/        /p' /tmp/mg-floor.out | tail -3
    BAD=$((BAD + 1))
else
    echo "  ok    a harness whose assertions were deleted fails the mutation gate"
fi
rm -rf "$_tinydir"

# The six predicates that used to live here read `ci.yml`, the release scripts, the Dockerfiles
# and the harnesses as text and never ran a script or entered a container: gate parity, orphan
# scripts, function-defined-before-called, CRLF endings, floor mounts, image identity. They are
# `tests/the_review_apparatus_is_rust_tests.rs` now, where they fail in `cargo test` next to the
# twenty-seven other gates that read this repo, rather than at the end of a release script.
#
# What is left in this file is the half that cannot move: `lift` pulls function bodies out of
# the harnesses and runs them, which tests the bytes CI actually executes, in the interpreter
# that executes them.
# The register's own arithmetic. Two files tracked one number by hand and disagreed four ways
# at once — `decisions.md` said 109, 107 and 104 in three places, `SPEC.md` said 107. Counting
# is the fix; checking the count on every push is what stops it coming back.
echo "== the decision register's counts match the register"
TOTAL=$((TOTAL + 1))
if sh "$(dirname "$0")/decision-count.sh" --check >/tmp/dc.out 2>&1; then
    echo "  ok    every documented decision count matches the register"
else
    echo "  BAD   documented decision counts disagree with the register:"
    sed -n 's/^  BAD   /        /p' /tmp/dc.out
    BAD=$((BAD + 1))
fi

echo "--------------------------------------------------------------"
echo " harness predicates: $((TOTAL - BAD))/$TOTAL ok"
[ "$BAD" = 0 ] || { echo " FAILED"; exit 1; }
