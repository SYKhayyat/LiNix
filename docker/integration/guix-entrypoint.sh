#!/bin/sh
# Bring the guix daemon up, install the harness's fixtures, then hand over to the harness.
#
# The daemon start lives in `guix-daemon-up.sh`, because the terminator probe step replaces this
# entrypoint with `sh` and needs the daemon just as much — see that file's header. What stays
# here is what only the lifecycle harness needs.
set -u

sh /usr/local/bin/guix-daemon-up || exit 1

# **The fixtures, installed here because the image could not install them.** A guix profile
# starts empty, and two of the harness's sections need something in it that has nothing to do
# with guix itself: `git` for the history and rollback proofs, and a package from Shall's
# protected list — `bash` — for the removal guard to have a victim it must refuse to delete.
# Every other image in this matrix gets both from its base.
#
# It cannot be a build layer: guix builds in a sandbox that calls `personality(2)`, Docker's
# default seccomp blocks it, and `docker build` takes no `--security-opt`. See the Dockerfile.
#
# Failure is reported and not fatal. These are fixtures for other people's checks; a guix that
# cannot fetch them is worth knowing about, and it is not a reason to run none of the sweep.
if ! guix install git bash >/tmp/guix-fixtures.log 2>&1; then
    echo "WARNING: could not install the git/bash fixtures; the history and guard proofs will" >&2
    echo "         report themselves unrunnable. Last lines:" >&2
    tail -5 /tmp/guix-fixtures.log >&2
fi
hash -r 2>/dev/null || true

exec sh /src/docker/integration/run-in-container.sh "$@"
