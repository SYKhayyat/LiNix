#!/usr/bin/env bash
set -u
export DEBIAN_FRONTEND=noninteractive
export LINIX_CONFIG_DIR=/tmp/kcfg LINIX_DATA_DIR=/tmp/kdata

echo "=== installing dkms ==="
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq dkms >/dev/null 2>&1 || { echo "could not install dkms"; exit 1; }
command -v dkms >/dev/null || { echo "dkms not on PATH"; exit 1; }

# A module that CANNOT build: the source does not compile.
mkdir -p /usr/src/willnotbuild-1.0
cat > /usr/src/willnotbuild-1.0/dkms.conf <<'EOF'
PACKAGE_NAME="willnotbuild"
PACKAGE_VERSION="1.0"
BUILT_MODULE_NAME[0]="willnotbuild"
DEST_MODULE_LOCATION[0]="/kernel/extra"
AUTOINSTALL="yes"
EOF
cat > /usr/src/willnotbuild-1.0/Makefile <<'EOF'
all:
	@echo "this module does not build" && exit 1
EOF
cat > /usr/src/willnotbuild-1.0/willnotbuild.c <<'EOF'
this is not valid C
EOF
dkms add -m willnotbuild -v 1.0 >/dev/null 2>&1

echo "=== dkms status (before) ==="
dkms status

echo
echo "=== a config declaring a kernel package ==="
rm -rf $LINIX_CONFIG_DIR $LINIX_DATA_DIR
mkdir -p $LINIX_CONFIG_DIR/modules $LINIX_CONFIG_DIR/profiles
printf 'apt\n'            > $LINIX_CONFIG_DIR/priority
printf 'use base\n'       > $LINIX_CONFIG_DIR/profiles/Dev
printf 'Dev\n'            > $LINIX_CONFIG_DIR/active
# The change set must actually CONTAIN a kernel package, or the rebuild never triggers. The
# image ships linux-headers-generic already, so a plain declaration is a no-op ("already up to
# date"). Remove it first, outside LiNix, so LiNix plans a real install of a kernel-shaped name.
echo "removing linux-headers-generic so the sync has something to install..."
apt-get remove -y -qq linux-headers-generic >/dev/null 2>&1 || true
printf 'apt:linux-headers-generic\n' > $LINIX_CONFIG_DIR/modules/base.txt

echo "--- does LiNix consider it a kernel package? ---"
linix eval 2>/dev/null | grep -A2 '"name"' | head -6

echo
echo "=== sync (should drive dkms and FAIL on willnotbuild) ==="
linix sync -y 2>&1 | grep -viE "^\[2m2026|INFO linix::core::state" | tail -20
echo "sync exit=${PIPESTATUS[0]}"

echo
echo "=== dkms status (after) ==="
dkms status
