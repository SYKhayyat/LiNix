#!/usr/bin/env bash
# 7g: does a sync that changes a kernel package drive DKMS, and does a module that will not
# build fail the sync loudly?
#
# Runs in a throwaway ubuntu container with real dkms installed. A container cannot load a
# kernel module, which is fine — the claim under test is "LiNix notices the kernel changed,
# calls dkms, reads what it says, and fails loudly on a module that did not make it", none of
# which needs a loadable module.
set -u
cd /mnt/c/Users/Administrator/Videos/Nexus/linix || exit 2

# Rebuilt from CURRENT source: the image must carry the binary being tested, or the run is
# about a different program (this bit me once already today).
echo "building linix-it-ubuntu from current source..."
docker build -q -f docker/integration/Dockerfile.ubuntu -t linix-it-ubuntu . >/tmp/dkms-build.log 2>&1   || { echo "BUILD FAIL"; tail -15 /tmp/dkms-build.log; exit 1; }

# --entrypoint overrides the image's run-in-container.sh entrypoint, which would otherwise
# swallow our script as arguments.
docker run --rm --entrypoint bash -v "$PWD/scratch_dkms_inner.sh:/inner.sh:ro" linix-it-ubuntu /inner.sh
