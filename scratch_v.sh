#!/usr/bin/env bash
set -u
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2
git config --global --add safe.directory /mnt/c/Users/Administrator/Videos/Nexus/linix 2>/dev/null || true
for d in ubuntu alpine; do
  be=apt; [ "$d" = alpine ] && be=apk
  echo "### $d"
  docker build -q -f docker/integration/Dockerfile.$d -t linix-it-$d . >/tmp/b.$d.log 2>&1 || { echo "BUILD FAIL"; tail -12 /tmp/b.$d.log; continue; }
  docker run --rm -v "$PWD/docker/integration/run-in-container.sh:/src/docker/integration/run-in-container.sh:ro" \
    linix-it-$d $be jq 2>&1 | tail -6
done
