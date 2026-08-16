#!/bin/sh
# Is the Nix that Shall GENERATES valid Nix? Asked of nix, not of a reviewer.
#
# **Why this exists as its own gate.** `backends/nixos.rs` writes a NixOS module into the user's
# `/etc/nixos`. That file is read by nix and by nothing Shall controls, so a mistake in it does
# not break Shall — it breaks the machine's entire system configuration, and the user finds out
# from `nixos-rebuild`, not from us. No Rust test can catch that: a Rust test suite cannot parse
# Nix. So the renderer writes every shape it can produce into `target/nix-fixtures/` (see
# `every_rendered_shape_is_written_for_nix_to_parse`) and this asks a real parser about them.
#
# `nixos/nix` rather than a NixOS image, and the difference is the point: **no container
# available is NixOS.** That image is the Nix package manager on a minimal base — measured, it
# has `/nix/store` and `nix`, and has no `/etc/NIXOS`, no `nixos-rebuild` and no systemd. It can
# therefore answer "is this valid Nix", which is the risk worth money, and cannot answer "does
# this system rebuild", which stays argv-checked and is recorded as such in `J5`.
#
#   ./scripts/nix-validate.sh              - parse every fixture
#   ./scripts/nix-validate.sh --self-test  - and prove the check can fail
#
# Reads the exit code directly and never through a pipe, for the reason `unix-check.sh` gives at
# length: `rc=$?` after a pipe reads the pipe.
set -u

IMAGE="${SHALL_NIX_IMAGE:-nixos/nix:latest}"
ROOT="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
FIXTURES="$ROOT/target/nix-fixtures"

# Git Bash rewrites anything that looks like a path into nonsense before docker sees it, and the
# failure arrives as an `invalid mode` exit 125 one line before a message about something else.
MSYS_NO_PATHCONV=1
MSYS2_ARG_CONV_EXCL='*'
export MSYS_NO_PATHCONV MSYS2_ARG_CONV_EXCL

DOCKER="docker"
command -v docker >/dev/null 2>&1 || DOCKER="wsl -- docker"

# A path WSL's docker can mount, derived exactly as `unix-check.sh` derives it: `wsl -- docker`
# is the daemon inside WSL and reads `/mnt/c/...`, while Git Bash hands out `/c/...` and cmd
# hands out a drive letter. **Getting this wrong does not error** — the mount is simply empty,
# the glob below matches nothing, and the first version of this script then "validated" the
# unexpanded pattern as though it were a filename.
case "$DOCKER" in
wsl*)
    case "$FIXTURES" in
    /mnt/*) ;;
    /[A-Za-z]/*) FIXTURES="/mnt$FIXTURES" ;;
    [A-Za-z]:*)
        _drive="$(printf '%s' "$FIXTURES" | cut -c1 | tr 'A-Z' 'a-z')"
        _rest="$(printf '%s' "$FIXTURES" | cut -c3- | tr '\\' '/')"
        FIXTURES="/mnt/$_drive$_rest"
        ;;
    esac
    ;;
esac

if [ ! -d "$ROOT/target/nix-fixtures" ]; then
    echo "nix-validate: no fixtures at $ROOT/target/nix-fixtures" >&2
    echo "  They are written by the test suite. Run:" >&2
    echo "    cargo test --lib nixos" >&2
    exit 2
fi

$DOCKER version >/dev/null 2>&1 || {
    echo "nix-validate: no reachable docker daemon." >&2
    echo "  This is the only check that asks a real parser about the Nix Shall writes." >&2
    echo "  Skipping it is a said-so, not a done." >&2
    exit 2
}

echo "nix-validate: parsing the generated modules with $IMAGE"

# **The container script is a FILE, never a quoted command string.** A multi-line `sh -c '...'`
# does not survive Git Bash -> wsl.exe -> docker: the `$seen` and `$rc` in it arrive empty, and
# the loop reports `[: : integer expected` while the gate goes on to print `ok`. That is this
# repo's oldest container trap and it is silent every time.
INNER="$ROOT/target/nix-validate-inner.sh"
cat > "$INNER" <<'INNEREOF'
rc=0
seen=0
for f in /fixtures/*.nix; do
    [ -f "$f" ] || continue
    seen=$((seen + 1))
    if nix-instantiate --parse "$f" >/dev/null 2>/tmp/e; then
        echo "  ok      $f"
    else
        echo "  INVALID $f"
        cat /tmp/e
        rc=1
    fi
done
if [ "$seen" -eq 0 ]; then
    echo "  nothing reached the container - the mount is empty, so this checked nothing"
    rc=2
fi
echo "  parsed $seen module(s)"
exit $rc
INNEREOF

INNER_MOUNT="$INNER"
case "$DOCKER" in
wsl*)
    case "$INNER_MOUNT" in
    /mnt/*) ;;
    /[A-Za-z]/*) INNER_MOUNT="/mnt$INNER_MOUNT" ;;
    [A-Za-z]:*)
        _d="$(printf '%s' "$INNER_MOUNT" | cut -c1 | tr 'A-Z' 'a-z')"
        _r="$(printf '%s' "$INNER_MOUNT" | cut -c3- | tr '\' '/')"
        INNER_MOUNT="/mnt/$_d$_r"
        ;;
    esac
    ;;
esac

$DOCKER run --rm -v "$FIXTURES:/fixtures:ro" -v "$INNER_MOUNT:/parse.sh:ro" "$IMAGE" sh /parse.sh
rc=$?

if [ "$rc" -ne 0 ]; then
    echo "nix-validate: FAILED (rc=$rc) - Shall would write a file nix cannot read." >&2
    exit 1
fi

for a in "$@"; do
    [ "$a" = "--self-test" ] || continue
    # **A gate nobody has watched fail is a gate nobody has tested.** An unbalanced module must
    # be refused; if this passes, everything above is decorative.
    echo "nix-validate: self-test — a deliberately broken module must be refused"
    $DOCKER run --rm "$IMAGE" sh -c '
        printf "{ pkgs, ... }:\n{ environment.systemPackages = with pkgs; [ ripgrep\n}\n" > /tmp/bad.nix
        nix-instantiate --parse /tmp/bad.nix >/dev/null 2>&1'
    if [ $? -eq 0 ]; then
        echo "  the self-test PASSED a broken module — this gate proves nothing." >&2
        exit 1
    fi
    echo "  ok — a broken module is refused, so a pass above means something"
done

echo "nix-validate: ok"
