#!/usr/bin/env bash
# Run an integration harness against a `shall` that does nothing and exits 0.
#
# **Every check that still passes is a check that did not examine the thing it names.**
# That is the whole idea: a harness cannot be trusted because it is green, only because its
# checks are capable of going red, and the cheapest way to find out is to break the product
# completely and see who notices.
#
#   ./scripts/harness-mutation-test.sh                 # report
#   ./scripts/harness-mutation-test.sh --check         # fail if the survival rate is over ceiling
#   SURVIVOR_RATE=600 ./scripts/harness-mutation-test.sh --check      # permille
#   CAUGHT_FLOOR=30 ./scripts/harness-mutation-test.sh --check
#   FAIL_SURVIVOR_RATE=100 FAIL_CAUGHT_FLOOR=90 ./scripts/harness-mutation-test.sh --check
#
# Two stubs run under --check: one that does nothing and succeeds, one that fails everything.
# The first finds checks that examine nothing; the second finds checks that cannot tell a
# deliberate refusal from a crash.
#
# **The survivor ceiling is a RATE, in permille, and it used to be a count.** A count cannot tell
# a harness that grew from one that got weaker, and on 2026-08-16 it stopped both harnesses for
# growing. The container harness went from 92 survivors of 136 checks to 120 of 198 — 62 new
# checks, of which 34 catch a do-nothing binary — and the gate reported the 120 and failed. The
# Windows harness did the same, 86 of 120 to 90 of 139, and was over its budget on a clean tree
# while green on the runner.
#
# The growth is the legitimate kind this file's own header describes two paragraphs down: an
# `lx -y sync` exit-code check paired with the assertion that looks at the effect, and
# preconditions like "no shim exists before the sync that deploys it". Each pair adds one
# survivor AND one catch, so it moves a count and leaves a rate where it was. A weakening does
# the opposite — it adds a survivor with no catch beside it, or turns a catch into a survivor —
# and moves the rate every time.
#
# **What a rate buys and what it costs, so the next person does not have to rediscover it.**
# It costs the strictest reading: adding 100 strong checks buys room for a few weak ones, which
# a count would have refused. That trade is deliberate. A count refused the weak ones and the
# strong ones alike, so the only way past it was to stop adding checks — and a gate whose
# cheapest satisfying move is "write no more tests" is worse than the leak it plugs. The half
# that cannot be traded away is `CAUGHT_FLOOR` below, which stays an absolute number.
#
# Still a ratchet, in the same direction: lower the rate whenever a batch is fixed, and never
# raise it to get green.
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
# The ceiling belongs to the harness, not to whoever calls it.
#
# There used to be one default — 86, the Windows harness's measured number — and the container
# harness's 92 lived only as `-e SURVIVOR_BUDGET=92` in `ci.yml`. So running this script the way
# the usage block above documents it failed on a clean tree, and the four-distro harness was
# mutation-tested in exactly one place while `harness-logic-test.sh` reported parity because the
# basename appeared in both release scripts.
#
# **The Windows harness is the clean demonstration of why this is a rate.** Between the last green
# CI run and today it grew from 131 checks to 139, its survivor COUNT went 85 to 90 — crossing a
# budget of 86 — and its survival RATE went 649 permille to 647. The count called that a
# regression. Nothing about it was one.
#
# Measured, do-nothing stub. "then" is CI run 31821612048, the last green main; "now" is CI
# nightly 31925296671, which a local `shall-it-ubuntu` reproduced to the check:
#
#             then                    now
#   container 91/157 = 580 permille   120/198 = 606 permille
#   windows   85/131 = 649            90/139  = 647
#
# **And three of the four cells got worse, which is stated rather than smoothed over.** Only the
# Windows do-nothing rate held. The container harness gained 41 checks and 29 of them survive a
# do-nothing binary against 12 that catch it, so the new batch is weaker than the harness it
# joined — precondition-heavy, for the reasons catalogued at the fail-stub numbers below. The
# ceilings here are therefore **a deliberate loosening**, set at today's measured rate plus room
# for the host wobble the old comments document. They are not evidence of an improvement and must
# not be quoted as one.
#
# That loosening is the price of replacing an instrument that could not measure the question at
# all. A count refuses a weak check and a strong one alike, so the cheapest way past it is to stop
# adding checks — and the four sections that arrived in those 41 are coverage this repository did
# not have. **That job was done rather than left as an aspiration**: the batch it named got its
# positive controls, the container do-nothing rate came back to 548 permille and the fail-stub
# rate to 51, and both ceilings below moved down with them. The numbers in the table above are
# kept as the "then" they were measured as, not rewritten to look better.
#
# Each rate is a ratchet from here: lower it when that batch is fixed, never raise it again.
# **Ratcheted 2026-08-16 (second round), measured not estimated.** The container harness
# moved from 120/198 = 606 permille to 107/195 = 548, and its caught count from 78 to 88,
# when the absence-after family got its positive controls. Both numbers move together, which
# is what a real strengthening looks like: a survivor became a catch rather than a check
# disappearing. The Windows row is untouched because nothing measured it this round.
case "$HARNESS" in
    */run-in-container.sh) DEFAULT_RATE=600; DEFAULT_FLOOR=80 ;;
    *)                     DEFAULT_RATE=690; DEFAULT_FLOOR=42 ;;
esac
RATE_CEILING="${SURVIVOR_RATE:-$DEFAULT_RATE}"
# The floor under CAUGHT — the half this gate did not have.
#
# A ceiling on survivors cannot tell "the checks got stronger" from "the checks were deleted".
# Proven rather than argued: pointed at a harness with three checks it reported `ok: 2
# survivors, within the budget of 92; 1 checks did their job` and exited 0. Deleting every
# effect assertion while still invoking every subcommand passed this gate, the lifecycle ratchet
# and the subcommand audit alike.
#
# Measured 2026-07-30 at 36 (Windows) and 44 (container); re-measured 2026-08-16 at 49 and 78,
# and the floors moved up with them — that is the direction this half ratchets. They stay a
# little under each, because this gate exists to catch a COLLAPSE — 35 down to 1 — and not a
# wobble of one or two checks between hosts. Ratchet them up when a batch of checks is
# strengthened; never down to get green, which is the same instruction the rate carries in the
# other direction.
#
# **This half stays an absolute count and must.** Once the survivor ceiling became a rate, the
# rate alone would be satisfied by deleting survivors and catches together in proportion — the
# three-check harness that reported `ok` still reports a fine rate. The floor is what says a
# harness this size exists at all.
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
# Lowering these rates means giving each a positive control, not deleting it. Ratchet down,
# never up.
#
# **Re-measured 2026-08-16 twice, and the second measurement is the one to read.** Against CI
# run 31821612048, the last green main, the container harness had gone 14/156 = 90 permille to
# 27/197 = 137 — the largest single loosening this file has ever carried, and the 27 were read
# one by one rather than summarised. Reading them is what produced the fix: **27 survivors of
# 197 became 10 of 194 — 51 permille — and the caught count went 170 to 184.**
#
# **The published 9/5/13 classification of those 27 was wrong**, and correcting it is most of
# what made them fixable. Seventeen of the 27 were controllable and are now controlled:
#
#   11 ABSENCE-AFTER assertions — "the shim is gone from disk", "the daemon really stopped",
#      "every file the tree placed is gone". Each now goes through the `witness` / `gone_ok`
#      pair in the harness: the sighting is recorded where the harness already asserts
#      presence, and an absence with no sighting behind it is a FAILURE, not a pass.
#    2 BOUNDS a zero satisfies — "adopt took no more than what apt calls user-chosen". A bound
#      over an empty set is not a weak assertion, it is no assertion, so it now reports
#      unmeasured rather than passing.
#    3 REFUSAL checks that scored any non-zero exit. `nok_saying` with the SUBJECT as the
#      pattern: the stub never echoes its arguments and every real refusal names what it
#      refused, so this is both the tighter check and the one a user needs (V.42).
#    1 PATTERN that matched the program's own name. `protected includes a system essential`
#      grepped for `shall\|libc\|systemd\|...`, and the stub prints `shall: this stub fails
#      everything`. The alternation no longer contains `shall`.
#
# **And one of the 27 had a published explanation that was tested and pronounced false.**
# `running the shim reaches the real tool` invokes `"$_bindir/$PKG" --version` and greps for
# the package name — so a shell asked to run a file that is not there prints the path in its
# own error and `bash: /root/.local/bin/jq: No such file or directory` matches `[jJ]q`. That
# hypothesis was recorded as disproved because the experiment used an EMPTY path, where the
# error names `/` and no longer contains the pattern. Right hypothesis, different command. It
# is guarded on the shim existing now.
#
# **The 10 that remain are three shapes a fail-everything stub cannot distinguish, plus one
# artefact of the instrument. None of them is a weak check and none of them is a job.**
#
#    5 PRECONDITIONS — "no shim exists before the sync that deploys it", "nothing is adopted
#      before adopt runs", "the link target does not exist before sync", "the tree's
#      destinations are empty before sync", "the service is disabled and stopped before the
#      declaration". These run BEFORE the product acts, so no product behaviour can move them.
#      They are the controls the checks after them depend on; deleting them to lower this
#      number would weaken the harness while improving its score.
#    3 assertions that the product showed RESTRAINT — "and freezes nothing", "and the
#      unapproved script did NOT run", "dry-run did NOT actually install jq". There is no
#      earlier presence to witness, because the whole claim is that nothing was ever written.
#      A stub that does nothing has exactly the same restraint, and no instrument of this shape
#      can tell the two apart.
#    1 assertion that Shall did not break what the image already had — "python3 still
#      installed after adopt". Same shape as the three above.
#    1 INSTRUMENT ARTEFACT — "git log shows a shall commit" greps for `shall:`, and the fail
#      stub writes `shall: this stub fails everything` to stderr, which `grep_ok` folds into
#      its output. The check is not weak; the stub's prefix collides with the product's
#      commit-subject prefix. Changing either to break the collision would make the stub or the
#      product less like itself, which is a worse trade than one known survivor.
#
# So the ceiling below is a MEASUREMENT with a few points of runner slack, not a loosening.
# Still a ratchet in the same direction: lower it when a batch is fixed, never raise it to get
# green. The Windows row is unchanged — the same `witness`/`gone_ok` pair landed in that
# harness for its two absence-after checks, and nothing measured the result this round.
case "$HARNESS" in
    */run-in-container.sh) DEFAULT_FAIL_RATE=70;  DEFAULT_FAIL_FLOOR=178 ;;
    *)                     DEFAULT_FAIL_RATE=90;  DEFAULT_FAIL_FLOOR=108 ;;
esac
FAIL_RATE_CEILING="${FAIL_SURVIVOR_RATE:-$DEFAULT_FAIL_RATE}"
FAIL_FLOOR="${FAIL_CAUGHT_FLOOR:-$DEFAULT_FAIL_FLOOR}"

# Survivors as a share of every check that ran, in permille. Integer arithmetic, because this is
# POSIX shell and a float here would be a second way to be wrong. `measure` guarantees the two
# are non-negative integers and the callers guarantee they are not both zero.
# Callers must have ruled out "no checks ran" first — a rate over nothing is not 0, it is a
# question with no subject, and dividing by it would print a shell error and set an empty RATE
# that every `[ "$RATE" -gt … ]` below then rejects as "integer expected". That is the exact
# already-once-fixed shape this script's `measure` comment describes: a number that is not a
# number is a broken gate, not a zero. Guarded here as well as at both call sites, because the
# call site that was missing its guard is what made this comment necessary.
survival_rate() { # survivors caught
    if [ "$(( $1 + $2 ))" -eq 0 ]; then
        echo " FAILED: asked for a survival rate over zero checks. The gate cannot judge this." >&2
        exit 2
    fi
    echo $(( $1 * 1000 / ($1 + $2) ))
}

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
RATE=$(survival_rate "$SURVIVORS" "$CAUGHT")
if [ "$RATE" -gt "$RATE_CEILING" ]; then
    echo " FAILED: $SURVIVORS of $((SURVIVORS + CAUGHT)) checks survive a do-nothing binary —"
    echo "         $RATE permille, over the ceiling of $RATE_CEILING."
    echo "         Add an assertion that looks at the effect, rather than raising the ceiling."
    exit 1
fi
if [ "$CAUGHT" -lt "$FLOOR" ]; then
    echo " FAILED: only $CAUGHT checks caught the do-nothing binary, under the floor of $FLOOR."
    echo "         The survivor RATE above cannot tell stronger checks from FEWER checks — a"
    echo "         harness cut in half keeps its rate. This floor is the half that notices."
    echo "         If assertions were deliberately removed, lower the floor in this file and say"
    echo "         why in the commit. If they were not, something stopped running."
    exit 1
fi
echo " ok: $SURVIVORS of $((SURVIVORS + CAUGHT)) survive — $RATE permille, within $RATE_CEILING;"
echo "     $CAUGHT checks did their job, at or above the floor of $FLOOR."

# The second stub, and its own ratchet. A check that passes here cannot tell a Shall that
# refused on purpose from one that simply broke -- which is the distinction the product
# publishes as exit 3 and the reason `refuses_with_3` exists beside `nok`.
measure "$FAILSTUB" "a shall that fails everything"
# The same guard the do-nothing stub gets, and it was missing here for as long as this block has
# existed. It did not matter while the threshold was a count — "0 survivors" passes a budget
# harmlessly, if uselessly — and it matters now, because a rate over nothing is a divide by zero.
# "Nothing caught it" and "nothing ran" are different findings; reporting the second as the first
# sends the reader looking for weak assertions in a run that had none.
if [ "$((SURVIVORS + CAUGHT))" -eq 0 ]; then
    echo " FAILED: the harness emitted no checks against the fail-everything stub — it did not"
    echo "         run, or it died first. This gate can say nothing about checks that never ran."
    exit 1
fi
FAIL_RATE=$(survival_rate "$SURVIVORS" "$CAUGHT")
if [ "$FAIL_RATE" -gt "$FAIL_RATE_CEILING" ]; then
    echo " FAILED: $SURVIVORS of $((SURVIVORS + CAUGHT)) checks pass against a binary that fails"
    echo "         everything — $FAIL_RATE permille, over the ceiling of $FAIL_RATE_CEILING. A"
    echo "         check that cannot tell a refusal from a crash is asserting the exit code is"
    echo "         non-zero and nothing else -- assert exit 3 with \`refuses_with_3\`, or look at"
    echo "         the effect."
    exit 1
fi
if [ "$CAUGHT" -lt "$FAIL_FLOOR" ]; then
    echo " FAILED: only $CAUGHT checks caught a binary that fails everything, under the floor"
    echo "         of $FAIL_FLOOR. Same reasoning as the floor above, other stub."
    exit 1
fi
echo " ok: $SURVIVORS of $((SURVIVORS + CAUGHT)) survive a fail-everything shall —"
echo "     $FAIL_RATE permille, within $FAIL_RATE_CEILING; $CAUGHT caught it, at or above $FAIL_FLOOR."
