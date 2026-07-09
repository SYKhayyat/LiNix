#!/usr/bin/env bash
# Build + run the LiNix integration harness across Linux distros in disposable containers,
# testing each distro's native package manager through the real `linix` binary.
#
#   ./docker/integration/run.sh                # all distros, package "jq"
#   ./docker/integration/run.sh htop           # override the test package
#   DISTROS="ubuntu arch" ./docker/integration/run.sh   # subset
#
# NOTE: the default canary is `jq`, not `tree` — busybox (Alpine) ships a `tree` applet,
# so removing the tree package still leaves /usr/bin/tree on PATH. See run-in-container.sh.
#
# Requires Docker. Run from anywhere in the repo.
set -u
# Resolve the repo root: prefer git; fall back to two levels up from this script
# (docker/integration/ -> repo root). Robust to `/mnt/c` "dubious ownership" git errors.
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || (cd "$(dirname "$0")/../.." && pwd))"
cd "$REPO_ROOT" || { echo "cannot cd to repo root ($REPO_ROOT)"; exit 1; }
echo "repo root: $REPO_ROOT"

PKG="${1:-jq}"
DISTROS="${DISTROS:-ubuntu fedora arch alpine}"

backend_for() {
    case "$1" in
        ubuntu) echo apt ;;
        fedora) echo dnf ;;
        arch)   echo pacman ;;
        alpine) echo apk ;;
        *)      echo "" ;;
    esac
}

summary=""
overall=0
for d in $DISTROS; do
    be="$(backend_for "$d")"
    [ -n "$be" ] || { echo "unknown distro: $d"; continue; }
    echo "############### BUILD $d ($be) ###############"
    if ! docker build -f "docker/integration/Dockerfile.$d" -t "linix-it-$d" . ; then
        summary="${summary}\n  ${d} (${be}): BUILD-FAIL"; overall=1; continue
    fi
    echo "############### RUN $d ($be) ###############"
    # Mount the current test script so edits to it don't require an image rebuild.
    SCRIPT_MOUNT="$PWD/docker/integration/run-in-container.sh:/src/docker/integration/run-in-container.sh:ro"
    if docker run --rm -v "$SCRIPT_MOUNT" "linix-it-$d" "$be" "$PKG"; then
        summary="${summary}\n  ${d} (${be}): PASS"
    else
        summary="${summary}\n  ${d} (${be}): FAIL"; overall=1
    fi
done

echo ""
echo "===================== SUMMARY ====================="
printf "%b\n" "$summary"
echo "==================================================="
exit $overall
