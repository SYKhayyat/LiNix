#!/bin/sh
# Measure what a sync actually costs, with the managers wrapped in counting shims.
#
# Y1's instrument, pointed at Y9's question. Three numbers, none of them opinions:
#
#   invocations   how many times each manager binary was executed, and with what argv
#   widest        the most package names LiNix put on one command line
#   overlap       LiNix's own `--timings`: summed child time / wall clock
#
# Run inside the integration image:  sh /src/docker/integration/measure-batching.sh
set -u

SHIM_DIR=/shim
LOG=/tmp/argv.log
export LINIX_CONFIG_DIR=/tmp/linix-config
export LINIX_DATA_DIR=/tmp/linix-data
rm -rf "$SHIM_DIR" "$LOG" "$LINIX_CONFIG_DIR" "$LINIX_DATA_DIR"
mkdir -p "$SHIM_DIR"

# A shim logs its argv and execs the real thing. `command -v` is deliberately not used to find
# the real binary: it answers from the shell's hash table and would find the shim.
for prog in pacman; do
    real=$(ls /usr/bin/$prog 2>/dev/null || true)
    [ -n "$real" ] || continue
    cat > "$SHIM_DIR/$prog" <<SHIM
#!/bin/sh
printf '%s' "$prog" >> $LOG
for a in "\$@"; do printf ' %s' "\$a" >> $LOG; done
printf '\n' >> $LOG
exec $real "\$@"
SHIM
    chmod 0755 "$SHIM_DIR/$prog"
done
export PATH="$SHIM_DIR:$PATH"

linix init >/dev/null 2>&1
mkdir -p "$LINIX_CONFIG_DIR/modules"

# Six pacman packages. Two pairs among them are related in pacman's own dependency graph —
# `jq` needs `oniguruma`, `wget` needs `libpsl` — which is the case that used to wire an edge
# and split the wave. Nobody wrote `@requires`, so nothing here may split it.
cat > "$LINIX_CONFIG_DIR/modules/bench.txt" <<'MODULE'
pacman:jq
pacman:oniguruma
pacman:wget
pacman:libpsl
pacman:tree
pacman:bc
MODULE
grep -q 'use bench' "$LINIX_CONFIG_DIR/profiles/Main" 2>/dev/null || echo 'use bench' >> "$LINIX_CONFIG_DIR/profiles/Main"

# Make sure none of them is already installed, or the plan is empty and measures nothing.
pacman -Rns --noconfirm jq oniguruma wget libpsl tree bc >/dev/null 2>&1 || true
: > "$LOG"

echo "=== linix --timings sync"
START=$(date +%s%N)
linix --timings sync --yes 2>&1 | tail -40
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
