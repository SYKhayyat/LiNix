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
case "$HARNESS" in --*) HARNESS="scripts/integration-windows.sh" ;; esac
CHECK=""
for a in "$@"; do [ "$a" = "--check" ] && CHECK=1; done
BUDGET="${SURVIVOR_BUDGET:-88}"

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

echo "== running $HARNESS against a do-nothing linix"
LINIX="$STUB" bash "$HARNESS" > "$WORK/out.txt" 2>&1
echo "   harness exit: $?"

SURVIVORS=$(grep -c "  PASS  " "$WORK/out.txt" 2>/dev/null || echo 0)
CAUGHT=$(grep -c "  FAIL  " "$WORK/out.txt" 2>/dev/null || echo 0)

echo "   $CAUGHT check(s) caught the do-nothing binary"
echo "   $SURVIVORS check(s) passed anyway — each of those examined nothing the stub broke"
echo
echo "== survivors"
grep "  PASS  " "$WORK/out.txt" | sed 's/^  PASS  /   /' | sort

if [ -z "$CHECK" ]; then exit 0; fi

echo
if [ "$CAUGHT" -eq 0 ]; then
    echo " FAILED: not one check noticed that LiNix did nothing at all."
    exit 1
fi
if [ "$SURVIVORS" -gt "$BUDGET" ]; then
    echo " FAILED: $SURVIVORS checks survive a do-nothing binary, over the budget of $BUDGET."
    echo "         Add an assertion that looks at the effect, rather than raising the budget."
    exit 1
fi
echo " ok: $SURVIVORS survivors, within the budget of $BUDGET; $CAUGHT checks did their job."
