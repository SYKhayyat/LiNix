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
# stopped re-deriving transience by retrying and started reading `linix-failure-class:`. A
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

    # ---- E6c/W4: "the binary is reachable" is a claim about the PRODUCT ------
    #
    # `on_path` alone asks the HOST. On a clean runner ~/go/bin, ~/.local/bin and yarn's
    # global directory are on nobody's PATH, so the old check failed three backends that
    # had done nothing wrong — and it would have passed all three had the machine happened
    # to be wired, which is the half that matters: an install that says NOTHING about an
    # unreachable binary is the defect (github and yarn, measured 2026-07-29).
    _rd="${TMPDIR:-/tmp}/linix-reach-$$"
    rm -rf "$_rd"; mkdir -p "$_rd"
    _rbin=linix-reach-zzz
    _rlog="$_rd/install.log"
    printf '%s\n' "  WARN linix::verbs::sync: \`go\` installs its executables into $_rd, which is not on your PATH — so what it just installed will answer \"command not found\"." > "$_rlog"

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

    # LiNix warned and the file is where it said: the product kept its promise, and the
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
    FAKE=""; _rbin=linix-reach-zzz

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
    echo "== subcommands invoked vs subcommands that exist: SKIPPED (set LINIX_BIN to a built binary)"
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
        PASS=0; FAILC=0; SOFTC=0; FAILED_NAMES=""; TO_LONG="timeout 900"
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

        # R-3, both directions. The classifier reads `linix-failure-class:` instead of retrying
        # the install to guess at it, and the two branches fail in opposite ways: a permanent
        # failure retried is a minute wasted per backend, and a transient one scored a defect is
        # a red CI leg over a rate-limit window that has since moved.
        LEDGER="$(mktemp -d)"; : > "$LEDGER/be-life-unmeasured"
        lx() { echo "the retry must not be reached"; return 1; }
        lx_slow() { lx "$@"; }

        FAILC=0; SOFTC=0; PASS=0
        printf 'linix-failure-class: permanent\n' > "$_log"
        # To a file, not a `$( )`: command substitution runs in a subshell, so `CLASS` set
        # inside it never reaches this scope and the assertion below reads the PREVIOUS call's
        # answer. It did, and reported `timeout`.
        _out="$(mktemp)"
        classify_install be spec 1 "$_log" > "$_out" 2>&1
        [ "$CLASS" = defect ] || { echo "  BAD   a permanent failure is not a defect (got '$CLASS')"; _bad=1; }
        grep -q retrying "$_out" && { echo "  BAD   a permanent failure was retried anyway"; _bad=1; }
        rm -f "$_out"

        FAILC=0; SOFTC=0
        printf 'linix-failure-class: transient\n' > "$_log"
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
LX="${LINIX:-linix}"
ok()  { echo "  PASS  $1"; }
nok() { echo "  FAIL  $1"; }
if "$LX" --version >/dev/null 2>&1; then ok "linix runs"; else nok "linix runs"; fi
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

# Gate parity: a local gate weaker than CI is a local GO that CI turns into a NO-GO, found
# after the push instead of before it. E3/E4 was one instance (`cargo fmt` informational
# locally, fatal in CI); the class is that three files list the gates and nobody diffs them.
#
# Derived, not listed. Every `scripts/*.sh` that ci.yml runs must be named by BOTH release
# scripts, so a gate added to CI tomorrow fails this until it is added locally too — which is
# the only version of this check that does not go stale the day it is written.
#
# **A basename is not a gate** (G-4). This compared basenames until 2026-07-28, and CI runs
# `harness-mutation-test.sh` twice — once bare, measuring the Windows harness, and once against
# `docker/integration/run-in-container.sh`, measuring the four-distro one. Both release scripts
# ran it once. Parity passed, because the string did appear in both files, while the harness
# that runs on every push against four distros was mutation-tested only in CI. So the unit here
# is the *invocation*: a gate plus the harness it is pointed at. Two targets are two gates.
echo "== every gate CI runs is also run by the local release scripts"
_ci=".github/workflows/ci.yml"
_here="$(cd "$(dirname "$0")/.." && pwd)"
if [ -f "$_here/$_ci" ]; then
    _gates="$(grep -oE 'scripts/[a-z-]+\.sh' "$_here/$_ci" | sort -u)"
    # And the targets: a gate script invoked with a harness path measures that harness, and a
    # local script that runs the same gate against a different harness is not running the same
    # gate. Rendered as `<gate>|<target>` pairs so one loop can check both kinds.
    _targeted="$(grep -oE 'scripts/[a-z-]+\.sh [^"]*(docker/integration/[a-z-]+\.sh|scripts/integration-[a-z]+\.sh)' "$_here/$_ci" \
        | sed -E 's#^(scripts/[a-z-]+\.sh).*[ ](docker/integration/[a-z-]+\.sh|scripts/integration-[a-z]+\.sh).*#\1|\2#' \
        | sort -u)"
    _n=0
    for _g in $_gates; do _n=$((_n + 1)); done
    TOTAL=$((TOTAL + 1))
    if [ "$_n" -lt 2 ]; then
        # The G2 shape again: an audit over an empty list passes without examining anything.
        echo "  BAD   found $_n gate script(s) in ci.yml — this check has stopped matching it"
        BAD=$((BAD + 1))
    else
        _missing=""
        for _rc in scripts/release-check.sh scripts/release-check.ps1; do
            for _g in $_gates; do
                grep -q "$(basename "$_g")" "$_here/$_rc" 2>/dev/null && continue
                _missing="$_missing\n        $(basename "$_rc") never runs $(basename "$_g")"
            done
            # The targeted invocations: the gate's own line in the local script has to name the
            # same harness. Grepping the file as a whole would pass on any mention of the path.
            for _pair in $_targeted; do
                _gate="${_pair%%|*}"
                _target="${_pair##*|}"
                [ "$_gate" = "$_target" ] && continue   # the gate IS that harness
                grep -F "$(basename "$_gate")" "$_here/$_rc" 2>/dev/null \
                    | grep -qF "$_target" && continue
                _missing="$_missing\n        $(basename "$_rc") runs $(basename "$_gate") but never against $_target"
            done
        done
        if [ -z "$_missing" ]; then
            echo "  ok    both release scripts run all $_n gate script(s) CI runs, against the same harnesses"
        else
            echo "  BAD   a local gate is weaker than CI:"
            printf "%b\n" "$_missing"
            BAD=$((BAD + 1))
        fi
    fi
else
    echo "  BAD   no $_ci to diff the local gates against"
    TOTAL=$((TOTAL + 1)); BAD=$((BAD + 1))
fi

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

# No gate script may sit in `scripts/` with nothing running it (G-5).
#
# `grader-red-tests.sh` was 131 lines of source-text greps, run by no CI job and neither
# release script, and its first check could never pass because it reproduced the very bug it
# tested inline. It sat there for a day claiming to be the list of what was still wrong. A
# permanently-red file nobody runs is worse than no file: it is the shape of check this whole
# effort exists to remove, and it is invisible precisely because nothing runs it.
#
# A script is accounted for when CI runs it, a release script runs it, or another gate runs it.
# Anything else has to be named below as a deliberate non-gate, with what it is instead.
echo "== every script in scripts/ is run by something, or declared not to be a gate"
TOTAL=$((TOTAL + 1))
_orphans=""
for _s in "$_here"/scripts/*.sh "$_here"/scripts/*.ps1; do
    [ -f "$_s" ] || continue
    _b="$(basename "$_s")"
    case "$_b" in
        # Not gates. `install.*` is what a user pipes from the web; `release-check.*` are the
        # top of the chain and are run by a person, which is their whole job.
        install.sh|install.ps1|release-check.sh|release-check.ps1) continue ;;
    esac
    # `-rl`, not `-rql`: `-q` prints nothing, so the pipe that drops the script's own file
    # would see an empty list and call every script an orphan. (It did, on the first run.)
    if grep -rl "$_b" "$_here/.github/workflows" "$_here/scripts/release-check.sh" \
        "$_here/scripts/release-check.ps1" "$_here/scripts/harness-logic-test.sh" 2>/dev/null \
        | grep -v "/$_b\$" | grep -q .
    then
        continue
    fi
    _orphans="$_orphans\n        $_b"
done
if [ -z "$_orphans" ]; then
    echo "  ok    no gate script in scripts/ is unreachable from CI or a release script"
else
    echo "  BAD   these scripts are run by nothing — delete them or wire them in:"
    printf "%b\n" "$_orphans"
    BAD=$((BAD + 1))
fi


# ---------------------------------------------------------------------------
# A harness function must be defined ABOVE the first place the script CALLS it.
#
# Shell reads top to bottom: a function called before its `f() {` has been evaluated is not a
# quiet no-op, it is `command not found` on stderr — and the harness keeps going. Measured on
# CI, 2026-07-29: three PATH helpers were defined beside `assert_binary_gone` at line 617 and
# called from section 5 at line 329, so one check reported `rc=127` and **one vanished entirely**
# — no PASS, no FAIL, no count. A check that does not run looks exactly like a check that
# passed, which is this file's whole subject.
#
# Only calls at the top level count. A name used inside another function body is resolved when
# that body runs, which is after the whole file has been read — `classify_install` calling
# `refused` is correct however they are ordered, and a checker that cannot tell the difference
# reports three false positives and gets switched off.
#
# Lexical rather than behavioural, deliberately: the stub-binary mutation run only reaches the
# checks it reaches, and this has to answer for every call site whether that run got there or not.
echo "== every harness function is defined before the script calls it"
for _src in $SOURCES; do
    TOTAL=$((TOTAL + 1))
    _bad_fns=""
    for _def in $(grep -nE '^[a-z_][a-z0-9_]*\(\) *\{' "$_src" | sed 's/(.*//' | tr -d ' '); do
        _line="${_def%%:*}"; _fn="${_def##*:}"
        _use="$(awk -v fn="$_fn" '
            /^[a-z_][a-z0-9_]*\(\)[ ]*\{/ { if ($0 !~ /\}/) infn = 1; next }
            infn && /^\}/ { infn = 0; next }
            !infn {
                line = $0
                sub(/#.*/, "", line)
                # Quoted text is not a call. `echo "[5] Real lifecycle"` and
                # `ok "sync commits" …` both name a function inside a description, and reading
                # those as call sites is how a checker earns its reputation for crying wolf.
                # Double quotes only: every description in these harnesses is double-quoted,
                # and the single-quoted text is `sh -c` bodies, which name no functions.
                gsub(/"[^"]*"/, "", line)
                if (line ~ "(^|[^-a-zA-Z0-9_])" fn "([^-a-zA-Z0-9_]|$)") { print NR; exit }
            }
        ' "$_src")"
        [ -n "$_use" ] || continue
        if [ "$_use" -lt "$_line" ]; then
            _bad_fns="$_bad_fns\n        $_fn: called at line $_use, defined at line $_line"
        fi
    done
    if [ -z "$_bad_fns" ]; then
        echo "  ok    $(basename "$_src") defines every function above the first call to it"
    else
        echo "  BAD   $(basename "$_src") calls functions that do not exist yet at that point:"
        printf "%b\n" "$_bad_fns"
        BAD=$((BAD + 1))
    fi
done

# ----------------------------------------------------------------------------
# Every container leg that runs the harness must also mount the ratchet's floor file.
#
# `.dockerignore` excludes `scripts/`, deliberately — editing a host script must not bust the
# image's cargo cache — so `scripts/lifecycle-floor.txt` is in no image and reaches a container
# only by being mounted. It was not, on any leg. The ratchet was in force on one host class of
# five: the Windows sweep, which has the least coverage, and absent from the four distro legs and
# the `tools` image, which have the most. Every one of those runs was green (N-5).
#
# This asks the question that would have caught it: for each `docker run` that mounts the
# harness, is the floor mounted too?
echo "== every container leg that runs the harness mounts the lifecycle floor"
TOTAL=$((TOTAL + 1))
_ci="$(cd "$(dirname "$0")/.." && pwd)/.github/workflows/ci.yml"
if [ ! -r "$_ci" ]; then
    echo "  BAD   cannot read $_ci"
    BAD=$((BAD + 1))
else
    # Count the harness mounts and the floor mounts. They travel together or the gate is inert
    # on the difference.
    _harness_mounts=$(grep -c "run-in-container.sh:/src/docker/integration/run-in-container.sh" "$_ci")
    _floor_mounts=$(grep -c "lifecycle-floor.txt:/src/scripts/lifecycle-floor.txt" "$_ci")
    if [ "$_harness_mounts" = "$_floor_mounts" ] && [ "$_harness_mounts" -gt 0 ]; then
        echo "  ok    all $_harness_mounts container leg(s) mount the floor file"
    else
        echo "  BAD   $_harness_mounts container leg(s) mount the harness, $_floor_mounts mount the floor"
        echo "        A leg without the floor runs the ratchet's else branch, which measures nothing."
        BAD=$((BAD + 1))
    fi
fi

# ----------------------------------------------------------------------------
# Every shell script this repo runs must have LF endings, in the working tree.
#
# Not a style rule. `run.sh` bind-mounts the host's copy of the harness into the container,
# where /bin/sh is dash; dash reads `set -u<CR>`, aborts with `set: Illegal option -`, and no
# check runs. `.gitattributes` pins `*.sh text eol=lf` and the committed blobs are LF, so CI is
# unaffected and nothing here ever fired — eol=lf governs what checkout writes, not what an
# editor writes afterwards. On 2026-07-29 four scripts in the development working tree were CRLF
# and the entire local container gate was silently unavailable (N-6).
#
# Checked here because both release scripts already run this gate, so it needs no new wiring and
# cannot fall out of the CI/local parity check one section up.
TOTAL=$((TOTAL + 1))
_root="$(cd "$(dirname "$0")/.." && pwd)"
# The scripts, plus every file bind-mounted into a container — read off the mounts themselves
# rather than listed here. `scripts/lifecycle-floor.txt` is data, not a script, so no glob covered
# it, and it is parsed inside the container with `awk '{print $2}'`: over a CRLF line that yields
# `7<CR>`, `[ -lt ]` errors on a non-integer, and the shell takes the else branch — the one that
# reports the ratchet satisfied. `.gitattributes` pins it now; this is what notices the next one.
_files="$(
    ls "$_root"/scripts/*.sh "$_root"/docker/integration/*.sh 2>/dev/null
    grep -hoE '[$]PWD/[^:"]+:' "$_root/.github/workflows/ci.yml"         "$_root/docker/integration/run.sh" 2>/dev/null |
        sed -e 's|:$||' -e "s|[$]PWD/|$_root/|"
)"
# `od -c` first, and not `grep` alone, because MSYS grep opens a file in text mode and
# normalises CRLF before matching: on Git Bash the pattern never fires on the endings this
# check exists to find. It caught a *lone* CR once — one not followed by LF, which no
# translation touches — which is how it passed its own review. `od` renders the byte as the
# two characters \r, and grep matching ordinary text is something every platform agrees on.
_has_cr() { head -c 65536 "$1" | od -c | grep -q '\\r'; }

# The detector, tried against a file that must trip it, before anything is concluded from it
# saying no. This gate was blind on the one platform where the bug it guards occurs.
_probe="${TMPDIR:-/tmp}/linix-crlf-selftest.$$"
printf 'x\r\n' > "$_probe"
if _has_cr "$_probe"; then
    rm -f "$_probe"
else
    rm -f "$_probe"
    echo "  BAD   the CRLF detector cannot see a CRLF file, so its verdict below means nothing"
    BAD=$((BAD + 1))
fi

_crlf=""
for _f in $(printf '%s\n' "$_files" | sort -u); do
    [ -f "$_f" ] || continue
    if _has_cr "$_f"; then
        _crlf="$_crlf $(basename "$_f")"
    fi
done
if [ -z "$_crlf" ]; then
    echo "  ok    every shell script has LF endings (dash aborts on CRLF before any check runs)"
else
    echo "  BAD   CRLF line endings in the working tree — dash cannot run these:"
    for _f in $_crlf; do echo "        $_f"; done
    echo "        fix: sed -i 's/\\r\$//' on each, or git add --renormalize . && git checkout -- ."
    BAD=$((BAD + 1))
fi

# ----------------------------------------------------------------------------
# Every integration image declares its own identity, and declares it correctly.
#
# The ratchet keys its floor on the image, and `/etc/os-release` cannot supply that: `tools` is
# built on Ubuntu, so it and the ubuntu image answered the same name and shared one record while
# doing 25 and 7 real lifecycles. A new Dockerfile that forgets the ENV silently rejoins whatever
# distro it is based on, which is the collision rather than a new host class.
TOTAL=$((TOTAL + 1))
_bad_img=""
for _df in "$_root"/docker/integration/Dockerfile.*; do
    [ -f "$_df" ] || continue
    _want="${_df##*/Dockerfile.}"
    _got="$(grep -E '^ENV LINIX_IT_IMAGE=' "$_df" | tail -1 | cut -d= -f2 | sed 's/[[:space:]]*$//')"
    [ "$_got" = "$_want" ] || _bad_img="$_bad_img Dockerfile.$_want(declares=${_got:-none})"
done
if [ -z "$_bad_img" ]; then
    echo "  ok    every integration image declares LINIX_IT_IMAGE matching its Dockerfile"
else
    echo "  BAD   image identity missing or wrong:$_bad_img"
    echo "        The ratchet then files this image under its base distro's record."
    BAD=$((BAD + 1))
fi

echo "--------------------------------------------------------------"
echo " harness predicates: $((TOTAL - BAD))/$TOTAL ok"
[ "$BAD" = 0 ] || { echo " FAILED"; exit 1; }
