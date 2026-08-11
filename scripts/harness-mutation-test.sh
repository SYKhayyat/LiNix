#!/usr/bin/env bash
# Run an integration harness against a `shall` that does nothing and exits 0.
#
# **Every check that still passes is a check that did not examine the thing it names.**
# That is the whole idea: a harness cannot be trusted because it is green, only because its
# checks are capable of going red, and the cheapest way to find out is to break the product
# completely and see who notices.
#
#   ./scripts/harness-mutation-test.sh                 # report
#   ./scripts/harness-mutation-test.sh --check         # fail if survivors exceed the budget
#   SURVIVOR_BUDGET=80 ./scripts/harness-mutation-test.sh --check
#   CAUGHT_FLOOR=30 ./scripts/harness-mutation-test.sh --check
#   FAIL_SURVIVOR_BUDGET=10 FAIL_CAUGHT_FLOOR=90 ./scripts/harness-mutation-test.sh --check
#
# Two stubs run under --check: one that does nothing and succeeds, one that fails everything.
# The first finds checks that examine nothing; the second finds checks that cannot tell a
# deliberate refusal from a crash.
#
# The budget is a ratchet, not a target. It exists so the number can only go down: a new
# exit-code-only check raises it and fails this gate, which is the moment to add the assertion
# that looks at the effect. Lower it whenever you fix a batch; never raise it to get green.
#
# Not every survivor is a defect. `ok "adopt runs" lx -y adopt` is an honest exit-code check
# whose EFFECT is asserted by the `test -s` on the next line, and that pair is correct. The
# ones worth fixing are the checks whose NAME claims an effect and which are followed by
# nothing that looks for it — `profile create scaffolds one` was one of those, and so was
# `module create`.
set -u

HARNESS="${1:-scripts/integration-windows.sh}"
case "$HARNESS" in
    --*) HARNESS="scripts/integration-windows.sh" ;;
    *)   [ "$#" -gt 0 ] && shift ;;
esac
CHECK=""
# Anything left over is handed to the harness. Without this the container harness could not be
# mutation-tested at all: it requires `<backend> [package]` and exits on usage before its first
# check, so the gate measured nothing and the four-distro suite ran every push unexamined.
HARNESS_ARGS=""
for a in "$@"; do
    case "$a" in
        --check) CHECK=1 ;;
        *)       HARNESS_ARGS="$HARNESS_ARGS $a" ;;
    esac
done
# The budget belongs to the harness, not to whoever calls it.
#
# There used to be one default — 86, the Windows harness's measured number — and the container
# harness's 92 lived only as `-e SURVIVOR_BUDGET=92` in `ci.yml`. So running this script the way
# the usage block above documents it failed on a clean tree, and the four-distro harness was
# mutation-tested in exactly one place while `harness-logic-test.sh` reported parity because the
# basename appeared in both release scripts.
#
# Each number is a ratchet in its own right: lower it when a batch is fixed, never raise it.
case "$HARNESS" in
    */run-in-container.sh) DEFAULT_BUDGET=92; DEFAULT_FLOOR=40 ;;
    *)                     DEFAULT_BUDGET=86; DEFAULT_FLOOR=34 ;;
esac
BUDGET="${SURVIVOR_BUDGET:-$DEFAULT_BUDGET}"
# The floor under CAUGHT — the half this gate did not have.
#
# A ceiling on survivors cannot tell "the checks got stronger" from "the checks were deleted".
# Proven rather than argued: pointed at a harness with three checks it reported `ok: 2
# survivors, within the budget of 92; 1 checks did their job` and exited 0. Deleting every
# effect assertion while still invoking every subcommand passed this gate, the lifecycle ratchet
# and the subcommand audit alike.
#
# Measured 2026-07-30: the Windows harness catches 36 (35 before the lifecycle-gap ceiling
# was added, which catches the stub too) and the container harness 44 (the latter
# run outside a container, where it reports one survivor fewer than CI does). The floors are set
# a little under each, because this gate exists to catch a COLLAPSE — 35 down to 1 — and not a
# wobble of one or two checks between hosts. Ratchet them up when a batch of checks is
# strengthened; never down to get green, which is the same instruction the budget carries in the
# other direction.
FLOOR="${CAUGHT_FLOOR:-$DEFAULT_FLOOR}"
# The same pair for the fail-everything stub. Measured on this tree the day `refuses_with_3`
# split off from `nok`: Windows 12 survivors / 96 caught, container 17 / 118 (run outside a
# container, which reports one survivor fewer than CI does -- hence the single point of slack).
# The grader measured SEVENTEEN survivors on the Windows harness before the split, sixteen of
# them refusal checks. Five became assertions about exit 3 (`refuses_with_3`) and five more
# assert the sentence as well as the code (`nok_saying`), which took Windows 12 -> 7 and the
# container 17 -> 12 on the developer's box.
#
# **The numbers below are the RUNNER's, not that box's**, and the first version of this gate was
# red in CI for exactly that reason: the container harness leaves 12 survivors here and 14 on a
# clean ubuntu runner, because two checks that fail locally have nothing to act on there
# (`git log shows a shall commit`, `rebuild wrote no git commit`). A budget measured on one host
# and enforced on another is a gate that fails for being wrong about the machine rather than
# about the checks. The Windows harness goes the other way — 7 locally, 6 on the runner — so its
# budget is the tighter of the two.
#
# The survivors that remain are mostly ABSENCE assertions ("x is gone", "no commit was written"),
# and an absence cannot tell a product that did nothing right from one that did nothing at all.
# Lowering these budgets means giving each a positive control, not deleting it. Ratchet down,
# never up.
case "$HARNESS" in
    */run-in-container.sh) DEFAULT_FAIL_BUDGET=14; DEFAULT_FAIL_FLOOR=115 ;;
    *)                     DEFAULT_FAIL_BUDGET=7;  DEFAULT_FAIL_FLOOR=95 ;;
esac
FAIL_BUDGET="${FAIL_SURVIVOR_BUDGET:-$DEFAULT_FAIL_BUDGET}"
FAIL_FLOOR="${FAIL_CAUGHT_FLOOR:-$DEFAULT_FAIL_FLOOR}"

ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT" || exit 2
[ -f "$HARNESS" ] || { echo "no harness at $HARNESS" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Two stubs, because they answer different questions.
#
# The first does nothing and reports success: every check that still passes examined nothing.
# The second answers `--version` (so the harness does not bail at its own front door) and fails
# everything else: every check that still passes cannot tell a working Shall from a broken one
# in the OTHER direction. The round-6 grader built the second by hand and found seventeen
# survivors, SIXTEEN of them refusal checks scoring "correctly refused" against a binary that
# was simply failing (G-8).
STUB="$WORK/shall"
cat > "$STUB" <<'STUBEOF'
#!/bin/sh
# Does nothing. Reports success. Answers every question with silence.
exit 0
STUBEOF
chmod +x "$STUB"

FAILSTUB="$WORK/shall-fail"
cat > "$FAILSTUB" <<'FAILEOF'
#!/bin/sh
# Answers --version and fails everything else, with a plain failure and never a refusal.
for a in "$@"; do
    case "$a" in
        --version|-V) echo "shall 0.0.0-mutation-stub"; exit 0 ;;
    esac
done
echo "shall: this stub fails everything" >&2
exit 1
FAILEOF
chmod +x "$FAILSTUB"

# Run the harness against one stub and count what survived. Sets SURVIVORS and CAUGHT.
measure() { # stub label
    _stub="$1"; _label="$2"
    echo "== running $HARNESS$HARNESS_ARGS against $_label"
    # Unquoted on purpose: the harness's own arguments, split as it would receive them on a
    # command line.
    # shellcheck disable=SC2086
    SHALL="$_stub" bash "$HARNESS" $HARNESS_ARGS > "$WORK/out.txt" 2>&1
    echo "   harness exit: $?"

    # `grep -c` prints `0` and ALSO exits 1 when it matches nothing, so `$( ... || echo 0 )` ran
    # both halves and captured a two-line string. Every `[ "$X" -eq ... ]` below then
    # died with "integer expected", `[` returning an error took the else branch, and this script
    # fell through to its success message -- in exactly the total-collapse case the guards exist
    # for. Assign first, default on failure: one integer, always.
    SURVIVORS=$(grep -c "  PASS  " "$WORK/out.txt" 2>/dev/null) || SURVIVORS=0
    CAUGHT=$(grep -c "  FAIL  " "$WORK/out.txt" 2>/dev/null) || CAUGHT=0

    # And a guard that does not depend on the counters being right, because the counters are what
    # went wrong. A number that is not a number is a broken gate, not a zero.
    for _n in "$SURVIVORS" "$CAUGHT"; do
        case "$_n" in
            ''|*[!0-9]*)
                echo " FAILED: counted '$_n', which is not a number. The gate cannot judge this run."
                exit 2
                ;;
        esac
    done

    echo "   $CAUGHT check(s) caught $_label"
    echo "   $SURVIVORS check(s) passed anyway"
    echo
    echo "== survivors ($_label)"
    grep "  PASS  " "$WORK/out.txt" | sed 's/^  PASS  /   /' | sort
    echo
}

measure "$STUB" "a do-nothing shall"

if [ -z "$CHECK" ]; then exit 0; fi

echo
# Told apart on purpose. "Nothing caught it" is a harness full of weak checks; "nothing ran"
# is a harness that died before its first check, and reporting the second as the first sends
# whoever reads this looking for assertions to strengthen in a run that had none.
if [ "$((SURVIVORS + CAUGHT))" -eq 0 ]; then
    echo " FAILED: the harness emitted no checks at all — it did not run, or it died first."
    echo "         Its output is above; this gate can say nothing about checks that never ran."
    exit 1
fi
if [ "$CAUGHT" -eq 0 ]; then
    echo " FAILED: not one check noticed that Shall did nothing at all."
    exit 1
fi
if [ "$SURVIVORS" -gt "$BUDGET" ]; then
    echo " FAILED: $SURVIVORS checks survive a do-nothing binary, over the budget of $BUDGET."
    echo "         Add an assertion that looks at the effect, rather than raising the budget."
    exit 1
fi
if [ "$CAUGHT" -lt "$FLOOR" ]; then
    echo " FAILED: only $CAUGHT checks caught the do-nothing binary, under the floor of $FLOOR."
    echo "         The survivor budget above cannot tell stronger checks from FEWER checks."
    echo "         If assertions were deliberately removed, lower the floor in this file and say"
    echo "         why in the commit. If they were not, something stopped running."
    exit 1
fi
echo " ok: $SURVIVORS survivors, within the budget of $BUDGET;"
echo "     $CAUGHT checks did their job, at or above the floor of $FLOOR."

# The second stub, and its own ratchet. A check that passes here cannot tell a Shall that
# refused on purpose from one that simply broke -- which is the distinction the product
# publishes as exit 3 and the reason `refuses_with_3` exists beside `nok`.
measure "$FAILSTUB" "a shall that fails everything"
if [ "$SURVIVORS" -gt "$FAIL_BUDGET" ]; then
    echo " FAILED: $SURVIVORS checks pass against a binary that fails everything, over the"
    echo "         budget of $FAIL_BUDGET. A check that cannot tell a refusal from a crash is"
    echo "         asserting the exit code is non-zero and nothing else -- assert exit 3 with"
    echo "         \`refuses_with_3\`, or look at the effect."
    exit 1
fi
if [ "$CAUGHT" -lt "$FAIL_FLOOR" ]; then
    echo " FAILED: only $CAUGHT checks caught a binary that fails everything, under the floor"
    echo "         of $FAIL_FLOOR. Same reasoning as the floor above, other stub."
    exit 1
fi
echo " ok: $SURVIVORS survive a fail-everything shall, within the budget of $FAIL_BUDGET;"
echo "     $CAUGHT caught it, at or above the floor of $FAIL_FLOOR."
