#!/bin/sh
# LiNix bootstrap installer — the 30-second first run.
#
#   curl -fsSL https://raw.githubusercontent.com/SYKhayyat/LiNix/HEAD/scripts/install.sh | sh
#
# It installs the `linix` binary, runs a health check, and offers to adopt the packages
# already on this machine into a LiNix manifest. Override defaults with env vars:
#   LINIX_REPO      git source           (default: the SYKhayyat/LiNix repo)
#   LINIX_REF       tag or branch        (default: the newest release tag)
#   LINIX_BIN_DIR   install location     (default: cargo's bin dir)
#   LINIX_NO_ADOPT  set to skip the `adopt` prompt
#
# Every name in that list is read below. It documented `LINIX_BIN_DIR`, which nothing read, and
# omitted `LINIX_REF`, which everything did — in the file users pipe from the internet, where the
# list is the only interface anyone sees.
set -eu

REPO="${LINIX_REPO:-https://github.com/SYKhayyat/LiNix}"
BIN_DIR="${LINIX_BIN_DIR:-}"

say() { printf '\033[1;36mlinix\033[0m %s\n' "$1"; }
err() { printf '\033[1;31mlinix\033[0m %s\n' "$1" >&2; }

say "bootstrapping — detecting toolchain..."

if ! command -v cargo >/dev/null 2>&1; then
  err "Rust/cargo not found."
  err "Install Rust from https://rustup.rs and re-run this script."
  exit 1
fi

# WHICH LiNix. `HEAD` is whatever was pushed last, which is not a thing anyone can ask for
# twice — two machines installed an hour apart got different programs and neither could say
# which. The default is the newest release TAG, and `LINIX_REF` overrides it:
#
#   LINIX_REF=main   ...install.sh | sh     # follow the branch, deliberately
#   LINIX_REF=v0.7.0 ...install.sh | sh     # a specific release
#
# A repo with no tags yet falls back to the branch and SAYS SO, rather than silently
# installing something else than it promised.
REF="${LINIX_REF:-}"
if [ -z "$REF" ]; then
  REF="$(git ls-remote --tags --refs --sort=-v:refname "$REPO" 'v*' 2>/dev/null            | head -1 | sed 's#.*/##')"
  if [ -z "$REF" ]; then
    say "no release tag published yet — installing from the default branch instead."
  fi
fi

if [ -n "$REF" ]; then
  say "building and installing $REF from $REPO (this can take a minute)..."
else
  say "building and installing from $REPO (this can take a minute)..."
fi

# `--locked`, and no fallback. This was a `--locked` attempt with its stderr sent to
# /dev/null and, on *any* non-zero exit, a second run without it — described in the comment as
# "fall back if the lock is unavailable". `Cargo.lock` is tracked in this repository, so the
# case the fallback named cannot happen; what it actually caught was a network blip or a
# compile error, and its response was to resolve 448 dependencies fresh, with the reason
# hidden. That is a supply-chain downgrade triggered by bad wifi, in the script a user pipes
# from the web.
#
# `--tag` only when there is one: `cargo install --git X --tag ""` is not the same command.
#
# `--root` when the caller named a directory. cargo installs into `$root/bin`, so a
# `LINIX_BIN_DIR` of `/usr/local/bin` is a root of `/usr/local` — computed here rather than
# demanded of the user, who was told this variable names the install location.
set -- --git "$REPO" --locked
# An `if`, not `[ -n "$REF" ] && set -- …`: under `set -e` a trailing `&&` list whose test fails
# is a failing command, so the no-tag path would have exited here.
if [ -n "$REF" ]; then
  set -- "$@" --tag "$REF"
fi
if [ -n "$BIN_DIR" ]; then
  case "$BIN_DIR" in
    */bin) ROOT="${BIN_DIR%/bin}" ;;
    # Any other directory: cargo cannot be told to use it directly, so install under a root
    # beside it and move the binary. Saying nothing and installing somewhere else would be the
    # variable documented-but-unread all over again.
    *) ROOT="" ;;
  esac
  if [ -n "$ROOT" ]; then
    cargo install "$@" --root "$ROOT"
    CARGO_BIN="$BIN_DIR"
  else
    STAGE="$(mktemp -d)"
    cargo install "$@" --root "$STAGE"
    mkdir -p "$BIN_DIR"
    cp "$STAGE/bin/linix" "$BIN_DIR/linix"
    chmod 755 "$BIN_DIR/linix"
    rm -rf "$STAGE"
    CARGO_BIN="$BIN_DIR"
    say "installed to $BIN_DIR (LINIX_BIN_DIR)"
  fi
else
  cargo install "$@"
  # cargo installs into ~/.cargo/bin; make sure the user can find it.
  CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
fi
# The shell caches where it found a name. Upgrading over an older `linix` on PATH leaves the
# cache pointing at the binary that was just replaced, and every line below would then run
# the old one — including the health check that is supposed to vouch for the new.
hash -r 2>/dev/null || true
if ! command -v linix >/dev/null 2>&1; then
  case ":$PATH:" in
    *":$CARGO_BIN:"*) : ;;
    *) err "Add $CARGO_BIN to your PATH to use \`linix\`." ;;
  esac
fi

# The binary just installed, by path, in preference to whatever `linix` resolves to on this
# PATH — that could be an older install elsewhere, and the health check is supposed to vouch
# for the one this script produced.
if [ -x "$CARGO_BIN/linix" ]; then
  LINIX="$CARGO_BIN/linix"
else
  LINIX="$(command -v linix || echo "$CARGO_BIN/linix")"
fi

say "running health check..."
"$LINIX" check health || true

if [ -z "${LINIX_NO_ADOPT:-}" ]; then
  printf '\033[1;36mlinix\033[0m adopt the packages already installed on this machine into a manifest now? [y/N] '
  # Read from the terminal even when the script itself arrived over a pipe.
  if [ -r /dev/tty ]; then read -r ans </dev/tty; else read -r ans || ans=n; fi
  case "$ans" in
    y | Y | yes | YES) "$LINIX" adopt ;;
    *) say "skipped — run \`linix adopt\` whenever you're ready." ;;
  esac
fi

say "done. Try \`linix check\` or \`linix sync\`."
