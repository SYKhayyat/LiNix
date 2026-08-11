#!/bin/sh
# Measure what a sync actually costs, with the managers wrapped in counting shims.
#
# Y1's instrument, pointed at Y9's question. Three numbers, none of them opinions:
#
#   invocations   how many times each manager binary was executed, and with what argv
#   widest        the most package names Shall put on one command line
#   overlap       Shall's own `--timings`: summed child time / wall clock
#
# Run inside the integration image:  sh /src/docker/integration/measure-batching.sh
set -u

SHIM_DIR=/shim
LOG=/tmp/argv.log
export SHALL_CONFIG_DIR=/tmp/shall-config
export SHALL_DATA_DIR=/tmp/shall-data
rm -rf "$SHIM_DIR" "$LOG" "$SHALL_CONFIG_DIR" "$SHALL_DATA_DIR"
mkdir -p "$SHIM_DIR"

# A shim logs its argv and execs the real thing. `command -v` is deliberately not used to find
# the real binary: it answers from the shell's hash table and would find the shim.
#
# One manager, named rather than looped over. This was `for prog in pacman`, a loop with one
# element — the shape of a list that used to have more, and a promise of generality the rest of
# the file does not keep: the module below is six *pacman* packages and the assertions read
# pacman's argv. A loop that can only run once says otherwise to whoever edits it next.
PROG=pacman
REAL=/usr/bin/$PROG
if [ -x "$REAL" ]; then
    cat > "$SHIM_DIR/$PROG" <<SHIM
#!/bin/sh
printf '%s' "$PROG" >> $LOG
for a in "\$@"; do printf ' %s' "\$a" >> $LOG; done
printf '\n' >> $LOG
exec $REAL "\$@"
SHIM
    chmod 0755 "$SHIM_DIR/$PROG"
else
    # Loudly, and not `continue`: this script's every number comes from that log, and with no
    # shim it would measure nothing and report zeroes as a result.
    echo "FATAL: $REAL is not here — this measurement only runs in the arch image." >&2
    exit 1
fi
export PATH="$SHIM_DIR:$PATH"

shall init >/dev/null 2>&1
mkdir -p "$SHALL_CONFIG_DIR/modules"

# Six pacman packages. Two pairs among them are related in pacman's own dependency graph —
# `jq` needs `oniguruma`, `wget` needs `libpsl` — which is the case that used to wire an edge
# and split the wave. Nobody wrote `@requires`, so nothing here may split it.
cat > "$SHALL_CONFIG_DIR/modules/bench.txt" <<'MODULE'
pacman:jq
pacman:oniguruma
pacman:wget
pacman:libpsl
pacman:tree
pacman:bc
MODULE
grep -q 'use bench' "$SHALL_CONFIG_DIR/profiles/Main" 2>/dev/null || echo 'use bench' >> "$SHALL_CONFIG_DIR/profiles/Main"

# Make sure none of them is already installed, or the plan is empty and measures nothing.
pacman -Rns --noconfirm jq oniguruma wget libpsl tree bc >/dev/null 2>&1 || true
: > "$LOG"

echo "=== shall --timings sync"
START=$(date +%s%N)
shall --timings sync --yes 2>&1 | tail -40
END=$(date +%s%N)
echo
echo "=== wall clock: $(( (END - START) / 1000000 )) ms"

echo
echo "=== every pacman invocation, verbatim"
cat "$LOG"

echo
echo "=== counts"
printf 'pacman invocations total : %s\n' "$(wc -l < "$LOG")"
printf 'dependency queries (-Si) : %s\n' "$(grep -c -- ' -Si ' "$LOG" || true)"
printf 'install commands (-S)    : %s\n' "$(grep -cE 'pacman -S( |$)' "$LOG" || true)"
printf 'widest install command   : %s names\n' \
    "$(grep -E 'pacman -S( |$)' "$LOG" | awk '{n=0; for(i=2;i<=NF;i++) if ($i !~ /^-/) n++; if (n>m) m=n} END {print m+0}')"
