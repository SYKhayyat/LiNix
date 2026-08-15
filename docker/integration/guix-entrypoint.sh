#!/bin/sh
# Start `guix-daemon`, then hand over to the harness.
#
# **Guix is the one manager in this matrix that will not answer without a daemon**, and the
# daemon is not something the image can start for itself: there is no init here, so PID 1 is
# whatever the container was told to run. That is this file.
#
# Two facts the exemption in `proving.rs` got wrong, both measured on 2026-08-14:
#
#   - `guix-daemon` is NOT on `PATH`. `/usr/local/bin/guix` is a wrapper; the daemon lives in
#     root's current-guix profile, and a plain `guix-daemon &` reports `not found` — into a log
#     nobody reads, after which every `guix` call fails with "failed to connect to
#     /var/guix/daemon-socket/socket" and looks like a daemon that refused rather than one that
#     was never started.
#   - The socket appears about a second after the daemon does, so the harness has to wait for it
#     rather than for the process. Waiting on the process is how a race gets written down as a
#     flaky manager.
#
# `--disable-chroot` because the build users cannot set up a chroot inside a container. The
# container itself is the isolation, which is the same trade `run.sh` already makes for the
# storage image.
set -u

DAEMON=/var/guix/profiles/per-user/root/current-guix/bin/guix-daemon
[ -x "$DAEMON" ] || DAEMON="$(find /gnu/store -maxdepth 3 -name guix-daemon -type f 2>/dev/null | head -1)"
if [ -z "${DAEMON:-}" ] || [ ! -x "$DAEMON" ]; then
    echo "FATAL: no guix-daemon in this image; every guix call would fail as 'cannot connect'." >&2
    exit 1
fi

"$DAEMON" --disable-chroot --build-users-group=guixbuild >/tmp/guix-daemon.log 2>&1 &

i=0
while [ "$i" -lt 30 ] && [ ! -S /var/guix/daemon-socket/socket ]; do
    i=$((i + 1))
    sleep 1
done
if [ ! -S /var/guix/daemon-socket/socket ]; then
    echo "FATAL: guix-daemon started and no socket appeared in ${i}s. Its log:" >&2
    cat /tmp/guix-daemon.log >&2
    exit 1
fi
echo "guix-daemon up, socket after ${i}s"

# **The profile goes on PATH, because on a guix machine a person puts it there once.** `guix
# install hello` writes `/root/.guix-profile/bin/hello` — measured, and it runs — and guix then
# prints *"see `guix package --search-paths'"* rather than touching your environment. So without
# this the harness installed the canary, found nothing on PATH, and reported a manager that had
# in fact done its job: `guix: hello is not on PATH and nothing said where it went`, with seven
# further checks cascading off it.
#
# Prepended rather than sourced from `$GUIX_PROFILE/etc/profile`, which is what the guix
# documentation tells a user to do: that file is created by the FIRST install, so at entrypoint
# time on a fresh container there is nothing to source. A PATH entry that does not exist yet
# costs nothing and is there when the directory appears.
PATH="/root/.guix-profile/bin:/root/.guix-profile/sbin:$PATH"
export PATH

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
