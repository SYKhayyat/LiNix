#!/usr/bin/env bash
set -u
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2
git config --global --add safe.directory /mnt/c/Users/Administrator/Videos/Nexus/linix 2>/dev/null || true
overall=0
for pair in "arch pacman" "alpine apk"; do
  set -- $pair; d="$1"; be="$2"
  echo "############### $d ($be) ###############"
  docker build -q -f "docker/integration/Dockerfile.$d" -t "linix-it-$d" . >/tmp/build.$d.log 2>&1 || { echo "$d BUILD FAIL"; tail -15 /tmp/build.$d.log; overall=1; continue; }
  if docker run --rm -v "$PWD/docker/integration/run-in-container.sh:/src/docker/integration/run-in-container.sh:ro" "linix-it-$d" "$be" jq 2>&1 | tail -8; then
    echo "$d: exit-ok"
  else
    echo "$d: FAIL"; overall=1
  fi
done
echo "OVERALL=$overall"
exit $overall
