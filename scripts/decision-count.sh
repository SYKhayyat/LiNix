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
    if ($0 ~ /Status:[ ]*ANSWERED/)      answered++
    else if ($0 ~ /Status:[ ]*PARKED/)   parked++
    else if ($0 ~ /Status:[ ]*BUILT/)    built++
    else if ($0 ~ /Status:[ ]*OPEN/)     open++
    else                                 other++
}
END { printf "%d %d %d %d %d %d\n", total, answered+0, parked+0, built+0, open+0, other+0 }
' "$REG")"

set -- $counts
TOTAL=$1; ANSWERED=$2; PARKED=$3; BUILT=$4; OPEN=$5; OTHER=$6

echo "decision register: $TOTAL entries"
echo "  ANSWERED           $ANSWERED"
echo "  PARKED             $PARKED"
echo "  BUILT, NEVER RULED $BUILT"
echo "  OPEN               $OPEN"
[ "$OTHER" = 0 ] || echo "  unrecognised       $OTHER"

[ -n "$CHECK" ] || exit 0

# --- the check: every number written down must equal the number counted ----------------
BAD=0
say_bad() { echo "  BAD   $1"; BAD=$((BAD + 1)); }

# Any figure the docs state about the register's size has to be one of the counted ones.
# Written as "every stated total is $TOTAL" rather than as a regex per sentence, so a new
# sentence stating a wrong total is caught too.
for f in "$REG" "$MAP"; do
    [ -f "$f" ] || continue
    stated="$(grep -oE '[Aa]ll [0-9]+ (decisions|are ruled)|register — all [0-9]+|[0-9]+ decisions\.' "$f" \
        | grep -oE '[0-9]+' | sort -u)"
    for n in $stated; do
        [ "$n" = "$TOTAL" ] || say_bad "$(basename "$f") states $n where the register holds $TOTAL"
    done
    # **A count is written in the status's own spelling — shouted — and that is what makes it
    # checkable.** `SPEC.md` said "125 answered, 2 parked" in lower case and went a week wrong
    # while this run said `ok`, because only the shouted form was scanned. Lower case is not
    # scanned instead, it is *banned* below: `[0-9]+ answered` also matches "D5 answered the
    # ownership half" and "59 open questions", so a case-blind grep reports ordinary prose as a
    # miscount and a checker that cries wolf gets switched off.
    for n in $(grep -oE '[0-9]+ ANSWERED' "$f" | grep -oE '^[0-9]+' | sort -u); do
        [ "$n" = "$ANSWERED" ] || say_bad "$(basename "$f") states $n ANSWERED where the register holds $ANSWERED"
    done
    for n in $(grep -oE '[0-9]+ PARKED' "$f" | grep -oE '^[0-9]+' | sort -u); do
        [ "$n" = "$PARKED" ] || say_bad "$(basename "$f") states $n PARKED where the register holds $PARKED"
    done
    # OPEN was never cross-checked in prose at all — only in the table below. The file that got
    # this wrong got it wrong on this status.
    for n in $(grep -oE '[0-9]+ OPEN' "$f" | grep -oE '^[0-9]+' | sort -u); do
        [ "$n" = "$OPEN" ] || say_bad "$(basename "$f") states $n OPEN where the register holds $OPEN"
    done
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
check_row() { # row-label counted-value
    _n="$(grep -E "^\| \*\*$1\*\*" "$REG" | grep -oE '\*\*[0-9]+\*\* *\|' | grep -oE '[0-9]+')"
    [ -n "$_n" ] || { say_bad "decisions.md has no count cell for $1"; return; }
    [ "$_n" = "$2" ] || say_bad "decisions.md's status table says $_n $1 where the register holds $2"
}
check_row "ANSWERED" "$ANSWERED"
check_row "PARKED" "$PARKED"
# OPEN is two rows — `OPEN — blocking` and `OPEN` — and the register counts them as one status.
# `check_row "OPEN"` matched only the second and compared it against the total, so the first
# blocking question in this checker's lifetime (`Q18`, 2026-07-30) made a correct table read as
# wrong. The two rows are summed instead, which is the number the register actually holds.
_open_rows="$(grep -E '^\| \*\*OPEN' "$REG" | grep -oE '\*\*[0-9]+\*\* *\|' | grep -oE '[0-9]+' \
    | awk '{s+=$1} END {print s+0}')"
if [ "$_open_rows" != "$OPEN" ]; then
    say_bad "decisions.md's status table says $_open_rows OPEN (both rows) where the register holds $OPEN"
fi

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
