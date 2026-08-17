#!/bin/sh
# Start `guix-daemon` and wait for its socket. Nothing else.
#
# **Guix is the one manager in this matrix that will not answer without a daemon**, and the
# daemon is not something the image can start for itself: there is no init here, so PID 1 is
# whatever the container was told to run.
#
# **Split out of `guix-entrypoint.sh` because two steps need it and only one runs the
# entrypoint.** The terminator probe step passes `--entrypoint sh`, which replaces the entrypoint
# outright — so on the leg added for guix, every guix verb died on *"failed to connect to
# /var/guix/daemon-socket/socket"* before it could parse an operand, and the probe reported
# guix's rows honestly unchecked while measuring `apt`. A leg that exists to ask guix questions
# has to be able to reach guix.
#
# Idempotent: a socket that is already there is the success case, not a second daemon.
#
# Two facts the exemption in `proving.rs` got wrong, both measured on 2026-08-14:
#
#   - `guix-daemon` is NOT on `PATH`. `/usr/local/bin/guix` is a wrapper; the daemon lives in
#     root's current-guix profile, and a plain `guix-daemon &` reports `not found` — into a log
#     nobody reads, after which every `guix` call fails as above and looks like a daemon that
#     refused rather than one that was never started.
#   - The socket appears about a second after the daemon does, so the caller has to wait for it
#     rather than for the process. Waiting on the process is how a race gets written down as a
#     flaky manager.
#
# `--disable-chroot` because the build users cannot set up a chroot inside a container. The
# container itself is the isolation, which is the same trade `run.sh` already makes for the
# storage image.
set -u

SOCKET=/var/guix/daemon-socket/socket

if [ -S "$SOCKET" ]; then
    echo "guix-daemon already up"
    exit 0
fi

DAEMON=/var/guix/profiles/per-user/root/current-guix/bin/guix-daemon
[ -x "$DAEMON" ] || DAEMON="$(find /gnu/store -maxdepth 3 -name guix-daemon -type f 2>/dev/null | head -1)"
if [ -z "${DAEMON:-}" ] || [ ! -x "$DAEMON" ]; then
    echo "FATAL: no guix-daemon in this image; every guix call would fail as 'cannot connect'." >&2
    exit 1
fi

"$DAEMON" --disable-chroot --build-users-group=guixbuild >/tmp/guix-daemon.log 2>&1 &

i=0
while [ "$i" -lt 30 ] && [ ! -S "$SOCKET" ]; do
    i=$((i + 1))
    sleep 1
done
if [ ! -S "$SOCKET" ]; then
    echo "FATAL: guix-daemon started and no socket appeared in ${i}s. Its log:" >&2
    cat /tmp/guix-daemon.log >&2
    exit 1
fi
echo "guix-daemon up, socket after ${i}s"
