#!/bin/sh
# Run one shell snippet until it succeeds, up to three times.
#
# The integration images install their managers from the network best-effort — `|| true`, so a
# manager that cannot install does not fail the build, the image comes up without it, and the
# coverage audit records the gap honestly. One attempt makes that trade badly. `get-helm-3`
# failed once on a morning it had worked six hours earlier, printing nothing but `Failed to
# install helm`; `|| true` swallowed it, the image shipped without helm, and the only thing that
# noticed was the real-lifecycle ratchet going red over a manager nobody had touched.
#
# **Retrying does not weaken the trade.** A manager that genuinely cannot install fails all three
# attempts, still degrades to `|| true`, and the ratchet still catches the lost coverage. The
# only thing that changes is that a blip stops costing a nightly.
#
# Deliberately not applied to the distro's own `apt-get`/`apk`/`pacman` layers. Those are not
# best-effort: they fail the build loudly, which is a re-run rather than a silent hole, and
# wrapping every RUN in a retry is how a build that is genuinely broken takes three times as
# long to say so.
#
# The snippet is one argument, run under `sh -c`, so pipelines and `&&` chains work unchanged:
#
#     RUN retry 'curl -fsSL https://example.invalid/install | bash' || true
set -u

ATTEMPTS=3
DELAY=5

if [ "$#" -eq 0 ]; then
    echo "retry: nothing to run" >&2
    exit 2
fi

n=1
while :; do
    sh -c "$*" && exit 0
    status=$?
    if [ "$n" -ge "$ATTEMPTS" ]; then
        echo "retry: giving up after $n attempts (exit $status): $*" >&2
        exit "$status"
    fi
    echo "retry: attempt $n of $ATTEMPTS failed (exit $status), ${DELAY}s and again: $*" >&2
    sleep "$DELAY"
    n=$((n + 1))
    DELAY=$((DELAY * 2))
done
