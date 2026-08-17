#!/bin/sh
# Count the decision register by reading it, and check the numbers written in the docs
# against the count.
#
# Two files used to track one number by hand and they disagreed three ways at once:
# `decisions.md` opened with "all 109", its status table added up to 107, its index said
# "All 104 are ruled: 102 ANSWERED", and `SPEC.md` said 107/105. Every one of those was
# typed. A number that is typed in two places is a number that will differ in two places.
#
#   ./scripts/decision-count.sh          - print the counts
#   ./scripts/decision-count.sh --check  - also verify every documented number matches
set -u

ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
REG="$ROOT/docs/spec/decisions.md"
MAP="$ROOT/docs/SPEC.md"

[ -f "$REG" ] || { echo "no register at $REG" >&2; exit 2; }

# Read the flag before `set --` below overwrites the argument list with the counts.
CHECK=""
for a in "$@"; do [ "$a" = "--check" ] && CHECK=1; done

# One entry is a `## <ID>` heading; its status is the first `**Status:` line beneath it.
# Counted from the headings, never from a table, because the table is the thing being checked.
counts="$(awk '
/^## [A-Z][0-9]+[a-z]?$/ { id=$2; found=0; total++; next }
id != "" && !found && /\*\*Status:/ {
    found=1
    if ($0 ~ /Status:[ ]*ANSWERED/)        answered++
    else if ($0 ~ /Status:[ ]*PARKED/)     parked++
    else if ($0 ~ /Status:[ ]*DEFERRED/)   deferred++
    else if ($0 ~ /Status:[ ]*HALF/)       half++
    else if ($0 ~ /Status:[ ]*BUILT/)      built++
    else if ($0 ~ /Status:[ ]*OPEN/)       open++
    else                                   other++
}
END {
    printf "%d %d %d %d %d %d %d %d\n",
        total, answered+0, parked+0, built+0, open+0, other+0, deferred+0, half+0
}
' "$REG")"

set -- $counts
TOTAL=$1; ANSWERED=$2; PARKED=$3; BUILT=$4; OPEN=$5; OTHER=$6; DEFERRED=$7; HALF=$8

echo "decision register: $TOTAL entries"
echo "  ANSWERED           $ANSWERED"
echo "  PARKED             $PARKED"
echo "  DEFERRED           $DEFERRED"
echo "  HALF RULED         $HALF"
echo "  BUILT, NEVER RULED $BUILT"
echo "  OPEN               $OPEN"
[ "$OTHER" = 0 ] || echo "  unrecognised       $OTHER"

[ -n "$CHECK" ] || exit 0

# --- the check: every number written down must equal the number counted ----------------
BAD=0
say_bad() { echo "  BAD   $1"; BAD=$((BAD + 1)); }

# An entry whose status this script cannot read is an entry it did not count into any of the
# four buckets — so every total below is computed over a register the script only partly
# understood, and the "ok, every documented count matches" at the end is a claim about the
# part it read. This printed `unrecognised 2` and exited 0 for as long as the check existed,
# while `SPEC.md` and `decisions.md` both advertised 164 against 166 entries.
[ "$OTHER" = 0 ] || say_bad "$OTHER entr(y/ies) have a **Status: this script cannot read, so \
every count below is over $((TOTAL - OTHER)) of $TOTAL entries"

# Any figure the docs state about the register's size has to be one of the counted ones.
# Written as "every stated total is $TOTAL" rather than as a regex per sentence, so a new
# sentence stating a wrong total is caught too.
for f in "$REG" "$MAP"; do
    [ -f "$f" ] || continue
    # `register — N entries` is the register's own H1, and it went stale by two while every
    # other figure in the file was correct and this check said `ok`. It was invisible because
    # the patterns above all spell the total as "all N": the title spells it "N entries", which
    # is a fourth place one number is typed — the exact thing this script exists to stop.
    stated="$(grep -oE '[Aa]ll [0-9]+ (decisions|are ruled)|register — all [0-9]+|[0-9]+ decisions\.|register — [0-9]+ entries' "$f" \
        | grep -oE '[0-9]+' | sort -u)"
    for n in $stated; do
        [ "$n" = "$TOTAL" ] || say_bad "$(basename "$f") states $n where the register holds $TOTAL"
    done

    # And the same title's "N ruled", which is everything not OPEN. Anchored on `entries:`
    # rather than matched bare: `[0-9]+ ruled` also hits "2 ruled" and "4 ruled" in ordinary
    # prose three times in this file, and a checker that reports prose as a miscount is one
    # somebody switches off.
    RULED=$((TOTAL - OPEN))
    for n in $(grep -oE 'entries: [0-9]+ ruled' "$f" | grep -oE '[0-9]+ ruled' | grep -oE '[0-9]+'); do
        [ "$n" = "$RULED" ] || say_bad "$(basename "$f") states $n ruled where the register holds $RULED (everything not OPEN)"
    done
    # **A count is written in the status's own spelling — shouted — and that is what makes it
    # checkable.** `SPEC.md` said "125 answered, 2 parked" in lower case and went a week wrong
    # while this run said `ok`, because only the shouted form was scanned. Lower case is not
    # scanned instead, it is *banned* below: `[0-9]+ answered` also matches "D5 answered the
    # ownership half" and "59 open questions", so a case-blind grep reports ordinary prose as a
    # miscount and a checker that cries wolf gets switched off.
    # OPEN was never cross-checked in prose at all — only in the table below. The file that got
    # this wrong got it wrong on this status.
    #
    # Every bucket, and as a loop rather than a block each, because three of the six were
    # checked and three were not: a breakdown reading "160 ANSWERED, 2 PARKED, 1 BUILT NEVER
    # RULED, 1 OPEN" adds to 164 and sat beside a total of 166 this same script had verified.
    # The unchecked buckets are exactly where it went wrong, which is what an unchecked bucket
    # is for.
    for pair in "ANSWERED:$ANSWERED" "PARKED:$PARKED" "DEFERRED:$DEFERRED" \
                "HALF:$HALF" "BUILT:$BUILT" "OPEN:$OPEN"; do
        word="${pair%%:*}"
        want="${pair##*:}"
        for n in $(grep -oE "[0-9]+ $word" "$f" | grep -oE '^[0-9]+' | sort -u); do
            [ "$n" = "$want" ] \
                || say_bad "$(basename "$f") states $n $word where the register holds $want"
        done
    done

    # A breakdown that leaves a bucket out states no wrong number anywhere — it is a right
    # number missing — so every check above passes it. Both files broke the register down as
    # "160 ANSWERED, 2 PARKED, 1 BUILT NEVER RULED, 1 OPEN", omitted DEFERRED and HALF RULED,
    # and so advertised 164 of 166 while each figure they printed was correct. A breakdown is
    # a claim about the whole register, and this is the half of it that says so.
    SUMS="${TMPDIR:-/tmp}/shall-decision-sums.$$"
    awk -v total="$TOTAL" '
    {
        n = 0; sum = 0; rest = $0
        while (match(rest, /[0-9]+ (ANSWERED|PARKED|DEFERRED|HALF|BUILT|OPEN)/)) {
            sum += substr(rest, RSTART, RLENGTH) + 0
            n++
            rest = substr(rest, RSTART + RLENGTH)
        }
        # Three statuses on one line is a breakdown; one or two is a sentence about a status.
        if (n >= 3 && sum != total)
            printf "line %d breaks the register down as %d, and it holds %d\n", NR, sum, total
    }' "$f" > "$SUMS"
    while IFS= read -r l; do
        [ -n "$l" ] && say_bad "$(basename "$f"): $l"
    done < "$SUMS"
    rm -f "$SUMS"
    # The ban: a count written in lower case is invisible to the three greps above, so it is a
    # failure in itself rather than a style note. Matched only where the number is a count and
    # not the tail of an ID (`D5 answered`) and the word is not starting a phrase
    # (`59 open questions`).
    # Read line by line, not word by word: `for x in $(...)` splits "125 answered," into two
    # tokens and reports each half as its own finding, which is a message that names nothing.
    grep -oE '(^|[^A-Za-z0-9])[0-9]+ (answered|parked|open)[,.)]' "$f" | sed 's/^[^0-9]*//' | sort -u \
    | while IFS= read -r bad_form; do
        echo "  BAD   $(basename "$f") writes the count as \"$bad_form\" — shout the status (ANSWERED/PARKED/OPEN) or this check cannot see it"
    done > "$ROOT/.count-lint.$$"
    if [ -s "$ROOT/.count-lint.$$" ]; then
        cat "$ROOT/.count-lint.$$"
        BAD=$((BAD + $(wc -l < "$ROOT/.count-lint.$$")))
    fi
    rm -f "$ROOT/.count-lint.$$"
done

# The status table's own count column. Prose greps cannot see it — it is a bare number in a
# cell — and it was one of the three figures inside `decisions.md` that disagreed with each
# other. A check that reads only the sentences would have passed this file while it was wrong.
#
# **Matched on the row's opening word, not its whole label.** `BUILT, NEVER RULED` is not
# `BUILT`, so an exact-label match found no cell for it — and `[ -n "$_n" ] || say_bad` would
# have caught that, except the row was never checked at all. See below.
check_row() { # row-label-prefix counted-value
    _n="$(grep -E "^\| \*\*$1" "$REG" | grep -oE '\*\*[0-9]+\*\* *\|' | grep -oE '[0-9]+')"
    [ -n "$_n" ] || { say_bad "decisions.md has no count cell for $1"; return; }
    [ "$_n" = "$2" ] || say_bad "decisions.md's status table says $_n $1 where the register holds $2"
}
# **All six, because three of them were not checked and that is where it went wrong — again.**
# This script says twice, in its own comments, that an unchecked bucket is exactly where a
# breakdown drifts: once about the prose greps ("three of the six were checked and three were
# not") and once about the per-series headings. The status table then repeated it. ANSWERED,
# PARKED and OPEN were checked here; DEFERRED, HALF RULED and BUILT were not, and the table
# read `10` against a register holding 12 for as long as anyone had been counting.
check_row "ANSWERED" "$ANSWERED"
check_row "PARKED" "$PARKED"
check_row "DEFERRED" "$DEFERRED"
check_row "HALF RULED" "$HALF"
check_row "BUILT" "$BUILT"
# OPEN is two rows — `OPEN — blocking` and `OPEN` — and the register counts them as one status.
# `check_row "OPEN"` matched only the second and compared it against the total, so the first
# blocking question in this checker's lifetime (`Q18`, 2026-07-30) made a correct table read as
# wrong. The two rows are summed instead, which is the number the register actually holds.
_open_rows="$(grep -E '^\| \*\*OPEN' "$REG" | grep -oE '\*\*[0-9]+\*\* *\|' | grep -oE '[0-9]+' \
    | awk '{s+=$1} END {print s+0}')"
if [ "$_open_rows" != "$OPEN" ]; then
    say_bad "decisions.md's status table says $_open_rows OPEN (both rows) where the register holds $OPEN"
fi

# The table as a whole, which is the half the per-row checks structurally cannot see.
#
# **A row that is absent states no wrong number.** Every check above compares a cell against a
# count; a status with no row at all has no cell to compare, so it passes all of them silently.
# The table carried five rows summing to 206 while the register held 210 — DEFERRED and HALF
# RULED were simply not in it — and this script printed `ok`. That is the same defect the prose
# breakdown check above was written for, on the table instead of on a sentence, and it is why
# that comment says a breakdown is a claim about the *whole* register.
_table_sum="$(grep -E '^\| \*\*(OPEN|ANSWERED|PARKED|DEFERRED|HALF|BUILT)' "$REG" \
    | grep -oE '\*\*[0-9]+\*\* *\|' | grep -oE '[0-9]+' | awk '{s+=$1} END {print s+0}')"
if [ "$_table_sum" != "$TOTAL" ]; then
    say_bad "decisions.md's status table breaks the register down as $_table_sum, and it holds \
$TOTAL — a status with no row of its own is a bucket nothing above can check"
fi

# --- the index's per-series counts -----------------------------------------------------
#
# `### U — the next round (Part XIII) — 38` is a count of the entries below it, typed by hand,
# and the only figure in this file that nothing read. On 2026-08-06 three of the nine were
# wrong at once — U said 38 against 43, Q said 16 against 47, Y said 8 against 12 — while the
# run above printed `ok`, because every number it checks is about the register's *total* and
# these are about its parts. A total can be right while every part is wrong; that is what a
# total is.
for _h in $(grep -oE '^### [A-Z] .*— [0-9]+$' "$REG" | sed 's/^### \([A-Z]\) .*— \([0-9]*\)$/\1:\2/'); do
    _series="${_h%%:*}"
    _stated="${_h##*:}"
    _actual="$(grep -cE "^## ${_series}[0-9]+[a-z]?$" "$REG")"
    [ "$_stated" = "$_actual" ] \
        || say_bad "decisions.md's ${_series} heading says $_stated where the series holds $_actual"
done

# --- every PARKED entry's condition must still be unmet --------------------------------
#
# **`PARKED` is not a state, it is a promise to come back**: "not asking you yet, and here is what
# I am waiting on". Nothing re-read those conditions when the thing arrived. `D15` said *parked
# until D5 is answered*; D5 was ruled on 2026-07-24 and built on the 26th, and the entry went on
# saying PARKED — filed in the bucket that means "needs nothing from you" — until someone asked
# what was open on the 31st and read D5 by hand.
#
# A checker that verifies the totals and not the conditions is half a checker: the count was
# right the entire week the register was wrong.
LINT="$ROOT/.decision-parked.$$"
awk '
/^## [A-Z][0-9]+[a-z]?$/ { id=$2; order[++n]=id; next }
id != "" && !seen[id] && /\*\*Status:/ { seen[id]=1; status[id]=$0 }
END {
    for (i = 1; i <= n; i++) {
        this = order[i]
        if (status[this] !~ /Status:[ ]*PARKED/) continue
        # A parked entry that does not say what it waits on can never be un-parked by anything
        # but luck, which is the failure this check exists to end.
        if (status[this] !~ /waits on/) {
            printf "%s has no \"waits on\" clause, so nothing can ever tell when it stops being parked\n", this
            continue
        }
        rest = status[this]
        sub(/.*waits on[ ]*/, "", rest)
        gsub(/[^A-Za-z0-9]/, " ", rest)
        sub(/^ +/, "", rest)
        split(rest, w, " ")
        dep = w[1]
        # A condition naming another decision is checkable; one naming an event out in the world
        # ("someone hitting it") is not, and is left alone rather than guessed at.
        if (dep ~ /^[A-Z][0-9]+[a-z]?$/ && status[dep] ~ /Status:[ ]*ANSWERED/)
            printf "%s waits on %s, and %s is ANSWERED — it is not parked any more, it is unasked\n", this, dep, dep
    }
}
' "$REG" > "$LINT"
if [ -s "$LINT" ]; then
    while IFS= read -r l; do say_bad "decisions.md: $l"; done < "$LINT"
fi
rm -f "$LINT"

if [ "$BAD" = 0 ]; then
    echo "  ok    every documented count matches the register, and every parked condition is unmet"
    exit 0
fi
echo " FAILED: $BAD claim(s) in the docs disagree with the register"
exit 1
