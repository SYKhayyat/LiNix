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

# Before the 18-minute image build: a CRLF shell script is bind-mounted into the container and
# read by dash, which parses `set -u<CR>` and aborts with `set: Illegal option -` before a single
# check runs. The committed blobs are LF and `.gitattributes` pins `*.sh text eol=lf`, so CI never
# sees this — but eol=lf governs what *checkout* writes, not what an editor writes afterwards, and
# on 2026-07-29 four scripts in this working tree had been rewritten to CRLF. The whole local
# container gate was unavailable on the machine this project is developed on, and the failure
# named a shell option rather than a line ending (N-6).
crlf=""
# `od -c` before `grep`, and this is not belt-and-braces. MSYS grep opens a file in text mode
# and normalises CRLF away before it matches, so on the Windows working tree this check ran on
# it could not see the endings it exists to find — the build went ahead and dash failed inside
# the container, which is the outcome the check was written to prevent. `od` turns the byte into
# the two characters \r first.
has_cr() { head -c 65536 "$1" | od -c | grep -q '\\r'; }
probe="${TMPDIR:-/tmp}/linix-crlf-selftest.$$"
printf 'x\r\n' > "$probe"
if ! has_cr "$probe"; then
    rm -f "$probe"
    echo "FATAL: this shell cannot detect a CRLF file, so the check below would pass blindly."
    exit 1
fi
rm -f "$probe"
# The floor file is mounted into the container and parsed there too, so it is in this list even
# though it is data rather than a script.
for f in docker/integration/*.sh scripts/*.sh scripts/lifecycle-floor.txt; do
    [ -f "$f" ] || continue
    if has_cr "$f"; then
        crlf="$crlf $f"
    fi
done
if [ -n "$crlf" ]; then
    echo "FATAL: these files have CRLF line endings in your working tree:"
    for f in $crlf; do echo "    $f"; done
    cat <<'EOM'

These are bind-mounted into the container and read there. dash reads `set -u<CR>` in a script
and aborts with `set: Illegal option -` before any check runs; in the mounted floor file a
trailing CR makes the ratchet compare against a value that is not a number. Git stores them
with LF, so something in this working tree rewrote them. Fix, from the repo root:

    sed -i 's/\r$//' <the files above>          # or: git add --renormalize . && git checkout -- .

Then re-run. Nothing was built, so this costs you nothing but the fix.
EOM
    exit 1
fi

PKG="${1:-jq}"
# `tools` is a broad cross-platform image whose expansion backends (composer, go, opam,
# luarocks, nimble, cabal, stack, mix, helm, krew, asdf, pixi, spack) each get a REAL
# install→list→remove; what it cannot run is plan-smoked and named.
# `gentoo` (emerge) is opt-in — it pulls a large base — via `DISTROS="... gentoo"`; it is always
# run SMOKE-ONLY (a source-building emerge lifecycle costs hours), handled per-distro below, so it
# composes into a full-matrix run without forcing the binary-package distros to smoke too.
# `opensuse` and `void` are in the DEFAULT matrix, not opt-in. `zypper` and `xbps` had been
# registered backends with no real lifecycle in any harness and no stated reason for it — and an
# image that closes that gap only when someone remembers to name it closes nothing. Opt-in is
# how a backend stays untested with a Dockerfile sitting next to it (Q4).
DISTROS="${DISTROS:-ubuntu fedora arch alpine opensuse void storage tools}"

backend_for() {
    case "$1" in
        ubuntu)   echo apt ;;
        fedora)   echo dnf ;;
        arch)     echo pacman ;;
        alpine)   echo apk ;;
        opensuse) echo zypper ;;
        void)     echo xbps ;;
        storage)  echo apt ;;   # Ubuntu base; the point of this image is btrfs/lvm/zfs in 13b
        tools)    echo apt ;;   # Ubuntu base; native lifecycle on apt, plan-smoke for the rest
        gentoo)   echo emerge ;;  # SMOKE-ONLY (baked into the image); no source builds
        *)        echo "" ;;
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
    # The real-lifecycle ratchet's floor (G-11). `.dockerignore` excludes `scripts/` so that
    # editing a host script never busts the image's cargo cache, which means the floor file was
    # in no image and the ratchet was in force on exactly one host class of five — the leg with
    # the least coverage, and absent from the four-distro gate and the `tools` image, which
    # carry the most (N-5). Mounted rather than copied, for the same cache reason and following
    # the pattern CI already uses for `harness-mutation-test.sh`.
    FLOOR_MOUNT="$PWD/scripts/lifecycle-floor.txt:/src/scripts/lifecycle-floor.txt:ro"
    # Forward the run-mode toggle into the container: SMOKE_ONLY=1 skips real mutation
    # (discovery and plan-smoke only), for a source-building image like gentoo. gentoo is
    # ALWAYS smoke-only — a real emerge install→remove builds from source and costs hours — so a
    # single `DISTROS="... gentoo"` run does the right thing per distro: full lifecycle for the
    # binary-package managers, smoke for Portage. A global SMOKE_ONLY still forces every distro.
    # The one image that gets `--privileged`, and it is named rather than inferred from a
    # variable the image could set: a Dockerfile must not be able to ask for privilege.
    # It needs real block devices for btrfs/lvm/zfs, which no other check here does (Q17).
    PRIV=""
    [ "$d" = storage ] && PRIV="--privileged"

    ENVFLAGS=""
    smoke="${SMOKE_ONLY:-}"
    [ "$d" = gentoo ] && smoke=1
    [ -n "$smoke" ] && ENVFLAGS="$ENVFLAGS -e SMOKE_ONLY=$smoke"
    # shellcheck disable=SC2086
    if docker run --rm $PRIV $ENVFLAGS -v "$SCRIPT_MOUNT" -v "$FLOOR_MOUNT" "linix-it-$d" "$be" "$PKG"; then
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
