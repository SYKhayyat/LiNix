#!/bin/sh
# Compile the tree for Unix, in a container, because this repository's verify chain cannot.
#
# **What this exists to catch.** On 2026-08-12 `d1b3618` named a private associated const across
# a module boundary. Both lines are `#[cfg(unix)]`, so a Windows `cargo build` never saw either
# one and the local chain was clean. Every Apple, Linux and MSRV job in CI went red, and so did
# all seven distro integration jobs — and because the container harness builds its binary
# in-image, a tree that does not compile on Linux takes every fault-injection check offline with
# it. The same commit therefore introduced a blocker AND disabled the only instrument that could
# report it. Neither was noticed for 26 commits.
#
# 45 `cfg`-gated blocks across 17 source files are invisible to a Windows-only chain. The cheap
# cross-check is not available: `cargo check --target x86_64-unknown-linux-gnu` from a Windows
# host dies in `mlua`'s vendored C build for want of `x86_64-linux-gnu-gcc`. So a container is
# the instrument, and this is it.
#
#   ./scripts/unix-check.sh              - cargo check --all-targets for Linux
#   ./scripts/unix-check.sh --lib        - the lib only, which is faster and catches most of it
#
# Reads the exit code directly and never through a pipe: `rc=$?` after a pipe reads the pipe,
# which cost the 2026-08-13 review three false results in one session.
set -u

IMAGE="${SHALL_UNIX_CHECK_IMAGE:-rust:1-slim}"
ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
WHAT="--all-targets"
for a in "$@"; do [ "$a" = "--lib" ] && WHAT="--lib"; done

DOCKER="docker"
command -v docker >/dev/null 2>&1 || DOCKER="wsl -- docker"

# **Git Bash rewrites arguments that look like paths, and it rewrites them into nonsense.**
# `-v /c/repo:/src:ro` reaches docker as `-v /c/repo:C:\Program Files\Git\src;ro`, which the
# daemon rejects with `invalid mode` — and the failure arrives as exit 125, one line before a
# message that calls it a compile error on every Unix. So the check that exists because the
# Windows chain cannot see Linux could not run on Windows at all. Both halves are fixed here:
# the conversion is switched off, and 125 is told apart from a real failure below.
MSYS_NO_PATHCONV=1
MSYS2_ARG_CONV_EXCL='*'
export MSYS_NO_PATHCONV MSYS2_ARG_CONV_EXCL

# A path WSL's docker can mount. `wsl -- docker` is the daemon inside WSL, so it reads
# `/mnt/c/...`; Git Bash hands out `/c/...` and cmd hands out `C:\...`, and neither is a path
# that daemon has ever heard of. A native Linux host falls through all of this unchanged.
case "$DOCKER" in
wsl*)
    case "$ROOT" in
    /mnt/*) ;;
    /[A-Za-z]/*) ROOT="/mnt$ROOT" ;;
    [A-Za-z]:*)
        _drive="$(printf '%s' "$ROOT" | cut -c1 | tr 'A-Z' 'a-z')"
        ROOT="/mnt/$_drive$(printf '%s' "$ROOT" | cut -c3- | tr '\\' '/')"
        ;;
    esac
    ;;
esac

$DOCKER version >/dev/null 2>&1 || {
    echo "unix-check: no reachable docker daemon." >&2
    echo "  This check is the only thing in the chain that compiles the 45 cfg(unix) blocks." >&2
    echo "  Skipping it is a said-so, not a done — say so if you push without it." >&2
    exit 2
}

echo "unix-check: cargo check $WHAT in $IMAGE"

# The source is copied in rather than bind-mounted read-write: the host's `target/` is a
# different platform's artifacts, and letting a Linux cargo touch it invalidates the local build
# for no reason. `CARGO_TARGET_DIR` and the registry live in named volumes so the second run is
# not a cold build.
#
# **`target/` is excluded from the copy rather than deleted after it.** `cp -a /src/.` followed
# by `rm -rf target` moves a 16 GB build directory across a bind mount in order to throw it
# away — eleven minutes, on this tree, before a single crate compiles. The same mistake, made
# the same afternoon, cost a container run in this session.
$DOCKER run --rm \
    -v "$ROOT":/src:ro \
    -v shall-unix-check-target:/target \
    -v shall-unix-check-registry:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/target \
    -e WHAT="$WHAT" \
    "$IMAGE" \
    sh -c '
        set -e
        apt-get update -qq >/dev/null 2>&1
        apt-get install -y -qq build-essential pkg-config libssl-dev >/dev/null 2>&1
        mkdir -p /work
        (cd /src && tar -cf - --exclude=./target --exclude=./mutants.out \
            --exclude=./mutants.out.old --exclude=./.git .) | (cd /work && tar -xf -)
        cd /work
        cargo check $WHAT --locked
    '
RC=$?

# **125 is the docker CLI refusing to start a container, not cargo refusing to compile.** They
# were reported as the same thing, and the message named the wrong culprit at maximum volume —
# an instrument that says "this is a compile error on every Unix" about its own broken
# invocation is worse than no instrument, because it sends the reader to look at source that is
# fine. A check that cannot tell its own failure from its subject's is this repository's whole
# subject.
if [ "$RC" -eq 0 ]; then
    echo "unix-check: ok"
elif [ "$RC" -eq 125 ]; then
    echo "unix-check: DID NOT RUN (docker exit 125 — the container never started)." >&2
    echo "  This says nothing about whether the tree compiles on Unix. It is a bad invocation:" >&2
    echo "  an unmountable path, a missing image, a daemon that refused. Fix the invocation and" >&2
    echo "  run it again — and do not read this as a green check." >&2
else
    echo "unix-check: FAILED (exit $RC)" >&2
    echo "  This is a compile error on every Unix. It is invisible to the Windows chain," >&2
    echo "  and it takes the container harness — and with it every fault-injection check —" >&2
    echo "  offline until it is fixed." >&2
fi
exit "$RC"
