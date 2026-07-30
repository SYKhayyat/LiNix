#!/usr/bin/env bash
# Run an integration harness against a `linix` that does nothing and exits 0.
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
    *)                     DEFAULT_BUDGET=86; DEFAULT_FLOOR=33 ;;
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
# Measured 2026-07-30: the Windows harness catches 35 and the container harness 44 (the latter
# run outside a container, where it reports one survivor fewer than CI does). The floors are set
# a little under each, because this gate exists to catch a COLLAPSE — 35 down to 1 — and not a
# wobble of one or two checks between hosts. Ratchet them up when a batch of checks is
# strengthened; never down to get green, which is the same instruction the budget carries in the
# other direction.
FLOOR="${CAUGHT_FLOOR:-$DEFAULT_FLOOR}"

ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT" || exit 2
[ -f "$HARNESS" ] || { echo "no harness at $HARNESS" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
STUB="$WORK/linix"
cat > "$STUB" <<'STUBEOF'
#!/bin/sh
# Does nothing. Reports success. Answers every question with silence.
exit 0
STUBEOF
chmod +x "$STUB"

echo "== running $HARNESS$HARNESS_ARGS against a do-nothing linix"
# Unquoted on purpose: the harness's own arguments, split as it would receive them on a
# command line.
# shellcheck disable=SC2086
LINIX="$STUB" bash "$HARNESS" $HARNESS_ARGS > "$WORK/out.txt" 2>&1
echo "   harness exit: $?"

# `grep -c` prints `0` and ALSO exits 1 when it matches nothing, so `$( … || echo 0 )` ran
# both halves and captured the two-line string "0\n0". Every `[ "$X" -eq … ]` below then died
# with "integer expected", `[` returning an error took the else branch, and this script fell
# through to its success message — in exactly the total-collapse case the guards exist for.
# Assign first, default on failure: one integer, always.
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

echo "   $CAUGHT check(s) caught the do-nothing binary"
echo "   $SURVIVORS check(s) passed anyway — each of those examined nothing the stub broke"
echo
echo "== survivors"
grep "  PASS  " "$WORK/out.txt" | sed 's/^  PASS  /   /' | sort

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
    echo " FAILED: not one check noticed that LiNix did nothing at all."
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
