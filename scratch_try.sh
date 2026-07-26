#!/usr/bin/env bash
# 7h's first exit condition, for real: a config with a deliberate error is rejected by `try`
# on a clean container, and a good one is accepted — with nothing touched on the host.
#
# The image must be built from CURRENT source, or the binary it carries predates the command
# being tested and every result is about a different program.
set -u
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2
git config --global --add safe.directory /mnt/c/Users/Administrator/Videos/Nexus/linix 2>/dev/null || true

echo "building linix-it-ubuntu from current source..."
docker build -q -f docker/integration/Dockerfile.ubuntu -t linix-it-ubuntu . >/tmp/try-build.log 2>&1 \
  || { echo "BUILD FAIL"; tail -20 /tmp/try-build.log; exit 1; }

LX=/tmp/linix-try-bin/linix
rm -rf /tmp/linix-try-bin; mkdir -p /tmp/linix-try-bin
docker create --name linix-try-extract linix-it-ubuntu >/dev/null 2>&1
for p in /usr/local/bin/linix /usr/bin/linix /linix; do
  docker cp "linix-try-extract:$p" "$LX" >/dev/null 2>&1 && break
done
docker rm linix-try-extract >/dev/null 2>&1
[ -x "$LX" ] || { echo "could not extract the linix binary from the image"; exit 1; }

"$LX" try --help >/dev/null 2>&1 || { echo "FATAL: the image's binary has no \`try\` — it is stale"; exit 1; }

mk() { # mk <dir> <module-line>
  rm -rf "$1"; mkdir -p "$1/modules" "$1/profiles"
  printf 'apt\n'      > "$1/priority"
  printf 'use base\n' > "$1/profiles/Dev"
  printf 'Dev\n'      > "$1/active"
  printf '%s\n' "$2"  > "$1/modules/base.txt"
}

echo "=============================================="
GOOD=/tmp/try-good
mk "$GOOD" 'apt:jq'
before=$(find "$GOOD" -type f | sort | xargs md5sum 2>/dev/null | md5sum)
LINIX_CONFIG_DIR=$GOOD LINIX_DATA_DIR=/tmp/try-good-data "$LX" try >/tmp/try-good.log 2>&1
echo "GOOD config   -> exit $?"
tail -4 /tmp/try-good.log | sed 's/^/    | /'
after=$(find "$GOOD" -type f | sort | xargs md5sum 2>/dev/null | md5sum)
[ "$before" = "$after" ] && echo "    host config UNCHANGED" || echo "    !! HOST CONFIG WAS MODIFIED"
[ -d /tmp/try-good-data ] && echo "    !! a data dir was created on the host" || echo "    no host data dir"

echo "=============================================="
BAD=/tmp/try-bad
mk "$BAD" 'apt:jq@nonsense=1'
LINIX_CONFIG_DIR=$BAD LINIX_DATA_DIR=/tmp/try-bad-data "$LX" try >/tmp/try-bad.log 2>&1
echo "BROKEN config -> exit $?"
tail -6 /tmp/try-bad.log | sed 's/^/    | /'

echo "=============================================="
echo "missing image:"
LINIX_CONFIG_DIR=$GOOD "$LX" try --image linix-no-such-image >/tmp/try-img.log 2>&1
echo "  -> exit $?"
tail -6 /tmp/try-img.log | sed 's/^/    | /'
echo "=============================================="
