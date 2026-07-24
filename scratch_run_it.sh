#!/usr/bin/env bash
set -u
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2
git config --global --add safe.directory /mnt/c/Users/Administrator/Videos/Nexus/linix 2>/dev/null || true
DISTROS="${DISTROS:-fedora arch alpine}" ./docker/integration/run.sh jq
