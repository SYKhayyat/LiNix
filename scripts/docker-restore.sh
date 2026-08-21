#!/usr/bin/env bash
# Put this machine's Docker back after it has been emptied.
#
#   ./scripts/docker-restore.sh            # check the daemon, rebuild every integration image
#   ./scripts/docker-restore.sh --check    # say what is missing and change nothing
#   DISTROS="ubuntu tools" ./scripts/docker-restore.sh    # a subset
#
# **Why this exists.** CI is the gate and it is comprehensive - every push builds and tests on
# three platforms and runs seven integration images; the nightly adds tools, gentoo, guix,
# slackware, macOS, Windows and thirteen mutation shards. What CI cannot do is answer WHY. When
# a nightly goes red, the container is how you find out: it settled the 2026-08-21 cabal root
# cause in four minutes when reasoning could not, and it proved a one-line harness fix in ninety
# seconds against a forty-minute nightly on a broken `main`.
#
# So the images are cache, not capability: they cost ~135 GB and rebuild from these Dockerfiles
# on demand, and this script is the way back. **Cache, not free** - see the timing below before
# emptying anything. Reclaiming space is usually not a choice between keeping the images and
# losing them: `docker builder prune` took 93 GB of build cache down to its 7 GB working set on
# this machine without touching a single image, which is most of what "Docker is full" means.
#
# **It does not contain a build.** `docker/integration/run.sh BUILD_ONLY=1` is the one place that
# knows `-f Dockerfile.$d -t shall-it-$d .`, and a second copy here would be exactly the
# two-of-everything this repository keeps paying for.
set -u

CHECK_ONLY=""
for a in "$@"; do [ "$a" = "--check" ] && CHECK_ONLY=1; done

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || (cd "$(dirname "$0")/.." && pwd))"
cd "$REPO_ROOT" || { echo "cannot cd to repo root ($REPO_ROOT)"; exit 1; }

say() { printf '%s\n' "$*"; }
problems=0

# ---- 0. Run where Docker actually is --------------------------------------------------------
#
# On this machine Docker lives INSIDE WSL and there is no client on the Windows PATH, so a
# `docker` invoked from git-bash is not a stopped daemon - it is nothing at all, and reporting
# that as a daemon problem sends the reader to fix the wrong thing. `unix-check.sh` already
# falls back to `wsl -- docker`; this goes one further and re-execs the whole script inside the
# distro, because `run.sh` below calls `docker build` directly and would otherwise need the
# same fallback in a second place.
if ! command -v docker >/dev/null 2>&1; then
    if command -v wsl >/dev/null 2>&1 && wsl -- command -v docker >/dev/null 2>&1; then
        # `pwd -W` first: git-bash reports `/c/Users/...`, which `wslpath` reads as a relative
        # unix path and prefixes with /mnt/c, producing `/mnt/c/c/Users/...`. `-W` gives the
        # Windows form wslpath is expecting. Measured - the first version of this line failed
        # exactly that way.
        win="$(pwd -W 2>/dev/null || pwd)"
        here="$(wsl -- wslpath -a "$win" 2>/dev/null | tr -d "\r")"
        if [ -n "$here" ]; then
            say "no docker on this PATH; re-running inside WSL, where it lives"
            exec wsl -- sh -c "cd '$here' && ./scripts/docker-restore.sh $*"
        fi
    fi
fi

# ---- 1. Is there a daemon at all? -----------------------------------------------------------
#
# On the machine this was written for, Docker lives INSIDE WSL - `/usr/bin/docker`, no Docker
# Desktop - so `docker` is absent from the Windows PATH by design and its absence there says
# nothing. This asks the daemon rather than the PATH, because a client with no daemon behind it
# answers every question with a connection error that reads like a missing feature.
if ! docker info >/dev/null 2>&1; then
    problems=$((problems + 1))
    say "NO DAEMON: \`docker info\` did not answer."
    say ""
    say "  If Docker is installed inside WSL (it was here), the daemon does not start itself:"
    say "      wsl -d Ubuntu -- sudo service docker start"
    say "  If the distro itself is gone:"
    say "      wsl --install -d Ubuntu"
    say "      wsl -d Ubuntu -- sh -c 'curl -fsSL https://get.docker.com | sudo sh'"
    say "      wsl -d Ubuntu -- sudo usermod -aG docker \$USER     # then restart the distro"
    say ""
    say "Nothing below can run until that answers."
    exit 1
fi
say "daemon: up ($(docker info --format '{{.ServerVersion}}' 2>/dev/null || echo "version unknown"))"

# ---- 2. What is missing? --------------------------------------------------------------------
#
# Named from the Dockerfiles rather than from a list here, for the reason the build lives in one
# place: a Dockerfile added next year is an image this script should already know about.
WANT="${DISTROS:-$(ls docker/integration/Dockerfile.* 2>/dev/null | sed 's/.*Dockerfile\.//' | tr '\n' ' ')}"
missing=""
for d in $WANT; do
    if docker image inspect "shall-it-$d" >/dev/null 2>&1; then
        say "  have  shall-it-$d"
    else
        say "  MISS  shall-it-$d"
        missing="$missing $d"
    fi
done

if [ -z "${missing# }" ]; then
    say ""
    say "every image this repo builds is present; nothing to do."
    exit 0
fi

if [ -n "$CHECK_ONLY" ]; then
    say ""
    say "missing:${missing}"
    say "run without --check to build them."
    exit 1
fi

# ---- 3. Build, through the one builder ------------------------------------------------------
#
# Hours, not minutes, and NOT because of `tools`. **Every image compiles `shall` in release
# inside itself** - that is what makes the harness test the tree rather than a binary copied in -
# and that compile alone was measured at 32m 14s for alpine, the smallest image here. So the
# floor is ~35 minutes per image whatever the distro, and `tools` adds its own OCaml, Haskell,
# Nim and HPC builds on top of that. Eleven images is most of a working day.
#
# This paragraph used to say "`tools` is 25-40 minutes on its own", which reads as though the
# others are quick. They are not. Measured on 2026-08-21 by deleting alpine and running this
# script to put it back.
say ""
say "building:${missing}"
say "this is slow: ~35 minutes MINIMUM per image - every one of them compiles shall in"
say "release inside itself - and \`tools\` is longer again. Nothing here needs supervising."
DISTROS="${missing# }" BUILD_ONLY=1 ./docker/integration/run.sh
rc=$?
say ""
if [ "$rc" -eq 0 ]; then
    say "done. \`./scripts/docker-restore.sh --check\` will confirm."
else
    say "at least one image did not build; the log above names which."
fi
exit "$rc"
