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
for f in docker/integration/run-in-container.sh docker/integration/*.sh scripts/*.sh; do
    [ -f "$f" ] || continue
    # `grep -c` on a binary-ish match: any CR before a newline is the fault.
    if head -c 65536 "$f" | grep -q $'\r'; then
        crlf="$crlf $f"
    fi
done
if [ -n "$crlf" ]; then
    echo "FATAL: these shell scripts have CRLF line endings in your working tree:"
    for f in $crlf; do echo "    $f"; done
    cat <<'EOM'

dash inside the container reads `set -u<CR>` and aborts before running any check, reporting
`set: Illegal option -`. Git stores these files with LF; something in this working tree rewrote
them. Fix, from the repo root:

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
DISTROS="${DISTROS:-ubuntu fedora arch alpine tools}"

backend_for() {
    case "$1" in
        ubuntu) echo apt ;;
        fedora) echo dnf ;;
        arch)   echo pacman ;;
        alpine) echo apk ;;
        tools)  echo apt ;;     # Ubuntu base; native lifecycle on apt, plan-smoke for the rest
        gentoo) echo emerge ;;  # SMOKE-ONLY (baked into the image); no source builds
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
    ENVFLAGS=""
    smoke="${SMOKE_ONLY:-}"
    [ "$d" = gentoo ] && smoke=1
    [ -n "$smoke" ] && ENVFLAGS="$ENVFLAGS -e SMOKE_ONLY=$smoke"
    # shellcheck disable=SC2086
    if docker run --rm $ENVFLAGS -v "$SCRIPT_MOUNT" -v "$FLOOR_MOUNT" "linix-it-$d" "$be" "$PKG"; then
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
