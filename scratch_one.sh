#!/usr/bin/env bash
set -u
d="$1"
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2
git config --global --add safe.directory /mnt/c/Users/Administrator/Videos/Nexus/linix 2>/dev/null || true
case "$d" in
  fedora) be=dnf ;; arch) be=pacman ;; alpine) be=apk ;; ubuntu) be=apt ;; *) echo "unknown $d"; exit 2 ;;
esac
echo "### building $d ($be)"
docker build -q -f "docker/integration/Dockerfile.$d" -t "linix-it-$d" . >/tmp/build.$d.log 2>&1 || { echo "BUILD FAIL"; tail -20 /tmp/build.$d.log; exit 1; }
echo "### running $d ($be)"
docker run --rm \
  -v "$PWD/docker/integration/run-in-container.sh:/src/docker/integration/run-in-container.sh:ro" \
  "linix-it-$d" "$be" jq 2>&1 | tail -25
