#!/usr/bin/env bash
set -u
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2
git config --global --add safe.directory /mnt/c/Users/Administrator/Videos/Nexus/linix 2>/dev/null || true
echo "building..."
docker build -q -f docker/integration/Dockerfile.ubuntu -t linix-it-ubuntu . > /tmp/b.log 2>&1
rc=$?
echo "build rc=$rc"
[ $rc -eq 0 ] || { tail -20 /tmp/b.log; exit 1; }
docker run --rm -v "$PWD/docker/integration/run-in-container.sh:/src/docker/integration/run-in-container.sh:ro" \
  linix-it-ubuntu apt jq 2>&1 | tail -18
